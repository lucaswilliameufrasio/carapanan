//! Private sealed runtime bytes, not an execution capability. No discovery, mounts or process launch.

use super::{
    RuntimeFileRole, RuntimeManifestError, RuntimeManifestPolicy, VerifiedRuntimeManifest,
};
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
pub(super) mod transport;
#[cfg(target_os = "linux")]
pub(crate) use transport::receive_expected as receive_expected_image;
#[cfg(target_os = "linux")]
pub(crate) use transport::{receive_selected as receive_selected_image, validate_selected_files};

#[cfg(target_os = "linux")]
fn validate_selected_destination(path: &Path) -> Result<(), RuntimeManifestError> {
    super::validate_path(path)?;
    if !path.starts_with("/workspace") || path == Path::new("/workspace") {
        return Err(RuntimeManifestError::InvalidPath);
    }
    Ok(())
}

/// Private wire metadata only, never source paths or runtime payload serialization.
#[cfg(target_os = "linux")]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExpectedRuntimeFile {
    pub(crate) destination: PathBuf,
    pub(crate) role: RuntimeFileRole,
    pub(crate) size_bytes: u64,
    pub(crate) sha256: [u8; 32],
}

/// Immutable bytes with exact target metadata. Intentionally no Debug/Clone/serialization.
pub struct RuntimeImageFile {
    destination: PathBuf,
    role: RuntimeFileRole,
    size_bytes: u64,
    sha256: [u8; 32],
    #[cfg(target_os = "linux")]
    file: std::fs::File,
}

impl RuntimeImageFile {
    pub fn destination(&self) -> &Path {
        &self.destination
    }
    pub fn role(&self) -> RuntimeFileRole {
        self.role
    }
    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }
    pub fn sha256(&self) -> &[u8; 32] {
        &self.sha256
    }
    /// Sealed bytes only. CLOEXEC remains set; the caller must explicitly map approved transports.
    /// This does not guarantee executable mode or authorize a mount/command.
    #[cfg(target_os = "linux")]
    pub fn descriptor(&self) -> std::os::fd::BorrowedFd<'_> {
        use std::os::fd::AsFd;
        self.file.as_fd()
    }
}

/// Ephemeral kernel-sealed files; no paths, cache, persistence or automatic descriptor inheritance.
/// Memory/shmem is bounded per image, not globally reserved or charged to the target cgroup.
///
/// ```compile_fail
/// fn log(image: &carapana_sandbox::runtime_manifest::RuntimeImage) {
///     let _ = format!("{image:?}");
/// }
/// ```
pub struct RuntimeImage {
    files: Vec<RuntimeImageFile>,
    total_bytes: u64,
}

impl RuntimeImage {
    #[cfg(target_os = "linux")]
    pub(crate) fn empty_selected() -> Self {
        Self {
            files: Vec::new(),
            total_bytes: 0,
        }
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn push_selected(
        &mut self,
        path: PathBuf,
        bytes: &[u8],
        sha256: [u8; 32],
    ) -> Result<(), RuntimeManifestError> {
        let destination = Path::new("/workspace").join(path);
        validate_selected_destination(&destination)?;
        if self.files.len() >= 256
            || bytes.len() > carapana_storage::MAX_WORKSPACE_FILE_READ_BYTES
            || self.total_bytes + bytes.len() as u64 > 16 * 1024 * 1024
        {
            return Err(RuntimeManifestError::ByteBudgetExceeded);
        }
        let file = linux::seal_selected_bytes(bytes, &sha256)?;
        self.total_bytes += bytes.len() as u64;
        self.files.push(RuntimeImageFile {
            destination,
            role: RuntimeFileRole::Support,
            size_bytes: bytes.len() as u64,
            sha256,
            file,
        });
        Ok(())
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn send_selected_descriptors(
        &self,
        socket: &std::os::unix::net::UnixDatagram,
        deadline: std::time::Instant,
    ) -> Result<(), RuntimeManifestError> {
        transport::send_selected(self, socket, deadline)
    }

    /// Send this image's descriptors only, without paths or implicit inheritance, over a private pair.
    #[cfg(target_os = "linux")]
    pub fn send_descriptors(
        &self,
        socket: &std::os::unix::net::UnixDatagram,
        deadline: std::time::Instant,
    ) -> Result<(), RuntimeManifestError> {
        transport::send(self, socket, deadline)
    }

    pub fn files(&self) -> impl ExactSizeIterator<Item = &RuntimeImageFile> {
        self.files.iter()
    }
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }
}

pub(super) fn freeze(
    manifest: VerifiedRuntimeManifest,
    policy: RuntimeManifestPolicy,
) -> Result<RuntimeImage, RuntimeManifestError> {
    #[cfg(target_os = "linux")]
    {
        linux::freeze(manifest, policy)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (manifest, policy);
        Err(RuntimeManifestError::UnsupportedPlatform)
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::runtime_manifest::linux::{Signature, verify_bytes};
    use rustix::fs::{MemfdFlags, SealFlags, fcntl_add_seals, fcntl_get_seals, memfd_create};
    use std::{fs::File, io::Write, os::unix::fs::FileExt};

    fn seals() -> SealFlags {
        SealFlags::WRITE | SealFlags::GROW | SealFlags::SHRINK | SealFlags::SEAL
    }

    // Use positional reads: never depend on or change a shared descriptor offset. The copied object
    // is sealed before hashing, so a source mutation after this point cannot change mount bytes.
    fn copy_sealed(
        source: &File,
        signature: &Signature,
        size: u64,
        expected: &[u8; 32],
        limit: u64,
    ) -> Result<File, RuntimeManifestError> {
        if size > limit {
            return Err(RuntimeManifestError::ByteBudgetExceeded);
        }
        if Signature::capture(source)? != *signature {
            return Err(RuntimeManifestError::SourceChanged);
        }
        let mut file = File::from(
            memfd_create(
                "carapana-runtime",
                MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING,
            )
            .map_err(|_| RuntimeManifestError::ImageUnavailable)?,
        );
        let mut offset = 0_u64;
        let mut chunk = [0_u8; 64 * 1024];
        loop {
            let requested = chunk.len().min((size - offset + 1) as usize);
            let count = match source.read_at(&mut chunk[..requested], offset) {
                Ok(count) => count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(RuntimeManifestError::SourceUnavailable),
            };
            if count == 0 {
                break;
            }
            offset += count as u64;
            if offset > size {
                return Err(RuntimeManifestError::SourceChanged);
            }
            file.write_all(&chunk[..count])
                .map_err(|_| RuntimeManifestError::ImageUnavailable)?;
        }
        if offset != size || Signature::capture(source)? != *signature {
            return Err(RuntimeManifestError::SourceChanged);
        }
        finish_sealing(file, expected, limit)
    }

    pub(super) fn seal_selected_bytes(
        bytes: &[u8],
        expected: &[u8; 32],
    ) -> Result<File, RuntimeManifestError> {
        let mut file = File::from(
            memfd_create(
                "carapana-selected-snapshot",
                MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING,
            )
            .map_err(|_| RuntimeManifestError::ImageUnavailable)?,
        );
        file.write_all(bytes)
            .map_err(|_| RuntimeManifestError::ImageUnavailable)?;
        finish_sealing(file, expected, bytes.len() as u64)
    }

    fn finish_sealing(
        file: File,
        expected: &[u8; 32],
        limit: u64,
    ) -> Result<File, RuntimeManifestError> {
        fcntl_add_seals(&file, seals()).map_err(|_| RuntimeManifestError::ImageSealRejected)?;
        if !fcntl_get_seals(&file)
            .map_err(|_| RuntimeManifestError::ImageSealRejected)?
            .contains(seals())
        {
            return Err(RuntimeManifestError::ImageSealRejected);
        }
        verify_bytes(&file, &Signature::capture(&file)?, expected, limit)?;
        Ok(file)
    }

    pub(super) fn freeze(
        manifest: VerifiedRuntimeManifest,
        policy: RuntimeManifestPolicy,
    ) -> Result<RuntimeImage, RuntimeManifestError> {
        // Check the complete budget before allocating any shmem, not after a partial image exists.
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
        manifest.revalidate()?;
        let mut files = Vec::with_capacity(manifest.files.len());
        let mut total_bytes = 0;
        for entry in manifest.files {
            let file = copy_sealed(
                &entry.file,
                &entry.signature,
                entry.size_bytes,
                &entry.spec.expected_sha256,
                policy
                    .max_file_bytes()
                    .min(policy.max_total_bytes() - total_bytes),
            )?;
            total_bytes += entry.size_bytes;
            files.push(RuntimeImageFile {
                destination: entry.spec.destination,
                role: entry.spec.role,
                sha256: entry.spec.expected_sha256,
                size_bytes: entry.size_bytes,
                file,
            });
        }
        Ok(RuntimeImage { files, total_bytes })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use sha2::{Digest, Sha256};

        fn source(bytes: &[u8]) -> File {
            let mut file =
                File::from(memfd_create("synthetic-source", MemfdFlags::CLOEXEC).unwrap());
            file.write_all(bytes).unwrap();
            file
        }

        #[test]
        fn should_seal_a_copy_against_writes_truncation_growth_and_later_source_changes() {
            let source = source(b"synthetic runtime");
            let digest = Sha256::digest(b"synthetic runtime").into();
            let copy = copy_sealed(
                &source,
                &Signature::capture(&source).unwrap(),
                17,
                &digest,
                17,
            )
            .unwrap();
            assert!(copy.write_at(b"x", 0).is_err());
            assert!(copy.set_len(0).is_err());
            assert!(copy.set_len(18).is_err());
            assert!(fcntl_get_seals(&copy).unwrap().contains(seals()));
            source.write_at(b"changed", 0).unwrap();
            let mut bytes = [0; 17];
            copy.read_exact_at(&mut bytes, 0).unwrap();
            assert_eq!(&bytes, b"synthetic runtime");
            use rustix::io::{FdFlags, fcntl_getfd};
            assert!(fcntl_getfd(&copy).unwrap().contains(FdFlags::CLOEXEC));
        }

        #[test]
        fn should_reject_wrong_digest_changed_identity_and_insufficient_copy_budget() {
            let source = source(b"synthetic");
            let signature = Signature::capture(&source).unwrap();
            assert!(matches!(
                copy_sealed(&source, &signature, 9, &[0; 32], 9),
                Err(RuntimeManifestError::HashMismatch)
            ));
            assert!(matches!(
                copy_sealed(&source, &signature, 9, &[0; 32], 8),
                Err(RuntimeManifestError::ByteBudgetExceeded)
            ));
            source.set_len(0).unwrap();
            assert!(matches!(
                copy_sealed(&source, &signature, 9, &[0; 32], 9),
                Err(RuntimeManifestError::SourceChanged)
            ));
        }

        #[test]
        fn should_copy_empty_and_multi_chunk_images_without_using_the_source_offset() {
            for bytes in [Vec::new(), vec![0xa5; 2 * 64 * 1024 + 1]] {
                let mut source = source(&bytes);
                use std::io::{Seek, SeekFrom};
                source.seek(SeekFrom::End(0)).unwrap();
                let signature = Signature::capture(&source).unwrap();
                let copy = copy_sealed(
                    &source,
                    &signature,
                    bytes.len() as u64,
                    &Sha256::digest(&bytes).into(),
                    bytes.len() as u64,
                )
                .unwrap();
                let mut actual = vec![0; bytes.len()];
                copy.read_exact_at(&mut actual, 0).unwrap();
                assert_eq!(actual, bytes);
                assert_eq!(source.stream_position().unwrap(), bytes.len() as u64);
            }
        }

        #[test]
        fn should_reject_short_or_growing_copy_lengths_before_returning_an_image() {
            let source = source(b"synthetic");
            let signature = Signature::capture(&source).unwrap();
            for size in [8, 10] {
                assert!(matches!(
                    copy_sealed(&source, &signature, size, &[0; 32], 10),
                    Err(RuntimeManifestError::SourceChanged)
                ));
            }
        }
    }
}
