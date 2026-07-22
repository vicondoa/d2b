use async_trait::async_trait;
use d2b_contracts::{
    unsafe_local_wire::{HelperScopeKind, HelperScopeState},
    v2_component_session::{
        AttachmentKind, AttachmentPolicy, AttachmentPolicyKind, AttachmentPurpose, EndpointPolicy,
        EndpointPurpose, EndpointRole, IdentityEvidenceRequirement, KernelObjectType, LimitProfile,
        Locality, NoiseProfile, PurposeClass, ServicePackage, TransportBinding, TransportClass,
    },
    v2_services::{
        SERVICE_INVENTORY,
        common::{
            CancelOutcome as WireCancelOutcome, CancelRequest as WireCancelRequest,
            CancelResponse as WireCancelResponse, DesiredState as WireDesiredState, ErrorEnvelope,
            ErrorKind, Outcome, RetryClass, ServiceRequest, ServiceResponse,
        },
        runtime_systemd_user_ttrpc::{
            RuntimeSystemdUserService as RuntimeSystemdUserTtrpc,
            create_runtime_systemd_user_service,
        },
        service_schema_fingerprint,
        shell_ttrpc::{ShellService as ShellTtrpc, create_shell_service},
        tty_ttrpc::{TtyService as TtyTtrpc, create_tty_service},
    },
};
use d2b_session::{
    ComponentSessionDriver, HandshakeCredentials, OwnedAttachment, SessionEngine,
    serve_ttrpc_services,
};
use d2b_session_unix::{
    ActivatedSeqpacketListeners, CreditPool, CreditScopeSet, DescriptorPolicyResolver,
    PeerIdentityPolicy, SeqpacketSocket, UnixAttachmentPayload, UnixSeqpacketTransport,
    UnixSessionError,
};
use nix::unistd::geteuid;
use protobuf::{EnumOrUnknown, MessageField};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    fmt,
    future::Future,
    os::fd::OwnedFd,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::task::JoinSet;
use ttrpc::r#async::TtrpcContext;

use crate::{
    controller_allowlist::ControllerAllowlist,
    services::{
        AuthenticatedRuntimeSession, CompositionError, RuntimeComposition,
        runtime_systemd_user::{
            AuthenticatedTerminalAttachment, CancelRequest, CancelResult,
            ConfiguredProcessResolver, DesiredState, ResolvedProcess, RuntimeMethod, RuntimeOwner,
            RuntimeProcessState, RuntimeRequest, RuntimeResource, RuntimeServiceError,
            SystemdUserRuntimePort, WaylandControlPort, WaylandDisplayLease,
        },
    },
    shell_runtime::{
        AuthenticatedSystemdUserRuntime,
        AuthenticatedTerminalAttachment as ShellTerminalAttachment,
        CancelOutcome as ShellCancelOutcome, ScopeInspection, ScopeOwnership, ScopeProcessState,
        ShellMethod, ShellOwner, ShellRequest, ShellServiceError, VerifiedTransientScope,
    },
    systemd::{ScopeError, SystemdUserScopeManager, UserScopeManager, VerifiedScope},
    tty_exec::{
        TransientUserScope, TtyOneShotError, TtyOneShotRequest, TtyOneShotRuntime, TtyOneShotSpec,
        ValidatedTerminal,
    },
};

pub const ACTIVATED_LISTENER_NAME: &str = "runtime-systemd-user";
const MAX_ACTIVE_SESSIONS: usize = 64;
const SHELL_OUTPUT_BUDGET: usize = crate::output_ring::MAX_TOTAL_RING_BYTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerError {
    Activation,
    InvalidIdentity,
    Generation,
    Signal,
    Allowlist,
}

impl fmt::Display for ServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Activation => "socket-activation-failed",
            Self::InvalidIdentity => "runtime-identity-invalid",
            Self::Generation => "runtime-generation-unavailable",
            Self::Signal => "shutdown-signal-unavailable",
            Self::Allowlist => "controller-allowlist-invalid",
        })
    }
}

impl std::error::Error for ServerError {}

/// Immutable Nix-owned document naming the exact, bounded set of enabled
/// host-local realm controller UIDs authorized to reach this requesting
/// user's endpoint (see `nixos-modules/unsafe-local-helper.nix`). Absent
/// means no wiring has been provisioned yet; the helper then falls back to
/// the safe same-uid-only default rather than failing to start.
const CONTROLLER_ALLOWLIST_ENV: &str = "D2B_UNSAFE_LOCAL_CONTROLLER_ALLOWLIST";

fn load_controller_allowlist(uid: u32) -> Result<ControllerAllowlist, ServerError> {
    let Some(path) = std::env::var_os(CONTROLLER_ALLOWLIST_ENV) else {
        return Ok(ControllerAllowlist::empty());
    };
    // The allowlist document is keyed by this process's own username, never
    // by anything peer-supplied, so a connecting peer can never select
    // which row authorizes it.
    let username = uzers::get_user_by_uid(uid)
        .and_then(|user| user.name().to_str().map(str::to_owned))
        .ok_or(ServerError::Allowlist)?;
    let document = std::fs::read(&path).map_err(|_| ServerError::Allowlist)?;
    // Stable controller *principal names* (for example `"d2bd"`, the
    // local-root controller's own account, whose uid NixOS allocates
    // dynamically rather than fixing at eval time) are resolved to their
    // exact current uid exactly once, right here at startup — never
    // per-connection, and never from anything peer-supplied.
    let principal_uid_lookup = |name: &str| uzers::get_user_by_name(name).map(|user| user.uid());
    ControllerAllowlist::resolve(&document, &username, &principal_uid_lookup)
        .map_err(|_| ServerError::Allowlist)
}

pub async fn run() -> Result<(), ServerError> {
    let uid = geteuid().as_raw();
    if uid == 0 || uid != nix::unistd::getuid().as_raw() {
        return Err(ServerError::InvalidIdentity);
    }
    let allowlist = load_controller_allowlist(uid)?;
    let generation = random_generation()?;
    let listeners = ActivatedSeqpacketListeners::from_systemd(&[ACTIVATED_LISTENER_NAME])
        .map_err(|_| ServerError::Activation)?;
    let shutdown = async {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .map_err(|_| ServerError::Signal)?;
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
                .map_err(|_| ServerError::Signal)?;
        tokio::select! {
            _ = terminate.recv() => Ok(()),
            _ = interrupt.recv() => Ok(()),
        }
    };
    serve_until_shutdown(&listeners, generation, &allowlist, shutdown).await
}

fn random_generation() -> Result<u64, ServerError> {
    let mut bytes = [0_u8; 8];
    getrandom::getrandom(&mut bytes).map_err(|_| ServerError::Generation)?;
    let generation = u64::from_ne_bytes(bytes);
    Ok(generation.max(1))
}

#[async_trait]
trait ActivatedListener {
    async fn accept(&self) -> Result<SeqpacketSocket, ServerError>;
}

#[async_trait]
impl ActivatedListener for ActivatedSeqpacketListeners {
    async fn accept(&self) -> Result<SeqpacketSocket, ServerError> {
        self.accept(ACTIVATED_LISTENER_NAME)
            .await
            .map_err(|_| ServerError::Activation)
    }
}

async fn serve_until_shutdown<L, S>(
    listener: &L,
    generation: u64,
    allowlist: &ControllerAllowlist,
    shutdown: S,
) -> Result<(), ServerError>
where
    L: ActivatedListener + Sync,
    S: Future<Output = Result<(), ServerError>>,
{
    tokio::pin!(shutdown);
    let mut sessions = JoinSet::new();
    // Constructed once for the whole process lifetime, not per accepted
    // connection: `PtyState` (the real shells/runtimes and their PTY master
    // fds) and the lazily-built `Composition` (the frozen registries of
    // verified transient scopes) must survive a client disconnect so a
    // reconnecting controller can list/inspect/attach the same resources
    // rather than finding an empty, freshly reconstructed responder.
    let state = Arc::new(SessionServices::new(generation));
    loop {
        tokio::select! {
            result = &mut shutdown => {
                result?;
                sessions.abort_all();
                while sessions.join_next().await.is_some() {}
                return Ok(());
            }
            accepted = listener.accept(), if sessions.len() < MAX_ACTIVE_SESSIONS => {
                let socket = accepted?;
                let allowlist = allowlist.clone();
                let state = Arc::clone(&state);
                sessions.spawn(async move {
                    let _ = serve_socket(socket, generation, allowlist, state).await;
                });
            }
            completed = sessions.join_next(), if !sessions.is_empty() => {
                let _ = completed;
            }
        }
    }
}

/// Whether an already-authenticated peer uid may open a session on this
/// endpoint. This is a pure boolean decision: it never selects, returns, or
/// otherwise influences which uid anything executes as. The helper always
/// executes as `own_uid` (enforced once, in `run`); this only gates which
/// *other* connecting uid is additionally trusted to reach it.
fn peer_is_authorized(peer_uid: u32, own_uid: u32, allowlist: &ControllerAllowlist) -> bool {
    peer_uid != 0 && own_uid != 0 && (peer_uid == own_uid || allowlist.contains(peer_uid))
}

/// One accepted connection on the runtime-agent seqpacket listener becomes
/// exactly one authenticated `RuntimeSystemdUser` `ComponentSession` here,
/// and that session is deliberately the *only* transport-level boundary in
/// this responder — never one session per service, per resource, or per
/// attach. [`service_registry`] multiplexes the generated
/// `runtime-systemd-user`, `shell`, and `tty` ttrpc service packages onto
/// that single session (matching the deployed service composition this
/// helper actually ships), so the session's scope is *identity and
/// generation*: which authenticated peer uid is talking, under which
/// channel-bound session generation, for as long as this one connection
/// lasts. It is emphatically not resource scope — a session carries no
/// per-shell/per-runtime state of its own.
///
/// The co-located services, by contrast, own real resource lifetime:
/// [`SessionServices`]'s `pty_state`/`composition` are constructed once for
/// the whole process (see `serve_until_shutdown`) and outlive any single
/// session, precisely so a client that reconnects (a *new* session, same
/// process) can still `List`/`Inspect`/`Attach` the shells and runtime
/// scopes an earlier, now-gone session created. A session ending (this
/// function returning) never tears down those resources; only an explicit
/// `Kill` does.
async fn serve_socket(
    socket: SeqpacketSocket,
    generation: u64,
    allowlist: ControllerAllowlist,
    state: Arc<SessionServices>,
) -> Result<(), ()> {
    let peer = socket.acceptor_peer_credentials().map_err(|_| ())?;
    let uid = geteuid().as_raw();
    if !peer_is_authorized(peer.uid().as_raw(), uid, &allowlist) {
        return Err(());
    }
    let gid = peer.gid().as_raw();
    let policy = endpoint_policy(uid, gid, generation).ok_or(())?;
    let expected_peer = peer;
    let resolver: DescriptorPolicyResolver =
        Arc::new(|_| Err(UnixSessionError::DescriptorMismatch));
    let transport = UnixSeqpacketTransport::new(
        socket,
        Locality::HostLocal,
        policy.limits,
        policy.attachment_policy,
        credit_scopes(policy.attachment_policy.max_per_session),
        resolver,
        PeerIdentityPolicy::accepted(expected_peer),
    )
    .map_err(|_| ())?;
    let engine = SessionEngine::establish_responder(
        transport,
        policy,
        HandshakeCredentials::Nn,
        Instant::now(),
    )
    .await
    .map_err(|_| ())?;
    let driver: Arc<dyn ComponentSessionDriver> = Arc::new(engine.into_driver());
    state.set_driver(Arc::clone(&driver));
    let services = service_registry(&state);
    let result = serve_ttrpc_services(driver, services).await.map_err(|_| ());
    state.clear_driver();
    result
}

pub fn endpoint_policy(uid: u32, gid: u32, generation: u64) -> Option<EndpointPolicy> {
    if uid == 0 || generation == 0 {
        return None;
    }
    let service = SERVICE_INVENTORY.iter().find(|service| {
        service.package == "d2b.runtime.systemd-user.v2"
            && service.service == "RuntimeSystemdUserService"
    })?;
    Some(EndpointPolicy {
        purpose: EndpointPurpose::RuntimeSystemdUser,
        purpose_class: PurposeClass::Local,
        initiator_role: EndpointRole::LocalRootController,
        responder_role: EndpointRole::RuntimeSystemdUserAgent,
        service: ServicePackage::RuntimeSystemdUserV2,
        schema_fingerprint: service_schema_fingerprint(service),
        noise_profile: NoiseProfile::Nn25519ChaChaPolySha256,
        limits: LimitProfile::local_default(),
        transport_binding: TransportBinding {
            transport: TransportClass::UnixSeqpacket,
            locality: Locality::HostLocal,
            channel_binding: channel_binding(uid, gid),
            identity_evidence: IdentityEvidenceRequirement::DirectionalUnix,
        },
        reconnect_generation: generation,
        attachment_policy: AttachmentPolicy {
            kind: AttachmentPolicyKind::PacketAtomic,
            max_per_packet: 1,
            max_per_request: 1,
            max_per_operation: 1,
            max_per_session: 64,
            credentials_allowed: false,
        },
    })
}

pub fn channel_binding(uid: u32, gid: u32) -> [u8; 32] {
    d2b_contracts::v2_component_session::runtime_systemd_user_channel_binding(uid, gid)
}

fn credit_scopes(limit: u16) -> CreditScopeSet {
    let limit = usize::from(limit.max(1));
    CreditScopeSet::new(
        CreditPool::new(limit).expect("positive packet credit"),
        CreditPool::new(limit).expect("positive request credit"),
        CreditPool::new(limit).expect("positive operation credit"),
        CreditPool::new(limit).expect("positive session credit"),
        CreditPool::new(limit).expect("positive process credit"),
        CreditPool::new(limit).expect("positive host credit"),
    )
}

/// Opaque, fixed per-service-method attachment binding for the
/// `RuntimeSystemdUser` service's `OpenTerminal` RPC. Shell's equivalent
/// binding (`ATTACH_METHOD_ID`) lives in `supervisor_protocol.rs`, which is
/// not an owned file for this seam; this constant plays the identical role
/// for the one method that needs it here. Its exact numeric value carries no
/// meaning beyond being fixed and distinct from zero.
const RUNTIME_OPEN_TERMINAL_METHOD_ID: u32 = 0x4F70_5431;

/// Bounded history of request ids this process has observed reach a
/// terminal (`Ok` or `Err`) dispatch outcome. `RuntimeComposition` exposes no
/// `cancel_shell` method, and true in-flight preemptive cancellation is not
/// observable because every dispatch is serialized behind the composition
/// mutex; the honest, non-fabricated answer the dedicated `Cancel` RPCs can
/// give is "already terminal" (this request finished) or "unknown" (it never
/// reached us, or we have no record). See `RealBackend` below.
const MAX_COMPLETED_REQUESTS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PtyError {
    AllocationFailed,
    SpawnFailed,
    ShellUnavailable,
}

/// One supervised PTY-backed process: the real systemd-user scope backing a
/// shell or runtime terminal resource, its PTY master end, and (once
/// attached) the background byte pump relaying it to the client's socket.
struct PtyEntry {
    pty_master: OwnedFd,
    child: Child,
    scope: VerifiedScope,
    pump: Option<Pump>,
}

/// Shared state behind [`RealBackend`]. Held both by the backend instance
/// itself (reached only through the frozen trait methods dispatched via
/// `RuntimeComposition`) and directly by [`SessionServices`] (reached by the
/// ttrpc adapters for outbound attachment/pump wiring that the frozen trait
/// signatures have no hook for). Both views are clones of the same `Arc`, so
/// mutations through either are immediately visible to the other.
#[derive(Default)]
struct PtyState {
    shells: BTreeMap<String, PtyEntry>,
    runtimes: BTreeMap<String, PtyEntry>,
    completed_requests: VecDeque<[u8; 16]>,
}

impl PtyState {
    fn record_completed(&mut self, request_id: [u8; 16]) {
        if self.completed_requests.len() >= MAX_COMPLETED_REQUESTS {
            self.completed_requests.pop_front();
        }
        self.completed_requests.push_back(request_id);
    }

    fn is_completed(&self, request_id: [u8; 16]) -> bool {
        self.completed_requests.contains(&request_id)
    }
}

/// Background thread relaying bytes between a PTY master and the client's
/// attached socket. Stopping it (`stop_and_join`) both ends the relay and
/// drops the pump's own socket duplicate, so the client observes end-of-file
/// exactly when the server detaches — whether that detach was client-driven
/// (hangup) or an explicit `Detach`/`Kill` RPC.
struct Pump {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Pump {
    fn stop_and_join(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Pump {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn dup_owned(fd: &OwnedFd) -> Result<OwnedFd, PtyError> {
    rustix::io::fcntl_dupfd_cloexec(fd, 0).map_err(|_| PtyError::AllocationFailed)
}

fn open_pty_pair() -> Result<(OwnedFd, std::path::PathBuf), PtyError> {
    use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
    let master =
        openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).map_err(|_| PtyError::AllocationFailed)?;
    grantpt(&master).map_err(|_| PtyError::AllocationFailed)?;
    unlockpt(&master).map_err(|_| PtyError::AllocationFailed)?;
    let name = ptsname(&master, Vec::new()).map_err(|_| PtyError::AllocationFailed)?;
    let path = std::path::PathBuf::from(
        name.to_str()
            .map_err(|_| PtyError::AllocationFailed)?
            .to_owned(),
    );
    Ok((master, path))
}

fn open_pty_slave(path: &std::path::Path) -> Result<OwnedFd, PtyError> {
    rustix::fs::open(path, rustix::fs::OFlags::RDWR, rustix::fs::Mode::empty())
        .map_err(|_| PtyError::AllocationFailed)
}

fn resolve_login_shell(uid: u32) -> Result<std::path::PathBuf, PtyError> {
    use uzers::os::unix::UserExt;
    let user = uzers::get_user_by_uid(uid).ok_or(PtyError::ShellUnavailable)?;
    let shell = user.shell();
    if shell.as_os_str().is_empty() || !shell.is_absolute() {
        return Err(PtyError::ShellUnavailable);
    }
    Ok(shell.to_path_buf())
}

/// Resolves the exact `setsid(1)` binary (from the authenticated user
/// manager's own `PATH`) used as the safe, non-`unsafe`-code substitute for
/// `CommandExt::pre_exec`: the workspace forbids `unsafe_code` outright (a
/// hard `-F unsafe-code`, not overridable by any item-level `#[allow]`), so
/// making the spawned shell a session leader with its controlling terminal
/// set to the inherited PTY slave is delegated to `setsid --ctty`, an
/// external, independently audited utility, rather than an in-process
/// post-fork closure.
fn resolve_setsid(
    environment: &crate::environment::ManagerEnvironment,
) -> Result<std::path::PathBuf, PtyError> {
    environment
        .resolve_program("setsid")
        .map_err(|_| PtyError::ShellUnavailable)
}

/// Spawns `shell_path` (via `setsid --ctty`, see [`resolve_setsid`]) with a
/// fresh session and controlling terminal set to the PTY slave at
/// `slave_path`, its stdio all connected to that slave, and its environment
/// replaced entirely by `environment`. Because `setsid --ctty <program>`
/// execs `<program>` in place (it forks only when the calling process is
/// already a process-group leader, which a freshly spawned child never is),
/// the returned `Child`'s pid is the exact pid handed to
/// `SystemdUserScopeManager::start_named_scope` as `supervisor_pid`.
fn spawn_pty_process(
    setsid_path: &std::path::Path,
    shell_path: &std::path::Path,
    slave_path: &std::path::Path,
    environment: &BTreeMap<String, String>,
) -> Result<Child, PtyError> {
    let stdin = open_pty_slave(slave_path)?;
    let stdout = open_pty_slave(slave_path)?;
    let stderr = open_pty_slave(slave_path)?;
    let mut command = Command::new(setsid_path);
    command.arg("--ctty").arg(shell_path);
    command.env_clear();
    command.envs(environment.iter());
    command.stdin(Stdio::from(stdin));
    command.stdout(Stdio::from(stdout));
    command.stderr(Stdio::from(stderr));
    command.spawn().map_err(|_| PtyError::SpawnFailed)
}

fn to_shell_inspection(inspection: crate::systemd::ScopeInspection) -> ScopeInspection {
    let ownership = if inspection.identity_matches {
        ScopeOwnership::Exact
    } else {
        ScopeOwnership::Mismatch
    };
    let process_state = match inspection.state {
        HelperScopeState::Starting | HelperScopeState::Active => ScopeProcessState::Running,
        _ => ScopeProcessState::Exited,
    };
    ScopeInspection {
        ownership,
        process_state,
    }
}

fn to_runtime_process_state(inspection: crate::systemd::ScopeInspection) -> RuntimeProcessState {
    if !inspection.identity_matches {
        return RuntimeProcessState::Degraded;
    }
    match inspection.state {
        HelperScopeState::Starting | HelperScopeState::Active => RuntimeProcessState::Running,
        HelperScopeState::Stopping | HelperScopeState::Exited => RuntimeProcessState::Stopped,
        HelperScopeState::Degraded => RuntimeProcessState::Degraded,
    }
}

fn scope_digest(scope: &VerifiedScope) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(scope.unit_name.as_bytes());
    hasher.update(scope.invocation_id.as_bytes());
    hasher.finalize().into()
}

/// Waits (bounded, polling) for `inspect` to report the process has exited.
/// Used after a `terminate_scope(SIGKILL)` so `kill_shell_scope` /
/// `stop_process` can report an immediate, verified `Exited` state rather
/// than an optimistic guess.
fn await_exit(
    scope_manager: &SystemdUserScopeManager,
    scope: &VerifiedScope,
    timeout: Duration,
) -> Result<crate::systemd::ScopeInspection, ScopeError> {
    let deadline = Instant::now() + timeout;
    loop {
        let inspection = scope_manager.inspect_scope(scope)?;
        if inspection.state == HelperScopeState::Exited || Instant::now() >= deadline {
            return Ok(inspection);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn spawn_pump(pty_master: OwnedFd, socket: OwnedFd) -> Pump {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop);
    let handle = std::thread::spawn(move || pump_loop(pty_master, socket, stop_clone));
    Pump {
        stop,
        handle: Some(handle),
    }
}

/// Relays bytes between a PTY master and a client-attached socket until
/// either side reaches EOF/error or `stop` is set (an explicit
/// `Detach`/`Kill`). Never observes or acts on shell command content beyond
/// raw byte copying.
///
/// Client-disconnect detection is two-layered by design, and both layers
/// observe the *same* underlying client socket independently (this thread
/// holds one duplicate for I/O; [`ShellSupervisor`]/the runtime registry
/// hold a second duplicate purely for liveness), so a peer close is never
/// missed even though nothing pushes an event between them:
/// - this loop notices `POLLHUP`/`Ok(0)` on its own duplicate and exits
///   within one 200ms poll tick, ending the byte relay immediately;
/// - [`ShellSupervisor::reconcile_hangup`] independently notices the same
///   kernel-level close on its own duplicate the next time a dispatch path
///   observes state (`List`/`Inspect`/`Attach`), and flips the tracked
///   attachment back to detached so a later `Attach` is accepted again.
///
/// Neither layer kills the scope or its process on a bare client
/// disconnect: only an explicit `Kill` request tears the real scope down.
fn pump_loop(pty_master: OwnedFd, socket: OwnedFd, stop: Arc<AtomicBool>) {
    use rustix::event::{PollFd, PollFlags, poll};
    let mut buffer = [0_u8; 8192];
    loop {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let mut fds = [
            PollFd::new(&pty_master, PollFlags::IN),
            PollFd::new(&socket, PollFlags::IN),
        ];
        match poll(&mut fds, 200) {
            Ok(_) => {}
            Err(_) => return,
        }
        let pty_events = fds[0].revents();
        let socket_events = fds[1].revents();
        if pty_events.intersects(PollFlags::IN) {
            match read_fd(&pty_master, &mut buffer) {
                Ok(0) | Err(_) => return,
                Ok(read) => {
                    if write_all_fd(&socket, &buffer[..read]).is_err() {
                        return;
                    }
                }
            }
        }
        if socket_events.intersects(PollFlags::IN) {
            match read_fd(&socket, &mut buffer) {
                Ok(0) | Err(_) => return,
                Ok(read) => {
                    if write_all_fd(&pty_master, &buffer[..read]).is_err() {
                        return;
                    }
                }
            }
        }
        if pty_events.intersects(PollFlags::HUP | PollFlags::ERR)
            || socket_events.intersects(PollFlags::HUP | PollFlags::ERR)
        {
            return;
        }
    }
}

fn read_fd(fd: &OwnedFd, buffer: &mut [u8]) -> std::io::Result<usize> {
    rustix::io::read(fd, buffer).map_err(Into::into)
}

fn write_all_fd(fd: &OwnedFd, bytes: &[u8]) -> std::io::Result<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        let written = rustix::io::write(fd, &bytes[offset..]).map_err(std::io::Error::from)?;
        if written == 0 {
            return Err(std::io::ErrorKind::WriteZero.into());
        }
        offset += written;
    }
    Ok(())
}

/// Real, production `AuthenticatedSystemdUserRuntime`, `SystemdUserRuntimePort`,
/// and `TtyOneShotRuntime` backend: verified transient systemd user scopes,
/// real PTY-backed child processes, and a background byte pump wired to
/// the inbound attachment fd the client sends in on `Attach`/`OpenTerminal`.
///
/// Shell `Create`, `Attach`, `Detach`, `List`, `Inspect`, and `Kill` are
/// real, as are Runtime `EnsureScope`, `InspectProcess`, `AdoptProcess`,
/// `StopProcess`, and `OpenTerminal`. Runtime `StartProcess` alone remains
/// a deliberate, honestly-reported `Unavailable`, because it additionally
/// requires a `ConfiguredProcessResolver` and `WaylandControlPort` wired to
/// a configured-launch item catalogue, which is out of scope for this seam.
///
/// `TtyOneShotRuntime` is never reachable over the wire: the `tty` ttrpc
/// service short-circuits through `TtyUnavailable` before ever reaching
/// composition, so it stays a thin, honestly-typed stub.
struct RealBackend {
    scope_manager: SystemdUserScopeManager,
    state: Arc<Mutex<PtyState>>,
}

impl RealBackend {
    fn new(scope_manager: SystemdUserScopeManager, state: Arc<Mutex<PtyState>>) -> Self {
        Self {
            scope_manager,
            state,
        }
    }

    fn create_entry(
        &self,
        resource_id: &str,
        unit_name: &str,
        kind: HelperScopeKind,
    ) -> Result<VerifiedScope, PtyError> {
        let manager_environment = self
            .scope_manager
            .manager_environment()
            .map_err(|_| PtyError::ShellUnavailable)?;
        let setsid_path = resolve_setsid(&manager_environment)?;
        let environment = manager_environment.persistent_shell_entries();
        let (master, slave_path) = open_pty_pair()?;
        let shell_path = resolve_login_shell(self.scope_manager.authenticated_uid())?;
        let child = spawn_pty_process(&setsid_path, &shell_path, &slave_path, &environment)?;
        let pid = child.id();
        let scope = match self.scope_manager.start_named_scope(pid, unit_name, kind) {
            Ok(scope) => scope,
            Err(_) => {
                let mut child = child;
                let _ = child.kill();
                let _ = child.wait();
                return Err(PtyError::AllocationFailed);
            }
        };
        let mut state = self.state.lock().map_err(|_| PtyError::AllocationFailed)?;
        let target = if kind == HelperScopeKind::PersistentShell {
            &mut state.shells
        } else {
            &mut state.runtimes
        };
        target.insert(
            resource_id.to_owned(),
            PtyEntry {
                pty_master: master,
                child,
                scope: scope.clone(),
                pump: None,
            },
        );
        Ok(scope)
    }
}

impl AuthenticatedSystemdUserRuntime for RealBackend {
    fn create_shell_scope(
        &mut self,
        owner: &ShellOwner,
        resource_id: &str,
        _operation_id: &str,
    ) -> Result<VerifiedTransientScope, ShellServiceError> {
        {
            let state = self
                .state
                .lock()
                .map_err(|_| ShellServiceError::RuntimeUnavailable)?;
            if state.shells.contains_key(resource_id) {
                return Err(ShellServiceError::AlreadyExists);
            }
        }
        let unit_name = format!("d2b-shell-{resource_id}.scope");
        let scope = self
            .create_entry(resource_id, &unit_name, HelperScopeKind::PersistentShell)
            .map_err(|_| ShellServiceError::RuntimeUnavailable)?;
        VerifiedTransientScope::new(
            resource_id.to_owned(),
            scope.unit_name,
            scope.invocation_id,
            scope.control_group,
            owner.uid(),
            owner.session_generation(),
        )
        .map_err(|_| ShellServiceError::ScopeOwnershipMismatch)
    }

    fn inspect_shell_scope(
        &mut self,
        _owner: &ShellOwner,
        scope: &VerifiedTransientScope,
    ) -> Result<ScopeInspection, ShellServiceError> {
        let systemd_scope = VerifiedScope {
            unit_name: scope.unit_name().to_owned(),
            invocation_id: scope.invocation_id().to_owned(),
            control_group: scope.control_group().to_owned(),
            kind: HelperScopeKind::PersistentShell,
        };
        let inspection = self
            .scope_manager
            .inspect_scope(&systemd_scope)
            .map_err(|_| ShellServiceError::RuntimeUnavailable)?;
        if let Ok(mut state) = self.state.lock()
            && let Some(entry) = state.shells.get_mut(scope.resource_id())
        {
            let _ = entry.child.try_wait();
        }
        Ok(to_shell_inspection(inspection))
    }

    fn adopt_shell_scope(
        &mut self,
        owner: &ShellOwner,
        scope: &VerifiedTransientScope,
        operation_id: &str,
    ) -> Result<ScopeInspection, ShellServiceError> {
        let _ = operation_id;
        self.inspect_shell_scope(owner, scope)
    }

    fn kill_shell_scope(
        &mut self,
        _owner: &ShellOwner,
        scope: &VerifiedTransientScope,
        _operation_id: &str,
    ) -> Result<ScopeInspection, ShellServiceError> {
        let systemd_scope = VerifiedScope {
            unit_name: scope.unit_name().to_owned(),
            invocation_id: scope.invocation_id().to_owned(),
            control_group: scope.control_group().to_owned(),
            kind: HelperScopeKind::PersistentShell,
        };
        self.scope_manager
            .terminate_scope(&systemd_scope, nix::libc::SIGKILL)
            .map_err(|_| ShellServiceError::RuntimeUnavailable)?;
        let inspection = await_exit(&self.scope_manager, &systemd_scope, Duration::from_secs(2))
            .map_err(|_| ShellServiceError::RuntimeUnavailable)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| ShellServiceError::RuntimeUnavailable)?;
        if let Some(mut entry) = state.shells.remove(scope.resource_id()) {
            if let Some(pump) = entry.pump.take() {
                pump.stop_and_join();
            }
            let _ = entry.child.kill();
            let _ = entry.child.wait();
        }
        Ok(to_shell_inspection(inspection))
    }

    fn cancel(&mut self, _owner: &ShellOwner, request_id: [u8; 16]) -> ShellCancelOutcome {
        match self.state.lock() {
            Ok(state) if state.is_completed(request_id) => ShellCancelOutcome::AlreadyTerminal,
            _ => ShellCancelOutcome::UnknownRequest,
        }
    }
}

impl SystemdUserRuntimePort for RealBackend {
    fn ensure_scope(
        &mut self,
        owner: &RuntimeOwner,
        resource_id: &str,
        _operation_id: &str,
    ) -> Result<RuntimeResource, RuntimeServiceError> {
        let _ = owner;
        {
            let state = self
                .state
                .lock()
                .map_err(|_| RuntimeServiceError::Unavailable)?;
            if state.runtimes.contains_key(resource_id) {
                return Err(RuntimeServiceError::Conflict);
            }
        }
        let unit_name = format!("d2b-runtime-{resource_id}.scope");
        let scope = self
            .create_entry(resource_id, &unit_name, HelperScopeKind::LauncherApp)
            .map_err(|_| RuntimeServiceError::Unavailable)?;
        Ok(RuntimeResource {
            handle: resource_id.to_owned(),
            result_digest: scope_digest(&scope),
            state: RuntimeProcessState::Running,
        })
    }

    fn start_process(
        &mut self,
        _owner: &RuntimeOwner,
        _operation_id: &str,
        _process: &ResolvedProcess,
        _display: Option<&WaylandDisplayLease>,
    ) -> Result<RuntimeResource, RuntimeServiceError> {
        Err(RuntimeServiceError::Unavailable)
    }

    fn inspect_process(
        &mut self,
        _owner: &RuntimeOwner,
        resource_id: &str,
    ) -> Result<RuntimeResource, RuntimeServiceError> {
        let scope = {
            let state = self
                .state
                .lock()
                .map_err(|_| RuntimeServiceError::Unavailable)?;
            state
                .runtimes
                .get(resource_id)
                .map(|entry| entry.scope.clone())
                .ok_or(RuntimeServiceError::NotFound)?
        };
        let inspection = self
            .scope_manager
            .inspect_scope(&scope)
            .map_err(|_| RuntimeServiceError::Unavailable)?;
        Ok(RuntimeResource {
            handle: resource_id.to_owned(),
            result_digest: scope_digest(&scope),
            state: to_runtime_process_state(inspection),
        })
    }

    fn adopt_process(
        &mut self,
        owner: &RuntimeOwner,
        resource_id: &str,
        _operation_id: &str,
    ) -> Result<RuntimeResource, RuntimeServiceError> {
        self.inspect_process(owner, resource_id)
    }

    fn stop_process(
        &mut self,
        _owner: &RuntimeOwner,
        resource_id: &str,
        _operation_id: &str,
    ) -> Result<RuntimeResource, RuntimeServiceError> {
        let scope = {
            let state = self
                .state
                .lock()
                .map_err(|_| RuntimeServiceError::Unavailable)?;
            state
                .runtimes
                .get(resource_id)
                .map(|entry| entry.scope.clone())
                .ok_or(RuntimeServiceError::NotFound)?
        };
        self.scope_manager
            .terminate_scope(&scope, nix::libc::SIGKILL)
            .map_err(|_| RuntimeServiceError::Unavailable)?;
        let inspection = await_exit(&self.scope_manager, &scope, Duration::from_secs(2))
            .map_err(|_| RuntimeServiceError::Unavailable)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| RuntimeServiceError::Unavailable)?;
        if let Some(mut entry) = state.runtimes.remove(resource_id) {
            if let Some(pump) = entry.pump.take() {
                pump.stop_and_join();
            }
            let _ = entry.child.kill();
            let _ = entry.child.wait();
        }
        Ok(RuntimeResource {
            handle: resource_id.to_owned(),
            result_digest: scope_digest(&scope),
            state: to_runtime_process_state(inspection),
        })
    }

    fn open_terminal(
        &mut self,
        _owner: &RuntimeOwner,
        resource_id: &str,
        _stream_id: &str,
        _attachment: &AuthenticatedTerminalAttachment,
    ) -> Result<RuntimeResource, RuntimeServiceError> {
        let state = self
            .state
            .lock()
            .map_err(|_| RuntimeServiceError::Unavailable)?;
        let entry = state
            .runtimes
            .get(resource_id)
            .ok_or(RuntimeServiceError::NotFound)?;
        let inspection = to_runtime_process_state(
            self.scope_manager
                .inspect_scope(&entry.scope)
                .map_err(|_| RuntimeServiceError::Unavailable)?,
        );
        if inspection != RuntimeProcessState::Running {
            return Err(RuntimeServiceError::Conflict);
        }
        Ok(RuntimeResource {
            handle: resource_id.to_owned(),
            result_digest: scope_digest(&entry.scope),
            state: inspection,
        })
    }

    fn cancel(&mut self, _owner: &RuntimeOwner, request_id: [u8; 16]) -> CancelResult {
        match self.state.lock() {
            Ok(state) if state.is_completed(request_id) => CancelResult::AlreadyTerminal,
            _ => CancelResult::UnknownRequest,
        }
    }
}

impl TtyOneShotRuntime for RealBackend {
    fn start_transient_user_scope(
        &mut self,
        _owner: &RuntimeOwner,
        _request: &TtyOneShotRequest,
        _spec: &TtyOneShotSpec,
        _terminal: ValidatedTerminal,
    ) -> Result<TransientUserScope, TtyOneShotError> {
        Err(TtyOneShotError::RuntimeUnavailable)
    }

    fn teardown_transient_user_scope(
        &mut self,
        _owner: &RuntimeOwner,
        _scope: &TransientUserScope,
    ) -> Result<(), TtyOneShotError> {
        Err(TtyOneShotError::RuntimeUnavailable)
    }
}

fn service_registry(state: &Arc<SessionServices>) -> HashMap<String, ttrpc::r#async::Service> {
    let mut services =
        create_runtime_systemd_user_service(Arc::new(RuntimeAdapter(Arc::clone(state))));
    services.extend(create_shell_service(Arc::new(ShellAdapter(Arc::clone(
        state,
    )))));
    services.extend(create_tty_service(Arc::new(TtyUnavailable)));
    services
}

type Composition = RuntimeComposition<UnavailableResolver, UnavailableWayland, RealBackend>;

struct SessionServices {
    generation: u64,
    composition: Mutex<Option<Composition>>,
    /// Shared with every `RealBackend` this process ever constructs (there is
    /// exactly one, built lazily inside `with_composition`): reachable both
    /// through the frozen trait methods dispatched via `RuntimeComposition`
    /// and directly by the ttrpc adapters below for outbound
    /// attachment/pump wiring and cancellation bookkeeping that the frozen
    /// trait signatures have no hook for.
    pty_state: Arc<Mutex<PtyState>>,
    /// The currently-connected ttrpc driver, set at the start of
    /// `serve_socket` and cleared when that connection ends. Reconnecting
    /// clients replace this with their own driver; only one connection is
    /// ever the "current" one for outbound attachment sends (`open_terminal`
    /// / `attach`), matching the single-controller-at-a-time shape of this
    /// responder. `composition`/`pty_state` outlive any single connection so
    /// list/inspect/adopt see the same resources across a reconnect.
    driver: Mutex<Option<Arc<dyn ComponentSessionDriver>>>,
}

impl SessionServices {
    fn new(generation: u64) -> Self {
        Self {
            generation,
            composition: Mutex::new(None),
            pty_state: Arc::new(Mutex::new(PtyState::default())),
            driver: Mutex::new(None),
        }
    }

    fn set_driver(&self, driver: Arc<dyn ComponentSessionDriver>) {
        if let Ok(mut guard) = self.driver.lock() {
            *guard = Some(driver);
        }
    }

    fn clear_driver(&self) {
        if let Ok(mut guard) = self.driver.lock() {
            *guard = None;
        }
    }

    fn current_driver(&self) -> Option<Arc<dyn ComponentSessionDriver>> {
        self.driver.lock().ok().and_then(|guard| guard.clone())
    }

    /// Records that `request_id` reached a terminal dispatch outcome, so a
    /// later `Cancel` RPC for the same id can honestly answer
    /// `AlreadyTerminal` instead of fabricating a running state. See
    /// [`PtyState::record_completed`].
    fn record_completed(&self, request_id: [u8; 16]) {
        if let Ok(mut state) = self.pty_state.lock() {
            state.record_completed(request_id);
        }
    }

    /// Wires the background byte pump for a just-attached shell: duplicates
    /// the resource's PTY master and relays it against `socket` (the
    /// server-held duplicate of the client's attachment) until the client
    /// hangs up or an explicit `Detach`/`Kill` stops it.
    fn spawn_terminal_pump_shell(&self, resource_id: &str, socket: OwnedFd) {
        let Ok(mut state) = self.pty_state.lock() else {
            return;
        };
        let Some(entry) = state.shells.get_mut(resource_id) else {
            return;
        };
        let Ok(master_dup) = dup_owned(&entry.pty_master) else {
            return;
        };
        entry.pump = Some(spawn_pump(master_dup, socket));
    }

    /// Runtime equivalent of [`Self::spawn_terminal_pump_shell`] for
    /// `OpenTerminal`.
    fn spawn_terminal_pump_runtime(&self, resource_id: &str, socket: OwnedFd) {
        let Ok(mut state) = self.pty_state.lock() else {
            return;
        };
        let Some(entry) = state.runtimes.get_mut(resource_id) else {
            return;
        };
        let Ok(master_dup) = dup_owned(&entry.pty_master) else {
            return;
        };
        entry.pump = Some(spawn_pump(master_dup, socket));
    }

    /// Stops and drops a shell's pump (if any) without touching its scope or
    /// child process: used by `Detach`, which must end the byte relay (so the
    /// client observes EOF) while leaving the shell itself running for a
    /// later re-`Attach`.
    fn stop_terminal_pump_shell(&self, resource_id: &str) {
        if let Ok(mut state) = self.pty_state.lock()
            && let Some(entry) = state.shells.get_mut(resource_id)
            && let Some(pump) = entry.pump.take()
        {
            pump.stop_and_join();
        }
    }

    fn is_completed(&self, request_id: [u8; 16]) -> bool {
        self.pty_state
            .lock()
            .is_ok_and(|state| state.is_completed(request_id))
    }

    fn with_composition<T>(
        &self,
        realm_id: &str,
        workload_id: &str,
        dispatch: impl FnOnce(&mut Composition) -> Result<T, CompositionError>,
    ) -> Result<T, CompositionError> {
        let mut guard = self
            .composition
            .lock()
            .map_err(|_| CompositionError::SessionUnavailable)?;
        if guard.is_none() {
            let session = AuthenticatedRuntimeSession::for_current_process(
                self.generation,
                realm_id.to_owned(),
                workload_id.to_owned(),
            )?;
            let scope_manager =
                SystemdUserScopeManager::for_authenticated_uid(nix::unistd::getuid().as_raw())
                    .map_err(|_| CompositionError::SessionUnavailable)?;
            let backend = RealBackend::new(scope_manager, Arc::clone(&self.pty_state));
            *guard = Some(RuntimeComposition::new(
                session,
                UnavailableResolver,
                UnavailableWayland,
                backend,
                SHELL_OUTPUT_BUDGET,
            )?);
        }
        dispatch(guard.as_mut().ok_or(CompositionError::SessionUnavailable)?)
    }
}

struct RuntimeAdapter(Arc<SessionServices>);
struct ShellAdapter(Arc<SessionServices>);
struct TtyUnavailable;

impl RuntimeAdapter {
    fn dispatch(
        &self,
        method: RuntimeMethod,
        wire: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        let request = match decode_runtime_request(&wire) {
            Ok(request) => request,
            Err(kind) => return Ok(error_response(kind)),
        };
        let now = now_unix_ms();
        let result =
            self.0
                .with_composition(&request.realm_id, &request.workload_id, |composition| {
                    composition.dispatch_runtime(method, &request, &[], now)
                });
        self.0.record_completed(request.request_id);
        Ok(match result {
            Ok(response) => runtime_wire_response(response),
            Err(error) => error_response(composition_error_kind(error)),
        })
    }

    fn cancel(&self, wire: WireCancelRequest) -> ttrpc::Result<WireCancelResponse> {
        let request_id: [u8; 16] = match wire.request_id.try_into() {
            Ok(value) if value != [0; 16] => value,
            _ => {
                return Ok(cancel_response(
                    WireCancelOutcome::CANCEL_OUTCOME_UNKNOWN_REQUEST,
                ));
            }
        };
        let request = CancelRequest {
            session_generation: wire.session_generation,
            request_id,
        };
        let result = self
            .0
            .composition
            .lock()
            .map_err(|_| rpc_internal())?
            .as_mut()
            .map(|composition| composition.cancel_runtime(&request));
        Ok(match result {
            Some(Ok(response)) => cancel_response(match response.outcome {
                CancelResult::CancelledBeforeDispatch => {
                    WireCancelOutcome::CANCEL_OUTCOME_CANCELLED_BEFORE_DISPATCH
                }
                CancelResult::CancellationSignalled => {
                    WireCancelOutcome::CANCEL_OUTCOME_CANCELLATION_SIGNALLED
                }
                CancelResult::AlreadyTerminal => WireCancelOutcome::CANCEL_OUTCOME_ALREADY_TERMINAL,
                CancelResult::UnknownRequest => WireCancelOutcome::CANCEL_OUTCOME_UNKNOWN_REQUEST,
            }),
            Some(Err(CompositionError::Runtime(RuntimeServiceError::GenerationMismatch))) => {
                cancel_response(WireCancelOutcome::CANCEL_OUTCOME_GENERATION_MISMATCH)
            }
            _ => cancel_response(WireCancelOutcome::CANCEL_OUTCOME_UNKNOWN_REQUEST),
        })
    }

    /// The one method that cannot go through the generic [`Self::dispatch`]:
    /// `OpenTerminal` is the sole Runtime method that carries a real
    /// attachment (see the module doc on [`SessionServices`]). The client
    /// creates a connected `SOCK_STREAM` pair, keeps one end for itself, and
    /// sends the other end in as this exact request's declared attachment;
    /// this handler consumes it, verifies composition accepts the binding,
    /// and — only on success — wires a background byte pump between that
    /// duplicated fd and the resource's real PTY master.
    async fn open_terminal_impl(&self, wire: ServiceRequest) -> ttrpc::Result<ServiceResponse> {
        let request = match decode_runtime_request(&wire) {
            Ok(request) => request,
            Err(kind) => return Ok(error_response(kind)),
        };
        let Some(driver) = self.0.current_driver() else {
            return Ok(error_response(ErrorKind::ERROR_KIND_UNAVAILABLE));
        };
        let attachments = match driver.receive_attachments().await {
            Ok(attachments) => attachments,
            Err(_) => return Ok(error_response(ErrorKind::ERROR_KIND_UNAVAILABLE)),
        };
        let Some(pump_fd) = take_terminal_attachment(
            &attachments,
            ServicePackage::RuntimeSystemdUserV2,
            RUNTIME_OPEN_TERMINAL_METHOD_ID,
            request.request_id,
            request.session_generation,
        ) else {
            return Ok(error_response(ErrorKind::ERROR_KIND_INVALID_REQUEST));
        };
        let attachment_metadata = AuthenticatedTerminalAttachment {
            index: 0,
            owner_uid: nix::unistd::getuid().as_raw(),
            session_generation: request.session_generation,
            request_id: request.request_id,
            connected_stream: true,
            cloexec: true,
        };
        let now = now_unix_ms();
        let resource_id = request.resource_id.clone();
        let result =
            self.0
                .with_composition(&request.realm_id, &request.workload_id, |composition| {
                    composition.dispatch_runtime(
                        RuntimeMethod::OpenTerminal,
                        &request,
                        std::slice::from_ref(&attachment_metadata),
                        now,
                    )
                });
        self.0.record_completed(request.request_id);
        match result {
            Ok(response) => {
                self.0.spawn_terminal_pump_runtime(&resource_id, pump_fd);
                Ok(runtime_wire_response(response))
            }
            Err(error) => {
                drop(pump_fd);
                Ok(error_response(composition_error_kind(error)))
            }
        }
    }
}

fn runtime_wire_response(
    response: crate::services::runtime_systemd_user::RuntimeResponse,
) -> ServiceResponse {
    let mut wire = ServiceResponse::new();
    wire.outcome = EnumOrUnknown::new(match response.outcome {
        crate::services::runtime_systemd_user::RuntimeOutcome::Succeeded => {
            Outcome::OUTCOME_SUCCEEDED
        }
        crate::services::runtime_systemd_user::RuntimeOutcome::Degraded => {
            Outcome::OUTCOME_DEGRADED
        }
    });
    wire.operation_id = response.operation_id;
    wire.resource_handle = response.resource_handle;
    wire.stream_id = response.stream_id;
    wire.result_digest = response.result_digest.to_vec();
    wire.attachment_indexes = response.attachment_indexes;
    wire
}

/// Validates that `attachments` is exactly the one connected, CLOEXEC,
/// unix-stream-socket `Terminal`-purpose attachment the client declared on
/// this exact request (matching index 0, this service and method, this
/// session generation, and this request id), and returns a fresh owned
/// duplicate of its underlying fd. The client keeps its own end and relays
/// it to the human-facing terminal; this duplicate becomes the far end of
/// the byte pump wired to the resource's real PTY master.
fn take_terminal_attachment(
    attachments: &[OwnedAttachment],
    service: ServicePackage,
    method_id: u32,
    request_id: [u8; 16],
    generation: u64,
) -> Option<OwnedFd> {
    let [attachment] = attachments else {
        return None;
    };
    let descriptor = attachment.descriptor()?;
    if descriptor.index != 0
        || descriptor.kind != AttachmentKind::FileDescriptor
        || descriptor.object_type != KernelObjectType::UnixStreamSocket
        || descriptor.purpose != AttachmentPurpose::Terminal
        || descriptor.service != service
        || descriptor.method_id != method_id
        || descriptor.request_id.as_bytes() != request_id
        || descriptor.reconnect_generation != generation
        || !descriptor.cloexec_required
    {
        return None;
    }
    attachment
        .payload()
        .and_then(|payload| payload.downcast_ref::<UnixAttachmentPayload>())
        .and_then(UnixAttachmentPayload::file)
        .and_then(|fd| fd.try_clone_to_owned().ok())
}

#[async_trait]
impl RuntimeSystemdUserTtrpc for RuntimeAdapter {
    async fn ensure_scope(
        &self,
        _context: &TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(RuntimeMethod::EnsureScope, request)
    }

    async fn start_process(
        &self,
        _context: &TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(RuntimeMethod::StartProcess, request)
    }

    async fn inspect_process(
        &self,
        _context: &TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(RuntimeMethod::InspectProcess, request)
    }

    async fn adopt_process(
        &self,
        _context: &TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(RuntimeMethod::AdoptProcess, request)
    }

    async fn stop_process(
        &self,
        _context: &TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(RuntimeMethod::StopProcess, request)
    }

    async fn open_terminal(
        &self,
        _context: &TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.open_terminal_impl(request).await
    }

    async fn cancel(
        &self,
        _context: &TtrpcContext,
        request: WireCancelRequest,
    ) -> ttrpc::Result<WireCancelResponse> {
        RuntimeAdapter::cancel(self, request)
    }
}

impl ShellAdapter {
    fn dispatch(
        &self,
        method: ShellMethod,
        wire: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        let request = match decode_shell_request(method, &wire) {
            Ok(request) => request,
            Err(kind) => return Ok(error_response(kind)),
        };
        let now = now_unix_ms();
        let resource_id = request.resource_id.clone();
        let result =
            self.0
                .with_composition(&request.realm_id, &request.workload_id, |composition| {
                    composition.dispatch_shell(&request, Vec::new(), now)
                });
        self.0.record_completed(request.request_id);
        // `Detach` never reaches `RealBackend` (it is pure
        // `ShellSupervisor` bookkeeping), so it is the one shell method
        // whose successful composition dispatch does not already stop the
        // background pump: do that here so the client observes EOF exactly
        // when detach succeeds, without touching the still-running scope.
        if method == ShellMethod::Detach && result.is_ok() {
            self.0.stop_terminal_pump_shell(&resource_id);
        }
        Ok(match result {
            Ok(response) => shell_wire_response(response),
            Err(error) => error_response(composition_error_kind(error)),
        })
    }

    fn cancel(&self, request: WireCancelRequest) -> ttrpc::Result<WireCancelResponse> {
        if request.session_generation != self.0.generation {
            return Ok(cancel_response(
                WireCancelOutcome::CANCEL_OUTCOME_GENERATION_MISMATCH,
            ));
        }
        let request_id: [u8; 16] = match request.request_id.try_into() {
            Ok(value) if value != [0; 16] => value,
            _ => {
                return Ok(cancel_response(
                    WireCancelOutcome::CANCEL_OUTCOME_UNKNOWN_REQUEST,
                ));
            }
        };
        // `RuntimeComposition` exposes no `cancel_shell` method (only
        // `cancel_runtime`), so this dedicated RPC consults the same
        // completed-request tracker `RealBackend`'s own `cancel()` trait
        // method uses, directly, bypassing composition entirely.
        Ok(cancel_response(if self.0.is_completed(request_id) {
            WireCancelOutcome::CANCEL_OUTCOME_ALREADY_TERMINAL
        } else {
            WireCancelOutcome::CANCEL_OUTCOME_UNKNOWN_REQUEST
        }))
    }

    /// The one shell method that cannot go through the generic
    /// [`Self::dispatch`]: `Attach` is the sole method that carries a real
    /// attachment. The client creates a connected `SOCK_STREAM` pair, keeps
    /// one end for itself, and sends the other end in as this exact
    /// request's declared attachment; this handler consumes it, hands a
    /// duplicate to composition (stored only for hangup detection, per
    /// `ShellSupervisor`), and — only on success — wires a background byte
    /// pump between a second duplicate and the shell's real PTY master.
    async fn attach_impl(&self, wire: ServiceRequest) -> ttrpc::Result<ServiceResponse> {
        let request = match decode_shell_request(ShellMethod::Attach, &wire) {
            Ok(request) => request,
            Err(kind) => return Ok(error_response(kind)),
        };
        let Some(driver) = self.0.current_driver() else {
            return Ok(error_response(ErrorKind::ERROR_KIND_UNAVAILABLE));
        };
        let attachments = match driver.receive_attachments().await {
            Ok(attachments) => attachments,
            Err(_) => return Ok(error_response(ErrorKind::ERROR_KIND_UNAVAILABLE)),
        };
        let Some(supervisor_fd) = take_terminal_attachment(
            &attachments,
            ServicePackage::ShellV2,
            crate::supervisor_protocol::ATTACH_METHOD_ID,
            request.request_id,
            request.session_generation,
        ) else {
            return Ok(error_response(ErrorKind::ERROR_KIND_INVALID_REQUEST));
        };
        let Ok(pump_fd) = dup_owned(&supervisor_fd) else {
            return Ok(error_response(ErrorKind::ERROR_KIND_UNAVAILABLE));
        };
        let owner_uid = nix::unistd::getuid().as_raw();
        let attachment = ShellTerminalAttachment::new(
            supervisor_fd,
            owner_uid,
            request.session_generation,
            request.request_id,
        );
        let now = now_unix_ms();
        let resource_id = request.resource_id.clone();
        let result =
            self.0
                .with_composition(&request.realm_id, &request.workload_id, |composition| {
                    composition.dispatch_shell(&request, vec![attachment], now)
                });
        self.0.record_completed(request.request_id);
        match result {
            Ok(response) => {
                self.0.spawn_terminal_pump_shell(&resource_id, pump_fd);
                Ok(shell_wire_response(response))
            }
            Err(error) => {
                drop(pump_fd);
                Ok(error_response(composition_error_kind(error)))
            }
        }
    }
}

fn shell_wire_response(response: crate::shell_runtime::ShellResponse) -> ServiceResponse {
    let mut wire = ServiceResponse::new();
    wire.outcome = EnumOrUnknown::new(match response.state {
        crate::shell_runtime::ShellState::Degraded => Outcome::OUTCOME_DEGRADED,
        _ => Outcome::OUTCOME_SUCCEEDED,
    });
    wire.operation_id = response.operation_id;
    wire.resource_handle = response.resource_id;
    wire.stream_id = response.stream_id;
    wire.attachment_indexes = response.attachment_indexes;
    wire
}

#[async_trait]
impl ShellTtrpc for ShellAdapter {
    async fn create(
        &self,
        _context: &TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(ShellMethod::Create, request)
    }

    async fn attach(
        &self,
        _context: &TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.attach_impl(request).await
    }

    async fn detach(
        &self,
        _context: &TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(ShellMethod::Detach, request)
    }

    async fn list(
        &self,
        _context: &TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(ShellMethod::List, request)
    }

    async fn inspect(
        &self,
        _context: &TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(ShellMethod::Inspect, request)
    }

    async fn kill(
        &self,
        _context: &TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(ShellMethod::Kill, request)
    }

    async fn cancel(
        &self,
        _context: &TtrpcContext,
        request: WireCancelRequest,
    ) -> ttrpc::Result<WireCancelResponse> {
        ShellAdapter::cancel(self, request)
    }
}

#[async_trait]
impl TtyTtrpc for TtyUnavailable {
    async fn enter_raw_mode(
        &self,
        _context: &TtrpcContext,
        _request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        Ok(error_response(ErrorKind::ERROR_KIND_UNAVAILABLE))
    }

    async fn restore_mode(
        &self,
        _context: &TtrpcContext,
        _request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        Ok(error_response(ErrorKind::ERROR_KIND_UNAVAILABLE))
    }

    async fn inspect(
        &self,
        _context: &TtrpcContext,
        _request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        Ok(error_response(ErrorKind::ERROR_KIND_UNAVAILABLE))
    }

    async fn cancel(
        &self,
        _context: &TtrpcContext,
        _request: WireCancelRequest,
    ) -> ttrpc::Result<WireCancelResponse> {
        Ok(cancel_response(
            WireCancelOutcome::CANCEL_OUTCOME_UNKNOWN_REQUEST,
        ))
    }
}

fn decode_runtime_request(wire: &ServiceRequest) -> Result<RuntimeRequest, ErrorKind> {
    let metadata = wire
        .metadata
        .as_ref()
        .ok_or(ErrorKind::ERROR_KIND_INVALID_REQUEST)?;
    let scope = wire
        .scope
        .as_ref()
        .ok_or(ErrorKind::ERROR_KIND_INVALID_REQUEST)?;
    Ok(RuntimeRequest {
        request_id: exact_array(&metadata.request_id)?,
        idempotency_key: optional_array(&metadata.idempotency_key)?,
        issued_at_unix_ms: metadata.issued_at_unix_ms,
        expires_at_unix_ms: metadata.expires_at_unix_ms,
        session_generation: metadata.session_generation,
        realm_id: scope.realm_id.clone(),
        workload_id: scope.workload_id.clone(),
        resource_id: wire.resource_id.clone(),
        operation_id: wire.operation_id.clone(),
        request_digest: optional_array(&wire.request_digest)?,
        stream_id: wire.stream_id.clone(),
        attachment_indexes: wire.attachment_indexes.clone(),
        desired_state: match wire.desired_state.enum_value() {
            Ok(WireDesiredState::DESIRED_STATE_UNSPECIFIED) => DesiredState::Unspecified,
            Ok(WireDesiredState::DESIRED_STATE_PRESENT) => DesiredState::Present,
            Ok(WireDesiredState::DESIRED_STATE_RUNNING) => DesiredState::Running,
            Ok(WireDesiredState::DESIRED_STATE_STOPPED) => DesiredState::Stopped,
            Ok(WireDesiredState::DESIRED_STATE_ATTACHED) => DesiredState::Attached,
            _ => return Err(ErrorKind::ERROR_KIND_INVALID_REQUEST),
        },
    })
}

fn decode_shell_request(
    method: ShellMethod,
    wire: &ServiceRequest,
) -> Result<ShellRequest, ErrorKind> {
    let metadata = wire
        .metadata
        .as_ref()
        .ok_or(ErrorKind::ERROR_KIND_INVALID_REQUEST)?;
    let scope = wire
        .scope
        .as_ref()
        .ok_or(ErrorKind::ERROR_KIND_INVALID_REQUEST)?;
    Ok(ShellRequest {
        method,
        request_id: exact_array(&metadata.request_id)?,
        idempotency_key: optional_array(&metadata.idempotency_key)?,
        issued_at_unix_ms: metadata.issued_at_unix_ms,
        expires_at_unix_ms: metadata.expires_at_unix_ms,
        session_generation: metadata.session_generation,
        realm_id: scope.realm_id.clone(),
        workload_id: scope.workload_id.clone(),
        resource_id: wire.resource_id.clone(),
        operation_id: wire.operation_id.clone(),
        stream_id: wire.stream_id.clone(),
        attachment_indexes: wire.attachment_indexes.clone(),
        output_ring_bytes: usize::try_from(wire.page_size)
            .map_err(|_| ErrorKind::ERROR_KIND_INVALID_REQUEST)?,
    })
}

fn exact_array<const N: usize>(bytes: &[u8]) -> Result<[u8; N], ErrorKind> {
    let value: [u8; N] = bytes
        .try_into()
        .map_err(|_| ErrorKind::ERROR_KIND_INVALID_REQUEST)?;
    (value != [0; N])
        .then_some(value)
        .ok_or(ErrorKind::ERROR_KIND_INVALID_REQUEST)
}

fn optional_array<const N: usize>(bytes: &[u8]) -> Result<Option<[u8; N]>, ErrorKind> {
    if bytes.is_empty() {
        Ok(None)
    } else {
        exact_array(bytes).map(Some)
    }
}

fn error_response(kind: ErrorKind) -> ServiceResponse {
    let mut error = ErrorEnvelope::new();
    error.kind = EnumOrUnknown::new(kind);
    error.retry = EnumOrUnknown::new(match kind {
        ErrorKind::ERROR_KIND_UNAVAILABLE => RetryClass::RETRY_CLASS_AFTER_OBSERVATION,
        ErrorKind::ERROR_KIND_DEADLINE_EXCEEDED => RetryClass::RETRY_CLASS_SAME_OPERATION,
        _ => RetryClass::RETRY_CLASS_NEVER,
    });
    let mut response = ServiceResponse::new();
    response.outcome = EnumOrUnknown::new(Outcome::OUTCOME_FAILED);
    response.error = MessageField::some(error);
    response
}

fn cancel_response(outcome: WireCancelOutcome) -> WireCancelResponse {
    let mut response = WireCancelResponse::new();
    response.outcome = EnumOrUnknown::new(outcome);
    response
}

fn composition_error_kind(error: CompositionError) -> ErrorKind {
    match error {
        CompositionError::OwnerMismatch
        | CompositionError::Runtime(RuntimeServiceError::OwnerMismatch)
        | CompositionError::Shell(ShellServiceError::OwnerMismatch) => {
            ErrorKind::ERROR_KIND_UNAUTHORIZED
        }
        CompositionError::Runtime(RuntimeServiceError::GenerationMismatch)
        | CompositionError::Shell(ShellServiceError::GenerationMismatch) => {
            ErrorKind::ERROR_KIND_GENERATION_MISMATCH
        }
        CompositionError::Runtime(RuntimeServiceError::DeadlineExpired)
        | CompositionError::Shell(ShellServiceError::DeadlineExpired) => {
            ErrorKind::ERROR_KIND_DEADLINE_EXCEEDED
        }
        CompositionError::Runtime(RuntimeServiceError::NotFound)
        | CompositionError::Shell(ShellServiceError::NotFound) => ErrorKind::ERROR_KIND_NOT_FOUND,
        CompositionError::Runtime(RuntimeServiceError::Conflict)
        | CompositionError::Shell(
            ShellServiceError::AlreadyExists | ShellServiceError::AlreadyAttached,
        ) => ErrorKind::ERROR_KIND_CONFLICT,
        CompositionError::Runtime(
            RuntimeServiceError::Unavailable | RuntimeServiceError::WaylandUnavailable,
        )
        | CompositionError::Shell(ShellServiceError::RuntimeUnavailable)
        | CompositionError::Tty(TtyOneShotError::RuntimeUnavailable)
        | CompositionError::SessionUnavailable => ErrorKind::ERROR_KIND_UNAVAILABLE,
        CompositionError::Shell(ShellServiceError::ReservationExhausted)
        | CompositionError::Tty(TtyOneShotError::CapacityExceeded) => {
            ErrorKind::ERROR_KIND_RESOURCE_EXHAUSTED
        }
        CompositionError::RecoveryMismatch
        | CompositionError::TeardownFailed
        | CompositionError::InvalidLifecycle
        | CompositionError::ReconnectLimit
        | CompositionError::Runtime(RuntimeServiceError::BackendInvariant)
        | CompositionError::Tty(TtyOneShotError::TeardownFailed) => {
            ErrorKind::ERROR_KIND_INVARIANT_VIOLATION
        }
        _ => ErrorKind::ERROR_KIND_INVALID_REQUEST,
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}

fn rpc_internal() -> ttrpc::Error {
    ttrpc::Error::RpcStatus(ttrpc::get_status(
        ttrpc::Code::INTERNAL,
        "runtime-service-failed".to_owned(),
    ))
}

#[derive(Clone, Copy)]
struct UnavailableResolver;

impl ConfiguredProcessResolver for UnavailableResolver {
    fn resolve(
        &mut self,
        _owner: &RuntimeOwner,
        _resource_id: &str,
        _request_digest: &[u8; 32],
    ) -> Result<ResolvedProcess, RuntimeServiceError> {
        Err(RuntimeServiceError::Unavailable)
    }
}

#[derive(Clone, Copy)]
struct UnavailableWayland;

impl WaylandControlPort for UnavailableWayland {
    fn open_display(
        &mut self,
        _owner: &RuntimeOwner,
        _process: &ResolvedProcess,
        _operation_id: &str,
    ) -> Result<WaylandDisplayLease, RuntimeServiceError> {
        Err(RuntimeServiceError::WaylandUnavailable)
    }

    fn close_display(&mut self, _lease: WaylandDisplayLease) {}
}

impl fmt::Debug for RuntimeAdapter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RuntimeAdapter(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v2_component_session::{
        AttachmentAccess, AttachmentCreditClass, AttachmentDescriptor, BoundedVec, CloseReason,
        Remediation, RequestId,
    };
    use d2b_contracts::v2_services::common::{IdentityScope, RequestMetadata};
    use d2b_session::{Cancellation, StreamEvent, StreamId};
    use d2b_session_unix::prearmed_seqpacket_pair;
    use d2b_session_unix::{DescriptorPolicy, ObjectIdentity, OwnedUnixAttachment};
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::oneshot;

    fn request(generation: u64, desired: WireDesiredState) -> ServiceRequest {
        let now = now_unix_ms();
        let mut metadata = RequestMetadata::new();
        metadata.request_id = vec![1; 16];
        metadata.idempotency_key = vec![2; 32];
        metadata.issued_at_unix_ms = now;
        metadata.expires_at_unix_ms = now + 1_000;
        metadata.session_generation = generation;
        let mut scope = IdentityScope::new();
        scope.realm_id = "work".to_owned();
        scope.workload_id = "shell".to_owned();
        let mut request = ServiceRequest::new();
        request.metadata = MessageField::some(metadata);
        request.scope = MessageField::some(scope);
        request.resource_id = "session".to_owned();
        request.operation_id = "operation".to_owned();
        request.request_digest = vec![3; 32];
        request.desired_state = EnumOrUnknown::new(desired);
        request
    }

    fn response_error_kind(response: &ServiceResponse) -> ErrorKind {
        response.error.as_ref().unwrap().kind.enum_value().unwrap()
    }

    /// Every real-backend dispatch makes at least one live dbus round trip
    /// against the real systemd-user manager (`EnsureScope`/`Create` start
    /// or verify a transient scope; `StopProcess`/`Kill` terminate and then
    /// poll one). Under the heavy parallel dbus load a full workspace test
    /// run creates, a single attempt can observe a transient `Unavailable`
    /// (exactly the condition `RETRY_CLASS_AFTER_OBSERVATION` exists for).
    /// Retry a bounded number of times in these hermetic tests rather than
    /// either flaking the suite or serializing every real-backend test.
    fn dispatch_until_settled(mut attempt: impl FnMut() -> ServiceResponse) -> ServiceResponse {
        for remaining in (0..5).rev() {
            let response = attempt();
            if response.outcome.enum_value() == Ok(Outcome::OUTCOME_SUCCEEDED) || remaining == 0 {
                return response;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        unreachable!()
    }

    fn dispatch_stop_process_until_settled(
        runtime: &RuntimeAdapter,
        generation: u64,
        resource_id: &str,
    ) -> ServiceResponse {
        dispatch_until_settled(|| {
            let mut stop = request(generation, WireDesiredState::DESIRED_STATE_STOPPED);
            stop.resource_id = resource_id.to_owned();
            runtime.dispatch(RuntimeMethod::StopProcess, stop).unwrap()
        })
    }

    fn dispatch_ensure_scope_until_settled(
        runtime: &RuntimeAdapter,
        generation: u64,
        resource_id: &str,
    ) -> ServiceResponse {
        dispatch_until_settled(|| {
            let mut ensure = request(generation, WireDesiredState::DESIRED_STATE_PRESENT);
            ensure.resource_id = resource_id.to_owned();
            runtime
                .dispatch(RuntimeMethod::EnsureScope, ensure)
                .unwrap()
        })
    }

    fn dispatch_shell_create_until_settled(
        shell: &ShellAdapter,
        generation: u64,
        resource_id: &str,
    ) -> ServiceResponse {
        dispatch_until_settled(|| {
            let mut create = request(generation, WireDesiredState::DESIRED_STATE_UNSPECIFIED);
            create.resource_id = resource_id.to_owned();
            create.page_size = 4096;
            shell.dispatch(ShellMethod::Create, create).unwrap()
        })
    }

    fn dispatch_kill_shell_until_settled(
        shell: &ShellAdapter,
        generation: u64,
        resource_id: &str,
    ) -> ServiceResponse {
        dispatch_until_settled(|| {
            let mut kill = request(generation, WireDesiredState::DESIRED_STATE_UNSPECIFIED);
            kill.resource_id = resource_id.to_owned();
            kill.metadata.as_mut().unwrap().request_id = vec![12; 16];
            shell.dispatch(ShellMethod::Kill, kill).unwrap()
        })
    }

    #[test]
    fn runtime_decoder_projects_a_complete_valid_request_without_debug_disclosure() {
        let mut wire = request(17, WireDesiredState::DESIRED_STATE_RUNNING);
        wire.stream_id = "terminal".to_owned();
        wire.attachment_indexes = vec![0];
        let decoded = decode_runtime_request(&wire).unwrap();

        assert_eq!(decoded.request_id, [1; 16]);
        assert_eq!(decoded.idempotency_key, Some([2; 32]));
        assert_eq!(decoded.session_generation, 17);
        assert!(decoded.realm_id == "work");
        assert!(decoded.workload_id == "shell");
        assert!(decoded.resource_id == "session");
        assert!(decoded.operation_id == "operation");
        assert_eq!(decoded.request_digest, Some([3; 32]));
        assert!(decoded.stream_id == "terminal");
        assert_eq!(decoded.attachment_indexes, [0]);
        assert_eq!(decoded.desired_state, DesiredState::Running);
        let debug = format!("{decoded:?}");
        for value in ["work", "shell", "session", "operation", "terminal"] {
            assert!(!debug.contains(value));
        }
    }

    #[test]
    fn shell_decoder_projects_every_method_and_action_without_debug_disclosure() {
        let cases = [
            (ShellMethod::Create, true),
            (ShellMethod::Attach, true),
            (ShellMethod::Detach, true),
            (ShellMethod::List, false),
            (ShellMethod::Inspect, false),
            (ShellMethod::Kill, true),
            (ShellMethod::Cancel, false),
        ];
        for (method, mutating) in cases {
            let mut wire = request(19, WireDesiredState::DESIRED_STATE_UNSPECIFIED);
            wire.stream_id = "terminal".to_owned();
            wire.attachment_indexes = vec![0];
            wire.page_size = 4096;
            let decoded = decode_shell_request(method, &wire).unwrap();

            assert_eq!(decoded.method, method);
            assert_eq!(decoded.method.mutating(), mutating);
            assert_eq!(decoded.request_id, [1; 16]);
            assert_eq!(decoded.idempotency_key, Some([2; 32]));
            assert_eq!(decoded.session_generation, 19);
            assert!(decoded.realm_id == "work");
            assert!(decoded.workload_id == "shell");
            assert!(decoded.resource_id == "session");
            assert!(decoded.operation_id == "operation");
            assert!(decoded.stream_id == "terminal");
            assert_eq!(decoded.attachment_indexes, [0]);
            assert_eq!(decoded.output_ring_bytes, 4096);
            let debug = format!("{decoded:?}");
            for value in ["work", "shell", "session", "operation", "terminal"] {
                assert!(!debug.contains(value));
            }
        }
    }

    #[test]
    fn decoders_reject_missing_metadata_scope_and_malformed_request_identity() {
        let mut wire = request(7, WireDesiredState::DESIRED_STATE_PRESENT);
        wire.metadata = MessageField::none();
        assert_eq!(
            decode_runtime_request(&wire).unwrap_err(),
            ErrorKind::ERROR_KIND_INVALID_REQUEST
        );
        assert_eq!(
            decode_shell_request(ShellMethod::Create, &wire).unwrap_err(),
            ErrorKind::ERROR_KIND_INVALID_REQUEST
        );

        let mut wire = request(7, WireDesiredState::DESIRED_STATE_PRESENT);
        wire.scope = MessageField::none();
        assert_eq!(
            decode_runtime_request(&wire).unwrap_err(),
            ErrorKind::ERROR_KIND_INVALID_REQUEST
        );
        assert_eq!(
            decode_shell_request(ShellMethod::Create, &wire).unwrap_err(),
            ErrorKind::ERROR_KIND_INVALID_REQUEST
        );

        for invalid in [Vec::new(), vec![0; 16], vec![1; 15], vec![1; 17]] {
            let mut wire = request(7, WireDesiredState::DESIRED_STATE_PRESENT);
            wire.metadata.as_mut().unwrap().request_id = invalid;
            assert_eq!(
                decode_runtime_request(&wire).unwrap_err(),
                ErrorKind::ERROR_KIND_INVALID_REQUEST
            );
            assert_eq!(
                decode_shell_request(ShellMethod::Create, &wire).unwrap_err(),
                ErrorKind::ERROR_KIND_INVALID_REQUEST
            );
        }
    }

    #[test]
    fn decoders_reject_malformed_optional_keys_and_runtime_digests() {
        for invalid in [vec![0; 32], vec![2; 31], vec![2; 33]] {
            let mut wire = request(7, WireDesiredState::DESIRED_STATE_PRESENT);
            wire.metadata.as_mut().unwrap().idempotency_key = invalid;
            assert_eq!(
                decode_runtime_request(&wire).unwrap_err(),
                ErrorKind::ERROR_KIND_INVALID_REQUEST
            );
            assert_eq!(
                decode_shell_request(ShellMethod::Create, &wire).unwrap_err(),
                ErrorKind::ERROR_KIND_INVALID_REQUEST
            );
        }

        for invalid in [vec![0; 32], vec![3; 31], vec![3; 33]] {
            let mut wire = request(7, WireDesiredState::DESIRED_STATE_PRESENT);
            wire.request_digest = invalid;
            assert_eq!(
                decode_runtime_request(&wire).unwrap_err(),
                ErrorKind::ERROR_KIND_INVALID_REQUEST
            );
        }

        let mut wire = request(7, WireDesiredState::DESIRED_STATE_PRESENT);
        wire.metadata.as_mut().unwrap().idempotency_key.clear();
        wire.request_digest.clear();
        let decoded = decode_runtime_request(&wire).unwrap();
        assert_eq!(decoded.idempotency_key, None);
        assert_eq!(decoded.request_digest, None);
        assert_eq!(
            decode_shell_request(ShellMethod::List, &wire)
                .unwrap()
                .idempotency_key,
            None
        );
    }

    #[test]
    fn runtime_decoder_accepts_only_the_closed_desired_state_set() {
        let accepted = [
            (
                WireDesiredState::DESIRED_STATE_UNSPECIFIED,
                DesiredState::Unspecified,
            ),
            (
                WireDesiredState::DESIRED_STATE_PRESENT,
                DesiredState::Present,
            ),
            (
                WireDesiredState::DESIRED_STATE_RUNNING,
                DesiredState::Running,
            ),
            (
                WireDesiredState::DESIRED_STATE_STOPPED,
                DesiredState::Stopped,
            ),
            (
                WireDesiredState::DESIRED_STATE_ATTACHED,
                DesiredState::Attached,
            ),
        ];
        for (wire_state, decoded_state) in accepted {
            assert_eq!(
                decode_runtime_request(&request(7, wire_state))
                    .unwrap()
                    .desired_state,
                decoded_state
            );
        }

        for rejected in [
            WireDesiredState::DESIRED_STATE_ABSENT,
            WireDesiredState::DESIRED_STATE_ENABLED,
            WireDesiredState::DESIRED_STATE_DISABLED,
            WireDesiredState::DESIRED_STATE_OPEN,
            WireDesiredState::DESIRED_STATE_CLOSED,
            WireDesiredState::DESIRED_STATE_DETACHED,
        ] {
            assert_eq!(
                decode_runtime_request(&request(7, rejected)).unwrap_err(),
                ErrorKind::ERROR_KIND_INVALID_REQUEST
            );
        }
        let mut wire = request(7, WireDesiredState::DESIRED_STATE_PRESENT);
        wire.desired_state = EnumOrUnknown::from_i32(i32::MAX);
        assert_eq!(
            decode_runtime_request(&wire).unwrap_err(),
            ErrorKind::ERROR_KIND_INVALID_REQUEST
        );
    }

    #[test]
    fn decoders_preserve_generation_and_attachment_bindings_for_service_validation() {
        let mut wire = request(0, WireDesiredState::DESIRED_STATE_ATTACHED);
        wire.attachment_indexes = vec![0, 0, u32::MAX];

        let runtime = decode_runtime_request(&wire).unwrap();
        assert_eq!(runtime.session_generation, 0);
        assert_eq!(runtime.attachment_indexes, [0, 0, u32::MAX]);

        let shell = decode_shell_request(ShellMethod::Attach, &wire).unwrap();
        assert_eq!(shell.session_generation, 0);
        assert_eq!(shell.attachment_indexes, [0, 0, u32::MAX]);
    }

    #[test]
    fn runtime_methods_have_closed_names_and_mutation_actions() {
        let cases = [
            (RuntimeMethod::EnsureScope, "EnsureScope", true),
            (RuntimeMethod::StartProcess, "StartProcess", true),
            (RuntimeMethod::InspectProcess, "InspectProcess", false),
            (RuntimeMethod::AdoptProcess, "AdoptProcess", true),
            (RuntimeMethod::StopProcess, "StopProcess", true),
            (RuntimeMethod::OpenTerminal, "OpenTerminal", true),
        ];
        for (method, name, mutating) in cases {
            assert_eq!(method.name(), name);
            assert_eq!(method.mutating(), mutating);
        }
    }

    #[test]
    fn adapters_close_malformed_identity_generation_and_attachment_requests() {
        if geteuid().is_root() {
            return;
        }
        let dispatch_runtime = |wire| {
            RuntimeAdapter(Arc::new(SessionServices::new(7)))
                .dispatch(RuntimeMethod::EnsureScope, wire)
                .unwrap()
        };
        let dispatch_shell = |wire| {
            ShellAdapter(Arc::new(SessionServices::new(7)))
                .dispatch(ShellMethod::Attach, wire)
                .unwrap()
        };

        let mut malformed_identity = request(7, WireDesiredState::DESIRED_STATE_PRESENT);
        malformed_identity.scope.as_mut().unwrap().realm_id.clear();
        assert_eq!(
            response_error_kind(&dispatch_runtime(malformed_identity)),
            ErrorKind::ERROR_KIND_UNAUTHORIZED
        );

        let zero_generation = request(0, WireDesiredState::DESIRED_STATE_PRESENT);
        assert_eq!(
            response_error_kind(&dispatch_runtime(zero_generation)),
            ErrorKind::ERROR_KIND_GENERATION_MISMATCH
        );

        let mut invalid_attachment = request(7, WireDesiredState::DESIRED_STATE_PRESENT);
        invalid_attachment.attachment_indexes = vec![0];
        assert_eq!(
            response_error_kind(&dispatch_runtime(invalid_attachment)),
            ErrorKind::ERROR_KIND_INVALID_REQUEST
        );

        let mut shell_attachment = request(7, WireDesiredState::DESIRED_STATE_UNSPECIFIED);
        shell_attachment.attachment_indexes = vec![0, 1];
        assert_eq!(
            response_error_kind(&dispatch_shell(shell_attachment)),
            ErrorKind::ERROR_KIND_INVALID_REQUEST
        );
    }

    #[test]
    fn composition_errors_map_to_closed_wire_kinds() {
        let cases = [
            (
                CompositionError::OwnerMismatch,
                ErrorKind::ERROR_KIND_UNAUTHORIZED,
            ),
            (
                CompositionError::SessionUnavailable,
                ErrorKind::ERROR_KIND_UNAVAILABLE,
            ),
            (
                CompositionError::InvalidLifecycle,
                ErrorKind::ERROR_KIND_INVARIANT_VIOLATION,
            ),
            (
                CompositionError::ReconnectLimit,
                ErrorKind::ERROR_KIND_INVARIANT_VIOLATION,
            ),
            (
                CompositionError::RecoveryMismatch,
                ErrorKind::ERROR_KIND_INVARIANT_VIOLATION,
            ),
            (
                CompositionError::TeardownFailed,
                ErrorKind::ERROR_KIND_INVARIANT_VIOLATION,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::Unauthenticated),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::ContractMismatch),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::OwnerMismatch),
                ErrorKind::ERROR_KIND_UNAUTHORIZED,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::GenerationMismatch),
                ErrorKind::ERROR_KIND_GENERATION_MISMATCH,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::DeadlineExpired),
                ErrorKind::ERROR_KIND_DEADLINE_EXCEEDED,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::InvalidRequest),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::InvalidResolvedProcess),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::ResolverMismatch),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::AttachmentMismatch),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::WaylandUnavailable),
                ErrorKind::ERROR_KIND_UNAVAILABLE,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::Unavailable),
                ErrorKind::ERROR_KIND_UNAVAILABLE,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::Conflict),
                ErrorKind::ERROR_KIND_CONFLICT,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::NotFound),
                ErrorKind::ERROR_KIND_NOT_FOUND,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::BackendInvariant),
                ErrorKind::ERROR_KIND_INVARIANT_VIOLATION,
            ),
            (
                CompositionError::Shell(ShellServiceError::Unauthenticated),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Shell(ShellServiceError::ContractMismatch),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Shell(ShellServiceError::OwnerMismatch),
                ErrorKind::ERROR_KIND_UNAUTHORIZED,
            ),
            (
                CompositionError::Shell(ShellServiceError::GenerationMismatch),
                ErrorKind::ERROR_KIND_GENERATION_MISMATCH,
            ),
            (
                CompositionError::Shell(ShellServiceError::DeadlineExpired),
                ErrorKind::ERROR_KIND_DEADLINE_EXCEEDED,
            ),
            (
                CompositionError::Shell(ShellServiceError::InvalidRequest),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Shell(ShellServiceError::AttachmentMismatch),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Shell(ShellServiceError::ScopeOwnershipMismatch),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Shell(ShellServiceError::ReservationExhausted),
                ErrorKind::ERROR_KIND_RESOURCE_EXHAUSTED,
            ),
            (
                CompositionError::Shell(ShellServiceError::AlreadyExists),
                ErrorKind::ERROR_KIND_CONFLICT,
            ),
            (
                CompositionError::Shell(ShellServiceError::AlreadyAttached),
                ErrorKind::ERROR_KIND_CONFLICT,
            ),
            (
                CompositionError::Shell(ShellServiceError::NotFound),
                ErrorKind::ERROR_KIND_NOT_FOUND,
            ),
            (
                CompositionError::Shell(ShellServiceError::RuntimeUnavailable),
                ErrorKind::ERROR_KIND_UNAVAILABLE,
            ),
            (
                CompositionError::Tty(TtyOneShotError::InvalidPolicy),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Tty(TtyOneShotError::InvalidRequest),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Tty(TtyOneShotError::OwnerMismatch),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Tty(TtyOneShotError::AttachmentMismatch),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Tty(TtyOneShotError::ScopeOwnershipMismatch),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Tty(TtyOneShotError::CapacityExceeded),
                ErrorKind::ERROR_KIND_RESOURCE_EXHAUSTED,
            ),
            (
                CompositionError::Tty(TtyOneShotError::RequestConflict),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Tty(TtyOneShotError::RuntimeUnavailable),
                ErrorKind::ERROR_KIND_UNAVAILABLE,
            ),
            (
                CompositionError::Tty(TtyOneShotError::TeardownFailed),
                ErrorKind::ERROR_KIND_INVARIANT_VIOLATION,
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(composition_error_kind(error), expected);
        }
    }

    #[test]
    fn error_responses_use_closed_outcomes_and_retry_classes() {
        let cases = [
            (
                ErrorKind::ERROR_KIND_UNAVAILABLE,
                RetryClass::RETRY_CLASS_AFTER_OBSERVATION,
            ),
            (
                ErrorKind::ERROR_KIND_DEADLINE_EXCEEDED,
                RetryClass::RETRY_CLASS_SAME_OPERATION,
            ),
            (
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
                RetryClass::RETRY_CLASS_NEVER,
            ),
            (
                ErrorKind::ERROR_KIND_UNAUTHENTICATED,
                RetryClass::RETRY_CLASS_NEVER,
            ),
            (
                ErrorKind::ERROR_KIND_UNAUTHORIZED,
                RetryClass::RETRY_CLASS_NEVER,
            ),
            (
                ErrorKind::ERROR_KIND_NOT_FOUND,
                RetryClass::RETRY_CLASS_NEVER,
            ),
            (
                ErrorKind::ERROR_KIND_CONFLICT,
                RetryClass::RETRY_CLASS_NEVER,
            ),
            (
                ErrorKind::ERROR_KIND_CAPABILITY_DENIED,
                RetryClass::RETRY_CLASS_NEVER,
            ),
            (
                ErrorKind::ERROR_KIND_CANCELLED,
                RetryClass::RETRY_CLASS_NEVER,
            ),
            (
                ErrorKind::ERROR_KIND_GENERATION_MISMATCH,
                RetryClass::RETRY_CLASS_NEVER,
            ),
            (
                ErrorKind::ERROR_KIND_RESOURCE_EXHAUSTED,
                RetryClass::RETRY_CLASS_NEVER,
            ),
            (
                ErrorKind::ERROR_KIND_INVARIANT_VIOLATION,
                RetryClass::RETRY_CLASS_NEVER,
            ),
            (
                ErrorKind::ERROR_KIND_INTERNAL,
                RetryClass::RETRY_CLASS_NEVER,
            ),
            (
                ErrorKind::ERROR_KIND_UNSPECIFIED,
                RetryClass::RETRY_CLASS_NEVER,
            ),
        ];
        for (kind, retry) in cases {
            let response = error_response(kind);
            assert_eq!(
                response.outcome.enum_value().unwrap(),
                Outcome::OUTCOME_FAILED
            );
            let error = response.error.as_ref().unwrap();
            assert_eq!(error.kind.enum_value().unwrap(), kind);
            assert_eq!(error.retry.enum_value().unwrap(), retry);
            assert!(error.correlation_id.is_empty());
        }
    }

    #[test]
    fn endpoint_policy_is_same_uid_and_runtime_specific() {
        assert!(endpoint_policy(0, 0, 1).is_none());
        let uid = geteuid().as_raw();
        if uid == 0 {
            return;
        }
        let policy = endpoint_policy(uid, nix::unistd::getegid().as_raw(), 9).unwrap();
        assert_eq!(policy.purpose, EndpointPurpose::RuntimeSystemdUser);
        assert_eq!(policy.responder_role, EndpointRole::RuntimeSystemdUserAgent);
        assert_eq!(policy.reconnect_generation, 9);
        assert_eq!(policy.attachment_policy.max_per_packet, 1);
    }

    #[tokio::test]
    async fn socket_peer_admission_requires_current_non_root_uid() {
        let (left, _right) = prearmed_seqpacket_pair().unwrap();
        let socket = SeqpacketSocket::from_owned(left).unwrap();
        let peer = socket.acceptor_peer_credentials().unwrap();
        assert_eq!(peer.uid().as_raw(), geteuid().as_raw());
        assert_eq!(
            peer_is_authorized(
                peer.uid().as_raw(),
                geteuid().as_raw(),
                &ControllerAllowlist::empty()
            ),
            geteuid().as_raw() != 0
        );
    }

    #[test]
    fn peer_admission_accepts_the_exact_allowlisted_controller() {
        let allowlist = ControllerAllowlist::resolve(
            controller_allowlist_document(&[("alice", &[1234])]),
            "alice",
            &no_principal_names(),
        )
        .unwrap();
        assert!(peer_is_authorized(1234, 1000, &allowlist));
    }

    #[test]
    fn peer_admission_denies_an_unrelated_controller() {
        let allowlist = ControllerAllowlist::resolve(
            controller_allowlist_document(&[("alice", &[1234])]),
            "alice",
            &no_principal_names(),
        )
        .unwrap();
        // 1300 is a real, distinct controller uid but was never granted to
        // this requester.
        assert!(!peer_is_authorized(1300, 1000, &allowlist));
    }

    #[test]
    fn peer_admission_denies_an_unrelated_users_controller() {
        // The document authorizes 1234 only for bob, not for alice.
        let allowlist = ControllerAllowlist::resolve(
            controller_allowlist_document(&[("bob", &[1234])]),
            "alice",
            &no_principal_names(),
        )
        .unwrap();
        assert!(!peer_is_authorized(1234, 1000, &allowlist));
    }

    #[test]
    fn peer_admission_denies_root_regardless_of_allowlist_or_own_uid() {
        // A document that ever tried to authorize uid 0 fails closed at
        // parse time (see controller_allowlist::tests), so uid 0 can never
        // legitimately appear in a resolved allowlist. `peer_is_authorized`
        // additionally defends in depth against uid 0 on either side.
        let empty = ControllerAllowlist::empty();
        assert!(!peer_is_authorized(0, 1000, &empty));
        assert!(!peer_is_authorized(1000, 0, &empty));
    }

    #[test]
    fn peer_admission_still_accepts_the_same_uid_direct_path() {
        let allowlist = ControllerAllowlist::empty();
        assert!(peer_is_authorized(1000, 1000, &allowlist));
    }

    #[test]
    fn malformed_allowlist_document_never_authorizes_a_foreign_uid() {
        for bytes in [
            &b"not-json"[..],
            br#"{"schemaVersion":1,"entries":[{"user":"alice","controllerUids":[0]}]}"#,
            br#"{"schemaVersion":1,"entries":[{"user":"alice","controllerUids":[1300,1234]}]}"#,
        ] {
            assert!(ControllerAllowlist::resolve(bytes, "alice", &no_principal_names()).is_err());
        }
    }

    #[test]
    fn admission_never_selects_or_changes_the_execution_uid() {
        // `peer_is_authorized` is a pure predicate: it takes both uids and
        // an allowlist and returns a bool. There is no code path by which an
        // authorized peer uid can become the uid the helper executes
        // requests as -- that identity is fixed once, in `run`, from the
        // process's own real/effective uid, before any peer is ever
        // accepted.
        let allowlist = ControllerAllowlist::resolve(
            controller_allowlist_document(&[("alice", &[1234])]),
            "alice",
            &no_principal_names(),
        )
        .unwrap();
        assert!(peer_is_authorized(1234, 1000, &allowlist));
        // Swapping which side is "own" vs "peer" must not also authorize --
        // authorization is not symmetric execution-identity selection.
        assert!(!peer_is_authorized(1000, 1234, &allowlist));
    }

    /// The local-root controller (`d2bd`) has no `stablePrincipalId`-derived
    /// fixed uid at Nix eval time, so it is granted by stable *name* in the
    /// generated document and resolved to its exact uid once, here, exactly
    /// as `load_controller_allowlist` does at real helper startup.
    #[test]
    fn peer_admission_accepts_local_root_controller_resolved_by_stable_name() {
        let document = controller_allowlist_document_with_names(&[("alice", &[], &["d2bd"])]);
        let lookup = |name: &str| (name == "d2bd").then_some(64_100);
        let allowlist = ControllerAllowlist::resolve(document, "alice", &lookup).unwrap();
        assert!(peer_is_authorized(64_100, 1000, &allowlist));
        // An unrelated principal name never resolved must grant nothing.
        assert!(!peer_is_authorized(64_200, 1000, &allowlist));
    }

    fn no_principal_names() -> impl Fn(&str) -> Option<u32> {
        |_| None
    }

    fn controller_allowlist_document(entries: &[(&str, &[u32])]) -> &'static [u8] {
        controller_allowlist_document_with_names(
            &entries
                .iter()
                .map(|(user, uids)| (*user, *uids, [].as_slice()))
                .collect::<Vec<_>>(),
        )
    }

    fn controller_allowlist_document_with_names(
        entries: &[(&str, &[u32], &[&str])],
    ) -> &'static [u8] {
        // Only literal fixtures are exercised through this helper; leaking a
        // small boxed buffer keeps call sites terse without unsafe code.
        let entries: Vec<serde_json::Value> = entries
            .iter()
            .map(|(user, uids, names)| {
                serde_json::json!({
                    "user": user,
                    "controllerUids": uids,
                    "controllerPrincipalNames": names,
                })
            })
            .collect();
        let document = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "entries": entries,
        }))
        .unwrap();
        Box::leak(document.into_boxed_slice())
    }

    #[test]
    fn registry_wires_real_runtime_and_shell_backends_while_tty_stays_one_shot() {
        // `TtyService` keeps the frozen one-shot-elsewhere-owned stub
        // (`TtyUnavailable`, exercised by
        // `unavailable_tty_method_returns_a_typed_result` below); Runtime and
        // Shell now dispatch through the real `SystemdUserScopeManager` +
        // `ShellSupervisor` composition, so `EnsureScope` actually creates a
        // transient user scope here and `StopProcess` tears it back down.
        if geteuid().is_root() {
            return;
        }
        let generation = 7;
        let state = Arc::new(SessionServices::new(generation));
        let registry = service_registry(&state);
        assert!(registry.contains_key("d2b.runtime.systemd-user.v2.RuntimeSystemdUserService"));
        assert!(registry.contains_key("d2b.shell.v2.ShellService"));
        assert!(registry.contains_key("d2b.tty.v2.TtyService"));

        let runtime = RuntimeAdapter(Arc::clone(&state));
        let ensured =
            dispatch_ensure_scope_until_settled(&runtime, generation, "runtime-registry-wiring");
        assert_eq!(
            ensured.outcome.enum_value().unwrap(),
            Outcome::OUTCOME_SUCCEEDED,
            "EnsureScope must reach the real backend, not a fabricated unavailable stub"
        );
        let stopped =
            dispatch_stop_process_until_settled(&runtime, generation, "runtime-registry-wiring");
        assert_eq!(
            stopped.outcome.enum_value().unwrap(),
            Outcome::OUTCOME_SUCCEEDED,
            "StopProcess must tear the real scope this test created back down"
        );

        let shell = ShellAdapter(Arc::clone(&state));
        let listed = shell
            .dispatch(
                ShellMethod::List,
                request(generation, WireDesiredState::DESIRED_STATE_UNSPECIFIED),
            )
            .unwrap();
        assert_eq!(
            listed.outcome.enum_value().unwrap(),
            Outcome::OUTCOME_SUCCEEDED
        );
    }

    #[tokio::test]
    async fn unavailable_tty_method_returns_a_typed_result() {
        let context = TtrpcContext {
            mh: ttrpc::MessageHeader::default(),
            metadata: HashMap::new(),
            timeout_nano: 0,
        };
        let response = TtyTtrpc::enter_raw_mode(&TtyUnavailable, &context, ServiceRequest::new())
            .await
            .unwrap();
        assert_eq!(
            response.error.as_ref().unwrap().kind.enum_value().unwrap(),
            ErrorKind::ERROR_KIND_UNAVAILABLE
        );
    }

    struct FakeActivatedListener {
        accepted: Mutex<Option<SeqpacketSocket>>,
        calls: AtomicUsize,
    }

    #[async_trait]
    impl ActivatedListener for FakeActivatedListener {
        async fn accept(&self) -> Result<SeqpacketSocket, ServerError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let accepted = self
                .accepted
                .lock()
                .map_err(|_| ServerError::Activation)?
                .take();
            match accepted {
                Some(socket) => Ok(socket),
                None => std::future::pending().await,
            }
        }
    }

    #[tokio::test]
    async fn activation_loop_stops_and_closes_active_sessions() {
        let (left, right) = prearmed_seqpacket_pair().unwrap();
        let listener = FakeActivatedListener {
            accepted: Mutex::new(Some(SeqpacketSocket::from_owned(left).unwrap())),
            calls: AtomicUsize::new(0),
        };
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let shutdown = async {
            shutdown_rx.await.map_err(|_| ServerError::Signal)?;
            Ok(())
        };
        let task = tokio::spawn(async move {
            serve_until_shutdown(&listener, 1, &ControllerAllowlist::empty(), shutdown)
                .await
                .map(|()| listener.calls.load(Ordering::Relaxed))
        });
        tokio::task::yield_now().await;
        shutdown_tx.send(()).unwrap();
        assert!(task.await.unwrap().unwrap() >= 1);
        drop(right);
    }

    #[test]
    fn debug_and_errors_do_not_expose_request_identity() {
        // A closed-error dispatch (mismatched session generation, rejected
        // by `admit_request` before ever touching `RealBackend`) must never
        // echo the caller-supplied resource id back in either the response
        // or the adapter's own `Debug` output.
        let canary = "private-request-canary";
        let adapter = RuntimeAdapter(Arc::new(SessionServices::new(1)));
        let mut wire = request(1, WireDesiredState::DESIRED_STATE_PRESENT);
        wire.resource_id = canary.to_owned();
        wire.metadata.as_mut().unwrap().session_generation = 999;
        let response = adapter.dispatch(RuntimeMethod::EnsureScope, wire).unwrap();
        assert_eq!(
            response.error.as_ref().unwrap().kind.enum_value().unwrap(),
            ErrorKind::ERROR_KIND_GENERATION_MISMATCH
        );
        assert!(!format!("{response:?}").contains(canary));
        assert!(!format!("{adapter:?}").contains(canary));
    }

    #[test]
    fn successful_response_only_echoes_the_callers_own_resource_id() {
        // By contrast, a *successful* dispatch legitimately echoes the
        // caller-supplied `resource_id` back as `resource_handle` (the
        // client already knows it; this is correlation, not a leak) while
        // still never exposing anything the backend derived internally
        // (real scope unit names, pty paths, uid).
        if geteuid().is_root() {
            return;
        }
        let generation = 1;
        let adapter = RuntimeAdapter(Arc::new(SessionServices::new(generation)));
        let resource_id = "caller-owned-resource";
        let response = dispatch_until_settled(|| {
            let mut wire = request(generation, WireDesiredState::DESIRED_STATE_PRESENT);
            wire.resource_id = resource_id.to_owned();
            adapter.dispatch(RuntimeMethod::EnsureScope, wire).unwrap()
        });
        assert_eq!(
            response.outcome.enum_value().unwrap(),
            Outcome::OUTCOME_SUCCEEDED
        );
        assert_eq!(response.resource_handle, resource_id);
        let stopped = dispatch_stop_process_until_settled(&adapter, generation, resource_id);
        assert_eq!(
            stopped.outcome.enum_value().unwrap(),
            Outcome::OUTCOME_SUCCEEDED
        );
    }

    /// Minimal `ComponentSessionDriver` test double. `open_terminal_impl`
    /// and `attach_impl` call exactly one driver method on this path —
    /// `receive_attachments()` — so every other method is deliberately
    /// `unimplemented!()` rather than faked: a hermetic test exercising the
    /// real inbound-attachment wiring must fail loudly if the production
    /// code path under test ever starts depending on one of them.
    struct FakeAttachmentDriver {
        attachments: Mutex<Option<Vec<OwnedAttachment>>>,
    }

    impl FakeAttachmentDriver {
        fn once(attachment: OwnedAttachment) -> Arc<Self> {
            Arc::new(Self {
                attachments: Mutex::new(Some(vec![attachment])),
            })
        }
    }

    #[async_trait]
    impl ComponentSessionDriver for FakeAttachmentDriver {
        fn generation(&self) -> u64 {
            0
        }

        async fn start_ttrpc(
            &self,
            _request_id: RequestId,
            _frame: Vec<u8>,
        ) -> d2b_session::Result<()> {
            unimplemented!("not exercised by the inbound-attachment round trip")
        }

        async fn complete_ttrpc(&self, _request_id: RequestId) -> d2b_session::Result<bool> {
            unimplemented!("not exercised by the inbound-attachment round trip")
        }

        async fn cancel(
            &self,
            _generation: u64,
            _request_id: RequestId,
        ) -> d2b_session::Result<()> {
            unimplemented!("not exercised by the inbound-attachment round trip")
        }

        async fn send_ttrpc(&self, _frame: Vec<u8>) -> d2b_session::Result<()> {
            unimplemented!("not exercised by the inbound-attachment round trip")
        }

        async fn receive_ttrpc(&self) -> d2b_session::Result<Vec<u8>> {
            unimplemented!("not exercised by the inbound-attachment round trip")
        }

        async fn register_inbound_call(
            &self,
            _request_id: RequestId,
        ) -> d2b_session::Result<Cancellation> {
            unimplemented!("not exercised by the inbound-attachment round trip")
        }

        async fn complete_inbound_call(&self, _request_id: RequestId) -> d2b_session::Result<bool> {
            unimplemented!("not exercised by the inbound-attachment round trip")
        }

        async fn remove_inbound_call(&self, _request_id: RequestId) -> d2b_session::Result<bool> {
            unimplemented!("not exercised by the inbound-attachment round trip")
        }

        async fn send_attachments(
            &self,
            _attachments: Vec<OwnedAttachment>,
        ) -> d2b_session::Result<()> {
            unimplemented!("not exercised by the inbound-attachment round trip")
        }

        async fn receive_attachments(&self) -> d2b_session::Result<Vec<OwnedAttachment>> {
            Ok(self.attachments.lock().unwrap().take().unwrap_or_default())
        }

        async fn open_named_stream(
            &self,
            _stream: StreamId,
            _send_credit: u32,
            _receive_credit: u32,
        ) -> d2b_session::Result<()> {
            unimplemented!("not exercised by the inbound-attachment round trip")
        }

        async fn send_named_stream(
            &self,
            _stream: StreamId,
            _bytes: Vec<u8>,
        ) -> d2b_session::Result<()> {
            unimplemented!("not exercised by the inbound-attachment round trip")
        }

        async fn receive_named_stream(&self) -> d2b_session::Result<StreamEvent> {
            unimplemented!("not exercised by the inbound-attachment round trip")
        }

        async fn grant_named_stream_credit(
            &self,
            _stream: StreamId,
            _bytes: u32,
        ) -> d2b_session::Result<()> {
            unimplemented!("not exercised by the inbound-attachment round trip")
        }

        async fn close_named_stream(&self, _stream: StreamId) -> d2b_session::Result<()> {
            unimplemented!("not exercised by the inbound-attachment round trip")
        }

        async fn reset_named_stream(&self, _stream: StreamId) -> d2b_session::Result<()> {
            unimplemented!("not exercised by the inbound-attachment round trip")
        }

        async fn drive_keepalive(&self, _now: Instant) -> d2b_session::Result<()> {
            unimplemented!("not exercised by the inbound-attachment round trip")
        }

        async fn receive_control(&self) -> d2b_session::Result<d2b_session::SessionEvent> {
            unimplemented!("not exercised by the inbound-attachment round trip")
        }

        async fn close(
            &self,
            _reason: CloseReason,
            _remediation: Remediation,
        ) -> d2b_session::Result<()> {
            unimplemented!("not exercised by the inbound-attachment round trip")
        }
    }

    /// Builds the exact bound attachment `take_terminal_attachment` accepts:
    /// a connected, CLOEXEC `SOCK_STREAM` half at index 0, `Terminal`
    /// purpose, matching service/method/session-generation/request-id, and
    /// the fixed six-class credit set every valid descriptor carries. This
    /// mirrors what the real client-side transport constructs when it sends
    /// its own socketpair half in as the request's declared attachment.
    fn terminal_attachment(
        fd: OwnedFd,
        service: ServicePackage,
        method_id: u32,
        request_id: [u8; 16],
        generation: u64,
    ) -> OwnedAttachment {
        terminal_attachment_with(fd, service, method_id, request_id, generation, 0, true).unwrap()
    }

    /// Full-control variant of [`terminal_attachment`] used by the negative
    /// `take_terminal_attachment` cases below to vary `index` and
    /// `cloexec_required` independently of the happy-path defaults.
    #[allow(clippy::too_many_arguments)]
    fn terminal_attachment_with(
        fd: OwnedFd,
        service: ServicePackage,
        method_id: u32,
        request_id: [u8; 16],
        generation: u64,
        index: u16,
        cloexec_required: bool,
    ) -> Result<OwnedAttachment, UnixSessionError> {
        let identity = ObjectIdentity::from_trusted(
            &fd,
            KernelObjectType::UnixStreamSocket,
            AttachmentAccess::ReadWrite,
        )
        .unwrap();
        let descriptor = AttachmentDescriptor {
            index,
            kind: AttachmentKind::FileDescriptor,
            object_type: KernelObjectType::UnixStreamSocket,
            access: AttachmentAccess::ReadWrite,
            purpose: AttachmentPurpose::Terminal,
            service,
            method_id,
            request_id: RequestId::new(request_id.to_vec()).unwrap(),
            operation_id: None,
            packet_sequence: 0,
            reconnect_generation: generation,
            duplicate_object_allowed: false,
            cloexec_required,
            credit_classes: BoundedVec::new(vec![
                AttachmentCreditClass::Packet,
                AttachmentCreditClass::Request,
                AttachmentCreditClass::Operation,
                AttachmentCreditClass::Session,
                AttachmentCreditClass::Process,
                AttachmentCreditClass::Host,
            ])
            .unwrap(),
        };
        OwnedUnixAttachment::file(descriptor, fd, DescriptorPolicy::File(identity))
    }

    fn test_context() -> TtrpcContext {
        TtrpcContext {
            mh: ttrpc::MessageHeader::default(),
            metadata: HashMap::new(),
            timeout_nano: 0,
        }
    }

    /// Reads from `stream` until either `needle` shows up in the
    /// accumulated bytes or the deadline passes, tolerating the shell's
    /// own startup chatter (prompts, job-control notices) ahead of the
    /// echoed command output.
    fn read_until_contains(stream: &mut UnixStream, needle: &[u8], deadline: Instant) -> Vec<u8> {
        stream
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        let mut collected = Vec::new();
        let mut buffer = [0u8; 512];
        while Instant::now() < deadline {
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    collected.extend_from_slice(&buffer[..n]);
                    if collected
                        .windows(needle.len().max(1))
                        .any(|window| window == needle)
                    {
                        break;
                    }
                }
                Err(ref error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.kind() == std::io::ErrorKind::TimedOut => {}
                Err(error) => panic!("pump read failed: {error}"),
            }
        }
        collected
    }

    #[tokio::test]
    async fn open_terminal_wires_a_real_byte_pump_through_the_inbound_attachment() {
        // The full production shape: `EnsureScope` creates a real
        // pty-backed resource, then `OpenTerminal` consumes a
        // client-constructed connected socketpair half as this exact
        // request's declared inbound attachment and wires a background
        // pump between it and that resource's real pty master — exercised
        // here exactly as the real client-side transport would drive it,
        // with a real byte round trip through the pty, not a fabricated
        // success.
        if geteuid().is_root() {
            return;
        }
        let generation = 31;
        let state = Arc::new(SessionServices::new(generation));
        let runtime = RuntimeAdapter(Arc::clone(&state));
        let resource_id = "runtime-terminal-round-trip";
        let ensured = dispatch_ensure_scope_until_settled(&runtime, generation, resource_id);
        assert_eq!(
            ensured.outcome.enum_value().unwrap(),
            Outcome::OUTCOME_SUCCEEDED
        );

        let (mut kept, given) = UnixStream::pair().unwrap();
        let request_id = [9u8; 16];
        let attachment = terminal_attachment(
            given.into(),
            ServicePackage::RuntimeSystemdUserV2,
            RUNTIME_OPEN_TERMINAL_METHOD_ID,
            request_id,
            generation,
        );
        state.set_driver(FakeAttachmentDriver::once(attachment));

        let mut wire = request(generation, WireDesiredState::DESIRED_STATE_ATTACHED);
        wire.resource_id = resource_id.to_owned();
        wire.metadata.as_mut().unwrap().request_id = request_id.to_vec();
        wire.stream_id = "terminal".to_owned();
        wire.attachment_indexes = vec![0];
        let response = RuntimeSystemdUserTtrpc::open_terminal(&runtime, &test_context(), wire)
            .await
            .unwrap();
        assert_eq!(
            response.outcome.enum_value().unwrap(),
            Outcome::OUTCOME_SUCCEEDED,
            "OpenTerminal must reach the real backend and wire the pump, not fabricate success"
        );

        let deadline = Instant::now() + Duration::from_secs(10);
        kept.write_all(b"echo pty-round-trip-ok\n").unwrap();
        let observed = read_until_contains(&mut kept, b"pty-round-trip-ok", deadline);
        assert!(
            observed
                .windows("pty-round-trip-ok".len())
                .any(|window| window == b"pty-round-trip-ok"),
            "expected the pty-echoed bytes to reach the client end of the real pump, got {observed:?}"
        );

        drop(kept);
        let stopped = dispatch_stop_process_until_settled(&runtime, generation, resource_id);
        assert_eq!(
            stopped.outcome.enum_value().unwrap(),
            Outcome::OUTCOME_SUCCEEDED
        );
    }

    #[tokio::test]
    async fn shell_attach_wires_a_real_byte_pump_through_the_inbound_attachment() {
        // Shell's equivalent of the Runtime test above: `Create` makes a
        // real persistent-shell scope, then `Attach` consumes a
        // client-constructed connected socketpair half and wires the same
        // kind of real background pump to that shell's pty master.
        if geteuid().is_root() {
            return;
        }
        let generation = 32;
        let state = Arc::new(SessionServices::new(generation));
        let shell = ShellAdapter(Arc::clone(&state));
        let resource_id = "shell-attach-round-trip";
        let created = dispatch_shell_create_until_settled(&shell, generation, resource_id);
        assert_eq!(
            created.outcome.enum_value().unwrap(),
            Outcome::OUTCOME_SUCCEEDED
        );

        let (mut kept, given) = UnixStream::pair().unwrap();
        let request_id = [11u8; 16];
        let attachment = terminal_attachment(
            given.into(),
            ServicePackage::ShellV2,
            crate::supervisor_protocol::ATTACH_METHOD_ID,
            request_id,
            generation,
        );
        state.set_driver(FakeAttachmentDriver::once(attachment));

        let mut wire = request(generation, WireDesiredState::DESIRED_STATE_UNSPECIFIED);
        wire.resource_id = resource_id.to_owned();
        wire.metadata.as_mut().unwrap().request_id = request_id.to_vec();
        wire.stream_id = "terminal".to_owned();
        wire.attachment_indexes = vec![0];
        let response = ShellTtrpc::attach(&shell, &test_context(), wire)
            .await
            .unwrap();
        assert_eq!(
            response.outcome.enum_value().unwrap(),
            Outcome::OUTCOME_SUCCEEDED,
            "Attach must reach the real backend and wire the pump, not fabricate success"
        );

        let deadline = Instant::now() + Duration::from_secs(10);
        kept.write_all(b"echo shell-round-trip-ok\n").unwrap();
        let observed = read_until_contains(&mut kept, b"shell-round-trip-ok", deadline);
        assert!(
            observed
                .windows("shell-round-trip-ok".len())
                .any(|window| window == b"shell-round-trip-ok"),
            "expected the pty-echoed bytes to reach the client end of the real pump, got {observed:?}"
        );

        drop(kept);
        let killed = dispatch_kill_shell_until_settled(&shell, generation, resource_id);
        assert_eq!(
            killed.outcome.enum_value().unwrap(),
            Outcome::OUTCOME_SUCCEEDED
        );
    }

    #[test]
    fn take_terminal_attachment_accepts_only_the_exact_declared_binding() {
        let service = ServicePackage::RuntimeSystemdUserV2;
        let method_id = RUNTIME_OPEN_TERMINAL_METHOD_ID;
        let request_id = [7u8; 16];
        let generation = 42;
        let fresh = || {
            let (_, given) = UnixStream::pair().unwrap();
            terminal_attachment(given.into(), service, method_id, request_id, generation)
        };

        // Baseline: an exact match yields the duplicated fd.
        assert!(
            take_terminal_attachment(&[fresh()], service, method_id, request_id, generation)
                .is_some()
        );

        // No attachments, or more than one, are both rejected: the wire
        // contract is exactly one declared attachment per request.
        assert!(
            take_terminal_attachment(&[], service, method_id, request_id, generation).is_none()
        );
        assert!(
            take_terminal_attachment(
                &[fresh(), fresh()],
                service,
                method_id,
                request_id,
                generation
            )
            .is_none()
        );

        // Wrong service package (e.g. a Shell attachment presented to the
        // Runtime `OpenTerminal` path).
        assert!(
            take_terminal_attachment(
                &[fresh()],
                ServicePackage::ShellV2,
                method_id,
                request_id,
                generation
            )
            .is_none()
        );

        // Wrong method id (a different request's declared attachment).
        assert!(
            take_terminal_attachment(
                &[fresh()],
                service,
                method_id.wrapping_add(1),
                request_id,
                generation
            )
            .is_none()
        );

        // Wrong request id (correlates to a different, unrelated request).
        assert!(
            take_terminal_attachment(&[fresh()], service, method_id, [8u8; 16], generation)
                .is_none()
        );

        // Wrong session generation (a stale attachment from a prior
        // connection generation must never bind to the current one).
        assert!(
            take_terminal_attachment(&[fresh()], service, method_id, request_id, generation + 1)
                .is_none()
        );

        // Wrong attachment index: the wire contract fixes the terminal
        // attachment at index 0.
        let (_, given) = UnixStream::pair().unwrap();
        let wrong_index = terminal_attachment_with(
            given.into(),
            service,
            method_id,
            request_id,
            generation,
            1,
            true,
        )
        .unwrap();
        assert!(
            take_terminal_attachment(&[wrong_index], service, method_id, request_id, generation)
                .is_none()
        );

        // A descriptor claiming `cloexec_required: false` on a
        // `FileDescriptor`-kind attachment is rejected by the transport
        // layer itself before `take_terminal_attachment` ever runs: the
        // wire contract has no non-CLOEXEC file-descriptor attachment.
        let (_, given) = UnixStream::pair().unwrap();
        assert!(matches!(
            terminal_attachment_with(
                given.into(),
                service,
                method_id,
                request_id,
                generation,
                0,
                false
            ),
            Err(UnixSessionError::DescriptorMismatch)
        ));
    }

    #[test]
    fn shell_cancel_reports_generation_mismatch_completed_and_unknown_requests() {
        let generation = 5;
        let state = Arc::new(SessionServices::new(generation));
        let shell = ShellAdapter(Arc::clone(&state));

        // Generation mismatch is reported before any request-id lookup.
        let mut wrong_generation = WireCancelRequest::new();
        wrong_generation.session_generation = generation + 1;
        wrong_generation.request_id = vec![1; 16];
        let response = shell.cancel(wrong_generation).unwrap();
        assert_eq!(
            response.outcome.enum_value().unwrap(),
            WireCancelOutcome::CANCEL_OUTCOME_GENERATION_MISMATCH
        );

        // An all-zero request id is never a real request.
        let mut zero_request = WireCancelRequest::new();
        zero_request.session_generation = generation;
        zero_request.request_id = vec![0; 16];
        let response = shell.cancel(zero_request).unwrap();
        assert_eq!(
            response.outcome.enum_value().unwrap(),
            WireCancelOutcome::CANCEL_OUTCOME_UNKNOWN_REQUEST
        );

        // A request id this state has never completed is unknown.
        let mut unknown = WireCancelRequest::new();
        unknown.session_generation = generation;
        unknown.request_id = vec![3; 16];
        let response = shell.cancel(unknown).unwrap();
        assert_eq!(
            response.outcome.enum_value().unwrap(),
            WireCancelOutcome::CANCEL_OUTCOME_UNKNOWN_REQUEST
        );

        // Once recorded completed, the same request id is reported
        // terminal instead of fabricating a still-running cancellation.
        state.record_completed([3; 16]);
        let mut completed = WireCancelRequest::new();
        completed.session_generation = generation;
        completed.request_id = vec![3; 16];
        let response = shell.cancel(completed).unwrap();
        assert_eq!(
            response.outcome.enum_value().unwrap(),
            WireCancelOutcome::CANCEL_OUTCOME_ALREADY_TERMINAL
        );
    }

    #[test]
    fn runtime_cancel_rejects_malformed_request_ids_before_touching_composition() {
        let generation = 6;
        let runtime = RuntimeAdapter(Arc::new(SessionServices::new(generation)));

        // No composition has ever been created; any request id is unknown.
        let mut wire = WireCancelRequest::new();
        wire.session_generation = generation;
        wire.request_id = vec![1; 16];
        let response = runtime.cancel(wire).unwrap();
        assert_eq!(
            response.outcome.enum_value().unwrap(),
            WireCancelOutcome::CANCEL_OUTCOME_UNKNOWN_REQUEST
        );

        // A wrong-length request id is rejected before ever touching
        // composition.
        let mut malformed = WireCancelRequest::new();
        malformed.session_generation = generation;
        malformed.request_id = vec![1; 4];
        let response = runtime.cancel(malformed).unwrap();
        assert_eq!(
            response.outcome.enum_value().unwrap(),
            WireCancelOutcome::CANCEL_OUTCOME_UNKNOWN_REQUEST
        );

        // An all-zero request id is never a real request either.
        let mut zero = WireCancelRequest::new();
        zero.session_generation = generation;
        zero.request_id = vec![0; 16];
        let response = runtime.cancel(zero).unwrap();
        assert_eq!(
            response.outcome.enum_value().unwrap(),
            WireCancelOutcome::CANCEL_OUTCOME_UNKNOWN_REQUEST
        );
    }

    #[test]
    fn runtime_cancel_reports_already_terminal_for_a_request_that_already_completed() {
        // Cancellation binds to the exact in-flight request id, deadline,
        // and session generation the completed-request tracker already
        // observed: a `Cancel` for a request that already reached a
        // terminal dispatch outcome must honestly report
        // `AlreadyTerminal`, never fabricate a still-cancellable one.
        if geteuid().is_root() {
            return;
        }
        let generation = 33;
        let state = Arc::new(SessionServices::new(generation));
        let runtime = RuntimeAdapter(Arc::clone(&state));
        let resource_id = "runtime-cancel-terminal";
        let ensured = dispatch_ensure_scope_until_settled(&runtime, generation, resource_id);
        assert_eq!(
            ensured.outcome.enum_value().unwrap(),
            Outcome::OUTCOME_SUCCEEDED
        );

        // `request()`'s fixed request id ([1; 16]) is exactly what
        // `EnsureScope` just completed under.
        let mut wire = WireCancelRequest::new();
        wire.session_generation = generation;
        wire.request_id = vec![1; 16];
        let response = runtime.cancel(wire).unwrap();
        assert_eq!(
            response.outcome.enum_value().unwrap(),
            WireCancelOutcome::CANCEL_OUTCOME_ALREADY_TERMINAL
        );

        let stopped = dispatch_stop_process_until_settled(&runtime, generation, resource_id);
        assert_eq!(
            stopped.outcome.enum_value().unwrap(),
            Outcome::OUTCOME_SUCCEEDED
        );
    }

    #[tokio::test]
    async fn shell_detach_stops_the_pump_without_killing_the_still_running_scope() {
        // `Detach` is pure `ShellSupervisor` bookkeeping: it must end the
        // byte relay (the client observes EOF) while leaving the shell's
        // real scope and process running, unlike `Kill`.
        if geteuid().is_root() {
            return;
        }
        let generation = 34;
        let state = Arc::new(SessionServices::new(generation));
        let shell = ShellAdapter(Arc::clone(&state));
        let resource_id = "shell-detach-round-trip";
        let created = dispatch_shell_create_until_settled(&shell, generation, resource_id);
        assert_eq!(
            created.outcome.enum_value().unwrap(),
            Outcome::OUTCOME_SUCCEEDED
        );

        let (mut kept, given) = UnixStream::pair().unwrap();
        let request_id = [13u8; 16];
        let attachment = terminal_attachment(
            given.into(),
            ServicePackage::ShellV2,
            crate::supervisor_protocol::ATTACH_METHOD_ID,
            request_id,
            generation,
        );
        state.set_driver(FakeAttachmentDriver::once(attachment));
        let mut wire = request(generation, WireDesiredState::DESIRED_STATE_UNSPECIFIED);
        wire.resource_id = resource_id.to_owned();
        wire.metadata.as_mut().unwrap().request_id = request_id.to_vec();
        wire.stream_id = "terminal".to_owned();
        wire.attachment_indexes = vec![0];
        let attached = ShellTtrpc::attach(&shell, &test_context(), wire)
            .await
            .unwrap();
        assert_eq!(
            attached.outcome.enum_value().unwrap(),
            Outcome::OUTCOME_SUCCEEDED
        );

        let mut detach = request(generation, WireDesiredState::DESIRED_STATE_UNSPECIFIED);
        detach.resource_id = resource_id.to_owned();
        detach.metadata.as_mut().unwrap().request_id = vec![14; 16];
        detach.stream_id = "terminal".to_owned();
        let detached = shell.dispatch(ShellMethod::Detach, detach).unwrap();
        assert_eq!(
            detached.outcome.enum_value().unwrap(),
            Outcome::OUTCOME_SUCCEEDED
        );

        // The pump is stopped synchronously by `Detach`. Any bytes the
        // shell had already written to the pty before the stop landed
        // (e.g. its startup prompt) may still be in flight, so drain
        // until the socket reaches real EOF rather than asserting the
        // very first read is already empty.
        kept.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut buffer = [0u8; 4096];
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match kept.read(&mut buffer) {
                Ok(0) => break,
                Ok(_) if Instant::now() < deadline => continue,
                Ok(_) => panic!(
                    "Detach must close the pump's end of the client socket, not leave it dangling"
                ),
                Err(error) => panic!("expected EOF after Detach, got a read error: {error}"),
            }
        }

        // The scope itself must still be running: `Inspect` still succeeds
        // (Detach never removed or killed the resource).
        let mut inspect = request(generation, WireDesiredState::DESIRED_STATE_UNSPECIFIED);
        inspect.resource_id = resource_id.to_owned();
        inspect.metadata.as_mut().unwrap().request_id = vec![15; 16];
        let inspected = shell.dispatch(ShellMethod::Inspect, inspect).unwrap();
        assert_eq!(
            inspected.outcome.enum_value().unwrap(),
            Outcome::OUTCOME_SUCCEEDED,
            "Detach must never kill the still-running scope it detaches from"
        );

        let killed = dispatch_kill_shell_until_settled(&shell, generation, resource_id);
        assert_eq!(
            killed.outcome.enum_value().unwrap(),
            Outcome::OUTCOME_SUCCEEDED
        );
    }
}
