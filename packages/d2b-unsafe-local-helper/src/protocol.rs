use crate::runtime::{RuntimeError, ScopeRuntime};
use crate::systemd::UserScopeManager;
use d2b_contracts::UNSAFE_LOCAL_HELPER_SOCKET_PATH;
use d2b_contracts::unsafe_local_wire::{
    DaemonToUnsafeLocalHelper, HELPER_SOCKET_BUFFER_REQUEST_BYTES, HelperFailureCode,
    HelperHeartbeat, HelperHello, HelperOperationRejected, MAX_HELPER_FRAME_SIZE,
    MAX_HELPER_QUEUE_DEPTH, MIN_EFFECTIVE_HELPER_SOCKET_BUFFER_BYTES,
    UNSAFE_LOCAL_HELPER_PROTOCOL_VERSION, UnsafeLocalHelperToDaemon,
    unsafe_local_helper_protocol_supported,
};
use d2b_realm_core::ids::OperationId;
use nix::cmsg_space;
use nix::libc;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::socket::{
    ControlMessage, ControlMessageOwned, MsgFlags, UnixAddr, getsockopt, recvmsg, send, sendmsg,
    sockopt::PeerCredentials,
};
use nix::unistd;
use socket2::{Domain, SockAddr, Socket, Type};
use std::fmt;
use std::io::{IoSlice, IoSliceMut, Read, Write};
use std::os::fd::{AsFd, AsRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use uzers::get_user_by_name;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolError {
    InvalidIdentity,
    ConnectFailed,
    PeerCredentialMismatch,
    BufferTooSmall,
    InvalidFrame,
    FrameTooLarge,
    ProtocolMismatch,
    GenerationMismatch,
    QueueClosed,
    RuntimeUnavailable,
}

pub struct HelperClient<M: UserScopeManager> {
    socket_path: PathBuf,
    expected_daemon_uid: u32,
    runtime: Arc<ScopeRuntime<M>>,
}

struct QueuedResponse {
    frame: UnsafeLocalHelperToDaemon,
    fd: Option<OwnedFd>,
}

impl QueuedResponse {
    fn ordinary(frame: UnsafeLocalHelperToDaemon) -> Self {
        Self { frame, fd: None }
    }
}

impl<M: UserScopeManager> fmt::Debug for HelperClient<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HelperClient")
            .field("socket_path", &"<redacted>")
            .field("expected_daemon_uid", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl<M: UserScopeManager> HelperClient<M> {
    pub fn new(
        socket_path: impl Into<PathBuf>,
        daemon_user: &str,
        runtime: ScopeRuntime<M>,
    ) -> Result<Self, ProtocolError> {
        let daemon = get_user_by_name(daemon_user).ok_or(ProtocolError::InvalidIdentity)?;
        if daemon.uid() == 0 {
            return Err(ProtocolError::InvalidIdentity);
        }
        Ok(Self {
            socket_path: socket_path.into(),
            expected_daemon_uid: daemon.uid(),
            runtime: Arc::new(runtime),
        })
    }

    #[cfg(test)]
    pub fn with_daemon_uid(
        socket_path: impl Into<PathBuf>,
        expected_daemon_uid: u32,
        runtime: ScopeRuntime<M>,
    ) -> Result<Self, ProtocolError> {
        if expected_daemon_uid == 0 {
            return Err(ProtocolError::InvalidIdentity);
        }
        Ok(Self {
            socket_path: socket_path.into(),
            expected_daemon_uid,
            runtime: Arc::new(runtime),
        })
    }

    pub fn run(&self) -> Result<(), ProtocolError> {
        let socket = connect_control_socket(&self.socket_path, self.expected_daemon_uid)?;
        let generation = random_generation()?;
        send_frame(
            &socket,
            &UnsafeLocalHelperToDaemon::Hello(HelperHello {
                protocol_version: UNSAFE_LOCAL_HELPER_PROTOCOL_VERSION,
                generation,
                features: Vec::new(),
            }),
        )?;

        let mut receive_buffer = vec![0u8; MAX_HELPER_FRAME_SIZE + 5];
        let accepted: DaemonToUnsafeLocalHelper = receive_frame(&socket, &mut receive_buffer)?;
        match accepted {
            DaemonToUnsafeLocalHelper::HelloAccepted(accepted)
                if unsafe_local_helper_protocol_supported(accepted.protocol_version)
                    && accepted.generation == generation => {}
            DaemonToUnsafeLocalHelper::HelloAccepted(_) => {
                return Err(ProtocolError::GenerationMismatch);
            }
            _ => return Err(ProtocolError::ProtocolMismatch),
        }
        let snapshot = self
            .runtime
            .snapshot(generation)
            .map_err(|_| ProtocolError::RuntimeUnavailable)?;
        send_frame(&socket, &UnsafeLocalHelperToDaemon::Snapshot(snapshot))?;

        let (response_wakeup_read, response_wakeup_write) =
            UnixStream::pair().map_err(|_| ProtocolError::RuntimeUnavailable)?;
        response_wakeup_read
            .set_nonblocking(true)
            .map_err(|_| ProtocolError::ConnectFailed)?;
        response_wakeup_write
            .set_nonblocking(true)
            .map_err(|_| ProtocolError::ConnectFailed)?;
        let (responses_tx, responses_rx) =
            mpsc::sync_channel::<QueuedResponse>(MAX_HELPER_QUEUE_DEPTH);
        let response_wakeup_write = Arc::new(response_wakeup_write);
        let active = Arc::new(AtomicUsize::new(0));

        loop {
            while let Ok(response) = responses_rx.try_recv() {
                send_queued_response(&socket, response)?;
            }
            match wait_for_control_or_response(&socket, &response_wakeup_read)? {
                ControlEvent::Response => continue,
                ControlEvent::Control => {}
            }
            let frame: DaemonToUnsafeLocalHelper = receive_frame(&socket, &mut receive_buffer)?;
            match frame {
                DaemonToUnsafeLocalHelper::Heartbeat(heartbeat) => {
                    if heartbeat.generation != generation {
                        return Err(ProtocolError::GenerationMismatch);
                    }
                    send_frame(
                        &socket,
                        &UnsafeLocalHelperToDaemon::Heartbeat(HelperHeartbeat {
                            generation,
                            sequence: heartbeat.sequence,
                        }),
                    )?;
                }
                DaemonToUnsafeLocalHelper::Launch(request) => {
                    if active.fetch_add(1, Ordering::AcqRel) >= MAX_HELPER_QUEUE_DEPTH {
                        active.fetch_sub(1, Ordering::AcqRel);
                        let rejected = rejection(
                            request.request_id,
                            request.operation_id,
                            HelperFailureCode::QueueFull,
                        );
                        send_frame(&socket, &rejected)?;
                        continue;
                    }
                    let runtime = Arc::clone(&self.runtime);
                    let responses = responses_tx.clone();
                    let response_wakeup = Arc::clone(&response_wakeup_write);
                    let active = Arc::clone(&active);
                    std::thread::Builder::new()
                        .name("d2b-unsafe-local-operation".to_owned())
                        .spawn(move || {
                            let request_id = request.request_id;
                            let operation_id = request.operation_id.clone();
                            let response = match runtime.launch(request) {
                                Ok(result) => UnsafeLocalHelperToDaemon::Operation(result),
                                Err(error) => {
                                    eprintln!("unsafe-local launch failed: {error:?}");
                                    rejection(request_id, operation_id, failure_code(error))
                                }
                            };
                            if responses.send(QueuedResponse::ordinary(response)).is_ok() {
                                let _ = wake_response_loop(&response_wakeup);
                            }
                            active.fetch_sub(1, Ordering::AcqRel);
                        })
                        .map_err(|_| ProtocolError::RuntimeUnavailable)?;
                }
                DaemonToUnsafeLocalHelper::Shell(request) => {
                    if active.fetch_add(1, Ordering::AcqRel) >= MAX_HELPER_QUEUE_DEPTH {
                        active.fetch_sub(1, Ordering::AcqRel);
                        let request_id = request.request_id();
                        let operation_id = request.operation_id().clone();
                        send_frame(
                            &socket,
                            &rejection(request_id, operation_id, HelperFailureCode::QueueFull),
                        )?;
                        continue;
                    }
                    let runtime = Arc::clone(&self.runtime);
                    let responses = responses_tx.clone();
                    let response_wakeup = Arc::clone(&response_wakeup_write);
                    let active = Arc::clone(&active);
                    std::thread::Builder::new()
                        .name("d2b-unsafe-local-shell-operation".to_owned())
                        .spawn(move || {
                            let request_id = request.request_id();
                            let operation_id = request.operation_id().clone();
                            let queued = match runtime.shell(request) {
                                Ok(dispatch) => QueuedResponse {
                                    frame: dispatch.response,
                                    fd: dispatch.terminal_fd,
                                },
                                Err(error) => QueuedResponse::ordinary(rejection(
                                    request_id,
                                    operation_id,
                                    failure_code(error),
                                )),
                            };
                            if responses.send(queued).is_ok() {
                                let _ = wake_response_loop(&response_wakeup);
                            }
                            active.fetch_sub(1, Ordering::AcqRel);
                        })
                        .map_err(|_| ProtocolError::RuntimeUnavailable)?;
                }
                DaemonToUnsafeLocalHelper::HelloAccepted(_) => {
                    return Err(ProtocolError::ProtocolMismatch);
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControlEvent {
    Control,
    Response,
}

fn wait_for_control_or_response(
    socket: &Socket,
    response_wakeup: &UnixStream,
) -> Result<ControlEvent, ProtocolError> {
    let interests = PollFlags::POLLIN | PollFlags::POLLERR | PollFlags::POLLHUP;
    let mut fds = [
        PollFd::new(socket.as_fd(), interests),
        PollFd::new(response_wakeup.as_fd(), interests),
    ];
    loop {
        match poll(&mut fds, PollTimeout::NONE) {
            Ok(_) => break,
            Err(nix::errno::Errno::EINTR) => continue,
            Err(_) => return Err(ProtocolError::ConnectFailed),
        }
    }

    let response_events = fds[1].revents().unwrap_or(PollFlags::empty());
    if response_events.intersects(PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL) {
        return Err(ProtocolError::QueueClosed);
    }
    if response_events.contains(PollFlags::POLLIN) {
        drain_response_wakeup(response_wakeup)?;
        return Ok(ControlEvent::Response);
    }

    let control_events = fds[0].revents().unwrap_or(PollFlags::empty());
    if control_events.intersects(PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL) {
        return Err(ProtocolError::ConnectFailed);
    }
    if control_events.contains(PollFlags::POLLIN) {
        return Ok(ControlEvent::Control);
    }
    Err(ProtocolError::ConnectFailed)
}

fn wake_response_loop(response_wakeup: &UnixStream) -> Result<(), ProtocolError> {
    let mut response_wakeup = response_wakeup;
    loop {
        match response_wakeup.write_all(&[1]) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Err(ProtocolError::QueueClosed),
        }
    }
}

fn drain_response_wakeup(response_wakeup: &UnixStream) -> Result<(), ProtocolError> {
    let mut response_wakeup = response_wakeup;
    let mut buffer = [0u8; MAX_HELPER_QUEUE_DEPTH];
    loop {
        match response_wakeup.read(&mut buffer) {
            Ok(0) => return Err(ProtocolError::QueueClosed),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Err(ProtocolError::QueueClosed),
        }
    }
}

pub fn default_helper_socket_path() -> &'static Path {
    Path::new(UNSAFE_LOCAL_HELPER_SOCKET_PATH)
}

fn connect_control_socket(path: &Path, expected_daemon_uid: u32) -> Result<Socket, ProtocolError> {
    let socket = Socket::new(
        Domain::UNIX,
        Type::from(libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC),
        None,
    )
    .map_err(|_| ProtocolError::ConnectFailed)?;
    configure_socket_buffers(&socket)?;
    let address = SockAddr::unix(path).map_err(|_| ProtocolError::ConnectFailed)?;
    socket
        .connect(&address)
        .map_err(|_| ProtocolError::ConnectFailed)?;
    let peer =
        getsockopt(&socket, PeerCredentials).map_err(|_| ProtocolError::PeerCredentialMismatch)?;
    if peer.uid() as u32 != expected_daemon_uid || peer.uid() == 0 {
        return Err(ProtocolError::PeerCredentialMismatch);
    }
    Ok(socket)
}

pub fn configure_socket_buffers(socket: &Socket) -> Result<(), ProtocolError> {
    socket
        .set_send_buffer_size(HELPER_SOCKET_BUFFER_REQUEST_BYTES)
        .map_err(|_| ProtocolError::BufferTooSmall)?;
    socket
        .set_recv_buffer_size(HELPER_SOCKET_BUFFER_REQUEST_BYTES)
        .map_err(|_| ProtocolError::BufferTooSmall)?;
    let send_size = socket
        .send_buffer_size()
        .map_err(|_| ProtocolError::BufferTooSmall)?;
    let recv_size = socket
        .recv_buffer_size()
        .map_err(|_| ProtocolError::BufferTooSmall)?;
    if !effective_socket_buffers_sufficient(send_size, recv_size) {
        return Err(ProtocolError::BufferTooSmall);
    }
    Ok(())
}

fn effective_socket_buffers_sufficient(send_size: usize, recv_size: usize) -> bool {
    send_size >= MIN_EFFECTIVE_HELPER_SOCKET_BUFFER_BYTES
        && recv_size >= MIN_EFFECTIVE_HELPER_SOCKET_BUFFER_BYTES
}

pub fn send_frame<T: serde::Serialize>(socket: &Socket, frame: &T) -> Result<(), ProtocolError> {
    let payload = serde_json::to_vec(frame).map_err(|_| ProtocolError::InvalidFrame)?;
    if payload.len() > MAX_HELPER_FRAME_SIZE {
        return Err(ProtocolError::FrameTooLarge);
    }
    let length = u32::try_from(payload.len()).map_err(|_| ProtocolError::FrameTooLarge)?;
    let mut encoded = Vec::with_capacity(payload.len() + 4);
    encoded.extend_from_slice(&length.to_le_bytes());
    encoded.extend_from_slice(&payload);
    let sent = send(socket.as_raw_fd(), &encoded, MsgFlags::MSG_NOSIGNAL)
        .map_err(|_| ProtocolError::ConnectFailed)?;
    if sent != encoded.len() {
        return Err(ProtocolError::InvalidFrame);
    }
    Ok(())
}

fn send_queued_response(socket: &Socket, response: QueuedResponse) -> Result<(), ProtocolError> {
    match (&response.frame, response.fd) {
        (UnsafeLocalHelperToDaemon::TerminalReady(_), Some(fd)) => {
            send_frame_with_fd(socket, &response.frame, &fd)
        }
        (UnsafeLocalHelperToDaemon::TerminalReady(_), None) | (_, Some(_)) => {
            Err(ProtocolError::InvalidFrame)
        }
        (_, None) => send_frame(socket, &response.frame),
    }
}

fn send_frame_with_fd<T: serde::Serialize>(
    socket: &Socket,
    frame: &T,
    fd: &OwnedFd,
) -> Result<(), ProtocolError> {
    let payload = serde_json::to_vec(frame).map_err(|_| ProtocolError::InvalidFrame)?;
    if payload.len() > MAX_HELPER_FRAME_SIZE {
        return Err(ProtocolError::FrameTooLarge);
    }
    let length = u32::try_from(payload.len()).map_err(|_| ProtocolError::FrameTooLarge)?;
    let mut encoded = Vec::with_capacity(payload.len() + 4);
    encoded.extend_from_slice(&length.to_le_bytes());
    encoded.extend_from_slice(&payload);
    let iov = [IoSlice::new(&encoded)];
    let raw = [fd.as_raw_fd()];
    let control = [ControlMessage::ScmRights(&raw)];
    let sent = sendmsg::<UnixAddr>(
        socket.as_raw_fd(),
        &iov,
        &control,
        MsgFlags::MSG_NOSIGNAL,
        None,
    )
    .map_err(|_| ProtocolError::ConnectFailed)?;
    if sent != encoded.len() {
        return Err(ProtocolError::InvalidFrame);
    }
    Ok(())
}

pub fn receive_frame<T: serde::de::DeserializeOwned>(
    socket: &Socket,
    encoded: &mut [u8],
) -> Result<T, ProtocolError> {
    if encoded.len() < MAX_HELPER_FRAME_SIZE + 5 {
        return Err(ProtocolError::FrameTooLarge);
    }
    let mut iov = [IoSliceMut::new(encoded)];
    let mut control = cmsg_space!([RawFd; 2]);
    let message = recvmsg::<UnixAddr>(
        socket.as_raw_fd(),
        &mut iov,
        Some(&mut control),
        MsgFlags::MSG_CMSG_CLOEXEC,
    )
    .map_err(|_| ProtocolError::ConnectFailed)?;
    let read = message.bytes;
    if message
        .flags
        .intersects(MsgFlags::MSG_TRUNC | MsgFlags::MSG_CTRUNC)
    {
        return Err(ProtocolError::FrameTooLarge);
    }
    let mut received_fds = Vec::new();
    for control in message.cmsgs().map_err(|_| ProtocolError::InvalidFrame)? {
        if let ControlMessageOwned::ScmRights(fds) = control {
            received_fds.extend(fds);
        }
    }
    let had_fds = !received_fds.is_empty();
    for fd in received_fds {
        let _ = unistd::close(fd);
    }
    if had_fds {
        return Err(ProtocolError::InvalidFrame);
    }
    if read < 4 {
        return Err(ProtocolError::InvalidFrame);
    }
    let declared = u32::from_le_bytes(
        encoded[..4]
            .try_into()
            .map_err(|_| ProtocolError::InvalidFrame)?,
    ) as usize;
    if declared > MAX_HELPER_FRAME_SIZE {
        return Err(ProtocolError::FrameTooLarge);
    }
    if read != declared + 4 {
        return Err(ProtocolError::InvalidFrame);
    }
    serde_json::from_slice(&encoded[4..read]).map_err(|_| ProtocolError::InvalidFrame)
}

fn random_generation() -> Result<u64, ProtocolError> {
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes).map_err(|_| ProtocolError::RuntimeUnavailable)?;
    let generation = u64::from_le_bytes(bytes);
    Ok(if generation == 0 { 1 } else { generation })
}

fn rejection(
    request_id: u64,
    operation_id: OperationId,
    code: HelperFailureCode,
) -> UnsafeLocalHelperToDaemon {
    UnsafeLocalHelperToDaemon::Rejected(HelperOperationRejected {
        request_id,
        operation_id,
        code,
    })
}

fn failure_code(error: RuntimeError) -> HelperFailureCode {
    match error {
        RuntimeError::InvalidRequest | RuntimeError::InvalidIdentity => {
            HelperFailureCode::InvalidRequest
        }
        RuntimeError::UserManagerUnavailable => HelperFailureCode::UserManagerUnavailable,
        RuntimeError::EnvironmentInvalid | RuntimeError::LedgerInvalid => {
            HelperFailureCode::EnvironmentInvalid
        }
        RuntimeError::ExecutableUnavailable => HelperFailureCode::ExecutableUnavailable,
        RuntimeError::ProxyUnavailable => HelperFailureCode::ProxyUnavailable,
        RuntimeError::WaylandUnavailable => HelperFailureCode::WaylandUnavailable,
        RuntimeError::FirstClientTimeout => HelperFailureCode::FirstClientTimeout,
        RuntimeError::ScopeCreateFailed => HelperFailureCode::ScopeCreateFailed,
        RuntimeError::ScopeIdentityMismatch => HelperFailureCode::ScopeIdentityMismatch,
        RuntimeError::OperationIdConflict => HelperFailureCode::OperationIdConflict,
        RuntimeError::OperationInProgress | RuntimeError::QuotaExceeded => {
            HelperFailureCode::QueueFull
        }
        RuntimeError::ShellUnavailable => HelperFailureCode::ShellUnavailable,
        RuntimeError::ShellNotFound => HelperFailureCode::ShellNotFound,
        RuntimeError::ShellAlreadyAttached => HelperFailureCode::ShellAlreadyAttached,
        RuntimeError::TerminalClosed => HelperFailureCode::TerminalClosed,
        RuntimeError::Timeout => HelperFailureCode::Timeout,
        RuntimeError::Internal => HelperFailureCode::Internal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::fcntl::{FcntlArg, FdFlag, fcntl};
    use nix::sys::socket::{
        AddressFamily, ControlMessageOwned, SockFlag, SockType, recvmsg, socketpair,
    };
    use std::os::fd::{AsRawFd, OwnedFd};

    #[test]
    fn socket_buffers_meet_frozen_effective_minimum() {
        let (left, right) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .unwrap();
        let left = socket_from_owned(left);
        let right = socket_from_owned(right);
        for socket in [&left, &right] {
            let result = configure_socket_buffers(socket);
            let send_size = socket.send_buffer_size().unwrap();
            let recv_size = socket.recv_buffer_size().unwrap();
            let sufficient = effective_socket_buffers_sufficient(send_size, recv_size);
            assert_eq!(result.is_ok(), sufficient);
            if !sufficient {
                assert_eq!(result, Err(ProtocolError::BufferTooSmall));
            }
        }
    }

    #[test]
    fn socket_buffer_minimum_is_exact_and_two_sided() {
        let minimum = MIN_EFFECTIVE_HELPER_SOCKET_BUFFER_BYTES;
        assert!(effective_socket_buffers_sufficient(minimum, minimum));
        assert!(!effective_socket_buffers_sufficient(minimum - 1, minimum));
        assert!(!effective_socket_buffers_sufficient(minimum, minimum - 1));
    }

    fn socket_from_owned(fd: OwnedFd) -> Socket {
        Socket::from(fd)
    }

    fn terminal_ready_frame() -> UnsafeLocalHelperToDaemon {
        use d2b_contracts::public_wire::{ShellName, ShellSessionState};
        use d2b_contracts::unsafe_local_wire::{
            HelperScopeKind, HelperShellAttachResult, HelperTerminalReady, HelperTerminalTransport,
            ScopeIdentity, UNSAFE_LOCAL_TERMINAL_PROTOCOL_VERSION,
        };

        UnsafeLocalHelperToDaemon::TerminalReady(HelperTerminalReady {
            request_id: 1,
            operation_id: OperationId::parse("op-terminal").unwrap(),
            terminal_protocol_version: UNSAFE_LOCAL_TERMINAL_PROTOCOL_VERSION,
            transport: HelperTerminalTransport::ConnectedUnixStream,
            scope: ScopeIdentity {
                invocation_id: "opaque".to_owned(),
                kind: HelperScopeKind::PersistentShell,
            },
            result: HelperShellAttachResult {
                resolved_name: ShellName::new("host").unwrap(),
                state: ShellSessionState::Attached,
                force_evicted: false,
            },
        })
    }

    #[test]
    fn completed_operation_wakes_idle_control_loop() {
        let (control, _peer) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .unwrap();
        let control = socket_from_owned(control);
        let (wakeup_read, wakeup_write) = UnixStream::pair().unwrap();
        wakeup_read.set_nonblocking(true).unwrap();
        wakeup_write.set_nonblocking(true).unwrap();

        wake_response_loop(&wakeup_write).unwrap();

        assert_eq!(
            wait_for_control_or_response(&control, &wakeup_read).unwrap(),
            ControlEvent::Response
        );
    }

    #[test]
    fn terminal_ready_queue_sends_exactly_one_cloexec_fd() {
        let (sender, receiver) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .unwrap();
        let sender = socket_from_owned(sender);
        let (payload_read, _payload_write) =
            rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).unwrap();
        send_queued_response(
            &sender,
            QueuedResponse {
                frame: terminal_ready_frame(),
                fd: Some(payload_read),
            },
        )
        .unwrap();

        let mut encoded = vec![0u8; MAX_HELPER_FRAME_SIZE + 5];
        let mut iov = [IoSliceMut::new(&mut encoded)];
        let mut control = cmsg_space!([RawFd; 2]);
        let message = recvmsg::<UnixAddr>(
            receiver.as_raw_fd(),
            &mut iov,
            Some(&mut control),
            MsgFlags::MSG_CMSG_CLOEXEC,
        )
        .unwrap();
        let mut received = Vec::new();
        for cmsg in message.cmsgs().unwrap() {
            if let ControlMessageOwned::ScmRights(fds) = cmsg {
                received.extend(fds);
            }
        }
        assert_eq!(received.len(), 1);
        let flags = FdFlag::from_bits_truncate(fcntl(received[0], FcntlArg::F_GETFD).unwrap());
        assert!(flags.contains(FdFlag::FD_CLOEXEC));
        unistd::close(received[0]).unwrap();
    }

    #[test]
    fn ordinary_queue_response_sends_zero_fds() {
        let (sender, receiver) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .unwrap();
        let sender = socket_from_owned(sender);
        send_queued_response(
            &sender,
            QueuedResponse::ordinary(UnsafeLocalHelperToDaemon::Heartbeat(HelperHeartbeat {
                generation: 1,
                sequence: 2,
            })),
        )
        .unwrap();
        let mut encoded = vec![0u8; MAX_HELPER_FRAME_SIZE + 5];
        let mut iov = [IoSliceMut::new(&mut encoded)];
        let mut control = cmsg_space!([RawFd; 2]);
        let message = recvmsg::<UnixAddr>(
            receiver.as_raw_fd(),
            &mut iov,
            Some(&mut control),
            MsgFlags::MSG_CMSG_CLOEXEC,
        )
        .unwrap();
        let rights = message
            .cmsgs()
            .unwrap()
            .filter_map(|cmsg| match cmsg {
                ControlMessageOwned::ScmRights(fds) => Some(fds.len()),
                _ => None,
            })
            .sum::<usize>();
        assert_eq!(rights, 0);
    }

    #[test]
    fn queue_rejects_terminal_ready_without_its_one_fd() {
        let (sender, _receiver) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .unwrap();
        let sender = socket_from_owned(sender);
        assert_eq!(
            send_queued_response(&sender, QueuedResponse::ordinary(terminal_ready_frame())),
            Err(ProtocolError::InvalidFrame)
        );
    }

    #[test]
    fn failed_fd_send_drops_queue_ownership() {
        let (sender, receiver) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .unwrap();
        let sender = socket_from_owned(sender);
        drop(receiver);
        let (payload_read, _payload_write) =
            rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).unwrap();
        let raw = payload_read.as_raw_fd();
        let result = send_queued_response(
            &sender,
            QueuedResponse {
                frame: terminal_ready_frame(),
                fd: Some(payload_read),
            },
        );
        assert_eq!(result, Err(ProtocolError::ConnectFailed));
        assert!(!std::path::Path::new(&format!("/proc/self/fd/{raw}")).exists());
    }
}
