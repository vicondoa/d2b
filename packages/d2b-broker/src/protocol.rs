use std::io;
use std::io::IoSliceMut;
use std::os::fd::{AsFd, AsRawFd, OwnedFd, RawFd};
use std::path::Path;
use std::time::Duration;

use nix::errno::Errno;
use nix::sys::socket::{
    AddressFamily, Backlog, MsgFlags, SockFlag, SockType, UnixAddr, accept4, bind, connect, listen,
    recv, recvmsg, send, socket,
};
use serde::{Serialize, de::DeserializeOwned};
use tokio::io::unix::AsyncFd;

/// The maximum JSON frame body size, excluding the 4-byte length
/// prefix: frames declaring a larger body are refused.
pub const MAX_FRAME_SIZE: usize = 1024 * 1024;

/// Connect a `SOCK_SEQPACKET` Unix socket to `path`, returning the
/// connected CLOEXEC fd.
///
/// # Errors
///
/// Returns the socket error when the socket cannot be created or connected.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
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

/// Bind-and-listen a `SOCK_SEQPACKET` Unix socket at `path`, returning
/// the listening CLOEXEC fd with a backlog of 64.
///
/// # Errors
///
/// Returns the socket error when create, bind, or listen fails.
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

/// Serialise `value` as JSON and send it as one frame on `fd`:a 4-byte
/// little-endian length prefix followed by the body, refusing bodies over
/// [`MAX_FRAME_SIZE`]. Byte-equivalent to
/// [`send_json_frame_with_fds`] when no descriptors are attached.
///
/// # Errors
///
/// Returns [`io::ErrorKind::InvalidData`] for serialisation or cap
/// violations, and socket / short-write errors for the send itself.
pub fn send_json_frame<T: Serialize>(fd: RawFd, value: &T) -> io::Result<()> {
    send_json_frame_with_fds(fd, value, &[])
}

/// Send a JSON frame body with zero-or-more accompanying `SCM_RIGHTS`
/// file descriptors. When the fd slice is empty this is byte-equivalent
/// to a pure `send()` frame for backward compatibility with all existing
/// broker / daemon callers; fd-bearing responses use the same framing.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
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

/// Receive one JSON frame from `fd`:a 4-byte little-endian length
/// prefix followed by the body, capped at [`MAX_FRAME_SIZE`]; returns
/// `None` when the peer closed the socket empty.
///
/// The 4-byte length prefix is peeked with `MSG_PEEK` first, so the body
/// buffer is allocated to the declared frame size instead of the
/// [`MAX_FRAME_SIZE`] ceiling on every receive. A socket type without
/// `MSG_PEEK` support falls back to the fixed ceiling allocation.
///
/// # Errors
///
/// Returns [`io::ErrorKind::UnexpectedEof`] for short frames and
/// [`io::ErrorKind::InvalidData`] for length-prefix mismatches and decode
/// failures.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
pub fn recv_json_frame<T: DeserializeOwned>(fd: RawFd) -> io::Result<Option<T>> {
    // Peek the 4-byte length prefix so the body buffer can be allocated to
    // the declared frame size instead of the 1 MiB ceiling. `MSG_PEEK` does
    // not consume the frame, and a `SOCK_SEQPACKET` receive is atomic, so
    // the peeked length is exactly the length the receive below gets.
    let mut prefix = [0_u8; 4];
    let peeked = {
        let mut iov = [IoSliceMut::new(&mut prefix)];
        match recvmsg::<()>(fd, &mut iov, None, MsgFlags::MSG_PEEK) {
            Ok(message) => message.bytes,
            // A socket type without `MSG_PEEK` support keeps the fixed
            // ceiling allocation; the receive itself is unchanged. (`ENOTSUP`
            // and `EOPNOTSUPP` are the same errno on Linux.)
            Err(Errno::EINVAL | Errno::ENOTSUP) => {
                return recv_json_frame_fixed(fd);
            }
            Err(err) => return Err(io_error(err)),
        }
    };
    if peeked == 0 {
        // The peer closed the socket empty; a queued zero-length packet is
        // reported as closed exactly like the fixed path reports it.
        return Ok(None);
    }
    if peeked < 4 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "frame shorter than 4-byte length prefix",
        ));
    }
    let declared = u32::from_le_bytes(prefix) as usize;
    if declared > MAX_FRAME_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "declared frame length exceeds 1 MiB maximum",
        ));
    }

    let mut buffer = vec![0_u8; declared + 4];
    let (bytes, truncated) = {
        // The receive borrows the iov (and through it the buffer) for the
        // lifetime of the returned message, so the message's fields are
        // copied out inside this scope and the buffer borrow ends here.
        let mut iov = [IoSliceMut::new(&mut buffer)];
        let message = recvmsg::<()>(fd, &mut iov, None, MsgFlags::empty()).map_err(io_error)?;
        (message.bytes, message.flags.contains(MsgFlags::MSG_TRUNC))
    };
    if bytes == 0 {
        return Ok(None);
    }
    if bytes < 4 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "frame shorter than 4-byte length prefix",
        ));
    }
    // An exact-size buffer truncates a packet larger than its declared
    // prefix+body; refuse it the way the fixed path refuses a length
    // mismatch, instead of decoding a truncated frame.
    decode_frame(&buffer[..bytes], declared, truncated)
}

/// The fixed-ceiling receive used when the socket type does not support
/// `MSG_PEEK`: one `MAX_FRAME_SIZE + 4` allocation per frame, the original
/// receive path unchanged.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn recv_json_frame_fixed<T: DeserializeOwned>(fd: RawFd) -> io::Result<Option<T>> {
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
    decode_frame(&buffer[..read], declared, false)
}

/// Validate a received frame against its declared length and decode it.
///
/// `truncated` reports a packet cut short by an undersized receive buffer
/// (`MSG_TRUNC`), which the fixed ceiling path cannot produce but the
/// exact-size path can.
fn decode_frame<T: DeserializeOwned>(
    frame: &[u8],
    declared: usize,
    truncated: bool,
) -> io::Result<Option<T>> {
    if declared > MAX_FRAME_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "declared frame length exceeds 1 MiB maximum",
        ));
    }
    if truncated || declared != frame.len() - 4 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame length prefix does not match seqpacket payload size",
        ));
    }
    serde_json::from_slice(&frame[4..])
        .map(Some)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

/// Receive one JSON frame and its close-on-exec SCM_RIGHTS attachments.
///
/// The 4-byte length prefix is peeked with `MSG_PEEK` first, so the payload
/// buffer is allocated to the declared frame size instead of the
/// [`MAX_FRAME_SIZE`] ceiling. The peek passes no control buffer, so no
/// descriptor is installed and the frame's SCM_RIGHTS attachment is still
/// delivered by the receive below. A socket type without `MSG_PEEK` support
/// falls back to the fixed ceiling allocation.
///
/// Request-side fd ownership is explicit: successful receipt transfers every
/// descriptor into an [`std::os::fd::OwnedFd`], while malformed frames and
/// decode failures close all descriptors before returning.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
pub fn recv_json_frame_with_fds<T: DeserializeOwned>(
    fd: RawFd,
) -> io::Result<Option<(T, Vec<std::os::fd::OwnedFd>)>> {
    // Peek the 4-byte length prefix so the payload buffer can be allocated
    // to the declared frame size instead of the 1 MiB ceiling. `MSG_PEEK`
    // does not consume the frame, and a `SOCK_SEQPACKET` receive is atomic,
    // so the peeked length is exactly the length the receive below gets.
    let mut prefix = [0_u8; 4];
    let peeked = {
        let mut iov = [IoSliceMut::new(&mut prefix)];
        match recvmsg::<()>(fd, &mut iov, None, MsgFlags::MSG_PEEK) {
            Ok(message) => message.bytes,
            // A socket type without `MSG_PEEK` support keeps the fixed
            // ceiling allocation; the receive itself is unchanged. (`ENOTSUP`
            // and `EOPNOTSUPP` are the same errno on Linux.)
            Err(Errno::EINVAL | Errno::ENOTSUP) => {
                return recv_json_frame_with_fds_fixed(fd);
            }
            Err(err) => return Err(io_error(err)),
        }
    };
    // A zero-length peek is either a closed socket or a zero-length packet,
    // which may still carry SCM_RIGHTS: the receive below disambiguates the
    // two exactly like the fixed path, so the zero-length packet needs no
    // payload buffer.
    let payload_capacity = if peeked == 0 {
        0
    } else {
        if peeked < 4 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "frame shorter than 4-byte length prefix",
            ));
        }
        let declared = u32::from_le_bytes(prefix) as usize;
        if declared > MAX_FRAME_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid SCM_RIGHTS frame length",
            ));
        }
        declared + 4
    };
    let (buffer, raw_fds) =
        crate::fd_passing::recv_fds_with_capacity_allow_empty(fd, payload_capacity)
            .map_err(fd_passing_error)?;
    decode_fds_frame(buffer, raw_fds)
}

/// The fixed-ceiling receive used when the socket type does not support
/// `MSG_PEEK`: one `MAX_FRAME_SIZE + 4` allocation per frame, the original
/// receive path unchanged.
fn recv_json_frame_with_fds_fixed<T: DeserializeOwned>(
    fd: RawFd,
) -> io::Result<Option<(T, Vec<std::os::fd::OwnedFd>)>> {
    let (buffer, raw_fds) =
        crate::fd_passing::recv_fds_with_capacity_allow_empty(fd, MAX_FRAME_SIZE + 4)
            .map_err(fd_passing_error)?;
    decode_fds_frame(buffer, raw_fds)
}

/// Validate a received SCM_RIGHTS frame against its declared length, close
/// every descriptor on a malformed frame or decode failure, and decode the
/// body.
fn decode_fds_frame<T: DeserializeOwned>(
    buffer: Vec<u8>,
    raw_fds: Vec<RawFd>,
) -> io::Result<Option<(T, Vec<std::os::fd::OwnedFd>)>> {
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

/// A frame receive that adapts one descriptor-passing error.
///
/// A nonblocking receive with nothing to read is not a malformed frame: the
/// caller waits for readability and retries, which requires the error kind
/// to reach it unchanged.
fn fd_passing_error(error: crate::fd_passing::FdPassingError) -> io::Error {
    if error == crate::fd_passing::FdPassingError::WouldBlock {
        return io::Error::new(io::ErrorKind::WouldBlock, "no frame is ready");
    }
    io::Error::new(io::ErrorKind::InvalidData, format!("{error:?}"))
}

/// A Unix `SOCK_SEQPACKET` connection whose frame I/O is driven by the tokio
/// reactor.
///
/// The framing is the synchronous one, unchanged: one packet is one frame,
/// received with the same atomic `recv`/`recvmsg` and sent with the same
/// `send`/`sendmsg`, and the payload ceiling is the same. What changes is
/// who waits - the descriptor is nonblocking and the reactor reports
/// readiness, so a caller that is slow to send, slow to answer, or absent
/// costs a waiting task rather than a blocked thread.
#[derive(Debug)]
pub struct AsyncSeqpacket {
    io: AsyncFd<OwnedFd>,
}

impl AsyncSeqpacket {
    /// Adopt one connected seqpacket descriptor into the reactor.
    ///
    /// The descriptor is switched to nonblocking, so every receive reports
    /// an empty socket as `WouldBlock` instead of waiting in the kernel.
    pub fn from_owned(fd: OwnedFd) -> io::Result<Self> {
        set_nonblocking(&fd)?;
        AsyncFd::new(fd).map(|io| Self { io })
    }

    /// Borrow the descriptor the connection is registered on.
    pub fn as_raw_fd(&self) -> RawFd {
        self.io.get_ref().as_raw_fd()
    }

    /// Receive one JSON frame, waiting for readability.
    pub async fn recv_json_frame<T: DeserializeOwned>(&self) -> io::Result<Option<T>> {
        self.readable(|fd| recv_json_frame::<T>(fd.as_raw_fd()))
            .await
    }

    /// Receive one JSON frame and its close-on-exec `SCM_RIGHTS`
    /// attachments, waiting for readability.
    pub async fn recv_json_frame_with_fds<T: DeserializeOwned>(
        &self,
    ) -> io::Result<Option<(T, Vec<OwnedFd>)>> {
        self.readable(|fd| recv_json_frame_with_fds::<T>(fd.as_raw_fd()))
            .await
    }

    /// Send one JSON frame, waiting for writability.
    pub async fn send_json_frame<T: Serialize>(&self, value: &T) -> io::Result<()> {
        self.writable(|fd| send_json_frame(fd.as_raw_fd(), value))
            .await
    }

    /// Send one JSON frame with `SCM_RIGHTS` attachments, waiting for
    /// writability.
    pub async fn send_json_frame_with_fds<T: Serialize>(
        &self,
        value: &T,
        fds: &[RawFd],
    ) -> io::Result<()> {
        self.writable(|fd| send_json_frame_with_fds(fd.as_raw_fd(), value, fds))
            .await
    }

    /// Run one receive against the ready registration, re-arming on a
    /// would-block race.
    async fn readable<T>(
        &self,
        mut receive: impl FnMut(&OwnedFd) -> io::Result<T>,
    ) -> io::Result<T> {
        loop {
            let mut ready = self.io.readable().await?;
            match ready.try_io(|inner| receive(inner.get_ref())) {
                Ok(result) => return result,
                Err(_would_block) => continue,
            }
        }
    }

    /// Run one send against the ready registration, re-arming on a
    /// would-block race.
    async fn writable<T>(&self, mut send: impl FnMut(&OwnedFd) -> io::Result<T>) -> io::Result<T> {
        loop {
            let mut ready = self.io.writable().await?;
            match ready.try_io(|inner| send(inner.get_ref())) {
                Ok(result) => return result,
                Err(_would_block) => continue,
            }
        }
    }
}

/// A listening `SOCK_SEQPACKET` socket whose accepts are driven by the tokio
/// reactor.
#[derive(Debug)]
pub struct AsyncSeqpacketListener {
    io: AsyncFd<OwnedFd>,
}

impl AsyncSeqpacketListener {
    /// Adopt one listening seqpacket descriptor into the reactor.
    pub fn from_owned(fd: OwnedFd) -> io::Result<Self> {
        set_nonblocking(&fd)?;
        AsyncFd::new(fd).map(|io| Self { io })
    }

    /// Accept one connection, waiting for it to arrive.
    pub async fn accept(&self) -> io::Result<AsyncSeqpacket> {
        loop {
            let mut ready = self.io.readable().await?;
            match ready.try_io(|inner| accept_seqpacket(inner.get_ref())) {
                Ok(Ok(fd)) => return AsyncSeqpacket::from_owned(fd),
                Ok(Err(error)) => return Err(error),
                Err(_would_block) => continue,
            }
        }
    }
}

/// Dial a seqpacket peer, bounding the whole dial by `timeout`.
///
/// A blocking `connect(2)` has no deadline: a peer whose listen queue is
/// full parks the caller in the kernel for as long as it likes. This dial
/// drives the same connect nonblocking and waits in async time, so the dial
/// gives up on its own budget and the caller keeps the thread it was on.
pub async fn connect_seqpacket_bounded(
    path: &Path,
    timeout: Duration,
) -> io::Result<AsyncSeqpacket> {
    let fd = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::SOCK_CLOEXEC,
        None,
    )
    .map_err(io_error)?;
    set_nonblocking(&fd)?;
    let address = UnixAddr::new(path).map_err(io_error)?;
    let deadline = tokio::time::Instant::now() + timeout;
    // The dial waits in the reactor: the descriptor is non-blocking, so the
    // connect syscall below never parks the caller, and the in-flight wait
    // is an AsyncFd writable edge (connect completion) instead of a poll.
    let io = AsyncFd::new(fd)?;
    loop {
        // R11 inventory: the raw connect is the non-blocking syscall inside
        // the AsyncFd dial (the deny list's own replacement for
        // `nix::sys::socket::connect` on a seqpacket socket, which tokio's
        // `UnixStream` cannot represent). `set_nonblocking` above means this
        // call never parks: the kernel answers EINPROGRESS/EAGAIN/EALREADY
        // immediately and the wait happens on the AsyncFd writable edge in
        // async time, bounded by `deadline`.
        #[allow(clippy::disallowed_methods, reason = "synchronous path")]
        match connect(io.get_ref().as_raw_fd(), &address) {
            Ok(()) => return AsyncSeqpacket::from_owned(io.into_inner()),
            // The kernel completed the dial between our attempts.
            Err(nix::errno::Errno::EISCONN) => {
                return AsyncSeqpacket::from_owned(io.into_inner());
            }
            Err(nix::errno::Errno::EINTR) => continue,
            // The kernel could not complete the dial yet - its listen queue
            // is full, or the connect is still in flight - but it is not an
            // answer: keep within the budget and try again.
            Err(nix::errno::Errno::EAGAIN)
            | Err(nix::errno::Errno::EINPROGRESS)
            | Err(nix::errno::Errno::EALREADY) => {}
            Err(err) => return Err(io_error(err)),
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("dial {} exceeded its budget", path.display()),
            ));
        }
        // Wait for the socket to become writable - the kernel's connect
        // completion signal - in async time. The guard's drop clears the
        // readiness, so a still-in-flight dial waits for the next edge
        // instead of spinning.
        let _ready = io.writable().await?;
    }
}

/// Accept one connection from a nonblocking listening socket.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn accept_seqpacket(listener: &OwnedFd) -> io::Result<OwnedFd> {
    loop {
        match accept4(
            listener.as_raw_fd(),
            SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
        ) {
            Ok(fd) => return Ok(crate::sys::owned_fd_from_raw(fd)),
            Err(nix::errno::Errno::EINTR) => continue,
            Err(err) => return Err(io_error(err)),
        }
    }
}

/// Switch one descriptor to nonblocking so its waits happen in the reactor.
fn set_nonblocking(fd: &impl AsFd) -> io::Result<()> {
    let flags = rustix::fs::fcntl_getfl(fd).map_err(rustix_io_error)?;
    rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK).map_err(rustix_io_error)
}

fn rustix_io_error(err: rustix::io::Errno) -> io::Error {
    io::Error::from(err)
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

    /// The async path carries what the synchronous one carried: one frame,
    /// with its `SCM_RIGHTS` attachment, over a nonblocking descriptor.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn async_frames_carry_attached_descriptors() {
        let (sender, receiver) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .expect("socket pair");
        let sender = AsyncSeqpacket::from_owned(sender).expect("register sender");
        let receiver = AsyncSeqpacket::from_owned(receiver).expect("register receiver");
        let (read_end, write_end) = pipe().expect("pipe");
        let expected = Frame {
            request: "UsbipBind".to_owned(),
        };

        sender
            .send_json_frame_with_fds(&expected, &[read_end.as_raw_fd()])
            .await
            .expect("send frame with descriptor");
        let (actual, fds) = receiver
            .recv_json_frame_with_fds::<Frame>()
            .await
            .expect("receive frame")
            .expect("frame");

        assert_eq!(actual, expected);
        assert_eq!(fds.len(), 1, "the attachment must cross with the frame");
        // The descriptor that crossed is the pipe the sender attached: what
        // is written to the sender's end reads back from the received end.
        let _ = &read_end;
        nix::unistd::write(&write_end, b"x").expect("write to the pipe");
        let mut buffer = [0_u8; 1];
        let read = nix::unistd::read(fds[0].as_raw_fd(), &mut buffer).expect("read the pipe");
        assert_eq!((read, buffer), (1, *b"x"));
    }

    /// A dial nothing is listening on refuses at once; it is the dial that
    /// must not wait forever for a peer, not the caller's budget.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn a_bounded_dial_to_an_absent_peer_refuses_instead_of_waiting() {
        let dir = tempfile::tempdir().expect("dir");
        // Virtual time: the dial's 5 s budget is driven by the paused
        // clock, so an immediate refusal must consume none of it. The
        // assertion is exact - wall-clock load cannot stretch it into a
        // flake, and a dial that waited out its budget would fail it.
        tokio::time::pause();
        let started = tokio::time::Instant::now();
        let error =
            connect_seqpacket_bounded(&dir.path().join("absent.sock"), Duration::from_secs(5))
                .await
                .expect_err("an absent peer refuses the dial");
        assert_eq!(error.kind(), io::ErrorKind::NotFound, "{error:?}");
        assert_eq!(
            started.elapsed(),
            Duration::ZERO,
            "the refusal must be immediate, not the budget: the dial consumed none of its 5 s budget"
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
