use std::{
    env, fs,
    fs::File,
    io::{Read, Result as IoResult},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use futures::stream;
use nixling_ipc::{
    guest_proto as pb,
    guest_wire::{GUEST_CONTROL_PROTOCOL_VERSION, READ_GUEST_FILE_MAX_BYTES},
};
use protobuf::{EnumOrUnknown, Message, MessageField};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    time::Duration,
};
use tokio_vsock::{VMADDR_CID_ANY, VsockAddr, VsockListener};

use crate::{
    auth::{
        AUTH_NONCE_LEN, AuthConnectionContext, AuthDirection, AuthPurpose, BootIdSource,
        CONNECTION_INSTANCE_LEN, CapabilitiesProvider, CapabilitiesSnapshot,
        GUEST_CONTROL_AUTH_PORT, GuestAuthCore, GuestAuthError, InMemoryChallengeStore, NonceRng,
        SharedSecretToken,
    },
    detached::{RunnerUnitPaths, SystemdRunUnitManager},
    detached_registry::{
        DetachedRegistry, RegistryConfig, RunSlotStore, SystemWallClock, TokioSleeper,
    },
    exec::{
        ExecCreateInput, ExecError, ExecPolicy, ExecRuntime, ExecSnapshot,
        ExecState as RtExecState, ExitOutcome, HARD_MAX_CHUNK_BYTES, MAX_ARG_BYTES, MAX_ARGV,
        MAX_CWD_BYTES, MAX_ENV_ENTRIES, MAX_ENV_KEY_BYTES, MAX_ENV_VALUE_BYTES, Stream as RtStream,
        TtyStdinSnapshot, ValidatedCommand,
    },
    exec_linux::LinuxProcessSpawner,
    exec_pty::linux::LinuxPtyProcessSpawner,
    generated::guest_control_ttrpc::{GuestControl, create_guest_control},
};

/// Absolute path to the guest login shell (NixOS system profile). Interactive
/// and non-interactive execs run the requested command through this shell with
/// `-l` so the profile (`/etc/set-environment`, `WAYLAND_DISPLAY`, …) is
/// sourced inside the PAM login session, reproducing an interactive login
/// environment (the surface `vm exec -it` drives) so graphical clients work.
const GUEST_LOGIN_SHELL_PATH: &str = "/run/current-system/sw/bin/bash";

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
/// Maximum concurrent in-flight `WriteStdin` handlers per connection. A
/// fifth concurrent handler is shed with `StdinBackpressure`.
const WRITE_STDIN_HANDLERS_PER_CONNECTION: u64 = 4;
/// In-flight decoded `WriteStdin` byte budget per connection. Exceeding it
/// (concurrently) is shed with `StdinByteBudgetExhausted`.
const DECODED_WRITE_STDIN_BYTES_PER_CONNECTION: u64 = 16 * 1024 * 1024;

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
    /// Default per-exec runtime ceiling, in seconds, for interactive (tty=true,
    /// non-detached) attached execs; 0 means unlimited. Non-TTY attached execs
    /// keep the fixed `MAX_EXEC_RUNTIME_MS` ceiling regardless of this value.
    pub interactive_max_runtime_sec: u64,
    /// Absolute host-declared path to the in-guest editable config working copy.
    /// `Some` => the guest advertises `GuestCapability::ReadGuestFile` and serves
    /// `ReadGuestFile { GuestConfig }` from this path via a fail-closed safe open.
    /// `None` => the capability is not advertised and `ReadGuestFile` is denied.
    pub guest_config_path: Option<PathBuf>,
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
            interactive_max_runtime_sec: 0,
            guest_config_path: None,
        })
    }

    /// Attach host-supplied detached runtime constants.
    pub fn with_detached(mut self, detached: DetachedRuntimeConfig) -> Self {
        self.detached = Some(detached);
        self
    }

    /// Set the default interactive (TTY) per-exec runtime ceiling in seconds.
    /// 0 (the default) means unlimited; non-TTY attached execs are unaffected.
    pub fn with_interactive_max_runtime_sec(mut self, seconds: u64) -> Self {
        self.interactive_max_runtime_sec = seconds;
        self
    }

    /// Attach the host-declared guest config working-copy path. Setting it makes
    /// the guest advertise `GuestCapability::ReadGuestFile` and serve the config
    /// over the typed `ReadGuestFile` RPC.
    pub fn with_guest_config_path(mut self, path: PathBuf) -> Self {
        self.guest_config_path = Some(path);
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
    /// Interactive TTY exec (WriteStdin/CloseStdin/TtyWinResize/ExecSignal over a
    /// PTY). Advertised only when exec is enabled AND a PTY helper is usable.
    pub exec_tty: bool,
    /// `ReadGuestFile`. Advertised when the host wired a guest config path into
    /// the guest unit. The RPC itself returns a typed file error (`FileNotFound`
    /// etc.) rather than gating the capability on the file currently existing.
    pub read_guest_file: bool,
}

/// Derive the advertised capability set from runtime presence.
///
/// Extracted as a pure function so the **`exec_attached` ⟺ `exec_logs`**
/// invariant is locked by a unit test: every attached exec session streams
/// stdout/stderr back via `ReadOutput`, so the host must never negotiate an
/// attached session it cannot stream. Both flags are therefore gated on the
/// SAME `exec_paths_present` input here, by construction — they can never
/// diverge. Detached exec is gated separately on a reconciled registry backed
/// by a resolved non-root workload user.
fn derive_capabilities_config(
    exec_paths_present: bool,
    exec_detached: bool,
    exec_tty: bool,
    read_guest_file: bool,
) -> CapabilitiesConfig {
    CapabilitiesConfig {
        exec_attached: exec_paths_present,
        exec_detached,
        exec_logs: exec_paths_present,
        exec_tty,
        read_guest_file,
    }
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

struct ServiceRuntime {
    auth: SharedAuthCore,
    exec: SharedExec,
    detached: SharedDetached,
    guest_config_path: Option<PathBuf>,
}

trait StartupProbe {
    fn classify_workload_user(&self, user: &str) -> crate::login_session::WorkloadUserUid;
    fn guest_boot_id(&self) -> Result<String, GuestAuthError>;
    fn path_is_file(&self, path: &Path) -> bool;
    fn detached_runtime_usable(&self, detached: &DetachedRuntimeConfig) -> bool;
    fn login_shell_path(&self) -> PathBuf {
        PathBuf::from(GUEST_LOGIN_SHELL_PATH)
    }
}

struct ProductionStartupProbe;

impl StartupProbe for ProductionStartupProbe {
    fn classify_workload_user(&self, user: &str) -> crate::login_session::WorkloadUserUid {
        crate::login_session::classify_workload_user(user)
    }

    fn guest_boot_id(&self) -> Result<String, GuestAuthError> {
        ProcBootIdSource.guest_boot_id()
    }

    fn path_is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn detached_runtime_usable(&self, detached: &DetachedRuntimeConfig) -> bool {
        detached_runtime_usable(detached)
    }
}

async fn prepare_service_runtime_with_probe<P: StartupProbe>(
    mut config: GuestdServeConfig,
    probe: &P,
) -> Result<ServiceRuntime, GuestdServiceError> {
    // Enforce the never-root contract by EFFECTIVE UID before wiring any
    // spawner. Arg parsing rejects only the literal name "root"; a non-root
    // name aliased to UID 0 would otherwise reach `systemd-run --uid=<name>`
    // and run the guest command as root. Resolve the workload user's UID from
    // the guest passwd DB and refuse UID 0 (any alias) or an unresolvable user
    // (fail closed). Refusal clears the workload user, disabling every exec
    // path (non-TTY pipe and interactive PTY) — never root.
    let mut exec_uid: Option<u32> = None;
    if let Some(user) = config.exec_policy.exec_user.clone() {
        match probe.classify_workload_user(&user) {
            crate::login_session::WorkloadUserUid::NonRoot(uid) => {
                exec_uid = Some(uid);
            }
            crate::login_session::WorkloadUserUid::Root => {
                eprintln!(
                    "nixling-guestd: refusing guest exec: workload user '{user}' resolves to \
                     UID 0; guest exec never runs as root"
                );
                config.exec_policy.exec_user = None;
            }
            crate::login_session::WorkloadUserUid::Unresolved => {
                eprintln!(
                    "nixling-guestd: refusing guest exec: workload user '{user}' is not \
                     resolvable in /etc/passwd; cannot prove non-root, failing closed"
                );
                config.exec_policy.exec_user = None;
            }
        }
    }

    // The host-fixed workload user every exec runs as (never root). When set,
    // exec is usable; the wire `user` field is never consulted for authz.
    let exec_user = config.exec_policy.exec_user.clone();
    let exec_enabled_user = config.exec_policy.enabled && exec_user.is_some();

    // Exec runtime paths (the `systemd-run` binary + the `nixling-exec-runner`
    // PTY helper) are wired by the host whenever exec is enabled. Both reachable
    // exec paths — the interactive PTY session and the non-interactive pipe —
    // run the requested command as the host-fixed workload user (never root)
    // inside a real PAM login session via `systemd-run --property=PAMName=login
    // --uid=<user>`. Both require these paths present and valid.
    let exec_paths: Option<&DetachedRuntimeConfig> = config.detached.as_ref().filter(|cfg| {
        exec_enabled_user
            && probe.path_is_file(&cfg.systemd_run_path)
            && probe.path_is_file(&cfg.exec_runner_path)
    });
    let login_shell = probe.login_shell_path();

    let detached: SharedDetached = match (config.detached.as_ref(), exec_user.clone(), exec_uid) {
        (Some(detached_cfg), Some(user), Some(uid))
            if detached_registry_allowed(
                exec_enabled_user,
                Some(uid),
                probe.detached_runtime_usable(detached_cfg),
                probe.path_is_file(&login_shell),
            ) =>
        {
            let guest_boot_id = probe.guest_boot_id().map_err(|_| GuestdServiceError::Io)?;
            let registry =
                build_detached_registry(detached_cfg, guest_boot_id, user, uid, &login_shell);
            registry.reconcile_on_startup().await;
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

    // Non-interactive (attached, non-TTY) spawner: runs as the workload user via
    // `systemd-run --pipe`, or disabled (fail closed, never root) when exec
    // paths are unavailable.
    let nontty_spawner = match (exec_paths, exec_user.clone()) {
        (Some(paths), Some(user)) => {
            crate::exec_linux::LinuxProcessSpawner::new(crate::exec_linux::WorkloadUserSpawn {
                systemd_run_path: paths.systemd_run_path.clone(),
                login_shell_path: login_shell.clone(),
                exec_user: user,
            })
        }
        _ => crate::exec_linux::LinuxProcessSpawner::disabled(),
    };

    let mut exec_runtime = ExecRuntime::new(nontty_spawner, OsExecIds, config.exec_policy);
    if let (Some(paths), Some(user)) = (exec_paths, exec_user.clone()) {
        // 0 means unlimited (the interactive default); a non-zero value caps the
        // per-session runtime. Non-TTY attached execs keep MAX_EXEC_RUNTIME_MS.
        let ceiling = (config.interactive_max_runtime_sec != 0)
            .then(|| Duration::from_secs(config.interactive_max_runtime_sec));
        exec_runtime = exec_runtime
            .with_pty_spawner(Arc::new(LinuxPtyProcessSpawner::new(
                paths.exec_runner_path.clone(),
                paths.systemd_run_path.clone(),
                login_shell.clone(),
                user,
            )))
            .with_interactive_ceiling(ceiling);
    }
    let exec: SharedExec = Arc::new(exec_runtime);

    let capabilities = derive_capabilities_config(
        // Non-TTY attached exec (and its required ReadOutput streaming) is served
        // iff the workload-user runtime paths are present.
        exec_paths.is_some(),
        detached.is_some(),
        exec.tty_usable(),
        config.guest_config_path.is_some(),
    );

    let auth = Arc::new(Mutex::new(build_runtime_auth_core(
        config.token,
        capabilities,
    )?));
    let guest_config_path = config.guest_config_path.clone();
    Ok(ServiceRuntime {
        auth,
        exec,
        detached,
        guest_config_path,
    })
}

pub async fn serve_vsock(config: GuestdServeConfig) -> Result<(), GuestdServiceError> {
    let vm_id = config.vm_id.clone();
    let ServiceRuntime {
        auth,
        exec,
        detached,
        guest_config_path,
    } = prepare_service_runtime_with_probe(config, &ProductionStartupProbe).await?;
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
        let vm_id = vm_id.clone();
        let guest_config_path = guest_config_path.clone();
        tokio::spawn(async move {
            if let Ok(context) = connection_context(vm_id, peer_addr.cid()) {
                let _ =
                    run_single_connection(stream, auth, exec, detached, context, guest_config_path)
                        .await;
            }
        });
    }
}

/// Build the production detached registry: `systemd-run`/`systemctl` unit
/// manager, `/run/nixling-exec` slot store, system wall clock, tokio sleeper,
/// and `/dev/urandom` exec ids.
///
fn build_detached_registry(
    detached: &DetachedRuntimeConfig,
    boot_id: String,
    exec_user: String,
    exec_uid: u32,
    login_shell_path: &Path,
) -> Arc<DetachedRegistry> {
    let units = Arc::new(SystemdRunUnitManager::new(
        detached.systemd_run_path.clone(),
    ));
    let store = Arc::new(RunSlotStore::new());
    let clock = Arc::new(SystemWallClock);
    let sleeper = Arc::new(TokioSleeper);
    let ids = Arc::new(OsExecIds);
    let registry_config = RegistryConfig {
        paths: RunnerUnitPaths::new(detached.exec_runner_path.clone()),
        boot_id,
        max_runtime_sec: detached.max_runtime_sec,
        exec_user,
        exec_uid,
        systemd_run_path: detached.systemd_run_path.to_string_lossy().into_owned(),
        login_shell_path: login_shell_path.to_string_lossy().into_owned(),
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
///
fn detached_runtime_usable(detached: &DetachedRuntimeConfig) -> bool {
    if !detached.systemd_run_path.is_file() || !detached.exec_runner_path.is_file() {
        return false;
    }
    match fs::symlink_metadata(nixling_exec_runner::paths::RUN_DIR) {
        Ok(meta) => meta.is_dir() && owner_is_safe(meta.uid()),
        Err(_) => false,
    }
}

fn detached_registry_allowed(
    exec_enabled_user: bool,
    exec_uid: Option<u32>,
    detached_runtime_usable: bool,
    login_shell_usable: bool,
) -> bool {
    exec_enabled_user
        && matches!(exec_uid, Some(uid) if uid != 0)
        && detached_runtime_usable
        && login_shell_usable
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
    guest_config_path: Option<PathBuf>,
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
    let service = Arc::new(
        GuestControlService::new(auth, exec, detached, context)
            .with_guest_config_path(guest_config_path),
    );
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
    // Per-connection interactive-stdin backpressure budgets (shared across the
    // Arc-cloned service for this connection): the count of in-flight
    // WriteStdin handlers and the in-flight decoded stdin byte budget.
    write_stdin_handlers: Arc<AtomicU64>,
    write_stdin_bytes: Arc<AtomicU64>,
    // Host-declared absolute path to the in-guest editable config working copy,
    // served by `ReadGuestFile { GuestConfig }`. `None` => no path was wired, the
    // `ReadGuestFile` capability is not advertised, and the RPC returns
    // `ReadDenied`.
    guest_config_path: Option<PathBuf>,
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
            write_stdin_handlers: Arc::new(AtomicU64::new(0)),
            write_stdin_bytes: Arc::new(AtomicU64::new(0)),
            guest_config_path: None,
        }
    }

    /// Attach the host-declared guest config working-copy path used to serve
    /// `ReadGuestFile { GuestConfig }`.
    pub fn with_guest_config_path(mut self, path: Option<PathBuf>) -> Self {
        self.guest_config_path = path;
        self
    }

    fn lock_auth(&self) -> Result<MutexGuard<'_, RuntimeAuthCore>, ttrpc::Error> {
        self.auth
            .lock()
            .map_err(|_| rpc_status(ttrpc::Code::INTERNAL, "guest-control-internal-error"))
    }

    /// Resolve a `ReadGuestFile` enum key to the host-declared path and read it
    /// with the fail-closed safe-open algorithm. Returns the file bytes or a
    /// typed `GuestControlErrorKind`. Only `GuestConfig` is supported; any other
    /// (or `Unspecified`/unknown) key maps to `PathUnsafe` because it names no
    /// safe target.
    fn read_guest_file_inner(
        &self,
        file_id: pb::GuestFileId,
    ) -> Result<Vec<u8>, pb::GuestControlErrorKind> {
        use pb::GuestControlErrorKind as K;
        match file_id {
            pb::GuestFileId::GUEST_FILE_ID_GUEST_CONFIG => {
                let path = self
                    .guest_config_path
                    .as_deref()
                    // No path wired => the capability was never advertised; a
                    // caller reaching here anyway is denied (not "not found").
                    .ok_or(K::GUEST_CONTROL_ERROR_KIND_READ_DENIED)?;
                read_guest_file_safely(path)
            }
            _ => Err(K::GUEST_CONTROL_ERROR_KIND_PATH_UNSAFE),
        }
    }

    /// Acquire a per-connection WriteStdin budget slot: at most
    /// `WRITE_STDIN_HANDLERS_PER_CONNECTION` concurrent handlers and at most
    /// `DECODED_WRITE_STDIN_BYTES_PER_CONNECTION` decoded bytes in flight.
    /// Returns a guard that releases both on drop. Sheds with
    /// `StdinBackpressure` / `StdinByteBudgetExhausted` rather than blocking.
    fn acquire_write_stdin_slot(&self, len: u64) -> Result<WriteStdinBudgetGuard, ExecError> {
        let prev_handlers = self.write_stdin_handlers.fetch_add(1, Ordering::SeqCst);
        if prev_handlers >= WRITE_STDIN_HANDLERS_PER_CONNECTION {
            self.write_stdin_handlers.fetch_sub(1, Ordering::SeqCst);
            return Err(ExecError::StdinBackpressure);
        }
        let prev_bytes = self.write_stdin_bytes.fetch_add(len, Ordering::SeqCst);
        if prev_bytes.saturating_add(len) > DECODED_WRITE_STDIN_BYTES_PER_CONNECTION {
            self.write_stdin_bytes.fetch_sub(len, Ordering::SeqCst);
            self.write_stdin_handlers.fetch_sub(1, Ordering::SeqCst);
            return Err(ExecError::StdinByteBudgetExhausted);
        }
        Ok(WriteStdinBudgetGuard {
            handlers: Arc::clone(&self.write_stdin_handlers),
            bytes: Arc::clone(&self.write_stdin_bytes),
            len,
        })
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

        let command = match validate_detached_command(&input, self.exec.policy()) {
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

/// RAII release of a per-connection WriteStdin budget slot (handler count +
/// in-flight decoded bytes), held for the lifetime of a single WriteStdin RPC.
struct WriteStdinBudgetGuard {
    handlers: Arc<AtomicU64>,
    bytes: Arc<AtomicU64>,
    len: u64,
}

impl Drop for WriteStdinBudgetGuard {
    fn drop(&mut self) {
        self.bytes.fetch_sub(self.len, Ordering::SeqCst);
        self.handlers.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Build a rejected `WriteStdinResponse` for a typed runtime/budget error. A
/// closed stdin reports `STDIN_STATE_CLOSED`; every other rejection leaves the
/// stream `OPEN` so the client may retry at the same offset.
fn write_stdin_error(error: ExecError) -> pb::WriteStdinResponse {
    let stdin_state = match error {
        ExecError::StdinClosed => pb::StdinState::STDIN_STATE_CLOSED,
        _ => pb::StdinState::STDIN_STATE_OPEN,
    };
    let mut response = pb::WriteStdinResponse::new();
    response.stdin_state = EnumOrUnknown::new(stdin_state);
    response.disposition = EnumOrUnknown::new(pb::WriteDisposition::WRITE_DISPOSITION_REJECTED);
    response.error = MessageField::some(guest_error_kind(error));
    response
}

fn inspect_response(snapshot: &ExecSnapshot) -> pb::ExecInspectResponse {
    let mut response = pb::ExecInspectResponse::new();
    response.state = EnumOrUnknown::new(wire_exec_state(snapshot.state));
    response.visible_terminal_status = wire_terminal_status(snapshot);
    // TTY-aware stdin disposition: a live interactive exec accepting
    // WriteStdin reports OPEN, a tearing-down one CLOSING, a VEOF-closed one
    // CLOSED. Non-interactive execs have no writable stdin and keep the
    // historical CLOSED report.
    response.stdin_state = EnumOrUnknown::new(match snapshot.stdin {
        TtyStdinSnapshot::NotInteractive => pb::StdinState::STDIN_STATE_CLOSED,
        TtyStdinSnapshot::Open => pb::StdinState::STDIN_STATE_OPEN,
        TtyStdinSnapshot::Closing => pb::StdinState::STDIN_STATE_CLOSING,
        TtyStdinSnapshot::Closed => pb::StdinState::STDIN_STATE_CLOSED,
    });
    response.stdout_start_offset = snapshot.stdout_start_offset;
    response.stdout_end_offset = snapshot.stdout_end_offset;
    response.stderr_start_offset = snapshot.stderr_start_offset;
    response.stderr_end_offset = snapshot.stderr_end_offset;
    response.stdout_dropped_bytes = snapshot.stdout_dropped_bytes;
    response.stderr_dropped_bytes = snapshot.stderr_dropped_bytes;
    response.stdout_truncated_for_retention = snapshot.stdout_truncated;
    response.stderr_truncated_for_retention = snapshot.stderr_truncated;
    response.state_generation = snapshot.state_generation;
    // Highest admitted resize/signal control sequence (0 for non-TTY execs).
    response.last_control_seq = snapshot.last_control_seq;
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
        ExecError::InvalidProgram => Pb::GUEST_CONTROL_ERROR_KIND_INVALID_PROGRAM,
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
        // Interactive TTY exec: reuse existing wire kinds (no regen).
        ExecError::InvalidTerminalSize => Pb::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR,
        ExecError::TtyStderrUnavailable => Pb::GUEST_CONTROL_ERROR_KIND_TTY_STDERR_UNAVAILABLE,
        ExecError::TtyRequired => Pb::GUEST_CONTROL_ERROR_KIND_TTY_REQUIRED,
        ExecError::StdinClosed => Pb::GUEST_CONTROL_ERROR_KIND_STDIN_CLOSED,
        ExecError::StdinOffsetMismatch => Pb::GUEST_CONTROL_ERROR_KIND_STDIN_OFFSET_MISMATCH,
        ExecError::StdinByteBudgetExhausted => {
            Pb::GUEST_CONTROL_ERROR_KIND_STDIN_BYTE_BUDGET_EXHAUSTED
        }
        ExecError::StdinBackpressure => Pb::GUEST_CONTROL_ERROR_KIND_STDIN_BACKPRESSURE,
        ExecError::ControlSeqMismatch => Pb::GUEST_CONTROL_ERROR_KIND_CONTROL_SEQ_MISMATCH,
        ExecError::InvalidSignal => Pb::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR,
        ExecError::ExecClosing => Pb::GUEST_CONTROL_ERROR_KIND_EXEC_ALREADY_EXITED,
        ExecError::Internal => Pb::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR,
    }
}

/// Advertised effective limits. Shared by the capabilities snapshot and
/// exec responses so both report identical bounds.
pub fn effective_limits() -> pb::GuestEffectiveLimits {
    let mut limits = pb::GuestEffectiveLimits::new();
    limits.max_chunk_bytes = 64 * 1024;
    limits.max_recv_message_bytes = 4 * 1024 * 1024;
    limits.decoded_write_stdin_bytes_per_connection = DECODED_WRITE_STDIN_BYTES_PER_CONNECTION;
    limits.write_stdin_handlers_per_connection = WRITE_STDIN_HANDLERS_PER_CONNECTION as u32;
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

    async fn read_guest_file(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::ReadGuestFileRequest,
    ) -> ttrpc::Result<pb::ReadGuestFileResponse> {
        // Auth is enforced BEFORE any path resolution, stat, or read (D20): an
        // unauthenticated caller never learns whether a config file exists.
        self.require_authenticated()?;
        self.validate_metadata(request.metadata.as_ref())?;

        let file_id = request.file_id.enum_value_or_default();
        let outcome = self.read_guest_file_inner(file_id);

        let mut response = pb::ReadGuestFileResponse::new();
        // Echo the requested key so the host can correlate; on error this is the
        // requested id, on success the resolved one (identical today).
        response.file_id = EnumOrUnknown::new(file_id);
        match outcome {
            Ok(bytes) => {
                // The guest reports size+sha256 for convenience, but the host
                // recomputes both from the received bytes and never trusts these
                // as integrity evidence (D4).
                response.sha256 = sha256_hex(&bytes);
                response.size_bytes = bytes.len() as u64;
                response.content = bytes;
            }
            Err(kind) => {
                // Fail closed: no content, no size, no hash leak on error.
                response.error = MessageField::some(guest_error(kind));
            }
        }
        Ok(response)
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
            // `tty && detached` is an unsupported mode regardless of whether
            // detached exec is configured on this host: reject it deterministically
            // here so the documented mode error does not depend on the detached
            // registry's availability (a missing registry would otherwise mask it
            // as GuestExecDisabled).
            if input.tty {
                let mut response = pb::ExecCreateResponse::new();
                response.state = EnumOrUnknown::new(pb::ExecState::EXEC_STATE_PROTOCOL_ERROR);
                response.error = MessageField::some(guest_error_kind(ExecError::UnsupportedMode));
                return Ok(response);
            }
            return self.exec_create_detached(input, &guest_boot_id).await;
        }

        // Interactive TTY exec: tty=true && !detached routes to the PTY-backed,
        // connection-owned attached path. `tty && detached` is an unsupported
        // mode (no new wire kind) — it is rejected by the detached validator
        // above; this branch only handles the supported interactive create.
        if input.tty {
            let initial_size = request
                .initial_terminal_size
                .as_ref()
                .map(|size| (size.rows, size.cols));
            return match self
                .exec
                .create_tty(self.connection_key(), guest_boot_id, input, initial_size)
                .await
            {
                Ok((exec_id, snapshot, control_seq)) => {
                    let mut response = pb::ExecCreateResponse::new();
                    response.exec_id = Some(exec_id);
                    response.control_seq = control_seq;
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
            };
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
                ));
            }
        };
        match registry
            .read_logs(
                exec_id,
                guest_boot_id,
                stream,
                request.offset,
                request.max_len,
            )
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
        request: pb::WriteStdinRequest,
    ) -> ttrpc::Result<pb::WriteStdinResponse> {
        self.require_authenticated()?;
        let (exec_id, guest_boot_id) = self.validate_exec_metadata(request.metadata.as_ref())?;
        let len = request.data.len() as u64;
        // Per-connection backpressure budgets (handler cap + in-flight byte
        // budget). Acquired before the runtime write; the guard releases both on
        // every return.
        let _budget = match self.acquire_write_stdin_slot(len) {
            Ok(guard) => guard,
            Err(error) => return Ok(write_stdin_error(error)),
        };
        match self
            .exec
            .write_stdin(
                &self.connection_key(),
                exec_id,
                guest_boot_id,
                request.offset,
                &request.data,
                request.close_after,
            )
            .await
        {
            Ok(out) => {
                let mut response = pb::WriteStdinResponse::new();
                response.accepted_offset = request.offset;
                // Partial-write-aware: report the bytes that actually
                // landed, not the requested length. A client retries any
                // remainder from `next_offset`.
                response.accepted_len = out.accepted_len;
                response.next_offset = out.next_offset;
                response.stdin_state = EnumOrUnknown::new(if out.closed {
                    pb::StdinState::STDIN_STATE_CLOSED
                } else {
                    pb::StdinState::STDIN_STATE_OPEN
                });
                response.disposition =
                    EnumOrUnknown::new(pb::WriteDisposition::WRITE_DISPOSITION_ACCEPTED);
                Ok(response)
            }
            Err(error) => Ok(write_stdin_error(error)),
        }
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
                ));
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
        request: pb::CloseStdinRequest,
    ) -> ttrpc::Result<pb::CloseStdinResponse> {
        self.require_authenticated()?;
        let (exec_id, guest_boot_id) = self.validate_exec_metadata(request.metadata.as_ref())?;
        match self
            .exec
            .close_stdin(
                &self.connection_key(),
                exec_id,
                guest_boot_id,
                request.offset,
            )
            .await
        {
            Ok((final_offset, duplicate)) => {
                let mut response = pb::CloseStdinResponse::new();
                response.stdin_state = EnumOrUnknown::new(pb::StdinState::STDIN_STATE_CLOSED);
                response.final_offset = final_offset;
                response.disposition = EnumOrUnknown::new(if duplicate {
                    pb::WriteDisposition::WRITE_DISPOSITION_DUPLICATE
                } else {
                    pb::WriteDisposition::WRITE_DISPOSITION_ACCEPTED
                });
                Ok(response)
            }
            Err(error) => {
                let mut response = pb::CloseStdinResponse::new();
                response.disposition =
                    EnumOrUnknown::new(pb::WriteDisposition::WRITE_DISPOSITION_REJECTED);
                response.error = MessageField::some(guest_error_kind(error));
                Ok(response)
            }
        }
    }

    async fn tty_win_resize(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::TtyWinResizeRequest,
    ) -> ttrpc::Result<pb::ControlAck> {
        self.require_authenticated()?;
        let (exec_id, guest_boot_id) = self.validate_exec_metadata(request.metadata.as_ref())?;
        let mut ack = pb::ControlAck::new();
        ack.control_seq = request.control_seq;
        if let Err(error) = self.exec.tty_resize(
            &self.connection_key(),
            exec_id,
            guest_boot_id,
            request.control_seq,
            request.rows,
            request.cols,
        ) {
            ack.error = MessageField::some(guest_error_kind(error));
        }
        Ok(ack)
    }

    async fn exec_signal(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::ExecSignalRequest,
    ) -> ttrpc::Result<pb::ControlAck> {
        self.require_authenticated()?;
        let (exec_id, guest_boot_id) = self.validate_exec_metadata(request.metadata.as_ref())?;
        let mut ack = pb::ControlAck::new();
        ack.control_seq = request.control_seq;
        // Only the foreground process group is a valid signal target. Any other
        // target (PROCESS_TREE / UNSPECIFIED / unknown) is rejected before the
        // control sequence is consumed, so the client can retry with a valid
        // target at the same seq.
        let target = request
            .target
            .enum_value()
            .unwrap_or(pb::SignalTarget::SIGNAL_TARGET_UNSPECIFIED);
        if !matches!(
            target,
            pb::SignalTarget::SIGNAL_TARGET_FOREGROUND_PROCESS_GROUP
        ) {
            ack.error = MessageField::some(guest_error_kind(ExecError::InvalidSignal));
            return Ok(ack);
        }
        if let Err(error) = self.exec.tty_signal(
            &self.connection_key(),
            exec_id,
            guest_boot_id,
            request.control_seq,
            request.signal,
        ) {
            ack.error = MessageField::some(guest_error_kind(error));
        }
        Ok(ack)
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

/// Read the host-declared guest config working copy with a fail-closed safe
/// open (D2, HARD invariant): resolve the absolute path component-by-component
/// from `/` using `openat` + `O_NOFOLLOW` (no symlink traversal at ANY level,
/// reject `.`/`..`/prefix components), open the leaf `O_RDONLY|O_NOFOLLOW|
/// O_CLOEXEC`, `fstat` the OPENED fd, reject non-regular, enforce the size cap
/// BEFORE any allocation/read, then read ONLY from that fd. There is no TOCTOU
/// (size and identity come from the opened fd, not a pre-open `stat`) and no fd
/// leak (rustix owns every fd and closes it on drop). The read loop re-checks
/// the cap so a file that grows after `fstat` cannot exceed the bound.
fn read_guest_file_safely(path: &Path) -> Result<Vec<u8>, pb::GuestControlErrorKind> {
    use pb::GuestControlErrorKind as K;
    use rustix::fs::{FileType, Mode, OFlags, fstat, open, openat};
    use rustix::io::Errno;
    use std::path::Component;

    fn map_open_err(err: Errno) -> pb::GuestControlErrorKind {
        match err {
            Errno::NOENT => K::GUEST_CONTROL_ERROR_KIND_FILE_NOT_FOUND,
            // ELOOP (a symlink component under O_NOFOLLOW) or a non-directory
            // ancestor component is an unsafe path, not a missing/denied file.
            Errno::LOOP | Errno::NOTDIR => K::GUEST_CONTROL_ERROR_KIND_PATH_UNSAFE,
            Errno::ACCESS | Errno::PERM => K::GUEST_CONTROL_ERROR_KIND_READ_DENIED,
            _ => K::GUEST_CONTROL_ERROR_KIND_READ_DENIED,
        }
    }

    if !path.is_absolute() {
        return Err(K::GUEST_CONTROL_ERROR_KIND_PATH_UNSAFE);
    }

    // Collect the normal (named) path components. Reject `.`/`..`/prefix: the
    // path is host-fixed, so anything other than a plain rooted chain of names
    // is a misconfiguration or a traversal attempt.
    let mut names: Vec<&std::ffi::OsStr> = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => names.push(name),
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(K::GUEST_CONTROL_ERROR_KIND_PATH_UNSAFE);
            }
        }
    }
    let Some((leaf, dirs)) = names.split_last() else {
        // The path was `/` — not a file.
        return Err(K::GUEST_CONTROL_ERROR_KIND_PATH_UNSAFE);
    };

    let dir_flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let file_flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC;

    // Open `/` then descend each ancestor directory via `openat`+`O_NOFOLLOW`
    // so no symlink is traversed at any level.
    let mut dir = open("/", dir_flags, Mode::empty()).map_err(map_open_err)?;
    for name in dirs {
        dir = openat(&dir, *name, dir_flags, Mode::empty()).map_err(map_open_err)?;
    }
    let file = openat(&dir, *leaf, file_flags, Mode::empty()).map_err(map_open_err)?;

    // `fstat` the OPENED fd; reject anything that is not a regular file. A
    // symlink leaf would already have failed the `O_NOFOLLOW` open with ELOOP.
    let st = fstat(&file).map_err(|_| K::GUEST_CONTROL_ERROR_KIND_READ_DENIED)?;
    if FileType::from_raw_mode(st.st_mode) != FileType::RegularFile {
        return Err(K::GUEST_CONTROL_ERROR_KIND_PATH_UNSAFE);
    }

    // Enforce the cap BEFORE allocating/reading. A negative or absurd size
    // (should not occur for a regular file) saturates to the cap+ and fails
    // closed as too-large.
    let declared_size = u64::try_from(st.st_size).unwrap_or(u64::MAX);
    if declared_size > READ_GUEST_FILE_MAX_BYTES {
        return Err(K::GUEST_CONTROL_ERROR_KIND_FILE_TOO_LARGE);
    }

    // Read only from the opened fd, bounding total bytes at the cap. The cap is
    // re-checked per chunk so a concurrent growth past `fstat` cannot exceed it.
    let cap = READ_GUEST_FILE_MAX_BYTES as usize;
    let mut buf: Vec<u8> = Vec::with_capacity(declared_size as usize);
    let mut chunk = [0_u8; 65536];
    loop {
        let n = rustix::io::read(&file, &mut chunk)
            .map_err(|_| K::GUEST_CONTROL_ERROR_KIND_READ_DENIED)?;
        if n == 0 {
            break;
        }
        if buf.len() + n > cap {
            return Err(K::GUEST_CONTROL_ERROR_KIND_FILE_TOO_LARGE);
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Ok(buf)
}

fn guest_error(kind: pb::GuestControlErrorKind) -> pb::GuestControlError {
    use pb::GuestControlErrorKind as K;
    use pb::HealthRemediation as R;
    let mut error = pb::GuestControlError::new();
    error.kind = EnumOrUnknown::new(kind);
    // Per-kind operator remediation. A blind RETRY is wrong for the two
    // detached retained-log faults: a quota breach is shed by the periodic
    // reaper (advise REDUCE_LOAD + a concrete retry window), while an unsafe
    // retained-log path is an internal guestd storage fault (advise checking
    // the guestd service — a caller retry cannot fix it).
    let (remediation, retry_after_ms) = match kind {
        K::GUEST_CONTROL_ERROR_KIND_RETAINED_LOG_QUOTA_EXCEEDED => (
            R::HEALTH_REMEDIATION_REDUCE_LOAD,
            Some(DETACHED_REAPER_INTERVAL_MS),
        ),
        K::GUEST_CONTROL_ERROR_KIND_RETAINED_LOG_PATH_UNSAFE => {
            (R::HEALTH_REMEDIATION_CHECK_GUESTD_SERVICE, None)
        }
        // ReadGuestFile faults are NOT retryable (D12): a missing/unsafe config
        // working copy or a denied read is fixed by guest-side setup, not by a
        // caller retry; an oversize config carries no in-enum remediation so the
        // host surfaces the actionable "shrink below the cap" message instead.
        K::GUEST_CONTROL_ERROR_KIND_FILE_NOT_FOUND
        | K::GUEST_CONTROL_ERROR_KIND_PATH_UNSAFE
        | K::GUEST_CONTROL_ERROR_KIND_READ_DENIED => {
            (R::HEALTH_REMEDIATION_CHECK_GUESTD_SERVICE, None)
        }
        K::GUEST_CONTROL_ERROR_KIND_FILE_TOO_LARGE => (R::HEALTH_REMEDIATION_NONE, None),
        _ => (R::HEALTH_REMEDIATION_RETRY, None),
    };
    error.remediation = EnumOrUnknown::new(remediation);
    error.retry_after_ms = retry_after_ms;
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

fn validate_detached_command(
    input: &ExecCreateInput,
    policy: &ExecPolicy,
) -> Result<ValidatedCommand, ExecError> {
    if !policy.enabled {
        return Err(ExecError::ExecDisabled);
    }
    if input.tty || input.stdin_open || input.has_terminal_size {
        return Err(ExecError::UnsupportedMode);
    }
    if policy.exec_user.is_none() {
        return Err(ExecError::ExecDisabled);
    }
    if input.max_chunk_bytes == 0 || input.max_chunk_bytes > HARD_MAX_CHUNK_BYTES {
        return Err(ExecError::MaxChunkExceeded);
    }

    if input.argv.is_empty() || input.argv.len() > MAX_ARGV {
        return Err(ExecError::InvalidArgv);
    }
    for arg in &input.argv {
        // NUL is rejected for every exec. Detached argv additionally must not
        // contain a newline or carriage return: the workload-identity reconciler
        // recovers the running command from `systemctl show -p ExecStart`, whose
        // `argv[]` is a single line, so a `\n`/`\r` byte would split the property
        // and make the live workload unmatchable (silently reaped). Reject at
        // create — as an invalid argument — so a detached job is not started
        // only to be reaped on the first reconcile. (The create error reuses the
        // existing InvalidArgv kind; its operator message is generic.)
        if arg.len() > MAX_ARG_BYTES
            || arg
                .as_bytes()
                .iter()
                .any(|&b| b == 0 || b == b'\n' || b == b'\r')
        {
            return Err(ExecError::InvalidArgv);
        }
    }
    let program = &input.argv[0];
    if program.is_empty() || program.starts_with('-') {
        return Err(ExecError::InvalidProgram);
    }
    if input.argv.iter().skip(1).any(|arg| arg.is_empty()) {
        return Err(ExecError::InvalidArgv);
    }

    let cwd = match input.cwd.as_deref() {
        Some(cwd) => {
            if cwd.is_empty()
                || cwd.len() > MAX_CWD_BYTES
                || !cwd.starts_with('/')
                || cwd.as_bytes().contains(&0)
            {
                return Err(ExecError::CwdInvalid);
            }
            PathBuf::from(cwd)
        }
        None => PathBuf::from("/"),
    };

    if input.env.len() > MAX_ENV_ENTRIES {
        return Err(ExecError::InvalidEnv);
    }
    let mut seen = std::collections::BTreeSet::new();
    for (key, value) in &input.env {
        if !valid_detached_env_key(key)
            || value.len() > MAX_ENV_VALUE_BYTES
            || value.as_bytes().contains(&0)
        {
            return Err(ExecError::InvalidEnv);
        }
        if !seen.insert(key.clone()) {
            return Err(ExecError::InvalidEnv);
        }
    }

    Ok(ValidatedCommand {
        program: PathBuf::from(program),
        args: input.argv[1..].to_vec(),
        cwd,
        env: input.env.clone(),
    })
}

fn valid_detached_env_key(key: &str) -> bool {
    if key.is_empty() || key.len() > MAX_ENV_KEY_BYTES {
        return false;
    }
    let mut chars = key.chars();
    let first = chars.next().unwrap_or('=');
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
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
        // Interactive TTY exec advertises the stdin/resize/signal control
        // surface as a unit: it is only usable when exec is enabled AND a PTY
        // helper is present.
        if config.exec_tty {
            capabilities.capabilities.push(EnumOrUnknown::new(
                pb::GuestCapability::GUEST_CAPABILITY_EXEC_TTY,
            ));
            capabilities.capabilities.push(EnumOrUnknown::new(
                pb::GuestCapability::GUEST_CAPABILITY_TTY_RESIZE,
            ));
            capabilities.capabilities.push(EnumOrUnknown::new(
                pb::GuestCapability::GUEST_CAPABILITY_SIGNALS,
            ));
        }
        // ReadGuestFile is advertised when the host wired a guest config path.
        // `config sync` REQUIRES this capability before any read attempt, so an
        // old/partial guest that authenticates but never advertises it fails
        // closed host-side instead of being probed for a config file.
        if config.read_guest_file {
            capabilities.capabilities.push(EnumOrUnknown::new(
                pb::GuestCapability::GUEST_CAPABILITY_READ_GUEST_FILE,
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

    use crate::TokenSource;
    use crate::auth::{ProofRole, encode_transcript};

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

    fn detached_input(argv0: &str) -> ExecCreateInput {
        ExecCreateInput {
            argv: vec![argv0.to_owned(), "arg".to_owned()],
            user: Some("ignored-wire-user".to_owned()),
            cwd: Some("/work".to_owned()),
            env: vec![("PATH".to_owned(), "/bin".to_owned())],
            tty: false,
            stdin_open: false,
            detached: true,
            has_terminal_size: false,
            max_chunk_bytes: 64 * 1024,
        }
    }

    fn exec_policy() -> ExecPolicy {
        ExecPolicy {
            enabled: true,
            exec_user: Some("alice".to_owned()),
        }
    }

    struct TestStartupProbe {
        uid: crate::login_session::WorkloadUserUid,
    }

    impl StartupProbe for TestStartupProbe {
        fn classify_workload_user(&self, _user: &str) -> crate::login_session::WorkloadUserUid {
            self.uid
        }

        fn guest_boot_id(&self) -> Result<String, GuestAuthError> {
            Ok("boot-1".to_owned())
        }

        fn path_is_file(&self, _path: &Path) -> bool {
            true
        }

        fn detached_runtime_usable(&self, _detached: &DetachedRuntimeConfig) -> bool {
            true
        }

        fn login_shell_path(&self) -> PathBuf {
            PathBuf::from("/run/current-system/sw/bin/bash")
        }
    }

    fn startup_config(user: &str) -> GuestdServeConfig {
        GuestdServeConfig::with_exec_policy(
            "corp-vm",
            TEST_TOKEN.to_vec(),
            ExecPolicy {
                enabled: true,
                exec_user: Some(user.to_owned()),
            },
        )
        .unwrap()
        .with_detached(DetachedRuntimeConfig {
            systemd_run_path: PathBuf::from("/run/current-system/sw/bin/systemd-run"),
            exec_runner_path: PathBuf::from("/run/current-system/sw/bin/nixling-exec-runner"),
            max_runtime_sec: 0,
        })
    }

    #[test]
    fn detached_command_validation_allows_bare_absolute_and_relative_argv0() {
        for argv0 in ["id", "/bin/true", "./script", "../script"] {
            let command =
                validate_detached_command(&detached_input(argv0), &exec_policy()).unwrap();
            assert_eq!(command.program, PathBuf::from(argv0));
            assert_eq!(command.args, vec!["arg".to_owned()]);
            assert_eq!(command.cwd, PathBuf::from("/work"));
        }
    }

    #[test]
    fn detached_command_validation_rejects_leading_dash_argv0() {
        let err = validate_detached_command(&detached_input("-sh"), &exec_policy()).unwrap_err();
        assert_eq!(err, ExecError::InvalidProgram);
    }

    #[test]
    fn detached_command_validation_rejects_empty_argv0_as_invalid_program() {
        let err = validate_detached_command(&detached_input(""), &exec_policy()).unwrap_err();
        assert_eq!(err, ExecError::InvalidProgram);
    }

    #[test]
    fn detached_command_validation_rejects_newline_or_cr_in_argv() {
        // A detached argv byte that would split the single-line `systemctl show
        // -p ExecStart` property (newline / carriage return) is rejected at
        // create — otherwise the running workload becomes unmatchable by the
        // identity reconciler and is silently reaped. Reject up front instead.
        for bad in ["a\nb", "a\rb", "trailing\n"] {
            let mut input = detached_input("/bin/sh");
            input.argv = vec!["/bin/sh".to_owned(), "-c".to_owned(), bad.to_owned()];
            let err = validate_detached_command(&input, &exec_policy()).unwrap_err();
            assert_eq!(err, ExecError::InvalidArgv, "argv {bad:?} must be rejected");
        }
        // A semicolon argument (now handled by raw identity matching) is still
        // accepted — the newline guard must not over-restrict the common
        // `sh -c 'a ; b'` pattern.
        let mut ok = detached_input("/bin/sh");
        ok.argv = vec![
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            "echo a ; echo b".to_owned(),
        ];
        assert!(validate_detached_command(&ok, &exec_policy()).is_ok());
    }

    #[test]
    fn derive_capabilities_locks_attached_implies_output() {
        // The host requires `ReadOutput` (EXEC_LOGS) for every attached exec, so
        // `exec_attached` and `exec_logs` MUST be gated on the same runtime
        // presence. Lock that they never diverge however the inputs vary.
        for exec_paths_present in [false, true] {
            for exec_detached in [false, true] {
                for exec_tty in [false, true] {
                    for read_guest_file in [false, true] {
                        let cfg = derive_capabilities_config(
                            exec_paths_present,
                            exec_detached,
                            exec_tty,
                            read_guest_file,
                        );
                        assert_eq!(
                            cfg.exec_attached, cfg.exec_logs,
                            "exec_attached must imply exec_logs (and vice-versa)"
                        );
                        assert_eq!(cfg.exec_attached, exec_paths_present);
                        assert_eq!(cfg.exec_logs, exec_paths_present);
                        assert_eq!(cfg.exec_detached, exec_detached);
                        assert_eq!(cfg.exec_tty, exec_tty);
                        assert_eq!(cfg.read_guest_file, read_guest_file);
                    }
                }
            }
        }
    }

    #[test]
    fn detached_registry_gate_requires_non_root_uid_and_runtime() {
        assert!(detached_registry_allowed(true, Some(1000), true, true));
        assert!(!detached_registry_allowed(true, Some(0), true, true));
        assert!(!detached_registry_allowed(true, None, true, true));
        assert!(!detached_registry_allowed(false, Some(1000), true, true));
        assert!(!detached_registry_allowed(true, Some(1000), false, true));
        assert!(!detached_registry_allowed(true, Some(1000), true, false));
    }

    #[tokio::test]
    async fn service_startup_disables_detached_for_root_alias_and_unresolved_workload_user() {
        for (uid, label, instance) in [
            (
                crate::login_session::WorkloadUserUid::Root,
                "uid-0 alias",
                31,
            ),
            (
                crate::login_session::WorkloadUserUid::Unresolved,
                "unresolved user",
                32,
            ),
        ] {
            let runtime = prepare_service_runtime_with_probe(
                startup_config("alice"),
                &TestStartupProbe { uid },
            )
            .await
            .unwrap();
            assert!(runtime.detached.is_none(), "{label}: no detached registry");
            let service = GuestControlService::new(
                runtime.auth,
                runtime.exec,
                runtime.detached,
                test_context(instance),
            );
            authenticate(&service).await;

            let ctx = ttrpc_context();
            let caps = service
                .capabilities(&ctx, capabilities_request())
                .await
                .unwrap();
            let cap_values = caps
                .capabilities
                .iter()
                .map(|cap| cap.enum_value().unwrap())
                .collect::<Vec<_>>();
            assert!(
                !cap_values.contains(&pb::GuestCapability::GUEST_CAPABILITY_EXEC_DETACHED),
                "{label}: EXEC_DETACHED must not be advertised"
            );

            let mut request = pb::ExecCreateRequest::new();
            request.metadata = metadata();
            request.argv = vec!["/bin/true".to_owned()];
            request.detached = true;
            let mut output_policy = pb::OutputPolicy::new();
            output_policy.max_chunk_bytes = 64 * 1024;
            request.output_policy = MessageField::some(output_policy);
            let response = service.exec_create(&ctx, request).await.unwrap();
            assert_disabled(
                response
                    .error
                    .as_ref()
                    .unwrap_or_else(|| panic!("{label}: detached create disabled error")),
            );
            assert!(response.exec_id.is_none(), "{label}: no exec id allocated");
        }
    }

    fn test_exec() -> SharedExec {
        Arc::new(ExecRuntime::new(
            crate::exec_linux::LinuxProcessSpawner::disabled(),
            OsExecIds,
            ExecPolicy::disabled(),
        ))
    }

    fn test_exec_root_enabled() -> SharedExec {
        Arc::new(ExecRuntime::new(
            crate::exec_linux::LinuxProcessSpawner::disabled(),
            OsExecIds,
            ExecPolicy {
                enabled: true,
                exec_user: Some("john".to_owned()),
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

    fn exec_metadata(exec_id: &str) -> MessageField<pb::ExecRequestMetadata> {
        let mut exec_meta = pb::ExecRequestMetadata::new();
        exec_meta.common = metadata();
        exec_meta.exec_id = exec_id.to_owned();
        exec_meta.guest_boot_id = "boot-1".to_owned();
        MessageField::some(exec_meta)
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
        assert!(
            service
                .capabilities(&ctx, capabilities_request())
                .await
                .is_ok()
        );

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
        assert_unauthenticated(service.exec_list(&ctx, pb::ExecListRequest::new()).await);
    }

    #[tokio::test]
    async fn detached_dependent_rpcs_disabled_without_registry() {
        let ctx = ttrpc_context();
        let service = test_service(5);
        authenticate(&service).await;
        // Detached-registry-gated RPCs remain typed-disabled when no detached
        // store is configured (these short-circuit before metadata validation).
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
                .exec_cancel(&ctx, pb::ExecCancelRequest::new())
                .await
                .unwrap()
                .error
                .as_ref()
                .unwrap(),
        );
        assert_disabled(
            service
                .exec_list(&ctx, pb::ExecListRequest::new())
                .await
                .unwrap()
                .error
                .as_ref()
                .unwrap(),
        );
    }

    #[tokio::test]
    async fn interactive_rpcs_reject_unknown_exec_and_bad_signal_target() {
        let ctx = ttrpc_context();
        let service = GuestControlService::new(
            test_auth(),
            test_exec_root_enabled(),
            None,
            test_context(15),
        );
        authenticate(&service).await;

        // WriteStdin / CloseStdin against an unknown exec map to a typed
        // not-found error (no PTY helper is wired in this unit config, so no
        // interactive exec can exist).
        let mut write = pb::WriteStdinRequest::new();
        write.metadata = exec_metadata("exec-unknown");
        write.data = b"hi".to_vec();
        let write_response = service.write_stdin(&ctx, write).await.unwrap();
        assert_eq!(
            write_response
                .error
                .as_ref()
                .unwrap()
                .kind
                .enum_value()
                .unwrap(),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_EXEC_NOT_FOUND
        );
        assert_eq!(
            write_response.disposition.enum_value().unwrap(),
            pb::WriteDisposition::WRITE_DISPOSITION_REJECTED
        );

        let mut close = pb::CloseStdinRequest::new();
        close.metadata = exec_metadata("exec-unknown");
        let close_response = service.close_stdin(&ctx, close).await.unwrap();
        assert_eq!(
            close_response
                .error
                .as_ref()
                .unwrap()
                .kind
                .enum_value()
                .unwrap(),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_EXEC_NOT_FOUND
        );

        // TtyWinResize against an unknown exec acks with a typed error.
        let mut resize = pb::TtyWinResizeRequest::new();
        resize.metadata = exec_metadata("exec-unknown");
        resize.control_seq = 1;
        resize.rows = 24;
        resize.cols = 80;
        let resize_ack = service.tty_win_resize(&ctx, resize).await.unwrap();
        assert_eq!(resize_ack.control_seq, 1);
        assert_eq!(
            resize_ack
                .error
                .as_ref()
                .unwrap()
                .kind
                .enum_value()
                .unwrap(),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_EXEC_NOT_FOUND
        );

        // ExecSignal with a non-foreground target is rejected before the exec is
        // even looked up (the control_seq is not consumed).
        let mut bad_target = pb::ExecSignalRequest::new();
        bad_target.metadata = exec_metadata("exec-unknown");
        bad_target.control_seq = 1;
        bad_target.signal = 2;
        bad_target.target = EnumOrUnknown::new(pb::SignalTarget::SIGNAL_TARGET_PROCESS_TREE);
        let bad_target_ack = service.exec_signal(&ctx, bad_target).await.unwrap();
        assert_eq!(
            bad_target_ack
                .error
                .as_ref()
                .unwrap()
                .kind
                .enum_value()
                .unwrap(),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR
        );

        // ExecSignal with the foreground target against an unknown exec maps to
        // a typed not-found error.
        let mut signal = pb::ExecSignalRequest::new();
        signal.metadata = exec_metadata("exec-unknown");
        signal.control_seq = 1;
        signal.signal = 2;
        signal.target =
            EnumOrUnknown::new(pb::SignalTarget::SIGNAL_TARGET_FOREGROUND_PROCESS_GROUP);
        let signal_ack = service.exec_signal(&ctx, signal).await.unwrap();
        assert_eq!(
            signal_ack
                .error
                .as_ref()
                .unwrap()
                .kind
                .enum_value()
                .unwrap(),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_EXEC_NOT_FOUND
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
    async fn detached_create_is_disabled_without_registry() {
        let ctx = ttrpc_context();
        let service = GuestControlService::new(
            test_auth(),
            test_exec_root_enabled(),
            None,
            test_context(16),
        );
        authenticate(&service).await;
        let mut request = pb::ExecCreateRequest::new();
        request.metadata = metadata();
        request.argv = vec!["/bin/true".to_owned()];
        request.detached = true;
        let mut output_policy = pb::OutputPolicy::new();
        output_policy.max_chunk_bytes = 64 * 1024;
        request.output_policy = MessageField::some(output_policy);
        let response = service.exec_create(&ctx, request).await.unwrap();
        assert_disabled(response.error.as_ref().expect("detached disabled"));
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
                ExecError::InvalidProgram,
                Pb::GUEST_CONTROL_ERROR_KIND_INVALID_PROGRAM,
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
            // Interactive TTY exec variants — none collapse silently; the
            // mode/validation faults map to ProtocolError deliberately.
            (
                ExecError::InvalidTerminalSize,
                Pb::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR,
            ),
            (
                ExecError::TtyStderrUnavailable,
                Pb::GUEST_CONTROL_ERROR_KIND_TTY_STDERR_UNAVAILABLE,
            ),
            (
                ExecError::TtyRequired,
                Pb::GUEST_CONTROL_ERROR_KIND_TTY_REQUIRED,
            ),
            (
                ExecError::StdinClosed,
                Pb::GUEST_CONTROL_ERROR_KIND_STDIN_CLOSED,
            ),
            (
                ExecError::StdinOffsetMismatch,
                Pb::GUEST_CONTROL_ERROR_KIND_STDIN_OFFSET_MISMATCH,
            ),
            (
                ExecError::StdinByteBudgetExhausted,
                Pb::GUEST_CONTROL_ERROR_KIND_STDIN_BYTE_BUDGET_EXHAUSTED,
            ),
            (
                ExecError::StdinBackpressure,
                Pb::GUEST_CONTROL_ERROR_KIND_STDIN_BACKPRESSURE,
            ),
            (
                ExecError::ControlSeqMismatch,
                Pb::GUEST_CONTROL_ERROR_KIND_CONTROL_SEQ_MISMATCH,
            ),
            (
                ExecError::InvalidSignal,
                Pb::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR,
            ),
            (
                ExecError::ExecClosing,
                Pb::GUEST_CONTROL_ERROR_KIND_EXEC_ALREADY_EXITED,
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

    #[test]
    fn detached_retained_log_faults_have_actionable_remediation() {
        use pb::GuestControlErrorKind as K;
        use pb::HealthRemediation as R;

        // Quota breach: shed load and retry after one reaper interval (the
        // periodic GC frees retained-log space) — NOT a generic blind RETRY.
        let quota = guest_error_kind(ExecError::RetainedLogQuotaExceeded);
        assert_eq!(
            quota.remediation.enum_value().unwrap(),
            R::HEALTH_REMEDIATION_REDUCE_LOAD
        );
        assert_eq!(quota.retry_after_ms, Some(DETACHED_REAPER_INTERVAL_MS));
        assert_eq!(
            quota.kind.enum_value().unwrap(),
            K::GUEST_CONTROL_ERROR_KIND_RETAINED_LOG_QUOTA_EXCEEDED
        );

        // Unsafe retained-log path: an internal storage fault — advise checking
        // the guestd service, and DO NOT advertise a retry window.
        let unsafe_path = guest_error_kind(ExecError::RetainedLogPathUnsafe);
        assert_eq!(
            unsafe_path.remediation.enum_value().unwrap(),
            R::HEALTH_REMEDIATION_CHECK_GUESTD_SERVICE
        );
        assert_eq!(unsafe_path.retry_after_ms, None);

        // A representative caller-retryable fault keeps the generic RETRY shape.
        let not_found = guest_error_kind(ExecError::ExecNotFound);
        assert_eq!(
            not_found.remediation.enum_value().unwrap(),
            R::HEALTH_REMEDIATION_RETRY
        );
        assert_eq!(not_found.retry_after_ms, None);
    }

    // ---- ExecList service-path fakes -------------------------------------
    //
    // Minimal in-memory backends so the `exec_list` handler can be exercised
    // against a real `DetachedRegistry` without spawning processes or units.
    // These are intentionally self-contained (a little duplicated shape from
    // the registry's own test fakes) so the service tests stay decoupled from
    // the registry's private test module. A seeded record always resolves
    // terminal (spawn-failed) so `list` returns a bounded, stable set.
    use crate::detached::{ManagedUnit, TransientUnitManager, UnitError};
    use crate::detached_registry::{DetachedCaps, Sleeper, SlotStore, WallClock};
    use crate::exec::{ExecIdSource, ValidatedCommand};
    use nixling_exec_runner::DETACHED_RETAINED_PER_VM;
    use nixling_exec_runner::filering::{FileRingError, RingChunk, StreamMeta};
    use nixling_exec_runner::paths::Stream as RunnerStream;
    use nixling_exec_runner::record::{DurableRecord, StatusPhase};
    use nixling_exec_runner::spec::ExecSpec;
    use std::sync::atomic::AtomicU64;

    #[derive(Default)]
    struct SvcStore;

    impl SlotStore for SvcStore {
        fn prepare_slot_dir(&self, _slot: u32) -> Result<(), ExecError> {
            Ok(())
        }
        fn write_record(&self, _slot: u32, _record: &DurableRecord) -> Result<(), ExecError> {
            Ok(())
        }
        fn read_record(&self, _slot: u32) -> Result<DurableRecord, ExecError> {
            Err(ExecError::ExecNotFound)
        }
        fn write_spec(&self, _slot: u32, _spec: &ExecSpec) -> Result<(), ExecError> {
            Ok(())
        }
        fn read_spec(&self, _slot: u32) -> Result<ExecSpec, ExecError> {
            Err(ExecError::Internal)
        }
        fn write_cancel(&self, _slot: u32) -> Result<(), ExecError> {
            Ok(())
        }
        fn read_status(&self, _slot: u32) -> Result<Option<StatusPhase>, ExecError> {
            // Every seeded create resolves immediately to a terminal,
            // retained record.
            Ok(Some(StatusPhase::SpawnFailed))
        }
        fn read_log_meta(
            &self,
            _slot: u32,
            _stream: RunnerStream,
        ) -> Result<Option<StreamMeta>, ExecError> {
            Ok(None)
        }
        fn read_log(
            &self,
            _slot: u32,
            _stream: RunnerStream,
            _offset: u64,
            _max_len: u64,
        ) -> Result<RingChunk, FileRingError> {
            Err(FileRingError::OffsetInFuture)
        }
        fn mark_lost(&self, _slot: u32) -> Result<(), ExecError> {
            Ok(())
        }
        fn delete_slot_dir(&self, _slot: u32) -> Result<(), ExecError> {
            Ok(())
        }
        fn scrub_slot_files(&self, _slot: u32) -> Result<(), ExecError> {
            Ok(())
        }
        fn validate_authenticity(&self, _slot: u32) -> Result<(), ExecError> {
            Ok(())
        }
        fn list_slot_dirs(&self) -> Result<Vec<u32>, ExecError> {
            Ok(Vec::new())
        }
    }

    #[derive(Default)]
    struct SvcUnits;

    #[async_trait]
    impl TransientUnitManager for SvcUnits {
        async fn start_transient_unit(
            &self,
            _slot: u32,
            _ceiling_sec: u64,
            _paths: &RunnerUnitPaths,
        ) -> Result<(), UnitError> {
            Ok(())
        }
        async fn stop_unit(&self, _slot: u32) -> Result<(), UnitError> {
            Ok(())
        }
        async fn reset_failed(&self, _slot: u32) -> Result<(), UnitError> {
            Ok(())
        }
        async fn list_managed_units(&self) -> Result<Vec<ManagedUnit>, UnitError> {
            Ok(Vec::new())
        }
    }

    struct SvcClock;
    impl WallClock for SvcClock {
        fn now_ms(&self) -> u64 {
            1_000
        }
    }

    struct SvcSleeper;
    #[async_trait]
    impl Sleeper for SvcSleeper {
        async fn sleep_ms(&self, _ms: u64) {}
    }

    struct SvcIds {
        next: AtomicU64,
    }
    impl ExecIdSource for SvcIds {
        fn next_exec_id(&self) -> Result<String, ExecError> {
            let n = self.next.fetch_add(1, Ordering::SeqCst);
            Ok(format!("{n:032x}"))
        }
    }

    fn svc_command(index: u32) -> ValidatedCommand {
        ValidatedCommand {
            program: "/bin/echo".into(),
            args: vec![format!("entry-{index}")],
            cwd: "/".into(),
            env: Vec::new(),
        }
    }

    /// Build a `DetachedRegistry` bound to `boot_id` and seed `count` terminal
    /// (retained) records, each with a distinct opaque id and argv hash.
    async fn seeded_detached(boot_id: &str, count: u32) -> Arc<DetachedRegistry> {
        let registry = DetachedRegistry::new(
            Arc::new(SvcUnits),
            Arc::new(SvcStore),
            Arc::new(SvcClock),
            Arc::new(SvcSleeper),
            Arc::new(SvcIds {
                next: AtomicU64::new(1),
            }),
            RegistryConfig {
                paths: RunnerUnitPaths::new("/run/current-system/sw/bin/nixling-exec-runner"),
                boot_id: boot_id.to_owned(),
                max_runtime_sec: 0,
                exec_user: "alice".to_owned(),
                exec_uid: 1000,
                systemd_run_path: "/run/current-system/sw/bin/systemd-run".to_owned(),
                login_shell_path: "/run/current-system/sw/bin/bash".to_owned(),
            },
        );
        for index in 0..count {
            registry
                .create(boot_id, svc_command(index), DetachedCaps::standard(0))
                .await
                .expect("seed detached record");
        }
        Arc::new(registry)
    }

    fn exec_list_request(guest_boot_id: &str) -> pb::ExecListRequest {
        let mut request = pb::ExecListRequest::new();
        request.metadata = metadata();
        request.guest_boot_id = guest_boot_id.to_owned();
        request
    }

    fn service_with_detached(instance: u8, detached: Arc<DetachedRegistry>) -> GuestControlService {
        GuestControlService::new(
            test_auth(),
            test_exec(),
            Some(detached),
            test_context(instance),
        )
    }

    #[tokio::test]
    async fn exec_list_requires_auth_before_touching_registry() {
        let ctx = ttrpc_context();
        let detached = seeded_detached("boot-A", 1).await;
        let service = service_with_detached(7, detached);
        assert_unauthenticated(service.exec_list(&ctx, exec_list_request("boot-A")).await);
    }

    #[tokio::test]
    async fn exec_list_same_boot_returns_bounded_argv_hash_only_entries() {
        let ctx = ttrpc_context();
        let detached = seeded_detached("boot-A", 3).await;
        let service = service_with_detached(7, detached);
        authenticate(&service).await;

        let response = service
            .exec_list(&ctx, exec_list_request("boot-A"))
            .await
            .expect("exec_list ok");
        assert!(response.error.is_none(), "no error on same-boot list");
        assert_eq!(response.entries.len(), 3, "every seeded record is listed");
        // Each entry exposes the argv DIGEST only — the wire entry structurally
        // has no raw argv/env/cwd field, so no command bytes can leak through
        // ExecList. The retained-cap bound is exercised non-vacuously by
        // `exec_list_is_bounded_at_the_retained_cap` (this list of 3 is far
        // below the cap, so asserting the bound here would be vacuous).
        let mut ids: Vec<String> = Vec::new();
        for entry in &response.entries {
            assert_eq!(
                entry.argv_sha256.len(),
                64,
                "argv_sha256 is a 32-byte hex digest"
            );
            assert!(
                entry.argv_sha256.bytes().all(|b| b.is_ascii_hexdigit()),
                "argv_sha256 is hex"
            );
            assert!(!entry.exec_id.is_empty());
            ids.push(entry.exec_id.clone());
        }
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 3, "exec ids are distinct");
    }

    #[tokio::test]
    async fn exec_list_is_bounded_at_the_retained_cap() {
        let ctx = ttrpc_context();
        // Fill the registry to the retained-per-VM cap. Each seeded create
        // resolves terminal (SpawnFailed) and releases its active count, so the
        // active cap never blocks reaching the retained cap.
        let detached = seeded_detached("boot-A", DETACHED_RETAINED_PER_VM as u32).await;

        // The registry refuses to retain MORE than the cap: every slot in
        // `0..DETACHED_RETAINED_PER_VM` is occupied, so a create past the cap is
        // rejected with capacity-exceeded. This is what makes the registry
        // structurally unable to hold more than the cap.
        let over_cap = detached
            .create(
                "boot-A",
                svc_command(DETACHED_RETAINED_PER_VM as u32),
                DetachedCaps::standard(0),
            )
            .await;
        assert!(
            matches!(over_cap, Err(ExecError::ExecCapacityExceeded)),
            "creating past the retained cap must be rejected, got {over_cap:?}"
        );

        let service = service_with_detached(7, detached);
        authenticate(&service).await;

        let response = service
            .exec_list(&ctx, exec_list_request("boot-A"))
            .await
            .expect("exec_list ok");
        assert!(response.error.is_none(), "no error on same-boot list");
        // Non-vacuous bound: the registry is full, so the handler returns
        // EXACTLY the cap. A regression that let the registry grow past the cap
        // (or the handler emit more than the cap) would trip these assertions.
        assert_eq!(
            response.entries.len(),
            DETACHED_RETAINED_PER_VM,
            "a full registry lists exactly the retained-per-VM cap"
        );
        assert!(
            response.entries.len() <= DETACHED_RETAINED_PER_VM,
            "ExecList response is bounded at the retained-per-VM cap"
        );
        // Every listed slot is in-range and unique — no slot leaks past the cap.
        let mut slots: Vec<u32> = response.entries.iter().map(|e| e.slot).collect();
        slots.sort_unstable();
        slots.dedup();
        assert_eq!(
            slots.len(),
            DETACHED_RETAINED_PER_VM,
            "every retained slot is listed exactly once"
        );
        assert!(
            slots
                .iter()
                .all(|&slot| (slot as usize) < DETACHED_RETAINED_PER_VM),
            "every listed slot is within the retained-per-VM range"
        );
    }

    #[tokio::test]
    async fn exec_list_boot_mismatch_is_stale_session() {
        let ctx = ttrpc_context();
        // Registry bound to a DIFFERENT boot than the request asks for.
        let detached = seeded_detached("boot-A", 2).await;
        let service = service_with_detached(7, detached);
        authenticate(&service).await;

        let response = service
            .exec_list(&ctx, exec_list_request("boot-B"))
            .await
            .expect("exec_list returns a typed error envelope, not a transport error");
        assert!(
            response.entries.is_empty(),
            "no entries leak across a boot mismatch"
        );
        let error = response
            .error
            .as_ref()
            .expect("stale-session error envelope");
        assert_eq!(
            error.kind.enum_value().unwrap(),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_STALE_SESSION
        );
    }

    #[tokio::test]
    async fn exec_list_rejects_empty_guest_boot_id() {
        let ctx = ttrpc_context();
        let detached = seeded_detached("boot-A", 1).await;
        let service = service_with_detached(7, detached);
        authenticate(&service).await;

        let result = service.exec_list(&ctx, exec_list_request("")).await;
        match result {
            Err(ttrpc::Error::RpcStatus(status)) => {
                assert_eq!(
                    status.code.enum_value().unwrap(),
                    ttrpc::Code::INVALID_ARGUMENT
                );
            }
            other => panic!("expected invalid-argument status, got {other:?}"),
        }
    }

    #[test]
    fn write_stdin_handler_budget_sheds_then_releases_on_drop() {
        // The per-connection WriteStdin handler budget sheds the
        // (N+1)th concurrent handler with StdinBackpressure, and dropping a
        // guard returns the slot so a subsequent acquire succeeds.
        let service = test_service(42);
        let mut held = Vec::new();
        for _ in 0..WRITE_STDIN_HANDLERS_PER_CONNECTION {
            held.push(
                service
                    .acquire_write_stdin_slot(1)
                    .expect("slot within budget"),
            );
        }
        assert!(
            matches!(
                service.acquire_write_stdin_slot(1),
                Err(ExecError::StdinBackpressure)
            ),
            "the handler over the ceiling is shed"
        );
        // Releasing one guard frees exactly one slot.
        drop(held.pop());
        let reclaimed = service
            .acquire_write_stdin_slot(1)
            .expect("a slot frees up after a guard drops");
        // ...and we are back at the ceiling.
        assert!(matches!(
            service.acquire_write_stdin_slot(1),
            Err(ExecError::StdinBackpressure)
        ));
        drop(reclaimed);
        drop(held);
    }

    #[test]
    fn write_stdin_byte_budget_exhaustion_releases_the_handler_slot() {
        // When the decoded-byte budget is exhausted, the acquire fails with
        // StdinByteBudgetExhausted AND must roll back the handler-count bump it
        // made first, so the connection is not left with a leaked handler slot.
        let service = test_service(43);
        let over = DECODED_WRITE_STDIN_BYTES_PER_CONNECTION + 1;
        assert!(matches!(
            service.acquire_write_stdin_slot(over),
            Err(ExecError::StdinByteBudgetExhausted)
        ));
        // The handler slot was rolled back: all N handlers are available again.
        let mut held = Vec::new();
        for _ in 0..WRITE_STDIN_HANDLERS_PER_CONNECTION {
            held.push(
                service
                    .acquire_write_stdin_slot(1)
                    .expect("handler slot was not leaked by the byte-budget rejection"),
            );
        }
        drop(held);
    }

    // ---- ReadGuestFile ------------------------------------------------

    fn scratch_dir(tag: &str) -> PathBuf {
        // Repo convention for guest-crate tests: scratch under the system temp
        // dir (respects TMPDIR), never the repo-relative ".".
        let base = std::env::temp_dir();
        let dir = base.join(format!(
            "guestd-rgf-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn service_with_config(instance: u8, path: Option<PathBuf>) -> GuestControlService {
        GuestControlService::new(test_auth(), test_exec(), None, test_context(instance))
            .with_guest_config_path(path)
    }

    fn read_guest_file_request(file_id: pb::GuestFileId) -> pb::ReadGuestFileRequest {
        let mut request = pb::ReadGuestFileRequest::new();
        request.metadata = metadata();
        request.file_id = EnumOrUnknown::new(file_id);
        request
    }

    fn assert_file_error(response: &pb::ReadGuestFileResponse, kind: pb::GuestControlErrorKind) {
        // Fail-closed shape: NO content/size/hash on error (D7/D10 no-leak).
        assert!(
            response.content.is_empty(),
            "content must not leak on error"
        );
        assert_eq!(response.size_bytes, 0, "size must not leak on error");
        assert!(response.sha256.is_empty(), "sha256 must not leak on error");
        let error = response.error.as_ref().expect("error set");
        assert_eq!(error.kind.enum_value().unwrap(), kind);
        // D12: file faults are never advertised as blindly retryable.
        assert_ne!(
            error.remediation.enum_value().unwrap(),
            pb::HealthRemediation::HEALTH_REMEDIATION_RETRY
        );
    }

    fn config_request() -> pb::ReadGuestFileRequest {
        read_guest_file_request(pb::GuestFileId::GUEST_FILE_ID_GUEST_CONFIG)
    }

    #[tokio::test]
    async fn read_guest_file_requires_authentication() {
        let dir = scratch_dir("auth");
        let path = dir.join("config");
        std::fs::write(&path, b"data").unwrap();
        let service = service_with_config(50, Some(path));
        let ctx = ttrpc_context();
        // No authenticate(): must fail UNAUTHENTICATED before any stat/read.
        assert_unauthenticated(service.read_guest_file(&ctx, config_request()).await);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn read_guest_file_enforces_size_cap_at_boundary() {
        let dir = scratch_dir("cap");
        let ctx = ttrpc_context();
        let cap = READ_GUEST_FILE_MAX_BYTES as usize;
        for (tag, size, expect_ok) in [
            ("cap-minus-1", cap - 1, true),
            ("cap", cap, true),
            ("cap-plus-1", cap + 1, false),
        ] {
            let path = dir.join(tag);
            std::fs::write(&path, vec![0x61_u8; size]).unwrap();
            let service = service_with_config(51, Some(path));
            authenticate(&service).await;
            let response = service
                .read_guest_file(&ctx, config_request())
                .await
                .unwrap();
            if expect_ok {
                assert!(response.error.is_none(), "{tag} should succeed");
                assert_eq!(response.size_bytes as usize, size, "{tag} size");
                assert_eq!(response.content.len(), size, "{tag} content len");
            } else {
                assert_file_error(
                    &response,
                    pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_FILE_TOO_LARGE,
                );
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn read_guest_file_returns_content_and_recomputed_hash() {
        let dir = scratch_dir("ok");
        let path = dir.join("config");
        let body = b"hostname = corp-vm\n".to_vec();
        std::fs::write(&path, &body).unwrap();
        let service = service_with_config(52, Some(path));
        authenticate(&service).await;
        let ctx = ttrpc_context();
        let response = service
            .read_guest_file(&ctx, config_request())
            .await
            .unwrap();
        assert!(response.error.is_none());
        assert_eq!(response.content, body);
        assert_eq!(response.size_bytes as usize, body.len());
        assert_eq!(response.sha256, sha256_hex(&body));
        assert_eq!(
            response.file_id.enum_value().unwrap(),
            pb::GuestFileId::GUEST_FILE_ID_GUEST_CONFIG
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn read_guest_file_missing_file_is_file_not_found() {
        let dir = scratch_dir("missing");
        let path = dir.join("does-not-exist");
        let service = service_with_config(53, Some(path));
        authenticate(&service).await;
        let ctx = ttrpc_context();
        let response = service
            .read_guest_file(&ctx, config_request())
            .await
            .unwrap();
        assert_file_error(
            &response,
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_FILE_NOT_FOUND,
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn read_guest_file_symlink_leaf_is_path_unsafe() {
        use std::os::unix::fs::symlink;
        let dir = scratch_dir("symlink");
        let target = dir.join("real");
        std::fs::write(&target, b"secret").unwrap();
        let link = dir.join("config");
        symlink(&target, &link).unwrap();
        let service = service_with_config(54, Some(link));
        authenticate(&service).await;
        let ctx = ttrpc_context();
        let response = service
            .read_guest_file(&ctx, config_request())
            .await
            .unwrap();
        // O_NOFOLLOW on the leaf rejects the symlink (ELOOP) -> PathUnsafe; the
        // symlink target bytes never leave the guest.
        assert_file_error(
            &response,
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_PATH_UNSAFE,
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn read_guest_file_directory_is_path_unsafe() {
        let dir = scratch_dir("isdir");
        let path = dir.join("config-dir");
        std::fs::create_dir(&path).unwrap();
        let service = service_with_config(55, Some(path));
        authenticate(&service).await;
        let ctx = ttrpc_context();
        let response = service
            .read_guest_file(&ctx, config_request())
            .await
            .unwrap();
        // A non-regular target (directory) is rejected after fstat.
        assert_file_error(
            &response,
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_PATH_UNSAFE,
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn read_guest_file_without_configured_path_is_read_denied() {
        let service = service_with_config(56, None);
        authenticate(&service).await;
        let ctx = ttrpc_context();
        let response = service
            .read_guest_file(&ctx, config_request())
            .await
            .unwrap();
        assert_file_error(
            &response,
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_READ_DENIED,
        );
    }

    #[tokio::test]
    async fn read_guest_file_unspecified_id_is_path_unsafe() {
        let dir = scratch_dir("unspec");
        let path = dir.join("config");
        std::fs::write(&path, b"data").unwrap();
        let service = service_with_config(57, Some(path));
        authenticate(&service).await;
        let ctx = ttrpc_context();
        let response = service
            .read_guest_file(
                &ctx,
                read_guest_file_request(pb::GuestFileId::GUEST_FILE_ID_UNSPECIFIED),
            )
            .await
            .unwrap();
        assert_file_error(
            &response,
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_PATH_UNSAFE,
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn read_guest_file_cap_response_fits_ttrpc_frame() {
        // D20a: a cap-sized ReadGuestFile response, protobuf-encoded for the
        // ttRPC frame, must stay below the ttRPC frame cap.
        let dir = scratch_dir("ttrpc-frame");
        let path = dir.join("config");
        std::fs::write(&path, vec![0x62_u8; READ_GUEST_FILE_MAX_BYTES as usize]).unwrap();
        let service = service_with_config(58, Some(path));
        authenticate(&service).await;
        let ctx = ttrpc_context();
        let response = service
            .read_guest_file(&ctx, config_request())
            .await
            .unwrap();
        assert!(response.error.is_none());
        let encoded = response.write_to_bytes().unwrap();
        assert!(
            (encoded.len() as u64) < nixling_ipc::guest_wire::TTRPC_FRAME_CAP_BYTES,
            "encoded cap response {} must fit ttRPC frame cap {}",
            encoded.len(),
            nixling_ipc::guest_wire::TTRPC_FRAME_CAP_BYTES
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_guest_file_capability_advertised_only_when_path_configured() {
        let with = RuntimeCapabilitiesProvider::new(CapabilitiesConfig {
            read_guest_file: true,
            ..CapabilitiesConfig::default()
        })
        .snapshot()
        .unwrap();
        let advertised = |caps: &[EnumOrUnknown<pb::GuestCapability>]| {
            caps.iter().any(|c| {
                c.enum_value().unwrap() == pb::GuestCapability::GUEST_CAPABILITY_READ_GUEST_FILE
            })
        };
        assert!(advertised(&with.capabilities.capabilities));
        // The negotiated cap is mirrored into the Health snapshot + hash.
        assert!(advertised(&with.health.capabilities));

        let without = RuntimeCapabilitiesProvider::new(CapabilitiesConfig::default())
            .snapshot()
            .unwrap();
        assert!(!advertised(&without.capabilities.capabilities));
        assert!(!advertised(&without.health.capabilities));
    }

    #[test]
    fn attached_exec_advertises_output_capability_for_read_output() {
        // The host requires the output capability (`EXEC_LOGS`) before it will
        // issue `ReadOutput` for a non-TTY attached exec to stream stdout/
        // stderr. A build that advertises attached exec but withholds the
        // output cap establishes a session and then fails fetching output, so
        // the two MUST be advertised together for the attached runtime.
        let advertised = |caps: &[EnumOrUnknown<pb::GuestCapability>], cap: pb::GuestCapability| {
            caps.iter().any(|c| c.enum_value().unwrap() == cap)
        };
        let snap = RuntimeCapabilitiesProvider::new(CapabilitiesConfig {
            exec_attached: true,
            exec_logs: true,
            ..CapabilitiesConfig::default()
        })
        .snapshot()
        .unwrap();
        assert!(advertised(
            &snap.capabilities.capabilities,
            pb::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED
        ));
        assert!(advertised(
            &snap.capabilities.capabilities,
            pb::GuestCapability::GUEST_CAPABILITY_EXEC_LOGS
        ));
        assert!(advertised(
            &snap.health.capabilities,
            pb::GuestCapability::GUEST_CAPABILITY_EXEC_LOGS
        ));
    }
}
