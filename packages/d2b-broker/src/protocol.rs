use std::io;
use std::os::fd::{AsFd, AsRawFd, OwnedFd, RawFd};
use std::path::Path;
use std::time::Duration;

use nix::sys::socket::{
    AddressFamily, Backlog, MsgFlags, SockFlag, SockType, UnixAddr, accept4, bind, connect, listen,
    recv, send, socket,
};
use serde::{Serialize, de::DeserializeOwned};
use tokio::io::unix::AsyncFd;

pub const MAX_FRAME_SIZE: usize = 1024 * 1024;

/// How long a dial that the kernel does not complete on the first attempt
/// waits before it tries again, bounded by the dial's own budget.
const DIAL_RETRY_INTERVAL: Duration = Duration::from_millis(5);

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

#[allow(clippy::disallowed_methods, reason = "synchronous path")]
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
            .map_err(fd_passing_error)?;
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
    loop {
        match connect(fd.as_raw_fd(), &address) {
            Ok(()) => return AsyncSeqpacket::from_owned(fd),
            Err(nix::errno::Errno::EINTR) => continue,
            // The kernel could not complete the dial yet - its listen queue
            // is full, or the connect is still in flight - but it is not an
            // answer: keep within the budget and try again.
            Err(nix::errno::Errno::EAGAIN) | Err(nix::errno::Errno::EINPROGRESS) => {}
            Err(err) => return Err(io_error(err)),
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("dial {} exceeded its budget", path.display()),
            ));
        }
        tokio::time::sleep(DIAL_RETRY_INTERVAL.min(remaining)).await;
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
    async fn a_bounded_dial_to_an_absent_peer_refuses_instead_of_waiting() {
        let dir = tempfile::tempdir().expect("dir");
        let started = std::time::Instant::now();
        let error =
            connect_seqpacket_bounded(&dir.path().join("absent.sock"), Duration::from_secs(5))
                .await
                .expect_err("an absent peer refuses the dial");
        assert_eq!(error.kind(), io::ErrorKind::NotFound, "{error:?}");
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "the refusal must be immediate, not the budget: {:?}",
            started.elapsed()
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
