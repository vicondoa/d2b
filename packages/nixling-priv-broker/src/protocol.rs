use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::path::Path;

use nix::sys::socket::{
    bind, connect, listen, recv, send, socket, AddressFamily, Backlog, MsgFlags, SockFlag,
    SockType, UnixAddr,
};
use serde::{de::DeserializeOwned, Serialize};

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
/// broker / daemon callers; only `OpenPidfd` / `SpawnRunner` responses
/// carry fds.
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

fn io_error(err: nix::errno::Errno) -> io::Error {
    io::Error::from_raw_os_error(err as i32)
}
