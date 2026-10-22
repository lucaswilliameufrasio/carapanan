//! One private helper socket transferred only after real guardian admission. No arbitrary fd list.

use super::ProcessError;
use rustix::net::{
    RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags, SendAncillaryBuffer,
    SendAncillaryMessage, SendFlags, recvmsg, sendmsg,
};
use std::{
    io::{IoSlice, IoSliceMut},
    mem::MaybeUninit,
    os::fd::{AsFd, OwnedFd},
    thread,
    time::{Duration, Instant},
};

const TAG: &[u8] = b"I";

fn pause(deadline: Instant) {
    thread::sleep(Duration::from_millis(5).min(deadline.saturating_duration_since(Instant::now())));
}

pub(super) fn send(
    control: &impl AsFd,
    input: &impl AsFd,
    deadline: Instant,
) -> Result<(), ProcessError> {
    let fds = [input.as_fd()];
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut ancillary = SendAncillaryBuffer::new(&mut space);
    if !ancillary.push(SendAncillaryMessage::ScmRights(&fds)) {
        return Err(ProcessError::HostProtocol);
    }
    loop {
        if Instant::now() >= deadline {
            return Err(ProcessError::HostTimedOut);
        }
        match sendmsg(
            control,
            &[IoSlice::new(TAG)],
            &mut ancillary,
            SendFlags::DONTWAIT | SendFlags::NOSIGNAL,
        ) {
            Ok(1) => return Ok(()),
            Ok(_) => return Err(ProcessError::HostProtocol),
            Err(rustix::io::Errno::INTR) => continue,
            Err(rustix::io::Errno::AGAIN) => pause(deadline),
            Err(_) => return Err(ProcessError::HostProtocol),
        }
    }
}

pub(super) fn receive(control: &impl AsFd, deadline: Instant) -> Result<OwnedFd, ProcessError> {
    let mut bytes = [0_u8; 1];
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut ancillary = RecvAncillaryBuffer::new(&mut space);
    let result = loop {
        if Instant::now() >= deadline {
            return Err(ProcessError::HostTimedOut);
        }
        match recvmsg(
            control,
            &mut [IoSliceMut::new(&mut bytes)],
            &mut ancillary,
            RecvFlags::DONTWAIT | RecvFlags::CMSG_CLOEXEC,
        ) {
            Ok(result) => break result,
            Err(rustix::io::Errno::INTR) => continue,
            Err(rustix::io::Errno::AGAIN) => pause(deadline),
            Err(_) => return Err(ProcessError::HostProtocol),
        }
    };
    if result.bytes != 1
        || bytes != TAG
        || result
            .flags
            .intersects(ReturnFlags::TRUNC | ReturnFlags::CTRUNC)
    {
        return Err(ProcessError::HostProtocol);
    }
    let mut descriptor = None;
    for message in ancillary.drain() {
        match message {
            RecvAncillaryMessage::ScmRights(rights) => {
                for fd in rights {
                    if descriptor.is_some() {
                        return Err(ProcessError::HostProtocol);
                    }
                    descriptor = Some(fd);
                }
            }
            _ => return Err(ProcessError::HostProtocol),
        }
    }
    let descriptor = descriptor.ok_or(ProcessError::HostProtocol)?;
    if rustix::net::sockopt::socket_type(&descriptor).map_err(|_| ProcessError::HostProtocol)?
        != rustix::net::SocketType::DGRAM
    {
        return Err(ProcessError::HostProtocol);
    }
    Ok(descriptor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::{UnixDatagram, UnixStream};

    #[test]
    fn should_transfer_only_a_datagram_socket_with_atomic_cloexec() {
        let (sender, receiver) = UnixStream::pair().unwrap();
        let (input, peer) = UnixDatagram::pair().unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        send(&sender, &input, deadline).unwrap();
        let fd = receive(&receiver, deadline).unwrap();
        assert!(
            rustix::io::fcntl_getfd(&fd)
                .unwrap()
                .contains(rustix::io::FdFlags::CLOEXEC)
        );
        let input = UnixDatagram::from(fd);
        peer.send(b"synthetic").unwrap();
        let mut bytes = [0_u8; 16];
        assert_eq!(input.recv(&mut bytes).unwrap(), 9);
        assert_eq!(&bytes[..9], b"synthetic");
    }

    #[test]
    fn should_reject_a_non_socket_descriptor_instead_of_mapping_it_to_the_helper() {
        let (sender, receiver) = UnixStream::pair().unwrap();
        let file = std::fs::File::open("/dev/null").unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        send(&sender, &file, deadline).unwrap();
        assert!(matches!(
            receive(&receiver, deadline),
            Err(ProcessError::HostProtocol)
        ));
    }

    #[test]
    fn should_stop_at_receive_deadline_and_reject_missing_rights() {
        use std::io::Write;
        let (mut sender, receiver) = UnixStream::pair().unwrap();
        assert!(matches!(
            receive(&receiver, Instant::now() + Duration::from_millis(20)),
            Err(ProcessError::HostTimedOut)
        ));
        sender.write_all(TAG).unwrap();
        assert!(matches!(
            receive(&receiver, Instant::now() + Duration::from_secs(1)),
            Err(ProcessError::HostProtocol)
        ));
    }
}
