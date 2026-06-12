use std::{
    env, fs,
    fs::File,
    io::{Read, Result as IoResult},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard,
    },
    task::{Context, Poll},
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use futures::stream;
use nixling_ipc::{guest_proto as pb, guest_wire::GUEST_CONTROL_PROTOCOL_VERSION};
use protobuf::{EnumOrUnknown, Message, MessageField};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    time::Duration,
};
use tokio_vsock::{VsockAddr, VsockListener, VMADDR_CID_ANY};

use crate::{
    auth::{
        AuthConnectionContext, AuthDirection, AuthPurpose, BootIdSource, CapabilitiesProvider,
        CapabilitiesSnapshot, GuestAuthCore, GuestAuthError, InMemoryChallengeStore, NonceRng,
        SharedSecretToken, AUTH_NONCE_LEN, CONNECTION_INSTANCE_LEN, GUEST_CONTROL_AUTH_PORT,
    },
    detached::{RunnerUnitPaths, SystemdRunUnitManager},
    detached_registry::{
        DetachedRegistry, RegistryConfig, RunSlotStore, SystemWallClock, TokioSleeper,
    },
    exec::{
        ExecCreateInput, ExecError, ExecPolicy, ExecRuntime, ExecSnapshot,
        ExecState as RtExecState, ExitOutcome, Stream as RtStream,
    },
    exec_linux::LinuxProcessSpawner,
    generated::guest_control_ttrpc::{create_guest_control, GuestControl},
};

/// Server-generated opaque exec id source backed by `/dev/urandom`.
pub struct OsExecIds;

impl crate::exec::ExecIdSource for OsExecIds {
    fn next_exec_id(&self) -> Result<String, ExecError> {
        let mut bytes = [0_u8; 16];
        OsNonceRng
            .fill_bytes(&mut bytes)
            .map_err(|_| ExecError::Internal)?;
        let mut out = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            use std::fmt::Write as _;
            let _ = write!(out, "{byte:02x}");
        }
        Ok(out)
    }
}

type RuntimeExec = ExecRuntime<LinuxProcessSpawner, OsExecIds>;
type SharedExec = Arc<RuntimeExec>;
/// The cross-connection detached-exec registry, present only when the host
/// wired detached runtime constants into the guest unit.
type SharedDetached = Option<Arc<DetachedRegistry>>;

const TOKEN_FILE_NAME: &str = "guest_control_token";
const MAX_TOKEN_BYTES: usize = 4096;
/// Cadence of the periodic detached-exec reaper (live reconciliation of
/// vanished units + terminal-record TTL/GC).
const DETACHED_REAPER_INTERVAL_MS: u64 = 30_000;

type RuntimeAuthCore = GuestAuthCore<
    SharedSecretToken,
    OsNonceRng,
    ProcBootIdSource,
    RuntimeCapabilitiesProvider,
    InMemoryChallengeStore,
    SystemClock,
>;
type SharedAuthCore = Arc<Mutex<RuntimeAuthCore>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestdServiceError {
    MissingCredentialsDirectory,
    UnsafeCredential,
    TokenUnavailable,
    Io,
    TimeUnavailable,
    Ttrpc,
}

impl GuestdServiceError {
    pub fn public_message(self) -> &'static str {
        match self {
            Self::MissingCredentialsDirectory => {
                "guest-control credential directory is unavailable"
            }
            Self::UnsafeCredential => "guest-control credential is unsafe",
            Self::TokenUnavailable => "guest-control token is unavailable",
            Self::Io => "guest-control I/O failed",
            Self::TimeUnavailable => "guest-control clock is unavailable",
            Self::Ttrpc => "guest-control service failed",
        }
    }
}

#[derive(Clone)]
pub struct GuestdServeConfig {
    pub vm_id: String,
    pub token: Vec<u8>,
    pub exec_policy: ExecPolicy,
    /// Present when the host wired detached-exec runtime constants into the
    /// guest unit. `None` => detached exec is unsupported (attached only).
    pub detached: Option<DetachedRuntimeConfig>,
}

/// Host-supplied, controlled-constant runtime configuration for detached exec.
/// All paths are absolute store paths passed by the guest module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetachedRuntimeConfig {
    /// Absolute path to `systemd-run`.
    pub systemd_run_path: PathBuf,
    /// Absolute path to the `nixling-exec-runner` binary.
    pub exec_runner_path: PathBuf,
    /// Default per-exec runtime ceiling in seconds; 0 means unlimited.
    pub max_runtime_sec: u64,
}

impl GuestdServeConfig {
    pub fn new(vm_id: impl Into<String>, token: Vec<u8>) -> Result<Self, GuestdServiceError> {
        Self::with_exec_policy(vm_id, token, ExecPolicy::disabled())
    }

    pub fn with_exec_policy(
        vm_id: impl Into<String>,
        token: Vec<u8>,
        exec_policy: ExecPolicy,
    ) -> Result<Self, GuestdServiceError> {
        let vm_id = vm_id.into();
        if vm_id.is_empty() || token.is_empty() {
            return Err(GuestdServiceError::TokenUnavailable);
        }
        Ok(Self {
            vm_id,
            token,
            exec_policy,
            detached: None,
        })
    }

    /// Attach host-supplied detached runtime constants.
    pub fn with_detached(mut self, detached: DetachedRuntimeConfig) -> Self {
        self.detached = Some(detached);
        self
    }
}

pub fn load_token_from_credentials_env() -> Result<Vec<u8>, GuestdServiceError> {
    let dir = env::var_os("CREDENTIALS_DIRECTORY")
        .map(PathBuf::from)
        .ok_or(GuestdServiceError::MissingCredentialsDirectory)?;
    load_token_from_credentials_dir(&dir)
}

pub fn load_token_from_credentials_dir(dir: &Path) -> Result<Vec<u8>, GuestdServiceError> {
    validate_safe_directory_path(dir)?;
    let path = dir.join(TOKEN_FILE_NAME);
    validate_token_path(dir, &path)?;
    let mut file = File::open(&path).map_err(|_| GuestdServiceError::TokenUnavailable)?;
    let mut data = Vec::new();
    file.by_ref()
        .take((MAX_TOKEN_BYTES + 1) as u64)
        .read_to_end(&mut data)
        .map_err(|_| GuestdServiceError::Io)?;
    if data.is_empty() || data.len() > MAX_TOKEN_BYTES {
        return Err(GuestdServiceError::TokenUnavailable);
    }
    while matches!(data.last(), Some(b'\n' | b'\r')) {
        data.pop();
    }
    if data.is_empty() {
        return Err(GuestdServiceError::TokenUnavailable);
    }
    Ok(data)
}

fn validate_token_path(dir: &Path, path: &Path) -> Result<(), GuestdServiceError> {
    if path.parent() != Some(dir) {
        return Err(GuestdServiceError::UnsafeCredential);
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| GuestdServiceError::TokenUnavailable)?;
    if metadata.file_type().is_symlink()
        || !metadata.file_type().is_file()
        || metadata.mode() & 0o077 != 0
        || !owner_is_safe(metadata.uid())
    {
        return Err(GuestdServiceError::UnsafeCredential);
    }
    Ok(())
}

fn validate_safe_directory_path(dir: &Path) -> Result<(), GuestdServiceError> {
    if !dir.is_absolute() {
        return Err(GuestdServiceError::MissingCredentialsDirectory);
    }
    if dir == Path::new("/nix/store") || dir.starts_with("/nix/store/") {
        return Err(GuestdServiceError::UnsafeCredential);
    }
    let mut current = PathBuf::new();
    for component in dir.components() {
        current.push(component);
        let metadata =
            fs::symlink_metadata(&current).map_err(|_| GuestdServiceError::UnsafeCredential)?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
            return Err(GuestdServiceError::UnsafeCredential);
        }
        let mode = metadata.mode();
        if !owner_is_safe(metadata.uid()) {
            return Err(GuestdServiceError::UnsafeCredential);
        }
        if mode & 0o022 != 0 {
            return Err(GuestdServiceError::UnsafeCredential);
        }
    }
    Ok(())
}

fn owner_is_safe(uid: u32) -> bool {
    uid == 0 || cfg!(test)
}

/// Runtime-usability flags that gate the advertised guest capabilities. A
/// feature is advertised only when it is both configured AND usable, so a
/// configured-but-broken feature's first call returns a typed error instead of
/// being silently advertised.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct CapabilitiesConfig {
    pub exec_attached: bool,
    pub exec_detached: bool,
    pub exec_logs: bool,
}

pub fn build_runtime_auth_core(
    token: Vec<u8>,
    capabilities: CapabilitiesConfig,
) -> Result<RuntimeAuthCore, GuestdServiceError> {
    let token = SharedSecretToken::new(token).map_err(|_| GuestdServiceError::TokenUnavailable)?;
    Ok(GuestAuthCore::new(
        token,
        OsNonceRng,
        ProcBootIdSource,
        RuntimeCapabilitiesProvider::new(capabilities),
        InMemoryChallengeStore::default(),
        SystemClock,
    ))
}

pub async fn serve_vsock(config: GuestdServeConfig) -> Result<(), GuestdServiceError> {
    let exec_enabled_root = config.exec_policy.enabled && config.exec_policy.allow_root;

    // Build the cross-connection detached registry (shared by all connections)
    // when the host wired detached runtime constants AND exec is usable.
    let detached: SharedDetached = match (&config.detached, exec_enabled_root) {
        (Some(detached_cfg), true) if detached_runtime_usable(detached_cfg) => {
            let boot_id = ProcBootIdSource
                .guest_boot_id()
                .map_err(|_| GuestdServiceError::Io)?;
            let registry = build_detached_registry(detached_cfg, boot_id);
            // Re-adopt durable records before serving so a guestd restart never
            // kills a still-running adopted unit and lost ids remain listable.
            registry.reconcile_on_startup().await;
            // Periodic reaper: live reconciliation of vanished units + TTL/GC.
            let reaper = Arc::clone(&registry);
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(DETACHED_REAPER_INTERVAL_MS)).await;
                    reaper.reap_once().await;
                }
            });
            Some(registry)
        }
        _ => None,
    };

    let capabilities = CapabilitiesConfig {
        exec_attached: exec_enabled_root,
        exec_detached: detached.is_some(),
        // Retained logs share the detached store + quota.
        exec_logs: detached.is_some(),
    };

    let auth = Arc::new(Mutex::new(build_runtime_auth_core(
        config.token,
        capabilities,
    )?));
    let exec: SharedExec = Arc::new(ExecRuntime::new(
        LinuxProcessSpawner,
        OsExecIds,
        config.exec_policy,
    ));
    let listener = VsockListener::bind(VsockAddr::new(VMADDR_CID_ANY, GUEST_CONTROL_AUTH_PORT))
        .map_err(|_| GuestdServiceError::Ttrpc)?;

    loop {
        let Ok((stream, peer_addr)) = listener.accept().await else {
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        };
        let auth = Arc::clone(&auth);
        let exec = Arc::clone(&exec);
        let detached = detached.clone();
        let vm_id = config.vm_id.clone();
        tokio::spawn(async move {
            if let Ok(context) = connection_context(vm_id, peer_addr.cid()) {
                let _ = run_single_connection(stream, auth, exec, detached, context).await;
            }
        });
    }
}

/// Build the production detached registry: `systemd-run`/`systemctl` unit
/// manager, `/run/nixling-exec` slot store, system wall clock, tokio sleeper,
/// and `/dev/urandom` exec ids.
fn build_detached_registry(
    detached: &DetachedRuntimeConfig,
    boot_id: String,
) -> Arc<DetachedRegistry> {
    let units = Arc::new(SystemdRunUnitManager::new(detached.systemd_run_path.clone()));
    let store = Arc::new(RunSlotStore::new());
    let clock = Arc::new(SystemWallClock);
    let sleeper = Arc::new(TokioSleeper);
    let ids = Arc::new(OsExecIds);
    let registry_config = RegistryConfig {
        paths: RunnerUnitPaths::new(detached.exec_runner_path.clone()),
        boot_id,
        max_runtime_sec: detached.max_runtime_sec,
    };
    Arc::new(DetachedRegistry::new(
        units,
        store,
        clock,
        sleeper,
        ids,
        registry_config,
    ))
}

/// Runtime-usability probe for detached exec: the `systemd-run` + runner
/// binaries must exist and `/run/nixling-exec` must be a root-owned directory.
fn detached_runtime_usable(detached: &DetachedRuntimeConfig) -> bool {
    if !detached.systemd_run_path.is_file() || !detached.exec_runner_path.is_file() {
        return false;
    }
    match fs::symlink_metadata(nixling_exec_runner::paths::RUN_DIR) {
        Ok(meta) => meta.is_dir() && owner_is_safe(meta.uid()),
        Err(_) => false,
    }
}

fn connection_context(
    vm_id: String,
    peer_cid: u32,
) -> Result<AuthConnectionContext, GuestdServiceError> {
    Ok(AuthConnectionContext {
        vm_id,
        protocol_version: GUEST_CONTROL_PROTOCOL_VERSION,
        guest_control_port: GUEST_CONTROL_AUTH_PORT,
        peer_cid,
        direction: AuthDirection::HostToGuest,
        purpose: AuthPurpose::GuestControlAuthV1,
        connection_instance: new_connection_instance()?,
    })
}

fn new_connection_instance() -> Result<[u8; CONNECTION_INSTANCE_LEN], GuestdServiceError> {
    let mut instance = [0_u8; CONNECTION_INSTANCE_LEN];
    let mut rng = OsNonceRng;
    rng.fill_bytes(&mut instance)
        .map_err(|_| GuestdServiceError::TokenUnavailable)?;
    Ok(instance)
}

pub async fn run_single_connection<S>(
    stream: S,
    auth: SharedAuthCore,
    exec: SharedExec,
    detached: SharedDetached,
    context: AuthConnectionContext,
) -> Result<(), GuestdServiceError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
{
    let cleanup = ConnectionCleanup::new(Arc::clone(&auth), Arc::clone(&exec), context.clone());
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let wrapped = CleanupStream::new(stream, cleanup.clone(), done_tx);
    let listener = ttrpc::r#async::transport::Listener::new(stream::once(async move {
        Ok::<_, std::io::Error>(wrapped)
    }));
    let service = Arc::new(GuestControlService::new(auth, exec, detached, context));
    let mut server = ttrpc::r#async::Server::new()
        .add_listener(listener)
        .register_service(create_guest_control(service));
    server
        .start()
        .await
        .map_err(|_| GuestdServiceError::Ttrpc)?;
    let _ = done_rx.await;
    cleanup.close();
    server.disconnect().await;
    Ok(())
}

#[derive(Clone)]
pub struct GuestControlService {
    auth: SharedAuthCore,
    exec: SharedExec,
    detached: SharedDetached,
    context: AuthConnectionContext,
}

impl GuestControlService {
    pub fn new(
        auth: SharedAuthCore,
        exec: SharedExec,
        detached: SharedDetached,
        context: AuthConnectionContext,
    ) -> Self {
        Self {
            auth,
            exec,
            detached,
            context,
        }
    }

    fn lock_auth(&self) -> Result<MutexGuard<'_, RuntimeAuthCore>, ttrpc::Error> {
        self.auth
            .lock()
            .map_err(|_| rpc_status(ttrpc::Code::INTERNAL, "guest-control-internal-error"))
    }

    fn require_authenticated(&self) -> Result<(), ttrpc::Error> {
        if self.lock_auth()?.is_authenticated(&self.context) {
            Ok(())
        } else {
            Err(rpc_status(
                ttrpc::Code::UNAUTHENTICATED,
                "guest-control-unauthenticated",
            ))
        }
    }

    fn validate_metadata(
        &self,
        metadata: Option<&pb::RequestMetadata>,
    ) -> Result<(), ttrpc::Error> {
        let metadata = metadata.ok_or_else(|| {
            rpc_status(
                ttrpc::Code::INVALID_ARGUMENT,
                "guest-control-metadata-invalid",
            )
        })?;
        if metadata.vm_id != self.context.vm_id
            || metadata.protocol_version != self.context.protocol_version
        {
            return Err(rpc_status(
                ttrpc::Code::INVALID_ARGUMENT,
                "guest-control-metadata-invalid",
            ));
        }
        Ok(())
    }

    /// The owning connection identity for execs created on this connection.
    fn connection_key(&self) -> Vec<u8> {
        self.context.connection_instance.to_vec()
    }

    /// Validate exec request metadata and return the bound `(exec_id,
    /// guest_boot_id)`. The common metadata must match the connection context.
    fn validate_exec_metadata<'a>(
        &self,
        metadata: Option<&'a pb::ExecRequestMetadata>,
    ) -> Result<(&'a str, &'a str), ttrpc::Error> {
        let metadata = metadata.ok_or_else(|| {
            rpc_status(
                ttrpc::Code::INVALID_ARGUMENT,
                "guest-control-metadata-invalid",
            )
        })?;
        self.validate_metadata(metadata.common.as_ref())?;
        if metadata.exec_id.is_empty() || metadata.guest_boot_id.is_empty() {
            return Err(rpc_status(
                ttrpc::Code::INVALID_ARGUMENT,
                "guest-control-metadata-invalid",
            ));
        }
        Ok((&metadata.exec_id, &metadata.guest_boot_id))
    }

    /// Handle a `detached = true` create: validate (allowing detached, rejecting
    /// interactive flags), then route to the cross-connection detached registry.
    /// When detached exec is unconfigured/unusable, return a typed disabled
    /// error (the `EXEC_DETACHED` capability is not advertised in that case).
    async fn exec_create_detached(
        &self,
        input: ExecCreateInput,
        guest_boot_id: &str,
    ) -> ttrpc::Result<pb::ExecCreateResponse> {
        let Some(registry) = self.detached.as_ref() else {
            let mut response = pb::ExecCreateResponse::new();
            response.state = EnumOrUnknown::new(pb::ExecState::EXEC_STATE_PROTOCOL_ERROR);
            response.error = MessageField::some(exec_disabled_error());
            return Ok(response);
        };

        let command =
            match crate::exec::validate_and_authorize_detached(&input, self.exec.policy()) {
                Ok(command) => command,
                Err(error) => {
                    let mut response = pb::ExecCreateResponse::new();
                    response.state = EnumOrUnknown::new(pb::ExecState::EXEC_STATE_PROTOCOL_ERROR);
                    response.error = MessageField::some(guest_error_kind(error));
                    return Ok(response);
                }
            };

        match registry
            .create(guest_boot_id, command, registry.default_caps())
            .await
        {
            Ok((exec_id, snapshot)) => {
                let mut response = pb::ExecCreateResponse::new();
                response.exec_id = Some(exec_id);
                response.stdout_cursor = snapshot.stdout_start_offset;
                response.stderr_cursor = snapshot.stderr_start_offset;
                response.effective_limits = MessageField::some(effective_limits());
                response.state = EnumOrUnknown::new(wire_exec_state(snapshot.state));
                Ok(response)
            }
            Err(error) => {
                let mut response = pb::ExecCreateResponse::new();
                response.state = EnumOrUnknown::new(pb::ExecState::EXEC_STATE_PROTOCOL_ERROR);
                response.error = MessageField::some(guest_error_kind(error));
                Ok(response)
            }
        }
    }
}

/// Map a runtime stream enum to/from the wire enum.
fn rt_stream(stream: pb::OutputStream) -> Result<RtStream, ttrpc::Error> {
    match stream {
        pb::OutputStream::OUTPUT_STREAM_STDOUT => Ok(RtStream::Stdout),
        pb::OutputStream::OUTPUT_STREAM_STDERR => Ok(RtStream::Stderr),
        pb::OutputStream::OUTPUT_STREAM_UNSPECIFIED => Err(rpc_status(
            ttrpc::Code::INVALID_ARGUMENT,
            "guest-control-stream-invalid",
        )),
    }
}

fn wire_stream(stream: RtStream) -> pb::OutputStream {
    match stream {
        RtStream::Stdout => pb::OutputStream::OUTPUT_STREAM_STDOUT,
        RtStream::Stderr => pb::OutputStream::OUTPUT_STREAM_STDERR,
    }
}

fn wire_exec_state(state: RtExecState) -> pb::ExecState {
    match state {
        RtExecState::Running => pb::ExecState::EXEC_STATE_RUNNING,
        RtExecState::Exited => pb::ExecState::EXEC_STATE_EXITED,
        RtExecState::Signaled => pb::ExecState::EXEC_STATE_SIGNALED,
        RtExecState::Cancelled => pb::ExecState::EXEC_STATE_CANCELLED,
        RtExecState::Reaped => pb::ExecState::EXEC_STATE_REAPED,
        RtExecState::LostGuestd => pb::ExecState::EXEC_STATE_LOST_GUESTD,
    }
}

fn wire_terminal_status(snapshot: &ExecSnapshot) -> MessageField<pb::TerminalStatus> {
    match snapshot.outcome {
        Some(ExitOutcome::Exited(code)) => {
            let mut status = pb::TerminalStatus::new();
            status.set_exit_code(code);
            MessageField::some(status)
        }
        Some(ExitOutcome::Signaled(signal)) => {
            let mut status = pb::TerminalStatus::new();
            status.set_signal(signal);
            MessageField::some(status)
        }
        None => MessageField::none(),
    }
}

fn guest_error_kind(error: ExecError) -> pb::GuestControlError {
    guest_error(wire_error_kind(error))
}

fn inspect_response(snapshot: &ExecSnapshot) -> pb::ExecInspectResponse {
    let mut response = pb::ExecInspectResponse::new();
    response.state = EnumOrUnknown::new(wire_exec_state(snapshot.state));
    response.visible_terminal_status = wire_terminal_status(snapshot);
    // Non-interactive execs run with stdin closed.
    response.stdin_state = EnumOrUnknown::new(pb::StdinState::STDIN_STATE_CLOSED);
    response.stdout_start_offset = snapshot.stdout_start_offset;
    response.stdout_end_offset = snapshot.stdout_end_offset;
    response.stderr_start_offset = snapshot.stderr_start_offset;
    response.stderr_end_offset = snapshot.stderr_end_offset;
    response.stdout_dropped_bytes = snapshot.stdout_dropped_bytes;
    response.stderr_dropped_bytes = snapshot.stderr_dropped_bytes;
    response.stdout_truncated_for_retention = snapshot.stdout_truncated;
    response.stderr_truncated_for_retention = snapshot.stderr_truncated;
    response.state_generation = snapshot.state_generation;
    response
}

fn wire_error_kind(error: ExecError) -> pb::GuestControlErrorKind {
    use pb::GuestControlErrorKind as Pb;
    // Exhaustive match on `ExecError` so adding a runtime variant without a
    // wire mapping is a compile error rather than a silent `ProtocolError`
    // swallow. The supported attached/detached protocol subset maps
    // mode/validation faults to `ProtocolError` deliberately.
    match error {
        ExecError::ExecDisabled => Pb::GUEST_CONTROL_ERROR_KIND_GUEST_EXEC_DISABLED,
        ExecError::RootDenied => Pb::GUEST_CONTROL_ERROR_KIND_GUEST_EXEC_ROOT_DENIED,
        ExecError::UserDenied => Pb::GUEST_CONTROL_ERROR_KIND_GUEST_EXEC_USER_DENIED,
        ExecError::UnsupportedMode => Pb::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR,
        ExecError::InvalidArgv => Pb::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR,
        ExecError::CwdInvalid => Pb::GUEST_CONTROL_ERROR_KIND_CWD_INVALID,
        ExecError::InvalidEnv => Pb::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR,
        ExecError::MaxChunkExceeded => Pb::GUEST_CONTROL_ERROR_KIND_MAX_CHUNK_EXCEEDED,
        ExecError::ExecCapacityExceeded => Pb::GUEST_CONTROL_ERROR_KIND_EXEC_CAPACITY_EXCEEDED,
        ExecError::AttachCapacityExceeded => {
            Pb::GUEST_CONTROL_ERROR_KIND_EXEC_ATTACH_CAPACITY_EXCEEDED
        }
        ExecError::WaitCapacityExceeded => Pb::GUEST_CONTROL_ERROR_KIND_WAIT_CAPACITY_EXCEEDED,
        ExecError::ReadWaitCapacityExceeded => {
            Pb::GUEST_CONTROL_ERROR_KIND_READ_WAIT_CAPACITY_EXCEEDED
        }
        ExecError::ExecNotFound => Pb::GUEST_CONTROL_ERROR_KIND_EXEC_NOT_FOUND,
        ExecError::OffsetExpired => Pb::GUEST_CONTROL_ERROR_KIND_OFFSET_EXPIRED,
        ExecError::OffsetInFuture => Pb::GUEST_CONTROL_ERROR_KIND_OFFSET_IN_FUTURE,
        ExecError::SpawnFailed => Pb::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR,
        ExecError::RetainedLogPathUnsafe => Pb::GUEST_CONTROL_ERROR_KIND_RETAINED_LOG_PATH_UNSAFE,
        ExecError::RetainedLogQuotaExceeded => {
            Pb::GUEST_CONTROL_ERROR_KIND_RETAINED_LOG_QUOTA_EXCEEDED
        }
        ExecError::StaleSession => Pb::GUEST_CONTROL_ERROR_KIND_STALE_SESSION,
        ExecError::ExecExpired => Pb::GUEST_CONTROL_ERROR_KIND_EXEC_EXPIRED,
        ExecError::Internal => Pb::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR,
    }
}

/// Advertised effective limits. Shared by the capabilities snapshot and
/// exec responses so both report identical bounds.
pub fn effective_limits() -> pb::GuestEffectiveLimits {
    let mut limits = pb::GuestEffectiveLimits::new();
    limits.max_chunk_bytes = 64 * 1024;
    limits.max_recv_message_bytes = 4 * 1024 * 1024;
    limits.decoded_write_stdin_bytes_per_connection = 16 * 1024 * 1024;
    limits.write_stdin_handlers_per_connection = 4;
    limits.stdin_queue_chunks_per_exec = 1;
    limits.stdout_live_buffer_bytes = crate::exec::STDOUT_LIVE_BUFFER_BYTES as u64;
    limits.stderr_live_buffer_bytes = crate::exec::STDERR_LIVE_BUFFER_BYTES as u64;
    limits.detached_stdout_log_bytes = nixling_exec_runner::DETACHED_STREAM_LOG_BYTES;
    limits.detached_stderr_log_bytes = nixling_exec_runner::DETACHED_STREAM_LOG_BYTES;
    limits.long_poll_timeout_ms = 100;
    limits.slow_consumer_grace_ms = 30_000;
    limits.exec_sessions_per_vm = crate::exec::EXEC_SESSIONS_PER_VM as u32;
    limits.attached_sessions_per_vm = crate::exec::ATTACHED_SESSIONS_PER_VM as u32;
    limits.pending_read_output_waits_per_stream =
        crate::exec::PENDING_READ_OUTPUT_WAITS_PER_STREAM as u32;
    limits.pending_exec_waits_per_vm = crate::exec::PENDING_EXEC_WAITS_PER_VM as u32;
    limits.rpc_rate_per_connection_per_second = 200;
    limits.rpc_rate_per_vm_burst = 1_000;
    limits
}

#[async_trait]
impl GuestControl for GuestControlService {
    async fn hello(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::HelloRequest,
    ) -> ttrpc::Result<pb::HelloResponse> {
        self.lock_auth()?
            .hello(&self.context, &request)
            .map_err(map_auth_rpc_error)
    }

    async fn authenticate(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::AuthenticateRequest,
    ) -> ttrpc::Result<pb::AuthenticateResponse> {
        match self.lock_auth()?.authenticate(&self.context, &request) {
            Ok(response) => Ok(response),
            Err(error) => {
                let mut response = pb::AuthenticateResponse::new();
                response.error = MessageField::some(guest_error(error_kind_for_auth(error)));
                Ok(response)
            }
        }
    }

    async fn capabilities(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::CapabilitiesRequest,
    ) -> ttrpc::Result<pb::CapabilitiesResponse> {
        self.validate_metadata(request.metadata.as_ref())?;
        self.lock_auth()?
            .capabilities(&self.context)
            .map_err(map_auth_rpc_error)
    }

    async fn health(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::HealthRequest,
    ) -> ttrpc::Result<pb::HealthResponse> {
        self.validate_metadata(request.metadata.as_ref())?;
        self.lock_auth()?
            .health(&self.context)
            .map_err(map_auth_rpc_error)
    }

    async fn exec_create(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::ExecCreateRequest,
    ) -> ttrpc::Result<pb::ExecCreateResponse> {
        // Auth is enforced before any validation, id allocation, or spawn.
        self.require_authenticated()?;
        self.validate_metadata(request.metadata.as_ref())?;

        let input = ExecCreateInput {
            argv: request.argv.clone(),
            user: request.user.clone(),
            cwd: request.cwd.clone(),
            env: request
                .env
                .iter()
                .map(|var| (var.key.clone(), var.value.clone()))
                .collect(),
            tty: request.tty,
            stdin_open: request.stdin_open,
            detached: request.detached,
            has_terminal_size: request.initial_terminal_size.is_some(),
            max_chunk_bytes: request
                .output_policy
                .as_ref()
                .map(|policy| policy.max_chunk_bytes)
                .unwrap_or(0),
        };

        // The exec is bound to the guest's current boot id so a stale client
        // after a guestd restart cannot match it.
        let guest_boot_id = ProcBootIdSource
            .guest_boot_id()
            .map_err(|_| rpc_status(ttrpc::Code::INTERNAL, "guest-control-internal-error"))?;

        if input.detached {
            return self.exec_create_detached(input, &guest_boot_id).await;
        }

        match self
            .exec
            .create(self.connection_key(), guest_boot_id, input)
            .await
        {
            Ok((exec_id, snapshot)) => {
                let mut response = pb::ExecCreateResponse::new();
                response.exec_id = Some(exec_id);
                response.stdout_cursor = snapshot.stdout_start_offset;
                response.stderr_cursor = snapshot.stderr_start_offset;
                response.effective_limits = MessageField::some(effective_limits());
                response.state = EnumOrUnknown::new(wire_exec_state(snapshot.state));
                Ok(response)
            }
            Err(error) => {
                let mut response = pb::ExecCreateResponse::new();
                response.state = EnumOrUnknown::new(pb::ExecState::EXEC_STATE_PROTOCOL_ERROR);
                response.error = MessageField::some(guest_error_kind(error));
                Ok(response)
            }
        }
    }

    async fn exec_inspect(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::ExecInspectRequest,
    ) -> ttrpc::Result<pb::ExecInspectResponse> {
        self.require_authenticated()?;
        let (exec_id, guest_boot_id) = self.validate_exec_metadata(request.metadata.as_ref())?;
        let mut result = self
            .exec
            .inspect(&self.connection_key(), exec_id, guest_boot_id);
        // Fall back to the cross-connection detached registry (same-VM + boot
        // visibility) when the id is not an attached exec on this connection.
        if let (Err(ExecError::ExecNotFound), Some(registry)) = (&result, self.detached.as_ref()) {
            result = registry.inspect(exec_id, guest_boot_id).await;
        }
        match result {
            Ok(snapshot) => Ok(inspect_response(&snapshot)),
            Err(error) => {
                let mut response = pb::ExecInspectResponse::new();
                response.state = EnumOrUnknown::new(pb::ExecState::EXEC_STATE_PROTOCOL_ERROR);
                response.error = MessageField::some(guest_error_kind(error));
                Ok(response)
            }
        }
    }

    async fn exec_wait(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::ExecWaitRequest,
    ) -> ttrpc::Result<pb::ExecWaitResponse> {
        self.require_authenticated()?;
        let (exec_id, guest_boot_id) = self.validate_exec_metadata(request.metadata.as_ref())?;
        let known = request.known_state_generation;
        let mut result = self
            .exec
            .wait(
                &self.connection_key(),
                exec_id,
                guest_boot_id,
                known,
                request.timeout_ms,
            )
            .await;
        // Fall back to the detached registry's bounded status-poll wait.
        if let (Err(ExecError::ExecNotFound), Some(registry)) = (&result, self.detached.as_ref()) {
            result = registry
                .wait(exec_id, guest_boot_id, known, request.timeout_ms)
                .await;
        }
        match result {
            Ok((snapshot, timed_out)) => {
                let mut response = pb::ExecWaitResponse::new();
                response.state = EnumOrUnknown::new(wire_exec_state(snapshot.state));
                response.visible_terminal_status = wire_terminal_status(&snapshot);
                response.state_generation = snapshot.state_generation;
                response.stdout_start_offset = snapshot.stdout_start_offset;
                response.stdout_end_offset = snapshot.stdout_end_offset;
                response.stderr_start_offset = snapshot.stderr_start_offset;
                response.stderr_end_offset = snapshot.stderr_end_offset;
                response.timed_out = timed_out;
                Ok(response)
            }
            Err(error) => {
                let mut response = pb::ExecWaitResponse::new();
                response.state = EnumOrUnknown::new(pb::ExecState::EXEC_STATE_PROTOCOL_ERROR);
                response.error = MessageField::some(guest_error_kind(error));
                Ok(response)
            }
        }
    }

    async fn exec_logs(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::ExecLogsRequest,
    ) -> ttrpc::Result<pb::ExecLogsResponse> {
        self.require_authenticated()?;
        let Some(registry) = self.detached.as_ref() else {
            let mut response = pb::ExecLogsResponse::new();
            response.error = MessageField::some(exec_disabled_error());
            return Ok(response);
        };
        let (exec_id, guest_boot_id) = self.validate_exec_metadata(request.metadata.as_ref())?;
        let stream = match request.stream.enum_value() {
            Ok(stream) => rt_stream(stream)?,
            Err(_) => {
                return Err(rpc_status(
                    ttrpc::Code::INVALID_ARGUMENT,
                    "guest-control-stream-invalid",
                ))
            }
        };
        match registry
            .read_logs(exec_id, guest_boot_id, stream, request.offset, request.max_len)
            .await
        {
            Ok(chunk) => {
                let mut response = pb::ExecLogsResponse::new();
                response.stream = EnumOrUnknown::new(wire_stream(stream));
                response.offset = request.offset;
                response.end_offset = chunk.end_offset;
                response.data = chunk.data;
                response.next_offset = chunk.next_offset;
                response.eof = chunk.eof;
                response.start_offset = chunk.start_offset;
                response.dropped_bytes = chunk.dropped_bytes;
                response.truncated = chunk.truncated;
                Ok(response)
            }
            Err(error) => {
                let mut response = pb::ExecLogsResponse::new();
                response.stream = EnumOrUnknown::new(wire_stream(stream));
                response.error = MessageField::some(guest_error_kind(error));
                Ok(response)
            }
        }
    }

    async fn exec_list(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::ExecListRequest,
    ) -> ttrpc::Result<pb::ExecListResponse> {
        self.require_authenticated()?;
        let Some(registry) = self.detached.as_ref() else {
            let mut response = pb::ExecListResponse::new();
            response.error = MessageField::some(exec_disabled_error());
            return Ok(response);
        };
        self.validate_metadata(request.metadata.as_ref())?;
        if request.guest_boot_id.is_empty() {
            return Err(rpc_status(
                ttrpc::Code::INVALID_ARGUMENT,
                "guest-control-metadata-invalid",
            ));
        }
        match registry.list(&request.guest_boot_id).await {
            Ok(entries) => {
                let mut response = pb::ExecListResponse::new();
                for entry in entries {
                    let mut wire = pb::ExecListEntry::new();
                    wire.exec_id = entry.exec_id;
                    wire.slot = entry.slot;
                    wire.state = EnumOrUnknown::new(wire_exec_state(entry.state));
                    wire.create_time_unix = entry.create_time_unix;
                    wire.argv_sha256 = entry.argv_sha256;
                    wire.stdout_truncated = entry.stdout_truncated;
                    wire.stderr_truncated = entry.stderr_truncated;
                    wire.dropped_bytes = entry.dropped_bytes;
                    response.entries.push(wire);
                }
                Ok(response)
            }
            Err(error) => {
                let mut response = pb::ExecListResponse::new();
                response.error = MessageField::some(guest_error_kind(error));
                Ok(response)
            }
        }
    }

    async fn write_stdin(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        _request: pb::WriteStdinRequest,
    ) -> ttrpc::Result<pb::WriteStdinResponse> {
        self.require_authenticated()?;
        let mut response = pb::WriteStdinResponse::new();
        response.error = MessageField::some(exec_disabled_error());
        Ok(response)
    }

    async fn read_output(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::ReadOutputRequest,
    ) -> ttrpc::Result<pb::ReadOutputResponse> {
        self.require_authenticated()?;
        let (exec_id, guest_boot_id) = self.validate_exec_metadata(request.metadata.as_ref())?;
        let stream = match request.stream.enum_value() {
            Ok(stream) => rt_stream(stream)?,
            Err(_) => {
                return Err(rpc_status(
                    ttrpc::Code::INVALID_ARGUMENT,
                    "guest-control-stream-invalid",
                ))
            }
        };
        match self
            .exec
            .read_output(
                &self.connection_key(),
                exec_id,
                guest_boot_id,
                stream,
                request.offset,
                request.max_len,
                request.wait,
                request.timeout_ms,
            )
            .await
        {
            Ok((chunk, timed_out)) => {
                let mut response = pb::ReadOutputResponse::new();
                response.stream = EnumOrUnknown::new(wire_stream(stream));
                response.offset = request.offset;
                response.end_offset = chunk.end_offset;
                response.data = chunk.data;
                response.next_offset = chunk.next_offset;
                response.eof = chunk.eof;
                response.start_offset = chunk.start_offset;
                response.dropped_bytes = chunk.dropped_bytes;
                response.truncated = chunk.truncated;
                response.timed_out = timed_out;
                Ok(response)
            }
            Err(error) => {
                let mut response = pb::ReadOutputResponse::new();
                response.stream = request.stream;
                response.error = MessageField::some(guest_error_kind(error));
                Ok(response)
            }
        }
    }

    async fn close_stdin(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        _request: pb::CloseStdinRequest,
    ) -> ttrpc::Result<pb::CloseStdinResponse> {
        self.require_authenticated()?;
        let mut response = pb::CloseStdinResponse::new();
        response.error = MessageField::some(exec_disabled_error());
        Ok(response)
    }

    async fn tty_win_resize(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        _request: pb::TtyWinResizeRequest,
    ) -> ttrpc::Result<pb::ControlAck> {
        self.require_authenticated()?;
        Ok(control_ack_disabled())
    }

    async fn exec_signal(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        _request: pb::ExecSignalRequest,
    ) -> ttrpc::Result<pb::ControlAck> {
        self.require_authenticated()?;
        Ok(control_ack_disabled())
    }

    async fn exec_cancel(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::ExecCancelRequest,
    ) -> ttrpc::Result<pb::ControlAck> {
        self.require_authenticated()?;
        let Some(registry) = self.detached.as_ref() else {
            return Ok(control_ack_disabled());
        };
        let (exec_id, guest_boot_id) = self.validate_exec_metadata(request.metadata.as_ref())?;
        match registry.cancel(exec_id, guest_boot_id).await {
            Ok(duplicate) => {
                let mut ack = pb::ControlAck::new();
                ack.control_seq = request.control_seq;
                ack.duplicate = duplicate;
                Ok(ack)
            }
            Err(error) => {
                let mut ack = pb::ControlAck::new();
                ack.control_seq = request.control_seq;
                ack.error = MessageField::some(guest_error_kind(error));
                Ok(ack)
            }
        }
    }
}

fn map_auth_rpc_error(error: GuestAuthError) -> ttrpc::Error {
    match error {
        GuestAuthError::Unauthenticated | GuestAuthError::MacRejected => rpc_status(
            ttrpc::Code::UNAUTHENTICATED,
            "guest-control-unauthenticated",
        ),
        GuestAuthError::ChallengeCapacityExceeded => rpc_status(
            ttrpc::Code::RESOURCE_EXHAUSTED,
            "guest-control-auth-capacity",
        ),
        GuestAuthError::MetadataMissing
        | GuestAuthError::MetadataMismatch
        | GuestAuthError::ProtocolVersionMismatch
        | GuestAuthError::TranscriptVersionMismatch
        | GuestAuthError::NonceLengthInvalid
        | GuestAuthError::TagLengthInvalid
        | GuestAuthError::BootIdMismatch
        | GuestAuthError::ChallengeNotFound
        | GuestAuthError::ChallengeExpired
        | GuestAuthError::ChallengeMismatch => {
            rpc_status(ttrpc::Code::INVALID_ARGUMENT, "guest-control-auth-invalid")
        }
        _ => rpc_status(ttrpc::Code::INTERNAL, "guest-control-internal-error"),
    }
}

fn rpc_status(code: ttrpc::Code, message: &'static str) -> ttrpc::Error {
    ttrpc::Error::RpcStatus(ttrpc::get_status(code, message.to_owned()))
}

fn guest_error(kind: pb::GuestControlErrorKind) -> pb::GuestControlError {
    let mut error = pb::GuestControlError::new();
    error.kind = EnumOrUnknown::new(kind);
    error.remediation = EnumOrUnknown::new(pb::HealthRemediation::HEALTH_REMEDIATION_RETRY);
    error
}

fn error_kind_for_auth(error: GuestAuthError) -> pb::GuestControlErrorKind {
    match error {
        GuestAuthError::Unauthenticated
        | GuestAuthError::MacRejected
        | GuestAuthError::TokenUnavailable => {
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_AUTH_FAILED
        }
        GuestAuthError::ProtocolVersionMismatch => {
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR
        }
        _ => pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR,
    }
}

fn exec_disabled_error() -> pb::GuestControlError {
    guest_error(pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_GUEST_EXEC_DISABLED)
}

fn control_ack_disabled() -> pb::ControlAck {
    let mut ack = pb::ControlAck::new();
    ack.error = MessageField::some(exec_disabled_error());
    ack
}

#[derive(Clone)]
struct ConnectionCleanup {
    auth: SharedAuthCore,
    exec: SharedExec,
    context: AuthConnectionContext,
    closed: Arc<AtomicBool>,
}

impl ConnectionCleanup {
    fn new(auth: SharedAuthCore, exec: SharedExec, context: AuthConnectionContext) -> Self {
        Self {
            auth,
            exec,
            context,
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    fn close(&self) {
        if !self.closed.swap(true, Ordering::SeqCst) {
            if let Ok(mut auth) = self.auth.lock() {
                auth.close_connection(&self.context);
            }
            // Terminate and forget every exec this connection owned.
            self.exec
                .close_connection(&self.context.connection_instance.to_vec());
        }
    }
}

struct CleanupStream<S> {
    inner: S,
    cleanup: ConnectionCleanup,
    done: Option<tokio::sync::oneshot::Sender<()>>,
}

impl<S> CleanupStream<S> {
    fn new(inner: S, cleanup: ConnectionCleanup, done: tokio::sync::oneshot::Sender<()>) -> Self {
        Self {
            inner,
            cleanup,
            done: Some(done),
        }
    }
}

impl<S> Drop for CleanupStream<S> {
    fn drop(&mut self) {
        self.cleanup.close();
        if let Some(done) = self.done.take() {
            let _ = done.send(());
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for CleanupStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<IoResult<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for CleanupStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<IoResult<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<IoResult<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<IoResult<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

pub struct OsNonceRng;

impl OsNonceRng {
    fn fill_bytes(&mut self, out: &mut [u8]) -> Result<(), OsNonceError> {
        let mut file = File::open("/dev/urandom").map_err(|_| OsNonceError)?;
        file.read_exact(out).map_err(|_| OsNonceError)
    }
}

impl NonceRng for OsNonceRng {
    fn fill_nonce(&mut self, out: &mut [u8; AUTH_NONCE_LEN]) -> Result<(), GuestAuthError> {
        self.fill_bytes(out)
            .map_err(|_| GuestAuthError::TokenUnavailable)
    }
}

pub struct ProcBootIdSource;

impl BootIdSource for ProcBootIdSource {
    fn guest_boot_id(&self) -> Result<String, GuestAuthError> {
        let boot_id = fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .map_err(|_| GuestAuthError::CapabilitiesUnavailable)?;
        let boot_id = boot_id.trim().to_owned();
        if boot_id.is_empty() || boot_id.len() > 128 {
            return Err(GuestAuthError::CapabilitiesUnavailable);
        }
        Ok(boot_id)
    }
}

pub struct SystemClock;

impl crate::auth::Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0)
    }
}

pub struct RuntimeCapabilitiesProvider {
    snapshot: CapabilitiesSnapshot,
}

impl RuntimeCapabilitiesProvider {
    pub fn new(config: CapabilitiesConfig) -> Self {
        let limits = effective_limits();

        let mut capabilities = pb::CapabilitiesResponse::new();
        capabilities.protocol_version = GUEST_CONTROL_PROTOCOL_VERSION;
        capabilities.capabilities.push(EnumOrUnknown::new(
            pb::GuestCapability::GUEST_CAPABILITY_HEALTH,
        ));
        capabilities.capabilities.push(EnumOrUnknown::new(
            pb::GuestCapability::GUEST_CAPABILITY_CAPABILITIES,
        ));
        // Exec capabilities are advertised only when both configured AND
        // usable, so a configured-but-broken feature's first call returns a
        // typed error instead of advertising a feature whose call would fail.
        if config.exec_attached {
            capabilities.capabilities.push(EnumOrUnknown::new(
                pb::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED,
            ));
        }
        if config.exec_detached {
            capabilities.capabilities.push(EnumOrUnknown::new(
                pb::GuestCapability::GUEST_CAPABILITY_EXEC_DETACHED,
            ));
        }
        if config.exec_logs {
            capabilities.capabilities.push(EnumOrUnknown::new(
                pb::GuestCapability::GUEST_CAPABILITY_EXEC_LOGS,
            ));
        }
        capabilities.limits = MessageField::some(limits);

        let mut health = pb::HealthResponse::new();
        health.origin = EnumOrUnknown::new(pb::HealthOrigin::HEALTH_ORIGIN_GUEST_REPORTED);
        health.state = EnumOrUnknown::new(pb::HealthState::HEALTH_STATE_HEALTHY);
        health.reason = EnumOrUnknown::new(pb::HealthReason::HEALTH_REASON_NONE);
        health.remediation = EnumOrUnknown::new(pb::HealthRemediation::HEALTH_REMEDIATION_NONE);
        health.protocol_version = GUEST_CONTROL_PROTOCOL_VERSION;
        health.capabilities = capabilities.capabilities.clone();

        let capabilities_hash = sha256_hex(&capabilities.write_to_bytes().unwrap_or_default());
        Self {
            snapshot: CapabilitiesSnapshot {
                capabilities_hash,
                health,
                capabilities,
            },
        }
    }
}

impl Default for RuntimeCapabilitiesProvider {
    fn default() -> Self {
        Self::new(CapabilitiesConfig::default())
    }
}

impl CapabilitiesProvider for RuntimeCapabilitiesProvider {
    fn snapshot(&self) -> Result<CapabilitiesSnapshot, GuestAuthError> {
        Ok(self.snapshot.clone())
    }
}

fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[derive(Debug, Clone, Copy)]
pub struct OsNonceError;

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs, os::unix::fs::PermissionsExt};

    use crate::auth::{encode_transcript, ProofRole};
    use crate::TokenSource;

    use super::*;

    const TEST_TOKEN: &[u8] = b"service-test-token";
    const HOST_NONCE: [u8; AUTH_NONCE_LEN] = [0x44; AUTH_NONCE_LEN];

    fn test_context(instance: u8) -> AuthConnectionContext {
        AuthConnectionContext {
            vm_id: "corp-vm".to_owned(),
            protocol_version: GUEST_CONTROL_PROTOCOL_VERSION,
            guest_control_port: GUEST_CONTROL_AUTH_PORT,
            peer_cid: 2,
            direction: AuthDirection::HostToGuest,
            purpose: AuthPurpose::GuestControlAuthV1,
            connection_instance: [instance; CONNECTION_INSTANCE_LEN],
        }
    }

    fn test_auth() -> SharedAuthCore {
        Arc::new(Mutex::new(
            build_runtime_auth_core(TEST_TOKEN.to_vec(), CapabilitiesConfig::default()).unwrap(),
        ))
    }

    fn test_exec() -> SharedExec {
        Arc::new(ExecRuntime::new(
            LinuxProcessSpawner,
            OsExecIds,
            ExecPolicy::disabled(),
        ))
    }

    fn test_exec_root_enabled() -> SharedExec {
        Arc::new(ExecRuntime::new(
            LinuxProcessSpawner,
            OsExecIds,
            ExecPolicy {
                enabled: true,
                allow_root: true,
            },
        ))
    }

    fn test_service(instance: u8) -> GuestControlService {
        GuestControlService::new(test_auth(), test_exec(), None, test_context(instance))
    }

    fn ttrpc_context() -> ttrpc::r#async::TtrpcContext {
        ttrpc::r#async::TtrpcContext {
            mh: ttrpc::proto::MessageHeader::new_request(1, 0),
            metadata: HashMap::new(),
            timeout_nano: 0,
        }
    }

    fn metadata() -> MessageField<pb::RequestMetadata> {
        let mut metadata = pb::RequestMetadata::new();
        metadata.vm_id = "corp-vm".to_owned();
        metadata.request_id = "req-1".to_owned();
        metadata.protocol_version = GUEST_CONTROL_PROTOCOL_VERSION;
        MessageField::some(metadata)
    }

    fn health_request() -> pb::HealthRequest {
        let mut request = pb::HealthRequest::new();
        request.metadata = metadata();
        request
    }

    fn capabilities_request() -> pb::CapabilitiesRequest {
        let mut request = pb::CapabilitiesRequest::new();
        request.metadata = metadata();
        request
    }

    async fn authenticate(service: &GuestControlService) {
        let ctx = ttrpc_context();
        let mut hello = pb::HelloRequest::new();
        hello.metadata = metadata();
        hello.host_nonce = HOST_NONCE.to_vec();
        hello.transcript_version = crate::auth::AUTH_TRANSCRIPT_VERSION;
        let hello_response = service.hello(&ctx, hello).await.unwrap();
        let guest_nonce: [u8; AUTH_NONCE_LEN] = hello_response
            .guest_nonce
            .as_slice()
            .try_into()
            .expect("fixed guest nonce");
        let transcript = encode_transcript(
            ProofRole::Host,
            &service.context,
            &HOST_NONCE,
            &guest_nonce,
            &hello_response.guest_boot_id,
            None,
        );
        let host_tag = SharedSecretToken::new(TEST_TOKEN.to_vec())
            .unwrap()
            .sign_tag(&transcript)
            .unwrap();

        let mut auth = pb::AuthenticateRequest::new();
        auth.metadata = metadata();
        auth.host_nonce = HOST_NONCE.to_vec();
        auth.guest_nonce = hello_response.guest_nonce;
        auth.guest_boot_id = hello_response.guest_boot_id;
        auth.transcript_version = crate::auth::AUTH_TRANSCRIPT_VERSION;
        auth.host_auth_tag = host_tag.to_vec();
        let response = service.authenticate(&ctx, auth).await.unwrap();
        assert!(response.error.is_none());
        assert!(response.health.is_some());
        assert!(response.capabilities.is_some());
    }

    fn assert_unauthenticated<T: std::fmt::Debug>(result: ttrpc::Result<T>) {
        match result {
            Err(ttrpc::Error::RpcStatus(status)) => {
                assert_eq!(
                    status.code.enum_value().unwrap(),
                    ttrpc::Code::UNAUTHENTICATED
                );
                assert!(!status.message.contains("token"));
            }
            other => panic!("expected unauthenticated status, got {other:?}"),
        }
    }

    fn assert_disabled(error: &pb::GuestControlError) {
        assert_eq!(
            error.kind.enum_value().unwrap(),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_GUEST_EXEC_DISABLED
        );
    }

    #[tokio::test]
    async fn health_and_capabilities_are_same_connection_authenticated() {
        let ctx = ttrpc_context();
        let service = test_service(1);
        assert_unauthenticated(service.health(&ctx, health_request()).await);
        assert_unauthenticated(service.capabilities(&ctx, capabilities_request()).await);

        authenticate(&service).await;
        assert!(service.health(&ctx, health_request()).await.is_ok());
        assert!(service
            .capabilities(&ctx, capabilities_request())
            .await
            .is_ok());

        let other = GuestControlService::new(
            Arc::clone(&service.auth),
            Arc::clone(&service.exec),
            None,
            test_context(2),
        );
        assert_unauthenticated(other.health(&ctx, health_request()).await);
    }

    #[tokio::test]
    async fn health_and_capabilities_validate_request_metadata() {
        let ctx = ttrpc_context();
        let service = test_service(7);
        authenticate(&service).await;

        let mut wrong = health_request();
        wrong.metadata.as_mut().unwrap().vm_id = "other-vm".to_owned();
        match service.health(&ctx, wrong).await {
            Err(ttrpc::Error::RpcStatus(status)) => {
                assert_eq!(
                    status.code.enum_value().unwrap(),
                    ttrpc::Code::INVALID_ARGUMENT
                );
                assert!(!status.message.contains("other-vm"));
            }
            other => panic!("expected invalid metadata status, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn close_connection_drops_authenticated_state() {
        let ctx = ttrpc_context();
        let service = test_service(3);
        authenticate(&service).await;
        assert!(service.health(&ctx, health_request()).await.is_ok());
        service
            .auth
            .lock()
            .unwrap()
            .close_connection(&service.context);
        assert_unauthenticated(service.health(&ctx, health_request()).await);
    }

    #[tokio::test]
    async fn exec_methods_require_auth_before_anything() {
        let ctx = ttrpc_context();
        let service = test_service(4);
        // Every exec/stdio/control RPC is rejected at auth before any
        // validation, id allocation, or spawn.
        assert_unauthenticated(
            service
                .exec_create(&ctx, pb::ExecCreateRequest::new())
                .await,
        );
        assert_unauthenticated(
            service
                .exec_inspect(&ctx, pb::ExecInspectRequest::new())
                .await,
        );
        assert_unauthenticated(service.exec_wait(&ctx, pb::ExecWaitRequest::new()).await);
        assert_unauthenticated(service.exec_logs(&ctx, pb::ExecLogsRequest::new()).await);
        assert_unauthenticated(
            service
                .write_stdin(&ctx, pb::WriteStdinRequest::new())
                .await,
        );
        assert_unauthenticated(
            service
                .read_output(&ctx, pb::ReadOutputRequest::new())
                .await,
        );
        assert_unauthenticated(
            service
                .close_stdin(&ctx, pb::CloseStdinRequest::new())
                .await,
        );
        assert_unauthenticated(
            service
                .tty_win_resize(&ctx, pb::TtyWinResizeRequest::new())
                .await,
        );
        assert_unauthenticated(
            service
                .exec_signal(&ctx, pb::ExecSignalRequest::new())
                .await,
        );
        assert_unauthenticated(
            service
                .exec_cancel(&ctx, pb::ExecCancelRequest::new())
                .await,
        );
    }

    #[tokio::test]
    async fn unsupported_rpcs_stay_disabled_after_auth() {
        let ctx = ttrpc_context();
        let service = test_service(5);
        authenticate(&service).await;
        // Out-of-scope RPCs remain typed-disabled even once authenticated.
        assert_disabled(
            service
                .exec_logs(&ctx, pb::ExecLogsRequest::new())
                .await
                .unwrap()
                .error
                .as_ref()
                .unwrap(),
        );
        assert_disabled(
            service
                .write_stdin(&ctx, pb::WriteStdinRequest::new())
                .await
                .unwrap()
                .error
                .as_ref()
                .unwrap(),
        );
        assert_disabled(
            service
                .close_stdin(&ctx, pb::CloseStdinRequest::new())
                .await
                .unwrap()
                .error
                .as_ref()
                .unwrap(),
        );
        assert_disabled(
            service
                .tty_win_resize(&ctx, pb::TtyWinResizeRequest::new())
                .await
                .unwrap()
                .error
                .as_ref()
                .unwrap(),
        );
        assert_disabled(
            service
                .exec_signal(&ctx, pb::ExecSignalRequest::new())
                .await
                .unwrap()
                .error
                .as_ref()
                .unwrap(),
        );
        assert_disabled(
            service
                .exec_cancel(&ctx, pb::ExecCancelRequest::new())
                .await
                .unwrap()
                .error
                .as_ref()
                .unwrap(),
        );
    }

    #[tokio::test]
    async fn exec_create_is_disabled_when_policy_off() {
        let ctx = ttrpc_context();
        let service = test_service(6);
        authenticate(&service).await;
        // The default test policy is fail-closed (exec disabled).
        let mut request = pb::ExecCreateRequest::new();
        request.metadata = metadata();
        request.argv = vec!["/bin/true".to_owned()];
        request.user = Some("root".to_owned());
        let mut output_policy = pb::OutputPolicy::new();
        output_policy.max_chunk_bytes = 64 * 1024;
        request.output_policy = MessageField::some(output_policy);
        assert_disabled(
            service
                .exec_create(&ctx, request)
                .await
                .unwrap()
                .error
                .as_ref()
                .unwrap(),
        );
    }

    #[tokio::test]
    async fn exec_create_rejects_terminal_size_even_with_zero_rows() {
        let ctx = ttrpc_context();
        // Root exec enabled so the request passes policy and reaches the
        // unsupported-mode check before any spawn.
        let service =
            GuestControlService::new(test_auth(), test_exec_root_enabled(), None, test_context(8));
        authenticate(&service).await;
        let mut request = pb::ExecCreateRequest::new();
        request.metadata = metadata();
        request.argv = vec!["/bin/true".to_owned()];
        request.user = Some("root".to_owned());
        let mut output_policy = pb::OutputPolicy::new();
        output_policy.max_chunk_bytes = 64 * 1024;
        request.output_policy = MessageField::some(output_policy);
        // A cols-only terminal size (rows = 0) must still be rejected: the
        // mere presence of initial_terminal_size is an unsupported mode.
        let mut size = pb::TerminalSize::new();
        size.rows = 0;
        size.cols = 80;
        request.initial_terminal_size = MessageField::some(size);
        let response = service.exec_create(&ctx, request).await.unwrap();
        let error = response.error.as_ref().expect("terminal size rejected");
        assert_eq!(
            error.kind.enum_value().unwrap(),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR
        );
        // No exec id is allocated for a rejected request.
        assert!(response.exec_id.is_none());
    }

    #[test]
    fn credential_loader_rejects_unsafe_sources_without_leaking_path() {
        let root = std::env::current_dir()
            .unwrap()
            .join(format!("nixling-guestd-cred-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let token = root.join(TOKEN_FILE_NAME);
        fs::write(&token, b"secret-token\n").unwrap();
        fs::set_permissions(&token, fs::Permissions::from_mode(0o600)).unwrap();

        assert_eq!(
            load_token_from_credentials_dir(&root).unwrap(),
            b"secret-token"
        );

        fs::set_permissions(&token, fs::Permissions::from_mode(0o666)).unwrap();
        let error = load_token_from_credentials_dir(&root).unwrap_err();
        assert_eq!(error, GuestdServiceError::UnsafeCredential);
        assert!(!error.public_message().contains("nixling-guestd-cred-test"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn exec_error_wire_mapping_is_exhaustive_and_typed() {
        use pb::GuestControlErrorKind as Pb;
        // Every ExecError variant maps to its expected typed wire kind; the
        // detached-introduced variants never collapse to ProtocolError.
        let cases: &[(ExecError, Pb)] = &[
            (
                ExecError::ExecDisabled,
                Pb::GUEST_CONTROL_ERROR_KIND_GUEST_EXEC_DISABLED,
            ),
            (
                ExecError::RootDenied,
                Pb::GUEST_CONTROL_ERROR_KIND_GUEST_EXEC_ROOT_DENIED,
            ),
            (
                ExecError::UserDenied,
                Pb::GUEST_CONTROL_ERROR_KIND_GUEST_EXEC_USER_DENIED,
            ),
            (
                ExecError::UnsupportedMode,
                Pb::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR,
            ),
            (
                ExecError::InvalidArgv,
                Pb::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR,
            ),
            (
                ExecError::CwdInvalid,
                Pb::GUEST_CONTROL_ERROR_KIND_CWD_INVALID,
            ),
            (
                ExecError::InvalidEnv,
                Pb::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR,
            ),
            (
                ExecError::MaxChunkExceeded,
                Pb::GUEST_CONTROL_ERROR_KIND_MAX_CHUNK_EXCEEDED,
            ),
            (
                ExecError::ExecCapacityExceeded,
                Pb::GUEST_CONTROL_ERROR_KIND_EXEC_CAPACITY_EXCEEDED,
            ),
            (
                ExecError::AttachCapacityExceeded,
                Pb::GUEST_CONTROL_ERROR_KIND_EXEC_ATTACH_CAPACITY_EXCEEDED,
            ),
            (
                ExecError::WaitCapacityExceeded,
                Pb::GUEST_CONTROL_ERROR_KIND_WAIT_CAPACITY_EXCEEDED,
            ),
            (
                ExecError::ReadWaitCapacityExceeded,
                Pb::GUEST_CONTROL_ERROR_KIND_READ_WAIT_CAPACITY_EXCEEDED,
            ),
            (
                ExecError::ExecNotFound,
                Pb::GUEST_CONTROL_ERROR_KIND_EXEC_NOT_FOUND,
            ),
            (
                ExecError::OffsetExpired,
                Pb::GUEST_CONTROL_ERROR_KIND_OFFSET_EXPIRED,
            ),
            (
                ExecError::OffsetInFuture,
                Pb::GUEST_CONTROL_ERROR_KIND_OFFSET_IN_FUTURE,
            ),
            (
                ExecError::SpawnFailed,
                Pb::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR,
            ),
            (
                ExecError::RetainedLogPathUnsafe,
                Pb::GUEST_CONTROL_ERROR_KIND_RETAINED_LOG_PATH_UNSAFE,
            ),
            (
                ExecError::RetainedLogQuotaExceeded,
                Pb::GUEST_CONTROL_ERROR_KIND_RETAINED_LOG_QUOTA_EXCEEDED,
            ),
            (
                ExecError::StaleSession,
                Pb::GUEST_CONTROL_ERROR_KIND_STALE_SESSION,
            ),
            (
                ExecError::ExecExpired,
                Pb::GUEST_CONTROL_ERROR_KIND_EXEC_EXPIRED,
            ),
            (
                ExecError::Internal,
                Pb::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR,
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(wire_error_kind(*error), *expected, "mapping for {error:?}");
        }
    }
}
