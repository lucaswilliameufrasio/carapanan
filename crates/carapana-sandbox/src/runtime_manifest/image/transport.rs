//! One bounded SCM_RIGHTS datagram over an already-private socketpair. No listener or spawning.

use super::*;
use crate::runtime_manifest::linux::{Signature, verify_bytes};
use rustix::{
    fs::{SealFlags, fcntl_get_seals},
    net::{
        RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags, SendAncillaryBuffer,
        SendAncillaryMessage, SendFlags, recvmsg, sendmsg,
    },
};
use std::{
    fs::File,
    io::{IoSlice, IoSliceMut},
    mem::MaybeUninit,
    os::unix::net::UnixDatagram,
    thread,
    time::{Duration, Instant},
};

const MAX_FILES: usize = 128;
const HEADER: &[u8] = b"carapana-runtime-fds-v1";

fn wait(deadline: Instant) -> Result<(), RuntimeManifestError> {
    if Instant::now() >= deadline {
        return Err(RuntimeManifestError::ImageTransportTimedOut);
    }
    thread::sleep(Duration::from_millis(5).min(deadline.saturating_duration_since(Instant::now())));
    Ok(())
}

pub(super) fn send(
    image: &RuntimeImage,
    socket: &UnixDatagram,
    deadline: Instant,
) -> Result<(), RuntimeManifestError> {
    if image.files.is_empty() || image.files.len() > MAX_FILES {
        return Err(RuntimeManifestError::ImageTransportProtocol);
    }
    let descriptors: Vec<_> = image
        .files
        .iter()
        .map(RuntimeImageFile::descriptor)
        .collect();
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(MAX_FILES))];
    let mut ancillary = SendAncillaryBuffer::new(&mut space);
    if !ancillary.push(SendAncillaryMessage::ScmRights(&descriptors)) {
        return Err(RuntimeManifestError::ImageTransportProtocol);
    }
    loop {
        if Instant::now() >= deadline {
            return Err(RuntimeManifestError::ImageTransportTimedOut);
        }
        match sendmsg(
            socket,
            &[IoSlice::new(HEADER)],
            &mut ancillary,
            SendFlags::DONTWAIT | SendFlags::NOSIGNAL,
        ) {
            Ok(count) if count == HEADER.len() => return Ok(()),
            Ok(_) => return Err(RuntimeManifestError::ImageTransportProtocol),
            Err(rustix::io::Errno::INTR) => continue,
            Err(rustix::io::Errno::AGAIN) => wait(deadline)?,
            Err(_) => return Err(RuntimeManifestError::ImageTransportUnavailable),
        }
    }
}

pub(in crate::runtime_manifest) fn receive(
    manifest: VerifiedRuntimeManifest,
    socket: &UnixDatagram,
    policy: RuntimeManifestPolicy,
    deadline: Instant,
) -> Result<RuntimeImage, RuntimeManifestError> {
    if manifest.files.len() > policy.max_files() {
        return Err(RuntimeManifestError::TooManyFiles);
    }
    if manifest.total_bytes > policy.max_total_bytes()
        || manifest
            .files
            .iter()
            .any(|file| file.size_bytes > policy.max_file_bytes())
    {
        return Err(RuntimeManifestError::ByteBudgetExceeded);
    }
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(MAX_FILES))];
    let mut ancillary = RecvAncillaryBuffer::new(&mut space);
    let mut header = [0_u8; HEADER.len()];
    let message = loop {
        if Instant::now() >= deadline {
            return Err(RuntimeManifestError::ImageTransportTimedOut);
        }
        match recvmsg(
            socket,
            &mut [IoSliceMut::new(&mut header)],
            &mut ancillary,
            RecvFlags::DONTWAIT | RecvFlags::CMSG_CLOEXEC,
        ) {
            Ok(message) => break message,
            Err(rustix::io::Errno::INTR) => continue,
            Err(rustix::io::Errno::AGAIN) => wait(deadline)?,
            Err(_) => return Err(RuntimeManifestError::ImageTransportUnavailable),
        }
    };
    // Ancillary buffer/drain own received FDs: early returns close rights, even on truncation.
    if message.bytes != HEADER.len()
        || header != HEADER
        || message
            .flags
            .intersects(ReturnFlags::TRUNC | ReturnFlags::CTRUNC)
    {
        return Err(RuntimeManifestError::ImageTransportProtocol);
    }
    let mut descriptors = Vec::new();
    for message in ancillary.drain() {
        match message {
            RecvAncillaryMessage::ScmRights(rights) => {
                for descriptor in rights {
                    descriptors.push(descriptor);
                }
            }
            _ => return Err(RuntimeManifestError::ImageTransportProtocol),
        }
    }
    if descriptors.len() != manifest.files.len() {
        return Err(RuntimeManifestError::ImageTransportProtocol);
    }
    let required = SealFlags::WRITE | SealFlags::GROW | SealFlags::SHRINK | SealFlags::SEAL;
    let mut files = Vec::with_capacity(descriptors.len());
    let mut total_bytes = 0;
    for (descriptor, expected) in descriptors.into_iter().zip(manifest.files) {
        let file = File::from(descriptor);
        let metadata = file
            .metadata()
            .map_err(|_| RuntimeManifestError::ImageTransportProtocol)?;
        if !metadata.is_file()
            || !fcntl_get_seals(&file)
                .map_err(|_| RuntimeManifestError::ImageSealRejected)?
                .contains(required)
        {
            return Err(RuntimeManifestError::ImageSealRejected);
        }
        if metadata.len() != expected.size_bytes {
            return Err(RuntimeManifestError::SourceChanged);
        }
        verify_bytes(
            &file,
            &Signature::capture(&file)?,
            &expected.spec.expected_sha256,
            policy
                .max_file_bytes()
                .min(policy.max_total_bytes() - total_bytes),
        )?;
        total_bytes += expected.size_bytes;
        files.push(RuntimeImageFile {
            destination: expected.spec.destination,
            role: expected.spec.role,
            size_bytes: expected.size_bytes,
            sha256: expected.spec.expected_sha256,
            file,
        });
    }
    Ok(RuntimeImage { files, total_bytes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_manifest::{RuntimeFileRole, RuntimeFileSpec};
    use rustix::fs::{MemfdFlags, memfd_create};
    use sha2::{Digest, Sha256};
    use std::{fs, io::Write, os::unix::fs::FileExt};

    fn policy() -> RuntimeManifestPolicy {
        RuntimeManifestPolicy::new(2, 1024 * 1024, 2 * 1024 * 1024).unwrap()
    }
    fn manifest() -> VerifiedRuntimeManifest {
        let path = fs::canonicalize("/usr/bin/true").unwrap();
        let hash = Sha256::digest(fs::read(&path).unwrap()).into();
        VerifiedRuntimeManifest::verify(
            vec![
                RuntimeFileSpec::new(
                    path,
                    PathBuf::from("/runtime/true"),
                    RuntimeFileRole::Executable,
                    hash,
                )
                .unwrap(),
            ],
            policy(),
        )
        .unwrap()
    }
    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(5)
    }

    #[test]
    fn should_receive_sealed_descriptors_with_cloexec_and_exact_expected_bytes() {
        let image = manifest().freeze(policy()).unwrap();
        let (sender, receiver) = UnixDatagram::pair().unwrap();
        image.send_descriptors(&sender, deadline()).unwrap();
        let received = manifest()
            .receive_image(&receiver, policy(), deadline())
            .unwrap();
        drop(image);
        assert_eq!(received.files().len(), 1);
        let file = received.files().next().unwrap();
        assert_eq!(file.destination(), Path::new("/runtime/true"));
        assert!(
            rustix::io::fcntl_getfd(file.descriptor())
                .unwrap()
                .contains(rustix::io::FdFlags::CLOEXEC)
        );
        assert!(rustix::io::pwrite(file.descriptor(), b"x", 0).is_err());
        let mut bytes = vec![0; file.size_bytes() as usize];
        file.file.read_exact_at(&mut bytes, 0).unwrap();
        assert_eq!(&Sha256::digest(bytes)[..], file.sha256());
    }

    #[test]
    fn should_reject_unsealed_rights_and_malformed_or_missing_rights() {
        let (sender, receiver) = UnixDatagram::pair().unwrap();
        let file = File::from(
            memfd_create("unsealed", MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING).unwrap(),
        );
        use std::os::fd::AsFd;
        let fds = [file.as_fd()];
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut ancillary = SendAncillaryBuffer::new(&mut space);
        assert!(ancillary.push(SendAncillaryMessage::ScmRights(&fds)));
        sendmsg(
            &sender,
            &[IoSlice::new(HEADER)],
            &mut ancillary,
            SendFlags::empty(),
        )
        .unwrap();
        assert!(matches!(
            manifest().receive_image(&receiver, policy(), deadline()),
            Err(RuntimeManifestError::ImageSealRejected)
        ));
        for bytes in [HEADER, b"malformed", b"carapana-runtime-fds-v1-extra"] {
            sender.send(bytes).unwrap();
            assert!(matches!(
                manifest().receive_image(&receiver, policy(), deadline()),
                Err(RuntimeManifestError::ImageTransportProtocol)
            ));
        }
    }

    #[test]
    fn should_reject_sealed_bytes_that_do_not_match_the_expected_manifest() {
        let mut image = manifest().freeze(policy()).unwrap();
        let mut wrong = File::from(
            memfd_create(
                "wrong-sealed",
                MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING,
            )
            .unwrap(),
        );
        wrong
            .write_all(&vec![0; image.total_bytes() as usize])
            .unwrap();
        rustix::fs::fcntl_add_seals(
            &wrong,
            SealFlags::WRITE | SealFlags::GROW | SealFlags::SHRINK | SealFlags::SEAL,
        )
        .unwrap();
        image.files[0].file = wrong;
        let (sender, receiver) = UnixDatagram::pair().unwrap();
        image.send_descriptors(&sender, deadline()).unwrap();
        assert!(matches!(
            manifest().receive_image(&receiver, policy(), deadline()),
            Err(RuntimeManifestError::HashMismatch)
        ));
    }

    #[test]
    fn should_honor_explicit_send_and_receive_deadlines_without_changing_socket_flags() {
        let (sender, receiver) = UnixDatagram::pair().unwrap();
        assert!(matches!(
            manifest().receive_image(
                &receiver,
                policy(),
                Instant::now() + Duration::from_millis(20)
            ),
            Err(RuntimeManifestError::ImageTransportTimedOut)
        ));
        assert!(matches!(
            manifest()
                .freeze(policy())
                .unwrap()
                .send_descriptors(&sender, Instant::now()),
            Err(RuntimeManifestError::ImageTransportTimedOut)
        ));
        assert!(
            !rustix::fs::fcntl_getfl(&sender)
                .unwrap()
                .contains(rustix::fs::OFlags::NONBLOCK)
        );
    }

    #[test]
    fn should_close_all_received_rights_on_count_mismatch_and_ancillary_truncation() {
        use std::{
            io::Read,
            os::{fd::AsFd, unix::net::UnixStream},
        };
        for count in [2, MAX_FILES + 1] {
            let (sender, receiver) = UnixDatagram::pair().unwrap();
            let (writer, mut reader) = UnixStream::pair().unwrap();
            reader.set_nonblocking(true).unwrap();
            let fds = vec![writer.as_fd(); count];
            let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(MAX_FILES + 1))];
            let mut ancillary = SendAncillaryBuffer::new(&mut space);
            assert!(ancillary.push(SendAncillaryMessage::ScmRights(&fds)));
            sendmsg(
                &sender,
                &[IoSlice::new(HEADER)],
                &mut ancillary,
                SendFlags::empty(),
            )
            .unwrap();
            assert!(matches!(
                manifest().receive_image(&receiver, policy(), deadline()),
                Err(RuntimeManifestError::ImageTransportProtocol)
            ));
            drop(fds);
            drop(writer);
            // Any leaked received right would keep this endpoint alive and yield WouldBlock.
            assert_eq!(reader.read(&mut [0_u8; 1]).unwrap(), 0);
        }
    }

    #[test]
    fn should_validate_receive_budget_before_consuming_any_datagram() {
        let image = manifest().freeze(policy()).unwrap();
        let (sender, receiver) = UnixDatagram::pair().unwrap();
        image.send_descriptors(&sender, deadline()).unwrap();
        assert!(matches!(
            manifest().receive_image(
                &receiver,
                RuntimeManifestPolicy::new(1, 1, 1).unwrap(),
                deadline()
            ),
            Err(RuntimeManifestError::ByteBudgetExceeded)
        ));
        assert!(
            manifest()
                .receive_image(&receiver, policy(), deadline())
                .is_ok()
        );
    }

    #[test]
    fn should_transfer_the_image_to_an_independent_process_without_inheriting_runtime_fds() {
        // Re-exec this exact fixture with only the private socket mapped to stdin. Runtime FDs are
        // CLOEXEC and must arrive through SCM_RIGHTS, not ambient descriptor inheritance.
        const CHILD: &str = "CARAPANA_SYNTHETIC_RUNTIME_RECEIVER";
        if std::env::var_os(CHILD).is_some() {
            let socket = UnixDatagram::from(rustix::io::dup(std::io::stdin()).unwrap());
            let image = manifest()
                .receive_image(&socket, policy(), deadline())
                .unwrap();
            assert_eq!(image.files().len(), 1);
            let entry = image.files().next().unwrap();
            assert!(
                fcntl_get_seals(entry.descriptor())
                    .unwrap()
                    .contains(SealFlags::WRITE)
            );
            return;
        }
        let (sender, receiver) = UnixDatagram::pair().unwrap();
        let image = manifest().freeze(policy()).unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "runtime_manifest::image::transport::tests::should_transfer_the_image_to_an_independent_process_without_inheriting_runtime_fds"])
            .env_clear().env(CHILD, "1").stdin(std::process::Stdio::from(std::os::fd::OwnedFd::from(receiver)))
            .stdout(std::process::Stdio::null()).spawn().unwrap();
        image.send_descriptors(&sender, deadline()).unwrap();
        let until = deadline();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            if Instant::now() >= until {
                let _ = child.kill();
                let _ = child.wait();
                panic!("synthetic image receiver exceeded deadline");
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}
