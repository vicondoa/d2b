//! Provider-neutral Unix seqpacket framing and descriptor transfer.

use std::io::{IoSlice, IoSliceMut};
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::path::Path;
use std::time::Duration;

use nix::cmsg_space;
use nix::fcntl::{FcntlArg, FdFlag, fcntl};
use nix::sys::socket::{
    AddressFamily, ControlMessage, ControlMessageOwned, MsgFlags, SockFlag, SockType, UnixAddr,
    connect, recv, recvmsg, send, sendmsg, socket,
};
use nix::unistd;
use serde::Serialize;
use socket2::{SockAddr, Socket};

use crate::typed_error::TypedError;

const REJECTION_DRAIN_DEADLINE: Duration = Duration::from_millis(10);

pub fn connect_seqpacket(path: &Path) -> Result<OwnedFd, TypedError> {
    let fd = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::SOCK_CLOEXEC,
        None,
    )
    .map_err(|err| TypedError::InternalIo {
        context: format!("create seqpacket socket {}", path.display()),
        detail: err.to_string(),
    })?;
    let address = UnixAddr::new(path).map_err(|err| TypedError::InternalIo {
        context: format!("encode seqpacket socket path {}", path.display()),
        detail: err.to_string(),
    })?;
    connect(fd.as_raw_fd(), &address).map_err(|err| TypedError::InternalBrokerUnavailable {
        path: path.to_path_buf(),
        detail: err.to_string(),
    })?;
    Ok(fd)
}

/// Connect a `SOCK_SEQPACKET` unix socket to `path`, bounding the connect
/// itself by `timeout` when set.
///
/// A plain blocking `connect(2)` on a backlogged / half-open broker
/// socket can stall unbounded, which defeats the readiness / config-sync
/// deadline that the caller is trying to honour. When `timeout` is set we
/// drive the connect nonblocking and poll for completion for at most
/// `timeout` (socket2's `connect_timeout` sets the fd nonblocking,
/// issues the connect, polls writability with the budget, checks
/// `SO_ERROR`, then restores blocking mode), so the subsequent
/// read/write-timeout-bounded I/O behaves exactly as before. With
/// `timeout == None` it falls back to the plain blocking connect.
pub fn connect_seqpacket_with_timeout(
    path: &Path,
    timeout: Option<Duration>,
) -> Result<OwnedFd, TypedError> {
    let Some(timeout) = timeout else {
        return connect_seqpacket(path);
    };
    let fd = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::SOCK_CLOEXEC,
        None,
    )
    .map_err(|err| TypedError::InternalIo {
        context: "create seqpacket socket".to_owned(),
        detail: err.to_string(),
    })?;
    let address = SockAddr::unix(path).map_err(|err| TypedError::InternalIo {
        context: "encode seqpacket socket path".to_owned(),
        detail: err.to_string(),
    })?;
    let socket = Socket::from(fd);
    socket.connect_timeout(&address, timeout).map_err(|err| {
        TypedError::InternalBrokerUnavailable {
            path: path.to_path_buf(),
            detail: err.to_string(),
        }
    })?;
    Ok(OwnedFd::from(socket))
}

pub fn round_trip(socket: &impl AsRawFd, frame_json: &str) -> Result<Vec<u8>, TypedError> {
    write_frame(socket, frame_json.as_bytes())?;
    read_frame(socket)
}

pub fn write_json_frame<T>(socket: &impl AsRawFd, value: &T) -> Result<(), TypedError>
where
    T: Serialize,
{
    write_json_frame_with_fds(socket, value, &[])
}

pub fn write_json_frame_with_fds<T>(
    socket: &impl AsRawFd,
    value: &T,
    fds: &[RawFd],
) -> Result<(), TypedError>
where
    T: Serialize,
{
    let bytes = serde_json::to_vec(value).map_err(|err| TypedError::InternalIo {
        context: "serialize JSON frame".to_owned(),
        detail: err.to_string(),
    })?;
    write_frame_with_fds(socket, &bytes, fds)
}

/// Set (or clear, with `None`) the read timeout on a connection socket.
/// Best-effort: a failure to set the deadline is non-fatal (the read
/// simply blocks as before). Used to bound hello/request frame reads so
/// a silent or slow-loris peer cannot pin a handler slot, and to CLEAR
/// the deadline before handing the socket to a blocking exec owner.
pub fn set_frame_read_deadline(socket: &Socket, deadline: Option<Duration>) {
    let _ = socket.set_read_timeout(deadline);
}

/// Write a JSON frame with a bounded write deadline, used for the
/// accept-loop refusal frames (authz reject / typed-busy) so the accept
/// loop never blocks on a peer that will not read. The deadline is
/// best-effort and the socket is closed by the caller afterwards.
pub fn write_json_frame_deadlined<T>(
    socket: &Socket,
    value: &T,
    deadline: Duration,
) -> Result<(), TypedError>
where
    T: Serialize,
{
    let _ = socket.set_write_timeout(Some(deadline));
    write_json_frame(socket, value)
}

/// Drain a rejected peer's already-buffered input before the socket is
/// closed. Authz-first and busy refusals write the rejection frame BEFORE
/// the peer's hello has been read; closing a `SOCK_SEQPACKET` socket while
/// input remains unread makes the kernel send RST, which the peer sees as a
/// connection reset (ECONNRESET) instead of cleanly reading the rejection.
/// Consuming the pending input first lets the close be graceful so the
/// rejection is delivered. Bounded by a short read deadline; the loop stops
/// at EOF, an error (incl. timeout), or after a few frames.
pub fn drain_rejected_peer_input(socket: &Socket) {
    let _ = socket.set_read_timeout(Some(REJECTION_DRAIN_DEADLINE));
    for _ in 0..4 {
        match read_frame(socket) {
            Ok(_) => continue,
            Err(_) => break,
        }
    }
}

pub fn write_frame(socket: &impl AsRawFd, body: &[u8]) -> Result<(), TypedError> {
    write_frame_with_fds(socket, body, &[])
}

pub fn write_frame_with_fds(
    socket: &impl AsRawFd,
    body: &[u8],
    fds: &[RawFd],
) -> Result<(), TypedError> {
    if body.len() > crate::wire::MAX_FRAME_SIZE {
        return Err(TypedError::WireFrameTooLarge {
            declared: body.len(),
        });
    }
    let mut frame = Vec::with_capacity(body.len() + 4);
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(body);
    let written = if fds.is_empty() {
        send(socket.as_raw_fd(), &frame, MsgFlags::empty())
    } else {
        let iov = [IoSlice::new(&frame)];
        let cmsgs = [ControlMessage::ScmRights(fds)];
        sendmsg::<()>(socket.as_raw_fd(), &iov, &cmsgs, MsgFlags::empty(), None)
    }
    .map_err(|err| TypedError::InternalIo {
        context: "send seqpacket frame".to_owned(),
        detail: err.to_string(),
    })?;
    if written != frame.len() {
        return Err(TypedError::InternalIo {
            context: "send seqpacket frame".to_owned(),
            detail: format!("short write: {written} of {}", frame.len()),
        });
    }
    Ok(())
}

pub fn read_frame(socket: &impl AsRawFd) -> Result<Vec<u8>, TypedError> {
    let mut buffer = vec![0u8; crate::wire::MAX_FRAME_SIZE + 5];
    let read = recv(socket.as_raw_fd(), &mut buffer, MsgFlags::empty()).map_err(|err| {
        TypedError::InternalIo {
            context: "recv seqpacket frame".to_owned(),
            detail: err.to_string(),
        }
    })?;
    if read == 0 {
        return Err(TypedError::InternalIo {
            context: "recv seqpacket frame".to_owned(),
            detail: "peer closed the socket".to_owned(),
        });
    }
    if read < 4 {
        return Err(TypedError::WireInvalidFrame {
            detail: format!("frame too short: {read} bytes"),
        });
    }
    let declared = u32::from_le_bytes(buffer[..4].try_into().expect("prefix slice")) as usize;
    if declared > crate::wire::MAX_FRAME_SIZE {
        return Err(TypedError::WireFrameTooLarge { declared });
    }
    if read - 4 != declared {
        return Err(TypedError::WireInvalidFrame {
            detail: format!("declared {declared} bytes but received {}", read - 4),
        });
    }
    Ok(buffer[4..read].to_vec())
}

pub fn mark_fd_cloexec(fd: RawFd, context: &str) -> Result<(), TypedError> {
    let current = fcntl(fd, FcntlArg::F_GETFD).map_err(|err| TypedError::InternalIo {
        context: context.to_owned(),
        detail: err.to_string(),
    })?;
    let flags = FdFlag::from_bits_truncate(current) | FdFlag::FD_CLOEXEC;
    fcntl(fd, FcntlArg::F_SETFD(flags)).map_err(|err| TypedError::InternalIo {
        context: context.to_owned(),
        detail: err.to_string(),
    })?;
    Ok(())
}

pub fn duplicate_fd_cloexec(fd: RawFd, context: &str) -> Result<OwnedFd, TypedError> {
    let pid = rustix::process::Pid::from_raw(std::process::id() as i32).ok_or_else(|| {
        TypedError::InternalIo {
            context: context.to_owned(),
            detail: "current pid is invalid".to_owned(),
        }
    })?;
    let self_pidfd = rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty())
        .map_err(|err| TypedError::InternalIo {
            context: context.to_owned(),
            detail: err.to_string(),
        })?;
    let duplicated =
        rustix::process::pidfd_getfd(&self_pidfd, fd, rustix::process::PidfdGetfdFlags::empty())
            .map_err(|err| TypedError::InternalIo {
                context: context.to_owned(),
                detail: err.to_string(),
            })?;
    if let Err(error) = mark_fd_cloexec(duplicated.as_raw_fd(), context) {
        drop(duplicated);
        return Err(error);
    }
    Ok(duplicated)
}

pub fn read_frame_with_fds(socket: &impl AsRawFd) -> Result<(Vec<u8>, Vec<RawFd>), TypedError> {
    let mut buffer = vec![0u8; crate::wire::MAX_FRAME_SIZE + 5];
    let mut iov = [IoSliceMut::new(&mut buffer)];
    let mut control = cmsg_space!([RawFd; 8]);
    let message = recvmsg::<UnixAddr>(
        socket.as_raw_fd(),
        &mut iov,
        Some(&mut control),
        MsgFlags::MSG_CMSG_CLOEXEC,
    )
    .map_err(|err| TypedError::InternalIo {
        context: "recv seqpacket frame with fds".to_owned(),
        detail: err.to_string(),
    })?;
    let read = message.bytes;
    let mut received_fds = Vec::new();
    for cmsg in message.cmsgs().map_err(|err| TypedError::InternalIo {
        context: "recv seqpacket frame with fds".to_owned(),
        detail: err.to_string(),
    })? {
        if let ControlMessageOwned::ScmRights(fds) = cmsg {
            received_fds.extend(fds);
        }
    }
    if message
        .flags
        .intersects(MsgFlags::MSG_TRUNC | MsgFlags::MSG_CTRUNC)
    {
        close_received_fds(&received_fds);
        return Err(TypedError::WireInvalidFrame {
            detail: "truncated seqpacket frame with fds".to_owned(),
        });
    }
    for fd in &received_fds {
        if let Err(error) = mark_fd_cloexec(*fd, "mark received fd cloexec") {
            close_received_fds(&received_fds);
            return Err(error);
        }
    }
    if read == 0 {
        close_received_fds(&received_fds);
        return Err(TypedError::InternalIo {
            context: "recv seqpacket frame with fds".to_owned(),
            detail: "peer closed the socket".to_owned(),
        });
    }
    if read < 4 {
        close_received_fds(&received_fds);
        return Err(TypedError::WireInvalidFrame {
            detail: format!("frame too short: {read} bytes"),
        });
    }
    let declared = u32::from_le_bytes(buffer[..4].try_into().expect("prefix slice")) as usize;
    if declared > crate::wire::MAX_FRAME_SIZE {
        close_received_fds(&received_fds);
        return Err(TypedError::WireFrameTooLarge { declared });
    }
    if read - 4 != declared {
        close_received_fds(&received_fds);
        return Err(TypedError::WireInvalidFrame {
            detail: format!("declared {declared} bytes but received {}", read - 4),
        });
    }
    Ok((buffer[4..read].to_vec(), received_fds))
}

pub fn close_received_fds(fds: &[RawFd]) {
    for fd in fds {
        let _ = unistd::close(*fd);
    }
}

#[cfg(test)]
mod broker_fd_tests {
    use super::*;
    use nix::{
        fcntl::{FcntlArg, FdFlag, fcntl},
        sys::socket::{AddressFamily, SockFlag, SockType, socketpair},
        unistd::pipe,
    };
    use serde_json::json;
    use std::os::fd::AsRawFd;

    #[test]
    fn request_fd_sender_transfers_one_cloexec_descriptor() {
        let (sender, receiver) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .expect("socket pair");
        let (read_end, _write_end) = pipe().expect("pipe");

        write_json_frame_with_fds(
            &sender,
            &json!({ "request": "peer-pidfd" }),
            &[read_end.as_raw_fd()],
        )
        .expect("send request fd");
        let (body, received) = read_frame_with_fds(&receiver).expect("receive request fd");

        assert_eq!(body, br#"{"request":"peer-pidfd"}"#);
        assert_eq!(received.len(), 1);
        assert!(
            FdFlag::from_bits_truncate(
                fcntl(received[0], FcntlArg::F_GETFD).expect("get received fd flags")
            )
            .contains(FdFlag::FD_CLOEXEC),
            "received request fd must be close-on-exec"
        );
        close_received_fds(&received);
    }
}

pub fn io_wrap(context: &'static str) -> impl FnOnce(nix::errno::Errno) -> TypedError {
    move |err| TypedError::InternalIo {
        context: context.to_owned(),
        detail: err.to_string(),
    }
}
