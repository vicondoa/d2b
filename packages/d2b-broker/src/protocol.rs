use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::path::Path;

use nix::sys::socket::{
    AddressFamily, Backlog, MsgFlags, SockFlag, SockType, UnixAddr, bind, connect, listen, recv,
    send, socket,
};
use serde::{Serialize, de::DeserializeOwned};

pub const MAX_FRAME_SIZE: usize = 1024 * 1024;

pub fn connect_seqpacket(path: &Path) -> io::Result<std::os::fd::OwnedFd> {
    let fd = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::SOCK_CLOEXEC,
        None,
    )
    .map_err(io_error)?;
    let addr = UnixAddr::new(path).map_err(io_error)?;
    connect(fd.as_raw_fd(), &addr).map_err(io_error)?;
    Ok(fd)
}

pub fn bind_seqpacket(path: &Path) -> io::Result<std::os::fd::OwnedFd> {
    let fd = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::SOCK_CLOEXEC,
        None,
    )
    .map_err(io_error)?;
    let addr = UnixAddr::new(path).map_err(io_error)?;
    bind(fd.as_raw_fd(), &addr).map_err(io_error)?;
    listen(&fd, Backlog::new(64).map_err(io_error)?).map_err(io_error)?;
    Ok(fd)
}

pub fn send_json_frame<T: Serialize>(fd: RawFd, value: &T) -> io::Result<()> {
    send_json_frame_with_fds(fd, value, &[])
}

/// Send a JSON frame body with zero-or-more accompanying `SCM_RIGHTS`
/// file descriptors. When the fd slice is empty this is byte-equivalent
/// to a pure `send()` frame for backward compatibility with all existing
/// broker / daemon callers; fd-bearing responses use the same framing.
pub fn send_json_frame_with_fds<T: Serialize>(
    fd: RawFd,
    value: &T,
    fds: &[RawFd],
) -> io::Result<()> {
    let body =
        serde_json::to_vec(value).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    if body.len() > MAX_FRAME_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame body exceeds 1 MiB maximum",
        ));
    }

    let mut frame = Vec::with_capacity(body.len() + 4);
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(&body);

    if fds.is_empty() {
        let written = send(fd, &frame, MsgFlags::empty()).map_err(io_error)?;
        if written != frame.len() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "short seqpacket send while writing frame",
            ));
        }
        return Ok(());
    }

    crate::fd_passing::send_fds(fd, &frame, fds)
}

pub fn recv_json_frame<T: DeserializeOwned>(fd: RawFd) -> io::Result<Option<T>> {
    let mut buffer = vec![0_u8; MAX_FRAME_SIZE + 4];
    let read = recv(fd, &mut buffer, MsgFlags::empty()).map_err(io_error)?;
    if read == 0 {
        return Ok(None);
    }

    if read < 4 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "frame shorter than 4-byte length prefix",
        ));
    }
    let declared = u32::from_le_bytes(buffer[..4].try_into().expect("prefix length")) as usize;
    if declared > MAX_FRAME_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "declared frame length exceeds 1 MiB maximum",
        ));
    }
    if declared != read - 4 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame length prefix does not match seqpacket payload size",
        ));
    }
    serde_json::from_slice(&buffer[4..read])
        .map(Some)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

/// Receive one JSON frame and its close-on-exec SCM_RIGHTS attachments.
///
/// Request-side fd ownership is explicit: successful receipt transfers every
/// descriptor into an [`std::os::fd::OwnedFd`], while malformed frames and
/// decode failures close all descriptors before returning.
pub fn recv_json_frame_with_fds<T: DeserializeOwned>(
    fd: RawFd,
) -> io::Result<Option<(T, Vec<std::os::fd::OwnedFd>)>> {
    let (buffer, raw_fds) =
        crate::fd_passing::recv_fds_with_capacity_allow_empty(fd, MAX_FRAME_SIZE + 4)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("{error:?}")))?;
    if buffer.is_empty() {
        if raw_fds.is_empty() {
            return Ok(None);
        }
        crate::fd_passing::close_received_fds(&raw_fds);
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "empty SCM_RIGHTS packet",
        ));
    }
    if buffer.len() < 4 {
        crate::fd_passing::close_received_fds(&raw_fds);
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "frame shorter than 4-byte length prefix",
        ));
    }
    let declared = u32::from_le_bytes(buffer[..4].try_into().expect("prefix length")) as usize;
    if declared > MAX_FRAME_SIZE || declared != buffer.len() - 4 {
        crate::fd_passing::close_received_fds(&raw_fds);
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid SCM_RIGHTS frame length",
        ));
    }
    let decoded = match serde_json::from_slice(&buffer[4..]) {
        Ok(decoded) => decoded,
        Err(error) => {
            crate::fd_passing::close_received_fds(&raw_fds);
            return Err(io::Error::new(io::ErrorKind::InvalidData, error));
        }
    };
    let fds = raw_fds
        .into_iter()
        .map(crate::sys::owned_fd_from_raw)
        .collect();
    Ok(Some((decoded, fds)))
}

fn io_error(err: nix::errno::Errno) -> io::Error {
    io::Error::from_raw_os_error(err as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::sys::socket::{AddressFamily, SockFlag, SockType, socketpair};
    use nix::unistd::pipe;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
    struct Frame {
        request: String,
    }

    #[test]
    fn request_receiver_preserves_ordinary_fd_free_frames() {
        let (sender, receiver) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .expect("socket pair");
        let expected = Frame {
            request: "Hello".to_owned(),
        };

        send_json_frame(sender.as_raw_fd(), &expected).expect("send JSON request");
        let (actual, fds) = recv_json_frame_with_fds::<Frame>(receiver.as_raw_fd())
            .expect("receive JSON request")
            .expect("request frame");

        assert_eq!(actual, expected);
        assert!(
            fds.is_empty(),
            "ordinary broker requests carry no SCM_RIGHTS"
        );
    }

    #[test]
    fn request_receiver_rejects_empty_packets_with_descriptors() {
        let (sender, receiver) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .expect("socket pair");
        let (read_end, _write_end) = pipe().expect("pipe");

        crate::fd_passing::send_fds(sender.as_raw_fd(), b"", &[read_end.as_raw_fd()])
            .expect("send empty packet with descriptor");

        let error = recv_json_frame_with_fds::<Frame>(receiver.as_raw_fd())
            .expect_err("empty packet with descriptor must fail closed");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
