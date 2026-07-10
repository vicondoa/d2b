use crate::runtime::{RuntimeError, ScopeRuntime};
use crate::systemd::UserScopeManager;
use d2b_contracts::UNSAFE_LOCAL_HELPER_SOCKET_PATH;
use d2b_contracts::unsafe_local_wire::{
    DaemonToUnsafeLocalHelper, HELPER_SOCKET_BUFFER_REQUEST_BYTES, HelperFailureCode,
    HelperHeartbeat, HelperHello, HelperOperationRejected, MAX_HELPER_FRAME_SIZE,
    MAX_HELPER_QUEUE_DEPTH, MIN_EFFECTIVE_HELPER_SOCKET_BUFFER_BYTES,
    UNSAFE_LOCAL_HELPER_PROTOCOL_VERSION, UnsafeLocalHelperToDaemon,
};
use d2b_realm_core::ids::OperationId;
use nix::cmsg_space;
use nix::fcntl::{FcntlArg, FdFlag, fcntl};
use nix::libc;
use nix::sys::socket::{
    ControlMessageOwned, MsgFlags, UnixAddr, getsockopt, recvmsg, send, sockopt::PeerCredentials,
};
use nix::unistd;
use socket2::{Domain, SockAddr, Socket, Type};
use std::fmt;
use std::io::IoSliceMut;
use std::os::fd::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;
use uzers::get_user_by_name;

const CONTROL_READ_TIMEOUT: Duration = Duration::from_secs(20);

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

        let accepted: DaemonToUnsafeLocalHelper = receive_frame(&socket)?;
        match accepted {
            DaemonToUnsafeLocalHelper::HelloAccepted(accepted)
                if accepted.protocol_version == UNSAFE_LOCAL_HELPER_PROTOCOL_VERSION
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

        socket
            .set_read_timeout(Some(CONTROL_READ_TIMEOUT))
            .map_err(|_| ProtocolError::ConnectFailed)?;
        let (responses_tx, responses_rx) =
            mpsc::sync_channel::<UnsafeLocalHelperToDaemon>(MAX_HELPER_QUEUE_DEPTH);
        let active = Arc::new(AtomicUsize::new(0));

        loop {
            while let Ok(response) = responses_rx.try_recv() {
                send_frame(&socket, &response)?;
            }
            let frame: DaemonToUnsafeLocalHelper = match receive_frame(&socket) {
                Ok(frame) => frame,
                Err(ProtocolError::ConnectFailed) => continue,
                Err(error) => return Err(error),
            };
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
                    let active = Arc::clone(&active);
                    std::thread::Builder::new()
                        .name("d2b-unsafe-local-operation".to_owned())
                        .spawn(move || {
                            let request_id = request.request_id;
                            let operation_id = request.operation_id.clone();
                            let response = match runtime.launch(request) {
                                Ok(result) => UnsafeLocalHelperToDaemon::Operation(result),
                                Err(error) => {
                                    rejection(request_id, operation_id, failure_code(error))
                                }
                            };
                            let _ = responses.try_send(response);
                            active.fetch_sub(1, Ordering::AcqRel);
                        })
                        .map_err(|_| ProtocolError::RuntimeUnavailable)?;
                }
                DaemonToUnsafeLocalHelper::Shell(request) => {
                    if let Some((request_id, operation_id)) = shell_operation_identity(request) {
                        send_frame(
                            &socket,
                            &rejection(
                                request_id,
                                operation_id,
                                HelperFailureCode::ShellUnavailable,
                            ),
                        )?;
                    }
                }
                DaemonToUnsafeLocalHelper::HelloAccepted(_) => {
                    return Err(ProtocolError::ProtocolMismatch);
                }
            }
        }
    }
}

pub fn default_helper_socket_path() -> &'static Path {
    Path::new(UNSAFE_LOCAL_HELPER_SOCKET_PATH)
}

fn connect_control_socket(path: &Path, expected_daemon_uid: u32) -> Result<Socket, ProtocolError> {
    let socket = Socket::new(Domain::UNIX, Type::from(libc::SOCK_SEQPACKET), None)
        .map_err(|_| ProtocolError::ConnectFailed)?;
    mark_cloexec(&socket)?;
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
    if send_size < MIN_EFFECTIVE_HELPER_SOCKET_BUFFER_BYTES
        || recv_size < MIN_EFFECTIVE_HELPER_SOCKET_BUFFER_BYTES
    {
        return Err(ProtocolError::BufferTooSmall);
    }
    Ok(())
}

fn mark_cloexec(socket: &Socket) -> Result<(), ProtocolError> {
    fcntl(socket.as_raw_fd(), FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))
        .map_err(|_| ProtocolError::ConnectFailed)?;
    Ok(())
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

pub fn receive_frame<T: serde::de::DeserializeOwned>(socket: &Socket) -> Result<T, ProtocolError> {
    let mut encoded = vec![0u8; MAX_HELPER_FRAME_SIZE + 5];
    let mut iov = [IoSliceMut::new(&mut encoded)];
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
        RuntimeError::InvalidIdentity => HelperFailureCode::InvalidRequest,
        RuntimeError::UserManagerUnavailable => HelperFailureCode::UserManagerUnavailable,
        RuntimeError::EnvironmentInvalid | RuntimeError::LedgerInvalid => {
            HelperFailureCode::EnvironmentInvalid
        }
        RuntimeError::ExecutableUnavailable => HelperFailureCode::ExecutableUnavailable,
        RuntimeError::ProxyUnavailable => HelperFailureCode::ProxyUnavailable,
        RuntimeError::ScopeCreateFailed => HelperFailureCode::ScopeCreateFailed,
        RuntimeError::ScopeIdentityMismatch => HelperFailureCode::ScopeIdentityMismatch,
        RuntimeError::Timeout => HelperFailureCode::Timeout,
        RuntimeError::Internal => HelperFailureCode::Internal,
    }
}

fn shell_operation_identity(
    request: d2b_contracts::unsafe_local_wire::HelperShellRequest,
) -> Option<(u64, OperationId)> {
    use d2b_contracts::unsafe_local_wire::HelperShellRequest;
    match request {
        HelperShellRequest::List { .. } => None,
        HelperShellRequest::Attach {
            request_id,
            operation_id,
            ..
        }
        | HelperShellRequest::Detach {
            request_id,
            operation_id,
            ..
        }
        | HelperShellRequest::Kill {
            request_id,
            operation_id,
            ..
        } => Some((request_id, operation_id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::sys::socket::{AddressFamily, SockFlag, SockType, socketpair};
    use std::os::fd::OwnedFd;

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
        configure_socket_buffers(&left).unwrap();
        configure_socket_buffers(&right).unwrap();
        assert!(left.send_buffer_size().unwrap() >= MIN_EFFECTIVE_HELPER_SOCKET_BUFFER_BYTES);
        assert!(right.recv_buffer_size().unwrap() >= MIN_EFFECTIVE_HELPER_SOCKET_BUFFER_BYTES);
    }

    fn socket_from_owned(fd: OwnedFd) -> Socket {
        Socket::from(fd)
    }
}
