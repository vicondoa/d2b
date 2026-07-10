use d2b_contracts::unsafe_local_wire::{
    DaemonToUnsafeLocalHelper, HELPER_SOCKET_BUFFER_REQUEST_BYTES, HelperFailureCode,
    HelperHeartbeat, HelperHelloAccepted, HelperLaunchRequest, HelperOperationDisposition,
    HelperOperationRejected, HelperOperationResult, HelperSnapshot, HelperTerminalReady,
    MAX_COMPLETED_OPERATION_AGE_SECS, MAX_COMPLETED_OPERATIONS_PER_UID, MAX_HELPER_FRAME_SIZE,
    MAX_HELPER_QUEUE_DEPTH, MAX_HELPER_SNAPSHOT_SCOPES, MIN_EFFECTIVE_HELPER_SOCKET_BUFFER_BYTES,
    UNSAFE_LOCAL_HELPER_PROTOCOL_VERSION, UNSAFE_LOCAL_TERMINAL_FD_COUNT,
    UNSAFE_LOCAL_TERMINAL_PROTOCOL_VERSION, UnsafeLocalHelperToDaemon,
};
use nix::cmsg_space;
use nix::fcntl::{FcntlArg, FdFlag, fcntl};
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::socket::{
    ControlMessageOwned, MsgFlags, UnixAddr, getpeername, getsockopt, recvmsg, send,
    sockopt::{AcceptConn, PeerCredentials, SockType as SocketTypeOpt},
};
use nix::unistd::{self, Gid};
use parking_lot::Mutex;
use serde::Serialize;
use sha2::{Digest, Sha256};
use socket2::{Domain, SockAddr, Socket, Type};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::fs;
use std::io::{IoSliceMut, Read, Write};
use std::os::fd::{AsFd, AsRawFd, OwnedFd, RawFd};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const HELPER_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
pub const HELPER_STALE_AFTER: Duration = Duration::from_secs(15);
pub const HELPER_OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
const HELPER_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
const HELPER_LOOP_TICK: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelperRegistryError {
    UnauthorizedPeer,
    InvalidPeer,
    SocketBufferTooSmall,
    InvalidFrame,
    FrameTooLarge,
    ProtocolMismatch,
    SnapshotTooLarge,
    HelperUnavailable,
    HelperStale,
    QueueFull,
    Timeout,
    GenerationSuperseded,
    RequestCorrelationMismatch,
    OperationIdConflict,
    OperationInProgress,
    OperationRejected(HelperFailureCode),
    InvalidTerminalFd,
    Io,
}

pub enum HelperReply {
    Operation(HelperOperationResult),
    Rejected(HelperOperationRejected),
    Terminal {
        ready: HelperTerminalReady,
        fd: OwnedFd,
    },
}

impl fmt::Debug for HelperReply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Operation(result) => f.debug_tuple("Operation").field(result).finish(),
            Self::Rejected(rejected) => f.debug_tuple("Rejected").field(rejected).finish(),
            Self::Terminal { ready, .. } => f
                .debug_struct("Terminal")
                .field("ready", ready)
                .field("fd", &"<redacted>")
                .finish(),
        }
    }
}

struct PendingRequest {
    operation_id: String,
    sender: mpsc::SyncSender<Result<HelperReply, HelperRegistryError>>,
}

struct HelperConnection {
    generation: u64,
    socket: Arc<Socket>,
    outbound: mpsc::SyncSender<DaemonToUnsafeLocalHelper>,
    outbound_wakeup: Arc<UnixStream>,
    pending: Mutex<HashMap<u64, PendingRequest>>,
    last_heartbeat_millis: AtomicU64,
    connected_at: Instant,
    closed: AtomicBool,
}

impl HelperConnection {
    fn touch(&self) {
        self.last_heartbeat_millis.store(
            self.connected_at
                .elapsed()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64,
            Ordering::Release,
        );
    }

    fn is_stale(&self) -> bool {
        let seen = self.last_heartbeat_millis.load(Ordering::Acquire);
        self.connected_at
            .elapsed()
            .saturating_sub(Duration::from_millis(seen))
            > HELPER_STALE_AFTER
    }

    fn close(&self, reason: HelperRegistryError) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        let _ = self.socket.shutdown(std::net::Shutdown::Both);
        let pending = std::mem::take(&mut *self.pending.lock());
        for (_, request) in pending {
            let _ = request.sender.try_send(Err(reason));
        }
    }

    fn queue_outbound(&self, frame: DaemonToUnsafeLocalHelper) -> Result<(), HelperRegistryError> {
        self.outbound
            .try_send(frame)
            .map_err(|_| HelperRegistryError::QueueFull)?;
        if signal_outbound(&self.outbound_wakeup).is_err() {
            self.close(HelperRegistryError::Io);
            return Err(HelperRegistryError::Io);
        }
        Ok(())
    }
}

#[derive(Default)]
struct RegistryState {
    connections: HashMap<u32, Arc<HelperConnection>>,
    snapshots: HashMap<u32, HelperSnapshot>,
}

pub struct HelperRegistry {
    daemon_uid: u32,
    allowed_uids: HashSet<u32>,
    state: Mutex<RegistryState>,
    operations: Mutex<OperationLedger>,
}

impl fmt::Debug for HelperRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state.lock();
        f.debug_struct("HelperRegistry")
            .field("allowed_uid_count", &self.allowed_uids.len())
            .field("active_helper_count", &state.connections.len())
            .finish()
    }
}

impl HelperRegistry {
    pub fn new(daemon_uid: u32, allowed_uids: impl IntoIterator<Item = u32>) -> Self {
        Self {
            daemon_uid,
            allowed_uids: allowed_uids.into_iter().collect(),
            state: Mutex::new(RegistryState::default()),
            operations: Mutex::new(OperationLedger::default()),
        }
    }

    pub fn accept_loop(self: Arc<Self>, listener: Socket) {
        loop {
            match listener.accept() {
                Ok((socket, _)) => {
                    let registry = Arc::clone(&self);
                    let _ = std::thread::Builder::new()
                        .name("d2b-unsafe-local-helper".to_owned())
                        .spawn(move || {
                            if registry.handle_socket(socket).is_err() {
                                tracing::warn!(
                                    provider = "unsafe-local",
                                    event_kind = "helper-register",
                                    result = "rejected",
                                    "unsafe-local helper connection rejected"
                                );
                            }
                        });
                }
                Err(error) => {
                    tracing::error!(
                        provider = "unsafe-local",
                        event_kind = "helper-accept",
                        result = "failed",
                        error_kind = ?error.kind(),
                        "unsafe-local helper accept failed"
                    );
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }

    pub fn active_generation(&self, uid: u32) -> Option<u64> {
        self.state
            .lock()
            .connections
            .get(&uid)
            .filter(|connection| !connection.closed.load(Ordering::Acquire))
            .map(|connection| connection.generation)
    }

    pub fn snapshot(&self, uid: u32) -> Option<HelperSnapshot> {
        self.state.lock().snapshots.get(&uid).cloned()
    }

    pub fn dispatch_launch(
        &self,
        requester_uid: u32,
        request: HelperLaunchRequest,
    ) -> Result<HelperOperationResult, HelperRegistryError> {
        let fingerprint = launch_fingerprint(&request)?;
        let operation_key = request.operation_id.to_string();
        let request_id = request.request_id;
        match self.operations.lock().begin(
            requester_uid,
            operation_key.clone(),
            fingerprint,
            now_epoch_seconds(),
        )? {
            LedgerBegin::Completed(mut result) => {
                result.request_id = request.request_id;
                result.disposition = HelperOperationDisposition::AlreadyCommitted;
                return Ok(result);
            }
            LedgerBegin::Rejected(code) => {
                return Err(HelperRegistryError::OperationRejected(code));
            }
            LedgerBegin::Started => {}
        }

        let connection = match self.state.lock().connections.get(&requester_uid).cloned() {
            Some(connection) => connection,
            None => {
                self.operations
                    .lock()
                    .abort_active(requester_uid, &operation_key);
                return Err(HelperRegistryError::HelperUnavailable);
            }
        };
        if connection.closed.load(Ordering::Acquire) {
            self.operations
                .lock()
                .abort_active(requester_uid, &operation_key);
            return Err(HelperRegistryError::HelperUnavailable);
        }
        if connection.is_stale() {
            return Err(HelperRegistryError::HelperStale);
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        {
            let mut pending = connection.pending.lock();
            if pending.len() >= MAX_HELPER_QUEUE_DEPTH {
                self.operations
                    .lock()
                    .abort_active(requester_uid, &operation_key);
                return Err(HelperRegistryError::QueueFull);
            }
            if pending
                .insert(
                    request.request_id,
                    PendingRequest {
                        operation_id: operation_key.clone(),
                        sender,
                    },
                )
                .is_some()
            {
                self.operations
                    .lock()
                    .abort_active(requester_uid, &operation_key);
                return Err(HelperRegistryError::RequestCorrelationMismatch);
            }
        }
        if let Err(error) = connection.queue_outbound(DaemonToUnsafeLocalHelper::Launch(request)) {
            connection.pending.lock().remove(&request_id);
            self.operations
                .lock()
                .abort_active(requester_uid, &operation_key);
            return Err(error);
        }

        match receiver.recv_timeout(HELPER_OPERATION_TIMEOUT) {
            Ok(Ok(HelperReply::Operation(result))) => {
                self.operations.lock().complete(
                    requester_uid,
                    &operation_key,
                    result.clone(),
                    now_epoch_seconds(),
                );
                Ok(result)
            }
            Ok(Ok(HelperReply::Rejected(rejected))) => {
                self.operations.lock().reject(
                    requester_uid,
                    &operation_key,
                    rejected.code,
                    now_epoch_seconds(),
                );
                Err(HelperRegistryError::OperationRejected(rejected.code))
            }
            Ok(Ok(HelperReply::Terminal { .. })) => {
                Err(HelperRegistryError::RequestCorrelationMismatch)
            }
            Ok(Err(error)) => Err(error),
            Err(_) => Err(HelperRegistryError::Timeout),
        }
    }

    fn handle_socket(&self, socket: Socket) -> Result<(), HelperRegistryError> {
        let peer =
            getsockopt(&socket, PeerCredentials).map_err(|_| HelperRegistryError::InvalidPeer)?;
        let uid = peer.uid() as u32;
        if uid == 0 || uid == self.daemon_uid || !self.allowed_uids.contains(&uid) {
            return Err(HelperRegistryError::UnauthorizedPeer);
        }
        configure_socket_buffers(&socket)?;
        socket
            .set_write_timeout(Some(HELPER_LOOP_TICK))
            .map_err(|_| HelperRegistryError::Io)?;
        socket
            .set_read_timeout(Some(HELPER_HANDSHAKE_TIMEOUT))
            .map_err(|_| HelperRegistryError::Io)?;

        let mut receive_buffer = vec![0u8; MAX_HELPER_FRAME_SIZE + 5];
        let (hello, fds) =
            receive_frame::<UnsafeLocalHelperToDaemon>(&socket, &mut receive_buffer)?;
        reject_unexpected_fds(fds)?;
        let UnsafeLocalHelperToDaemon::Hello(hello) = hello else {
            return Err(HelperRegistryError::ProtocolMismatch);
        };
        if hello.protocol_version != UNSAFE_LOCAL_HELPER_PROTOCOL_VERSION || hello.generation == 0 {
            return Err(HelperRegistryError::ProtocolMismatch);
        }
        send_frame(
            &socket,
            &DaemonToUnsafeLocalHelper::HelloAccepted(HelperHelloAccepted {
                protocol_version: UNSAFE_LOCAL_HELPER_PROTOCOL_VERSION,
                generation: hello.generation,
                heartbeat_interval_secs: HELPER_HEARTBEAT_INTERVAL.as_secs() as u32,
                operation_timeout_secs: HELPER_OPERATION_TIMEOUT.as_secs() as u32,
            }),
        )?;
        let (snapshot, fds) =
            receive_frame::<UnsafeLocalHelperToDaemon>(&socket, &mut receive_buffer)?;
        reject_unexpected_fds(fds)?;
        let UnsafeLocalHelperToDaemon::Snapshot(snapshot) = snapshot else {
            return Err(HelperRegistryError::ProtocolMismatch);
        };
        if snapshot.generation != hello.generation {
            return Err(HelperRegistryError::ProtocolMismatch);
        }
        if snapshot.scopes.len() > MAX_HELPER_SNAPSHOT_SCOPES {
            return Err(HelperRegistryError::SnapshotTooLarge);
        }

        socket
            .set_read_timeout(None)
            .map_err(|_| HelperRegistryError::Io)?;
        let socket = Arc::new(socket);
        let (outbound, outbound_rx) = mpsc::sync_channel(MAX_HELPER_QUEUE_DEPTH);
        let (outbound_wakeup_read, outbound_wakeup_write) =
            UnixStream::pair().map_err(|_| HelperRegistryError::Io)?;
        outbound_wakeup_read
            .set_nonblocking(true)
            .map_err(|_| HelperRegistryError::Io)?;
        outbound_wakeup_write
            .set_nonblocking(true)
            .map_err(|_| HelperRegistryError::Io)?;
        let connection = Arc::new(HelperConnection {
            generation: hello.generation,
            socket: Arc::clone(&socket),
            outbound,
            outbound_wakeup: Arc::new(outbound_wakeup_write),
            pending: Mutex::new(HashMap::new()),
            last_heartbeat_millis: AtomicU64::new(0),
            connected_at: Instant::now(),
            closed: AtomicBool::new(false),
        });
        connection.touch();

        let replaced = {
            let mut state = self.state.lock();
            state.snapshots.insert(uid, snapshot.clone());
            state.connections.insert(uid, Arc::clone(&connection))
        };
        if let Some(replaced) = replaced {
            replaced.close(HelperRegistryError::GenerationSuperseded);
            tracing::info!(
                provider = "unsafe-local",
                event_kind = "helper-supersede",
                result = "success",
                "unsafe-local helper generation superseded"
            );
        } else {
            tracing::info!(
                provider = "unsafe-local",
                event_kind = "helper-register",
                result = "success",
                "unsafe-local helper registered"
            );
        }
        self.operations
            .lock()
            .adopt_snapshot(uid, &snapshot, now_epoch_seconds());

        let result = self.connection_loop(
            uid,
            &connection,
            outbound_rx,
            &outbound_wakeup_read,
            &mut receive_buffer,
        );
        let mut state = self.state.lock();
        if state
            .connections
            .get(&uid)
            .is_some_and(|active| Arc::ptr_eq(active, &connection))
        {
            state.connections.remove(&uid);
        }
        drop(state);
        connection.close(HelperRegistryError::HelperUnavailable);
        result
    }

    fn connection_loop(
        &self,
        uid: u32,
        connection: &Arc<HelperConnection>,
        outbound: mpsc::Receiver<DaemonToUnsafeLocalHelper>,
        outbound_wakeup: &UnixStream,
        receive_buffer: &mut [u8],
    ) -> Result<(), HelperRegistryError> {
        let mut heartbeat_sequence = 0u64;
        let mut next_heartbeat = Instant::now() + HELPER_HEARTBEAT_INTERVAL;
        loop {
            if connection.closed.load(Ordering::Acquire) {
                return Err(HelperRegistryError::GenerationSuperseded);
            }
            while let Ok(frame) = outbound.try_recv() {
                send_frame(&connection.socket, &frame)?;
            }
            if Instant::now() >= next_heartbeat {
                heartbeat_sequence = heartbeat_sequence.wrapping_add(1);
                send_frame(
                    &connection.socket,
                    &DaemonToUnsafeLocalHelper::Heartbeat(HelperHeartbeat {
                        generation: connection.generation,
                        sequence: heartbeat_sequence,
                    }),
                )?;
                next_heartbeat = Instant::now() + HELPER_HEARTBEAT_INTERVAL;
            }
            if connection.is_stale() {
                tracing::warn!(
                    provider = "unsafe-local",
                    event_kind = "helper-stale",
                    result = "stale",
                    "unsafe-local helper heartbeat stale"
                );
                return Err(HelperRegistryError::HelperStale);
            }

            match wait_for_connection_event(&connection.socket, outbound_wakeup)? {
                ConnectionEvent::Outbound => continue,
                ConnectionEvent::Tick => continue,
                ConnectionEvent::Incoming => {}
            }
            match receive_frame::<UnsafeLocalHelperToDaemon>(&connection.socket, receive_buffer) {
                Ok((frame, fds)) => {
                    if !self.is_active_generation(uid, connection) {
                        close_raw_fds(fds);
                        return Err(HelperRegistryError::GenerationSuperseded);
                    }
                    connection.touch();
                    self.handle_incoming(connection, frame, fds)?;
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn is_active_generation(&self, uid: u32, connection: &Arc<HelperConnection>) -> bool {
        self.state
            .lock()
            .connections
            .get(&uid)
            .is_some_and(|active| Arc::ptr_eq(active, connection))
    }

    fn handle_incoming(
        &self,
        connection: &HelperConnection,
        frame: UnsafeLocalHelperToDaemon,
        fds: Vec<RawFd>,
    ) -> Result<(), HelperRegistryError> {
        match frame {
            UnsafeLocalHelperToDaemon::Heartbeat(heartbeat) => {
                reject_unexpected_fds(fds)?;
                if heartbeat.generation != connection.generation {
                    return Err(HelperRegistryError::GenerationSuperseded);
                }
                Ok(())
            }
            UnsafeLocalHelperToDaemon::Operation(result) => {
                reject_unexpected_fds(fds)?;
                complete_pending(
                    connection,
                    result.request_id,
                    result.operation_id.to_string(),
                    HelperReply::Operation(result),
                )
            }
            UnsafeLocalHelperToDaemon::Rejected(rejected) => {
                reject_unexpected_fds(fds)?;
                complete_pending(
                    connection,
                    rejected.request_id,
                    rejected.operation_id.to_string(),
                    HelperReply::Rejected(rejected),
                )
            }
            UnsafeLocalHelperToDaemon::TerminalReady(ready) => {
                let fd = validate_terminal_fd(&ready, fds)?;
                complete_pending(
                    connection,
                    ready.request_id,
                    ready.operation_id.to_string(),
                    HelperReply::Terminal { ready, fd },
                )
            }
            UnsafeLocalHelperToDaemon::Hello(_) | UnsafeLocalHelperToDaemon::Snapshot(_) => {
                reject_unexpected_fds(fds)?;
                Err(HelperRegistryError::ProtocolMismatch)
            }
        }
    }
}

fn complete_pending(
    connection: &HelperConnection,
    request_id: u64,
    operation_id: String,
    reply: HelperReply,
) -> Result<(), HelperRegistryError> {
    let Some(pending) = connection.pending.lock().remove(&request_id) else {
        return Err(HelperRegistryError::RequestCorrelationMismatch);
    };
    if pending.operation_id != operation_id {
        let _ = pending
            .sender
            .try_send(Err(HelperRegistryError::RequestCorrelationMismatch));
        return Err(HelperRegistryError::RequestCorrelationMismatch);
    }
    pending
        .sender
        .try_send(Ok(reply))
        .map_err(|_| HelperRegistryError::RequestCorrelationMismatch)
}

pub fn bind_helper_socket(
    path: &Path,
    socket_gid: Gid,
    set_group: bool,
) -> Result<Socket, HelperRegistryError> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_socket() {
            fs::remove_file(path).map_err(|_| HelperRegistryError::Io)?;
        } else {
            return Err(HelperRegistryError::Io);
        }
    }
    let socket = Socket::new(
        Domain::UNIX,
        Type::from(libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC),
        None,
    )
    .map_err(|_| HelperRegistryError::Io)?;
    let address = SockAddr::unix(path).map_err(|_| HelperRegistryError::Io)?;
    socket.bind(&address).map_err(|_| HelperRegistryError::Io)?;
    socket.listen(128).map_err(|_| HelperRegistryError::Io)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o660))
        .map_err(|_| HelperRegistryError::Io)?;
    if set_group {
        unistd::chown(path, None, Some(socket_gid)).map_err(|_| HelperRegistryError::Io)?;
    }
    Ok(socket)
}

fn configure_socket_buffers(socket: &Socket) -> Result<(), HelperRegistryError> {
    socket
        .set_send_buffer_size(HELPER_SOCKET_BUFFER_REQUEST_BYTES)
        .map_err(|_| HelperRegistryError::SocketBufferTooSmall)?;
    socket
        .set_recv_buffer_size(HELPER_SOCKET_BUFFER_REQUEST_BYTES)
        .map_err(|_| HelperRegistryError::SocketBufferTooSmall)?;
    let send = socket
        .send_buffer_size()
        .map_err(|_| HelperRegistryError::SocketBufferTooSmall)?;
    let receive = socket
        .recv_buffer_size()
        .map_err(|_| HelperRegistryError::SocketBufferTooSmall)?;
    if !effective_socket_buffers_sufficient(send, receive) {
        return Err(HelperRegistryError::SocketBufferTooSmall);
    }
    Ok(())
}

fn effective_socket_buffers_sufficient(send: usize, receive: usize) -> bool {
    send >= MIN_EFFECTIVE_HELPER_SOCKET_BUFFER_BYTES
        && receive >= MIN_EFFECTIVE_HELPER_SOCKET_BUFFER_BYTES
}

fn send_frame<T: Serialize>(socket: &Socket, frame: &T) -> Result<(), HelperRegistryError> {
    let payload = serde_json::to_vec(frame).map_err(|_| HelperRegistryError::InvalidFrame)?;
    if payload.len() > MAX_HELPER_FRAME_SIZE {
        return Err(HelperRegistryError::FrameTooLarge);
    }
    let length = u32::try_from(payload.len()).map_err(|_| HelperRegistryError::FrameTooLarge)?;
    let mut encoded = Vec::with_capacity(payload.len() + 4);
    encoded.extend_from_slice(&length.to_le_bytes());
    encoded.extend_from_slice(&payload);
    let sent = send(socket.as_raw_fd(), &encoded, MsgFlags::MSG_NOSIGNAL)
        .map_err(|_| HelperRegistryError::Io)?;
    if sent != encoded.len() {
        return Err(HelperRegistryError::InvalidFrame);
    }
    Ok(())
}

fn receive_frame<T: serde::de::DeserializeOwned>(
    socket: &Socket,
    encoded: &mut [u8],
) -> Result<(T, Vec<RawFd>), HelperRegistryError> {
    if encoded.len() < MAX_HELPER_FRAME_SIZE + 5 {
        return Err(HelperRegistryError::FrameTooLarge);
    }
    let mut iov = [IoSliceMut::new(encoded)];
    let mut control = cmsg_space!([RawFd; 2]);
    let message = match recvmsg::<UnixAddr>(
        socket.as_raw_fd(),
        &mut iov,
        Some(&mut control),
        MsgFlags::MSG_CMSG_CLOEXEC,
    ) {
        Ok(message) => message,
        Err(_) => return Err(HelperRegistryError::Io),
    };
    let read = message.bytes;
    if read == 0 {
        return Err(HelperRegistryError::Io);
    }
    if message
        .flags
        .intersects(MsgFlags::MSG_TRUNC | MsgFlags::MSG_CTRUNC)
    {
        return Err(HelperRegistryError::FrameTooLarge);
    }
    let mut fds = Vec::new();
    for control in message
        .cmsgs()
        .map_err(|_| HelperRegistryError::InvalidFrame)?
    {
        if let ControlMessageOwned::ScmRights(rights) = control {
            fds.extend(rights);
        }
    }
    if read < 4 {
        close_raw_fds(fds);
        return Err(HelperRegistryError::InvalidFrame);
    }
    let declared = u32::from_le_bytes(
        encoded[..4]
            .try_into()
            .map_err(|_| HelperRegistryError::InvalidFrame)?,
    ) as usize;
    if declared > MAX_HELPER_FRAME_SIZE {
        close_raw_fds(fds);
        return Err(HelperRegistryError::FrameTooLarge);
    }
    if read != declared + 4 {
        close_raw_fds(fds);
        return Err(HelperRegistryError::InvalidFrame);
    }
    let frame = serde_json::from_slice(&encoded[4..read]).map_err(|_| {
        close_raw_fds(fds.clone());
        HelperRegistryError::InvalidFrame
    })?;
    Ok((frame, fds))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionEvent {
    Incoming,
    Outbound,
    Tick,
}

fn wait_for_connection_event(
    socket: &Socket,
    outbound_wakeup: &UnixStream,
) -> Result<ConnectionEvent, HelperRegistryError> {
    let interests = PollFlags::POLLIN | PollFlags::POLLERR | PollFlags::POLLHUP;
    let mut fds = [
        PollFd::new(socket.as_fd(), interests),
        PollFd::new(outbound_wakeup.as_fd(), interests),
    ];
    let timeout = PollTimeout::try_from(HELPER_LOOP_TICK).map_err(|_| HelperRegistryError::Io)?;
    loop {
        match poll(&mut fds, timeout) {
            Ok(0) => return Ok(ConnectionEvent::Tick),
            Ok(_) => break,
            Err(nix::errno::Errno::EINTR) => continue,
            Err(_) => return Err(HelperRegistryError::Io),
        }
    }

    let outbound_events = fds[1].revents().unwrap_or(PollFlags::empty());
    if outbound_events.intersects(PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL) {
        return Err(HelperRegistryError::Io);
    }
    if outbound_events.contains(PollFlags::POLLIN) {
        drain_outbound_wakeup(outbound_wakeup)?;
        return Ok(ConnectionEvent::Outbound);
    }

    let socket_events = fds[0].revents().unwrap_or(PollFlags::empty());
    if socket_events.intersects(PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL) {
        return Err(HelperRegistryError::Io);
    }
    if socket_events.contains(PollFlags::POLLIN) {
        return Ok(ConnectionEvent::Incoming);
    }
    Err(HelperRegistryError::Io)
}

fn signal_outbound(outbound_wakeup: &UnixStream) -> Result<(), HelperRegistryError> {
    let mut outbound_wakeup = outbound_wakeup;
    loop {
        match outbound_wakeup.write_all(&[1]) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Err(HelperRegistryError::Io),
        }
    }
}

fn drain_outbound_wakeup(outbound_wakeup: &UnixStream) -> Result<(), HelperRegistryError> {
    let mut outbound_wakeup = outbound_wakeup;
    let mut buffer = [0u8; MAX_HELPER_QUEUE_DEPTH];
    loop {
        match outbound_wakeup.read(&mut buffer) {
            Ok(0) => return Err(HelperRegistryError::Io),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Err(HelperRegistryError::Io),
        }
    }
}

fn validate_terminal_fd(
    ready: &HelperTerminalReady,
    fds: Vec<RawFd>,
) -> Result<OwnedFd, HelperRegistryError> {
    if ready.terminal_protocol_version != UNSAFE_LOCAL_TERMINAL_PROTOCOL_VERSION
        || fds.len() != UNSAFE_LOCAL_TERMINAL_FD_COUNT
    {
        close_raw_fds(fds);
        return Err(HelperRegistryError::InvalidTerminalFd);
    }
    let raw = fds[0];
    let duplicated = duplicate_received_fd(raw)?;
    close_raw_fds(fds);
    let flags = fcntl(duplicated.as_raw_fd(), FcntlArg::F_GETFD)
        .map_err(|_| HelperRegistryError::InvalidTerminalFd)?;
    if !FdFlag::from_bits_truncate(flags).contains(FdFlag::FD_CLOEXEC)
        || getsockopt(&duplicated, SocketTypeOpt)
            .map_err(|_| HelperRegistryError::InvalidTerminalFd)?
            != nix::sys::socket::SockType::Stream
        || getsockopt(&duplicated, AcceptConn)
            .map_err(|_| HelperRegistryError::InvalidTerminalFd)?
        || getpeername::<UnixAddr>(duplicated.as_raw_fd()).is_err()
    {
        return Err(HelperRegistryError::InvalidTerminalFd);
    }
    Ok(duplicated)
}

fn duplicate_received_fd(raw: RawFd) -> Result<OwnedFd, HelperRegistryError> {
    let pid = rustix::process::Pid::from_raw(std::process::id() as i32)
        .ok_or(HelperRegistryError::InvalidTerminalFd)?;
    let pidfd = rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty())
        .map_err(|_| HelperRegistryError::InvalidTerminalFd)?;
    let duplicated =
        rustix::process::pidfd_getfd(&pidfd, raw, rustix::process::PidfdGetfdFlags::empty())
            .map_err(|_| HelperRegistryError::InvalidTerminalFd)?;
    fcntl(
        duplicated.as_raw_fd(),
        FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC),
    )
    .map_err(|_| HelperRegistryError::InvalidTerminalFd)?;
    Ok(duplicated)
}

fn close_raw_fds(fds: Vec<RawFd>) {
    for fd in fds {
        let _ = unistd::close(fd);
    }
}

fn reject_unexpected_fds(fds: Vec<RawFd>) -> Result<(), HelperRegistryError> {
    let unexpected = !fds.is_empty();
    close_raw_fds(fds);
    if unexpected {
        Err(HelperRegistryError::InvalidTerminalFd)
    } else {
        Ok(())
    }
}

#[derive(Clone)]
enum LedgerState {
    Active,
    Completed {
        result: HelperOperationResult,
        completed_at: u64,
    },
    Rejected {
        code: HelperFailureCode,
        completed_at: u64,
    },
    Adopted {
        completed_at: u64,
    },
}

#[derive(Clone)]
struct LedgerEntry {
    fingerprint: Option<[u8; 32]>,
    state: LedgerState,
}

#[derive(Default)]
struct OperationLedger {
    by_uid: BTreeMap<u32, HashMap<String, LedgerEntry>>,
}

enum LedgerBegin {
    Started,
    Completed(HelperOperationResult),
    Rejected(HelperFailureCode),
}

impl OperationLedger {
    fn begin(
        &mut self,
        uid: u32,
        operation_id: String,
        fingerprint: [u8; 32],
        now: u64,
    ) -> Result<LedgerBegin, HelperRegistryError> {
        self.reap_uid(uid, now);
        let entries = self.by_uid.entry(uid).or_default();
        if let Some(entry) = entries.get(&operation_id) {
            if entry.fingerprint != Some(fingerprint) {
                return Err(HelperRegistryError::OperationIdConflict);
            }
            return match &entry.state {
                LedgerState::Active => Err(HelperRegistryError::OperationInProgress),
                LedgerState::Completed { result, .. } => Ok(LedgerBegin::Completed(result.clone())),
                LedgerState::Rejected { code, .. } => Ok(LedgerBegin::Rejected(*code)),
                LedgerState::Adopted { .. } => Err(HelperRegistryError::OperationIdConflict),
            };
        }
        entries.insert(
            operation_id,
            LedgerEntry {
                fingerprint: Some(fingerprint),
                state: LedgerState::Active,
            },
        );
        Ok(LedgerBegin::Started)
    }

    fn complete(&mut self, uid: u32, operation_id: &str, result: HelperOperationResult, now: u64) {
        if let Some(entry) = self
            .by_uid
            .get_mut(&uid)
            .and_then(|entries| entries.get_mut(operation_id))
        {
            entry.state = LedgerState::Completed {
                result,
                completed_at: now,
            };
        }
        self.reap_uid(uid, now);
    }

    fn abort_active(&mut self, uid: u32, operation_id: &str) {
        let Some(entries) = self.by_uid.get_mut(&uid) else {
            return;
        };
        if entries
            .get(operation_id)
            .is_some_and(|entry| matches!(entry.state, LedgerState::Active))
        {
            entries.remove(operation_id);
        }
    }

    fn reject(&mut self, uid: u32, operation_id: &str, code: HelperFailureCode, now: u64) {
        if let Some(entry) = self
            .by_uid
            .get_mut(&uid)
            .and_then(|entries| entries.get_mut(operation_id))
        {
            entry.state = LedgerState::Rejected {
                code,
                completed_at: now,
            };
        }
        self.reap_uid(uid, now);
    }

    fn adopt_snapshot(&mut self, uid: u32, snapshot: &HelperSnapshot, now: u64) {
        let entries = self.by_uid.entry(uid).or_default();
        for scope in &snapshot.scopes {
            let operation_id = scope.operation_id.to_string();
            let adopted_result = HelperOperationResult {
                request_id: 0,
                operation_id: scope.operation_id.clone(),
                disposition: HelperOperationDisposition::AlreadyCommitted,
                scope: Some(scope.scope.clone()),
            };
            match entries.entry(operation_id) {
                std::collections::hash_map::Entry::Occupied(mut occupied) => {
                    if matches!(occupied.get().state, LedgerState::Active) {
                        occupied.get_mut().state = LedgerState::Completed {
                            result: adopted_result,
                            completed_at: now,
                        };
                    }
                }
                std::collections::hash_map::Entry::Vacant(vacant) => {
                    vacant.insert(LedgerEntry {
                        fingerprint: None,
                        state: LedgerState::Adopted { completed_at: now },
                    });
                }
            }
        }
        self.reap_uid(uid, now);
    }

    fn reap_uid(&mut self, uid: u32, now: u64) {
        let Some(entries) = self.by_uid.get_mut(&uid) else {
            return;
        };
        entries.retain(|_, entry| match &entry.state {
            LedgerState::Active => true,
            LedgerState::Completed { completed_at, .. }
            | LedgerState::Rejected { completed_at, .. }
            | LedgerState::Adopted { completed_at, .. } => {
                now.saturating_sub(*completed_at) < MAX_COMPLETED_OPERATION_AGE_SECS
            }
        });
        let mut completed: Vec<(String, u64)> = entries
            .iter()
            .filter_map(|(id, entry)| match &entry.state {
                LedgerState::Active => None,
                LedgerState::Completed { completed_at, .. }
                | LedgerState::Rejected { completed_at, .. }
                | LedgerState::Adopted { completed_at, .. } => Some((id.clone(), *completed_at)),
            })
            .collect();
        if completed.len() > MAX_COMPLETED_OPERATIONS_PER_UID {
            completed.sort_by_key(|(id, completed_at)| (*completed_at, id.clone()));
            let excess = completed
                .len()
                .saturating_sub(MAX_COMPLETED_OPERATIONS_PER_UID);
            for (id, _) in completed.into_iter().take(excess) {
                entries.remove(&id);
            }
        }
    }
}

fn launch_fingerprint(request: &HelperLaunchRequest) -> Result<[u8; 32], HelperRegistryError> {
    let encoded = serde_json::to_vec(&(
        &request.workload,
        &request.item_id,
        &request.argv,
        request.graphical,
    ))
    .map_err(|_| HelperRegistryError::InvalidFrame)?;
    Ok(Sha256::digest(encoded).into())
}

fn now_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_core::configured_argv::ConfiguredArgv;
    use d2b_realm_core::{ids::OperationId, token::ProtocolToken};
    use nix::sys::socket::{AddressFamily, SockFlag, socketpair};
    use std::os::fd::OwnedFd;

    fn launch(request_id: u64, operation_id: &str, arg: &str) -> HelperLaunchRequest {
        HelperLaunchRequest {
            request_id,
            operation_id: OperationId::parse(operation_id).unwrap(),
            workload: serde_json::from_value(serde_json::json!({
                "workloadId": "tools",
                "realmId": "host",
                "realmPath": ["host"],
                "canonicalTarget": "tools.host.d2b"
            }))
            .unwrap(),
            item_id: ProtocolToken::parse("browser").unwrap(),
            argv: ConfiguredArgv::new(vec![arg.to_owned()]).unwrap(),
            graphical: false,
        }
    }

    fn completed(request: &HelperLaunchRequest) -> HelperOperationResult {
        HelperOperationResult {
            request_id: request.request_id,
            operation_id: request.operation_id.clone(),
            disposition: HelperOperationDisposition::Committed,
            scope: None,
        }
    }

    #[test]
    fn queued_launch_wakes_idle_connection_loop() {
        let (control, _peer) = seqpacket_pair();
        let (wakeup_read, wakeup_write) = UnixStream::pair().unwrap();
        wakeup_read.set_nonblocking(true).unwrap();
        wakeup_write.set_nonblocking(true).unwrap();

        signal_outbound(&wakeup_write).unwrap();

        assert_eq!(
            wait_for_connection_event(&control, &wakeup_read).unwrap(),
            ConnectionEvent::Outbound
        );
    }

    #[test]
    fn bound_helper_listener_is_close_on_exec() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("helper.sock");
        let socket = bind_helper_socket(&path, unistd::getgid(), false).unwrap();
        let flags = fcntl(socket.as_raw_fd(), FcntlArg::F_GETFD).unwrap();

        assert!(FdFlag::from_bits_truncate(flags).contains(FdFlag::FD_CLOEXEC));
    }

    #[test]
    fn socket_buffer_minimum_is_exact_and_two_sided() {
        let minimum = MIN_EFFECTIVE_HELPER_SOCKET_BUFFER_BYTES;
        assert!(effective_socket_buffers_sufficient(minimum, minimum));
        assert!(!effective_socket_buffers_sufficient(minimum - 1, minimum));
        assert!(!effective_socket_buffers_sufficient(minimum, minimum - 1));
    }

    #[test]
    fn operation_ledger_rejects_reuse_with_new_fingerprint() {
        let mut ledger = OperationLedger::default();
        let first = launch(1, "op-1", "first");
        assert!(matches!(
            ledger
                .begin(
                    1000,
                    "op-1".to_owned(),
                    launch_fingerprint(&first).unwrap(),
                    1
                )
                .unwrap(),
            LedgerBegin::Started
        ));
        ledger.complete(1000, "op-1", completed(&first), 2);
        let different = launch(2, "op-1", "different");
        assert!(matches!(
            ledger.begin(
                1000,
                "op-1".to_owned(),
                launch_fingerprint(&different).unwrap(),
                3
            ),
            Err(HelperRegistryError::OperationIdConflict)
        ));
    }

    #[test]
    fn operation_ledger_never_reaps_active_entries() {
        let mut ledger = OperationLedger::default();
        let active = launch(1, "active-op", "program");
        ledger
            .begin(
                1000,
                "active-op".to_owned(),
                launch_fingerprint(&active).unwrap(),
                0,
            )
            .unwrap();
        for index in 0..=MAX_COMPLETED_OPERATIONS_PER_UID {
            let request = launch(index as u64 + 2, &format!("done-{index}"), "program");
            let key = request.operation_id.to_string();
            ledger
                .begin(
                    1000,
                    key.clone(),
                    launch_fingerprint(&request).unwrap(),
                    index as u64 + 1,
                )
                .unwrap();
            ledger.complete(1000, &key, completed(&request), index as u64 + 1);
        }
        let entries = ledger.by_uid.get(&1000).unwrap();
        assert!(matches!(
            entries.get("active-op").map(|entry| &entry.state),
            Some(LedgerState::Active)
        ));
        assert!(
            entries
                .values()
                .filter(|entry| !matches!(entry.state, LedgerState::Active))
                .count()
                <= MAX_COMPLETED_OPERATIONS_PER_UID
        );
    }

    #[test]
    fn reconnect_snapshot_reconciles_an_ambiguous_active_launch() {
        let mut ledger = OperationLedger::default();
        let request = launch(1, "op-reconcile", "program");
        let fingerprint = launch_fingerprint(&request).unwrap();
        ledger
            .begin(1000, "op-reconcile".to_owned(), fingerprint, 1)
            .unwrap();
        ledger.adopt_snapshot(
            1000,
            &HelperSnapshot {
                generation: 2,
                scopes: vec![d2b_contracts::unsafe_local_wire::HelperScopeSnapshot {
                    operation_id: request.operation_id.clone(),
                    workload: request.workload.clone(),
                    scope: d2b_contracts::unsafe_local_wire::ScopeIdentity {
                        invocation_id: "00112233445566778899aabbccddeeff".to_owned(),
                        kind: d2b_contracts::unsafe_local_wire::HelperScopeKind::LauncherApp,
                    },
                    state: d2b_contracts::unsafe_local_wire::HelperScopeState::Active,
                }],
            },
            2,
        );
        assert!(matches!(
            ledger.begin(1000, "op-reconcile".to_owned(), fingerprint, 3),
            Ok(LedgerBegin::Completed(HelperOperationResult {
                disposition: HelperOperationDisposition::AlreadyCommitted,
                ..
            }))
        ));
    }

    #[test]
    fn registry_never_selects_a_different_uid_helper() {
        let registry = HelperRegistry::new(42, [1000, 1001]);
        assert!(registry.active_generation(1000).is_none());
        assert!(registry.active_generation(1001).is_none());
        assert!(!registry.allowed_uids.contains(&0));
        assert!(!registry.allowed_uids.contains(&42));
    }

    #[test]
    fn debug_surfaces_hide_uid_and_fingerprint_material() {
        let registry = HelperRegistry::new(42, [1000]);
        let debug = format!("{registry:?}");
        assert!(!debug.contains("1000"));
        assert!(!debug.contains("42"));
        let request = launch(1, "op-1", "argv-private-canary");
        let fingerprint = launch_fingerprint(&request).unwrap();
        assert!(!format!("{fingerprint:?}").contains("argv-private-canary"));
    }

    #[test]
    fn same_uid_helper_registers_and_reconnect_supersedes_generation() {
        if unistd::getuid().is_root() {
            return;
        }
        if !host_supports_helper_socket_buffers() {
            return;
        }
        let uid = unistd::getuid().as_raw();
        let registry = Arc::new(HelperRegistry::new(uid.wrapping_add(1), [uid]));
        let first = register_helper(Arc::clone(&registry), 11);
        assert_eq!(registry.active_generation(uid), Some(11));
        let second = register_helper(Arc::clone(&registry), 12);
        assert_eq!(registry.active_generation(uid), Some(12));
        drop(first);
        drop(second);
    }

    #[test]
    fn helper_registration_rejects_uid_outside_eligibility_set() {
        if unistd::getuid().is_root() {
            return;
        }
        let uid = unistd::getuid().as_raw();
        let registry = HelperRegistry::new(uid.wrapping_add(1), []);
        let (server, client) = seqpacket_pair();
        drop(client);
        assert_eq!(
            registry.handle_socket(server),
            Err(HelperRegistryError::UnauthorizedPeer)
        );
    }

    #[test]
    fn registered_helper_dispatches_correlated_launch_without_uid_in_frame() {
        if unistd::getuid().is_root() {
            return;
        }
        if !host_supports_helper_socket_buffers() {
            return;
        }
        let uid = unistd::getuid().as_raw();
        let registry = Arc::new(HelperRegistry::new(uid.wrapping_add(1), [uid]));
        let helper = register_helper(Arc::clone(&registry), 21);
        let request = launch(91, "op-correlated", "/bin/true");
        let expected_operation = request.operation_id.clone();
        let dispatch_registry = Arc::clone(&registry);
        let dispatch = std::thread::spawn(move || dispatch_registry.dispatch_launch(uid, request));
        let mut receive_buffer = vec![0u8; MAX_HELPER_FRAME_SIZE + 5];

        loop {
            let (frame, fds) =
                receive_frame::<DaemonToUnsafeLocalHelper>(&helper, &mut receive_buffer)
                    .expect("daemon request");
            close_raw_fds(fds);
            match frame {
                DaemonToUnsafeLocalHelper::Launch(request) => {
                    assert_eq!(request.request_id, 91);
                    send_frame(
                        &helper,
                        &UnsafeLocalHelperToDaemon::Operation(HelperOperationResult {
                            request_id: request.request_id,
                            operation_id: request.operation_id,
                            disposition: HelperOperationDisposition::Committed,
                            scope: None,
                        }),
                    )
                    .unwrap();
                    break;
                }
                DaemonToUnsafeLocalHelper::Heartbeat(heartbeat) => {
                    send_frame(&helper, &UnsafeLocalHelperToDaemon::Heartbeat(heartbeat)).unwrap();
                }
                other => panic!("unexpected daemon frame: {other:?}"),
            }
        }
        let result = dispatch.join().unwrap().unwrap();
        assert_eq!(result.operation_id, expected_operation);
    }

    #[test]
    fn queue_saturation_does_not_leave_operation_replayable_as_active() {
        if unistd::getuid().is_root() {
            return;
        }
        let uid = unistd::getuid().as_raw();
        let registry = HelperRegistry::new(uid.wrapping_add(1), [uid]);
        let (socket, peer) = seqpacket_pair();
        let (outbound, _outbound_rx) = mpsc::sync_channel(1);
        let (_outbound_wakeup_read, outbound_wakeup_write) = UnixStream::pair().unwrap();
        outbound
            .try_send(DaemonToUnsafeLocalHelper::Heartbeat(HelperHeartbeat {
                generation: 1,
                sequence: 1,
            }))
            .unwrap();
        let connection = Arc::new(HelperConnection {
            generation: 1,
            socket: Arc::new(socket),
            outbound,
            outbound_wakeup: Arc::new(outbound_wakeup_write),
            pending: Mutex::new(HashMap::new()),
            last_heartbeat_millis: AtomicU64::new(0),
            connected_at: Instant::now(),
            closed: AtomicBool::new(false),
        });
        registry.state.lock().connections.insert(uid, connection);
        let request = launch(1, "queue-op", "/bin/true");
        assert_eq!(
            registry.dispatch_launch(uid, request.clone()),
            Err(HelperRegistryError::QueueFull)
        );
        assert_eq!(
            registry.dispatch_launch(uid, request),
            Err(HelperRegistryError::QueueFull)
        );
        drop(peer);
    }

    #[test]
    fn heartbeat_age_marks_helper_stale_without_global_cleanup() {
        let (socket, peer) = seqpacket_pair();
        let (outbound, _receiver) = mpsc::sync_channel(1);
        let (_outbound_wakeup_read, outbound_wakeup_write) = UnixStream::pair().unwrap();
        let connection = HelperConnection {
            generation: 1,
            socket: Arc::new(socket),
            outbound,
            outbound_wakeup: Arc::new(outbound_wakeup_write),
            pending: Mutex::new(HashMap::new()),
            last_heartbeat_millis: AtomicU64::new(0),
            connected_at: Instant::now() - HELPER_STALE_AFTER - Duration::from_millis(1),
            closed: AtomicBool::new(false),
        };
        assert!(connection.is_stale());
        drop(peer);
    }

    #[test]
    fn terminal_fd_validation_accepts_only_connected_cloexec_stream() {
        let (stream, peer): (OwnedFd, OwnedFd) = socketpair(
            AddressFamily::Unix,
            nix::sys::socket::SockType::Stream,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .unwrap();
        let raw = unistd::dup(stream.as_raw_fd()).unwrap();
        let ready = HelperTerminalReady {
            request_id: 1,
            operation_id: OperationId::parse("op-terminal").unwrap(),
            terminal_protocol_version: UNSAFE_LOCAL_TERMINAL_PROTOCOL_VERSION,
            transport:
                d2b_contracts::unsafe_local_wire::HelperTerminalTransport::ConnectedUnixStream,
            scope: d2b_contracts::unsafe_local_wire::ScopeIdentity {
                invocation_id: "00112233445566778899aabbccddeeff".to_owned(),
                kind: d2b_contracts::unsafe_local_wire::HelperScopeKind::PersistentShell,
            },
        };
        let validated = validate_terminal_fd(&ready, vec![raw]).unwrap();
        let flags = fcntl(validated.as_raw_fd(), FcntlArg::F_GETFD).unwrap();
        assert!(FdFlag::from_bits_truncate(flags).contains(FdFlag::FD_CLOEXEC));
        drop(peer);

        let (datagram, datagram_peer): (OwnedFd, OwnedFd) = socketpair(
            AddressFamily::Unix,
            nix::sys::socket::SockType::Datagram,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .unwrap();
        let raw = unistd::dup(datagram.as_raw_fd()).unwrap();
        assert!(matches!(
            validate_terminal_fd(&ready, vec![raw]),
            Err(HelperRegistryError::InvalidTerminalFd)
        ));
        drop(datagram_peer);
    }

    fn register_helper(registry: Arc<HelperRegistry>, generation: u64) -> Socket {
        let uid = unistd::getuid().as_raw();
        let (server, client) = seqpacket_pair();
        let server_registry = Arc::clone(&registry);
        std::thread::spawn(move || {
            let _ = server_registry.handle_socket(server);
        });
        configure_socket_buffers(&client).unwrap();
        send_frame(
            &client,
            &UnsafeLocalHelperToDaemon::Hello(d2b_contracts::unsafe_local_wire::HelperHello {
                protocol_version: UNSAFE_LOCAL_HELPER_PROTOCOL_VERSION,
                generation,
                features: Vec::new(),
            }),
        )
        .unwrap();
        let mut receive_buffer = vec![0u8; MAX_HELPER_FRAME_SIZE + 5];
        let (accepted, fds) =
            receive_frame::<DaemonToUnsafeLocalHelper>(&client, &mut receive_buffer)
                .expect("hello accepted");
        close_raw_fds(fds);
        assert!(matches!(
            accepted,
            DaemonToUnsafeLocalHelper::HelloAccepted(HelperHelloAccepted {
                generation: observed,
                ..
            }) if observed == generation
        ));
        send_frame(
            &client,
            &UnsafeLocalHelperToDaemon::Snapshot(HelperSnapshot {
                generation,
                scopes: Vec::new(),
            }),
        )
        .unwrap();
        for _ in 0..100 {
            if registry.active_generation(uid) == Some(generation) {
                return client;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("helper generation did not register");
    }

    fn seqpacket_pair() -> (Socket, Socket) {
        let (left, right): (OwnedFd, OwnedFd) = socketpair(
            AddressFamily::Unix,
            nix::sys::socket::SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .unwrap();
        (Socket::from(left), Socket::from(right))
    }

    fn host_supports_helper_socket_buffers() -> bool {
        let (left, right) = seqpacket_pair();
        configure_socket_buffers(&left).is_ok() && configure_socket_buffers(&right).is_ok()
    }
}
