use std::{
    collections::BTreeMap,
    env, fs,
    fs::{File, OpenOptions},
    io::{self, Read, Result as IoResult, Write},
    net::IpAddr,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    pin::Pin,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use d2b_contracts::{
    guest_proto as pb,
    guest_wire::{GUEST_CONTROL_PROTOCOL_VERSION, READ_GUEST_FILE_MAX_BYTES},
};
use futures::stream;
use protobuf::{EnumOrUnknown, Message, MessageField};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    process::Command as TokioCommand,
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
    shell::{ShellRuntime, ShellRuntimeConfig, ShellRuntimeError},
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
type SharedShell = Arc<ShellRuntime>;

const TOKEN_FILE_NAME: &str = "guest_control_token";
const MAX_TOKEN_BYTES: usize = 4096;
/// Cadence of the periodic detached-exec reaper (live reconciliation of
/// vanished units + terminal-record TTL/GC).
const DETACHED_REAPER_INTERVAL_MS: u64 = 30_000;
const USBIP_COMMAND_TIMEOUT_MS: u64 = 10_000;
const USBIP_STATUS_MAX_IMPORTS: usize = 64;
const SHELL_DAEMON_READY_TIMEOUT_MS: u64 = 5_000;
const SHELL_DAEMON_READY_POLL_MS: u64 = 50;
const ACTIVATION_MAX_TIMEOUT_MS: u64 = 60 * 60 * 1_000;
#[cfg(test)]
const ACTIVATION_DEFAULT_TIMEOUT_MS: u64 = 10 * 60 * 1_000;
const ACTIVATION_TIMEOUT_STOP_SEC: u64 = 10;
const WPCTL_COMMAND_TIMEOUT_MS: u64 = 5_000;
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
type SharedActivation = Option<Arc<ActivationRuntime>>;

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
    /// Absolute path to the guest `usbip` binary. Present only for guests whose
    /// host VM declaration enabled USBIP; advertises the `UsbipImport` RPC.
    pub usbip_path: Option<PathBuf>,
    /// Host-owned persistent shell contract policy. guestd parses and stores
    /// this policy so guest units can be rendered, but runtime shell operations
    /// remain fail-closed until the shell runtime is available.
    pub shell_policy: ShellPolicy,
    /// Guest-root system activation runtime. This is independent of guest exec:
    /// activation always runs as root inside the guest via a transient systemd
    /// unit, never through workload-user exec.
    pub activation: Option<ActivationRuntimeConfig>,
    /// Audio runtime configuration: absolute path to `wpctl`. When set and the
    /// binary is reachable at startup, guestd advertises AudioStatus/AudioSet and
    /// serves PipeWire queries targeting the workload user's session.
    /// `None` (default) means audio is not configured; capabilities are not
    /// advertised and handlers return `AudioPipeWireUnavailable` fail-closed.
    pub audio: Option<AudioRuntimeConfig>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellPolicy {
    pub enabled: bool,
    pub default_name: String,
    pub max_sessions: u32,
    pub max_attached: u32,
    pub runner_path: Option<PathBuf>,
    pub systemctl_path: Option<PathBuf>,
}

impl ShellPolicy {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            default_name: "default".to_owned(),
            max_sessions: 8,
            max_attached: 1,
            runner_path: None,
            systemctl_path: None,
        }
    }
}

/// Host-supplied, controlled-constant runtime configuration for detached exec.
/// All paths are absolute store paths passed by the guest module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetachedRuntimeConfig {
    /// Absolute path to `systemd-run`.
    pub systemd_run_path: PathBuf,
    /// Absolute path to the `d2b-exec-runner` binary.
    pub exec_runner_path: PathBuf,
    /// Default per-exec runtime ceiling in seconds; 0 means unlimited.
    pub max_runtime_sec: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivationRuntimeConfig {
    /// Absolute path to `systemd-run` inside the guest.
    pub systemd_run_path: PathBuf,
    /// Absolute path to `systemctl` inside the guest.
    pub systemctl_path: PathBuf,
    /// Root-owned 0700 status directory for rejoinable activation state.
    pub status_dir: PathBuf,
    /// Maximum accepted activation runtime in milliseconds.
    pub max_timeout_ms: u64,
}

/// Host-supplied audio runtime configuration for guestd.
///
/// Holds the absolute path to `wpctl`. The workload-user UID is resolved
/// at startup and combined with this path to build the active audio runtime.
/// Capability advertisement and RPC dispatch are both gated on this being
/// present and the wpctl binary being reachable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioRuntimeConfig {
    /// Absolute path to the `wpctl` binary (Nix-store path from `wireplumber`).
    pub wpctl_path: PathBuf,
}

/// Active audio runtime derived at startup from [`AudioRuntimeConfig`] and
/// the resolved workload-user UID. Only present when the wpctl binary exists
/// and the workload user was successfully resolved.
#[derive(Clone, Debug)]
pub(crate) struct AudioRuntime {
    wpctl_path: PathBuf,
    /// Workload-user UID; used to build `PIPEWIRE_RUNTIME_DIR=/run/user/<uid>`
    /// for every wpctl subprocess so they target the user's PipeWire session.
    workload_uid: u32,
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
            usbip_path: None,
            shell_policy: ShellPolicy::disabled(),
            activation: None,
            audio: None,
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

    /// Attach the host-declared guest USBIP binary path used for
    /// `UsbipImport`. Setting it makes guestd advertise the USBIP capability.
    pub fn with_usbip_path(mut self, path: PathBuf) -> Self {
        self.usbip_path = Some(path);
        self
    }

    pub fn with_shell_policy(mut self, policy: ShellPolicy) -> Self {
        self.shell_policy = policy;
        self
    }

    pub fn with_activation_runtime(mut self, activation: ActivationRuntimeConfig) -> Self {
        self.activation = Some(activation);
        self
    }

    /// Attach audio runtime configuration: the absolute path to the `wpctl`
    /// binary. When set and the binary is reachable at startup, guestd advertises
    /// `AudioStatus`/`AudioSet` and serves PipeWire queries in the workload
    /// user's session. Capability advertisement requires the workload user to
    /// also be configured (`--exec-user`).
    pub fn with_audio_runtime(mut self, config: AudioRuntimeConfig) -> Self {
        self.audio = Some(config);
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
    std::io::Read::by_ref(&mut file)
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
    /// Guest-side USBIP import lifecycle. Advertised only when the USBIP guest
    /// component wired an explicit `usbip` binary path into guestd.
    pub usbip_import: bool,
    /// Side-effect-free guest USBIP import status/list. Advertised with the same
    /// explicit `usbip` binary path as `UsbipImport`.
    pub usbip_status: bool,
    /// Guest-root system activation via authenticated guest-control. This is
    /// independent of workload-user exec and is advertised only when guestd has
    /// a usable systemd-run/systemctl pair plus secure status storage.
    pub system_activation: bool,
    pub shell_attached: bool,
    pub shell_management: bool,
    pub shell_force_attach: bool,
    pub shell_sessions_per_vm: u32,
    pub shell_attached_sessions_per_vm: u32,
    /// Audio status query (read). Advertised when guestd is configured with
    /// audio enabled and can query the in-guest PipeWire session.
    pub audio_status: bool,
    /// Audio set (mute/unmute/volume). Advertised alongside `audio_status`.
    pub audio_set: bool,
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
    usbip_import: bool,
    system_activation: bool,
    shell_limits: Option<(u32, u32)>,
    audio_usable: bool,
) -> CapabilitiesConfig {
    let shell_usable = shell_limits.is_some();
    let (shell_sessions_per_vm, shell_attached_sessions_per_vm) = shell_limits.unwrap_or((0, 0));
    // Audio capabilities are advertised only when the wpctl binary was found at
    // startup and the workload user was resolved. Both conditions are captured
    // in `audio_usable`; without them the handlers return
    // AudioPipeWireUnavailable fail-closed.
    CapabilitiesConfig {
        exec_attached: exec_paths_present,
        exec_detached,
        exec_logs: exec_paths_present,
        exec_tty,
        read_guest_file,
        usbip_import,
        usbip_status: usbip_import,
        system_activation,
        shell_attached: shell_usable,
        shell_management: shell_usable,
        shell_force_attach: shell_usable,
        shell_sessions_per_vm,
        shell_attached_sessions_per_vm,
        audio_status: audio_usable,
        audio_set: audio_usable,
    }
}

/// Map guest-control shell capability fragments into the ADR 0039 core
/// capability contract. The core `persistent-shell` capability is advertised
/// only when guestd can attach, manage, force-attach, and report bounded shell
/// limits; partial guest-control fragments remain fail-closed.
pub fn constellation_shell_capability_set(
    config: &CapabilitiesConfig,
) -> d2b_constellation_core::CapabilitySet {
    let shell_ready = config.shell_attached
        && config.shell_management
        && config.shell_force_attach
        && (1..=256).contains(&config.shell_sessions_per_vm)
        && (1..=64).contains(&config.shell_attached_sessions_per_vm)
        && config.shell_attached_sessions_per_vm <= config.shell_sessions_per_vm;
    if shell_ready {
        d2b_constellation_core::CapabilitySet::empty()
            .with(d2b_constellation_core::Capability::PersistentShell)
    } else {
        d2b_constellation_core::CapabilitySet::empty()
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

#[derive(Clone)]
pub struct ServiceRuntime {
    auth: SharedAuthCore,
    exec: SharedExec,
    detached: SharedDetached,
    shell: SharedShell,
    activation: SharedActivation,
    guest_config_path: Option<PathBuf>,
    usbip_path: Option<PathBuf>,
    audio: Option<Arc<AudioRuntime>>,
}

trait StartupProbe {
    fn classify_workload_user(&self, user: &str) -> crate::login_session::WorkloadUserUid;
    fn guest_boot_id(&self) -> Result<String, GuestAuthError>;
    fn path_is_file(&self, path: &Path) -> bool;
    fn detached_runtime_usable(&self, detached: &DetachedRuntimeConfig) -> bool;
    fn activation_status_dir_usable(&self, path: &Path) -> bool {
        validate_activation_status_dir(path).is_ok()
    }
    fn shpool_service_usable(&self, systemctl_path: &Path) -> bool {
        self.path_is_file(systemctl_path)
    }
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

    fn activation_status_dir_usable(&self, path: &Path) -> bool {
        validate_activation_status_dir(path).is_ok()
    }

    fn shpool_service_usable(&self, systemctl_path: &Path) -> bool {
        shpool_service_usable(systemctl_path)
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
                    "d2b-guestd: refusing guest exec: workload user '{user}' resolves to \
                     UID 0; guest exec never runs as root"
                );
                config.exec_policy.exec_user = None;
            }
            crate::login_session::WorkloadUserUid::Unresolved => {
                eprintln!(
                    "d2b-guestd: refusing guest exec: workload user '{user}' is not \
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

    // Exec runtime paths (the `systemd-run` binary + the `d2b-exec-runner`
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
    let usbip_path = config
        .usbip_path
        .as_ref()
        .filter(|path| probe.path_is_file(path))
        .cloned();
    let shell_runtime_usable = shell_policy_runtime_usable(
        &config.shell_policy,
        exec_paths.is_some(),
        exec_uid,
        |path| probe.path_is_file(path),
        |path| probe.shpool_service_usable(path),
    );
    let activation: SharedActivation = config
        .activation
        .as_ref()
        .filter(|activation| {
            probe.path_is_file(&activation.systemd_run_path)
                && probe.path_is_file(&activation.systemctl_path)
                && probe.activation_status_dir_usable(&activation.status_dir)
        })
        .map(|activation| {
            Arc::new(ActivationRuntime::new(
                Arc::new(ProductionActivationUnitManager::new(
                    activation.systemd_run_path.clone(),
                    activation.systemctl_path.clone(),
                )),
                activation.status_dir.clone(),
                activation.max_timeout_ms,
            ))
        });

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
    let shell: SharedShell = if shell_runtime_usable {
        let guest_boot_id = probe.guest_boot_id().map_err(|_| GuestdServiceError::Io)?;
        let workload_uid = exec_uid.expect("shell_runtime_usable requires a workload uid");
        let guestd_instance_id = generate_protocol_instance_id("guestd")?;
        let shell_daemon_instance_id = generate_protocol_instance_id("shpool")?;
        Arc::new(ShellRuntime::enabled(ShellRuntimeConfig {
            default_name: config.shell_policy.default_name.clone(),
            max_sessions: config.shell_policy.max_sessions,
            max_attached: config.shell_policy.max_attached,
            workload_user: exec_user.clone(),
            workload_uid: Some(workload_uid),
            guest_boot_id,
            guestd_instance_id,
            daemon_instance_id: shell_daemon_instance_id,
            runner_path: config.shell_policy.runner_path.clone().unwrap_or_default(),
            systemctl_path: config
                .shell_policy
                .systemctl_path
                .clone()
                .unwrap_or_default(),
            socket_path: PathBuf::from(format!("/run/user/{workload_uid}/d2b-shpool.sock")),
        }))
    } else {
        Arc::new(ShellRuntime::disabled())
    };

    // Audio runtime: usable only when the wpctl binary is present AND the
    // workload user was resolved. Without both conditions the handlers return
    // AudioPipeWireUnavailable fail-closed and no capabilities are advertised.
    let audio_runtime: Option<Arc<AudioRuntime>> = config
        .audio
        .as_ref()
        .filter(|cfg| probe.path_is_file(&cfg.wpctl_path))
        .and_then(|cfg| {
            exec_uid.map(|uid| {
                Arc::new(AudioRuntime {
                    wpctl_path: cfg.wpctl_path.clone(),
                    workload_uid: uid,
                })
            })
        });

    let capabilities = derive_capabilities_config(
        // Non-TTY attached exec (and its required ReadOutput streaming) is served
        // iff the workload-user runtime paths are present.
        exec_paths.is_some(),
        detached.is_some(),
        exec.tty_usable(),
        config.guest_config_path.is_some(),
        usbip_path.is_some(),
        activation.is_some(),
        shell.is_enabled().then_some((
            config.shell_policy.max_sessions,
            config.shell_policy.max_attached,
        )),
        audio_runtime.is_some(),
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
        shell,
        activation,
        guest_config_path,
        usbip_path,
        audio: audio_runtime,
    })
}

pub async fn serve_vsock(config: GuestdServeConfig) -> Result<(), GuestdServiceError> {
    let vm_id = config.vm_id.clone();
    let runtime = prepare_service_runtime_with_probe(config, &ProductionStartupProbe).await?;
    let listener = VsockListener::bind(VsockAddr::new(VMADDR_CID_ANY, GUEST_CONTROL_AUTH_PORT))
        .map_err(|_| GuestdServiceError::Ttrpc)?;

    loop {
        let Ok((stream, peer_addr)) = listener.accept().await else {
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        };
        let runtime = runtime.clone();
        let vm_id = vm_id.clone();
        tokio::spawn(async move {
            if let Ok(context) = connection_context(vm_id, peer_addr.cid()) {
                let _ = run_single_connection(stream, runtime, context).await;
            }
        });
    }
}

/// Build the production detached registry: `systemd-run`/`systemctl` unit
/// manager, `/run/d2b-exec` slot store, system wall clock, tokio sleeper,
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
/// binaries must exist and `/run/d2b-exec` must be a root-owned directory.
///
fn detached_runtime_usable(detached: &DetachedRuntimeConfig) -> bool {
    if !detached.systemd_run_path.is_file() || !detached.exec_runner_path.is_file() {
        return false;
    }
    match fs::symlink_metadata(d2b_exec_runner::paths::RUN_DIR) {
        Ok(meta) => meta.is_dir() && owner_is_safe(meta.uid()),
        Err(_) => false,
    }
}

fn shpool_service_usable(systemctl_path: &Path) -> bool {
    if !systemctl_path.is_file() {
        return false;
    }
    Command::new(systemctl_path)
        .arg("--no-pager")
        .arg("cat")
        .arg("d2b-shpool-daemon.service")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivationError {
    InvalidId,
    InvalidPath,
    InvalidMode,
    NotFound,
    StatusUnavailable,
    TimedOut,
    SpawnFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActivationStatusSnapshot {
    state: pb::GuestActivationState,
    exit_code: Option<i32>,
    signal: Option<u32>,
    status_code: Option<i32>,
}

impl ActivationStatusSnapshot {
    fn running() -> Self {
        Self {
            state: pb::GuestActivationState::GUEST_ACTIVATION_STATE_RUNNING,
            exit_code: None,
            signal: None,
            status_code: None,
        }
    }

    fn failed_status(status_code: i32) -> Self {
        Self {
            state: pb::GuestActivationState::GUEST_ACTIVATION_STATE_FAILED,
            exit_code: None,
            signal: None,
            status_code: Some(status_code),
        }
    }
}

#[async_trait]
trait ActivationUnitManager: Send + Sync + 'static {
    async fn start_unit(
        &self,
        unit_name: &str,
        switch_script_path: &Path,
        mode_arg: &str,
        timeout_ms: u64,
    ) -> Result<(), ActivationError>;

    async fn query_unit(
        &self,
        unit_name: &str,
    ) -> Result<Option<ActivationStatusSnapshot>, ActivationError>;

    async fn cleanup_terminal_unit(
        &self,
        unit_name: &str,
        snapshot: &ActivationStatusSnapshot,
    ) -> Result<(), ActivationError>;
}

struct ProductionActivationUnitManager {
    systemd_run_path: PathBuf,
    systemctl_path: PathBuf,
}

impl ProductionActivationUnitManager {
    fn new(systemd_run_path: PathBuf, systemctl_path: PathBuf) -> Self {
        Self {
            systemd_run_path,
            systemctl_path,
        }
    }
}

#[async_trait]
impl ActivationUnitManager for ProductionActivationUnitManager {
    async fn start_unit(
        &self,
        unit_name: &str,
        switch_script_path: &Path,
        mode_arg: &str,
        timeout_ms: u64,
    ) -> Result<(), ActivationError> {
        let timeout_sec = timeout_ms.div_ceil(1_000).max(1);
        let mut cmd = TokioCommand::new(&self.systemd_run_path);
        cmd.arg("--quiet")
            .arg(format!("--unit={unit_name}"))
            .arg("-p")
            .arg("User=root")
            .arg("-p")
            .arg("Type=exec")
            .arg("-p")
            .arg("RemainAfterExit=yes")
            .arg("-p")
            .arg("StandardInput=null")
            .arg("-p")
            .arg("KillMode=control-group")
            .arg("-p")
            .arg(format!("TimeoutStopSec={ACTIVATION_TIMEOUT_STOP_SEC}"))
            .arg("-p")
            .arg(format!("RuntimeMaxSec={timeout_sec}"))
            .arg(switch_script_path)
            .arg(mode_arg)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let status = cmd
            .status()
            .await
            .map_err(|_| ActivationError::SpawnFailed)?;
        if status.success() {
            Ok(())
        } else {
            Err(ActivationError::SpawnFailed)
        }
    }

    async fn query_unit(
        &self,
        unit_name: &str,
    ) -> Result<Option<ActivationStatusSnapshot>, ActivationError> {
        let mut cmd = TokioCommand::new(&self.systemctl_path);
        cmd.arg("--no-pager")
            .arg("show")
            .arg(unit_name)
            .arg("-p")
            .arg("LoadState")
            .arg("-p")
            .arg("ActiveState")
            .arg("-p")
            .arg("SubState")
            .arg("-p")
            .arg("Result")
            .arg("-p")
            .arg("ExecMainCode")
            .arg("-p")
            .arg("ExecMainStatus")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let output = cmd
            .output()
            .await
            .map_err(|_| ActivationError::StatusUnavailable)?;
        let text =
            String::from_utf8(output.stdout).map_err(|_| ActivationError::StatusUnavailable)?;
        let fields = parse_systemctl_show(&text);
        if fields
            .get("LoadState")
            .is_some_and(|value| value == "not-found")
        {
            return Ok(None);
        }
        if !output.status.success() && fields.is_empty() {
            return Ok(None);
        }
        let active_state = fields.get("ActiveState").map(String::as_str);
        let sub_state = fields.get("SubState").map(String::as_str);
        if matches!(active_state, Some("active"))
            && matches!(sub_state, Some("exited"))
            && fields.get("Result").map(String::as_str) == Some("success")
        {
            return Ok(Some(ActivationStatusSnapshot {
                state: pb::GuestActivationState::GUEST_ACTIVATION_STATE_SUCCEEDED,
                exit_code: Some(0),
                signal: None,
                status_code: None,
            }));
        }
        if matches!(active_state, Some("active" | "activating" | "reloading")) {
            return Ok(Some(ActivationStatusSnapshot::running()));
        }
        match fields.get("Result").map(String::as_str) {
            Some("success") => Ok(Some(ActivationStatusSnapshot {
                state: pb::GuestActivationState::GUEST_ACTIVATION_STATE_SUCCEEDED,
                exit_code: Some(0),
                signal: None,
                status_code: None,
            })),
            Some("timeout") => Ok(Some(ActivationStatusSnapshot {
                state: pb::GuestActivationState::GUEST_ACTIVATION_STATE_TIMED_OUT,
                exit_code: None,
                signal: None,
                status_code: None,
            })),
            Some(_) => {
                let code = fields
                    .get("ExecMainCode")
                    .and_then(|value| value.parse::<i32>().ok());
                let status = fields
                    .get("ExecMainStatus")
                    .and_then(|value| value.parse::<i32>().ok())
                    .unwrap_or(1);
                let mut snapshot = ActivationStatusSnapshot::failed_status(status);
                match code {
                    Some(1) => {
                        snapshot.exit_code = Some(status);
                        snapshot.status_code = None;
                    }
                    Some(2) if status >= 0 => {
                        snapshot.signal = Some(status as u32);
                        snapshot.status_code = None;
                    }
                    _ => {}
                }
                Ok(Some(snapshot))
            }
            None => Err(ActivationError::StatusUnavailable),
        }
    }

    async fn cleanup_terminal_unit(
        &self,
        unit_name: &str,
        snapshot: &ActivationStatusSnapshot,
    ) -> Result<(), ActivationError> {
        match snapshot.state {
            pb::GuestActivationState::GUEST_ACTIVATION_STATE_SUCCEEDED => {
                let _ = TokioCommand::new(&self.systemctl_path)
                    .arg("--no-pager")
                    .arg("stop")
                    .arg(unit_name)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .await;
            }
            pb::GuestActivationState::GUEST_ACTIVATION_STATE_FAILED
            | pb::GuestActivationState::GUEST_ACTIVATION_STATE_TIMED_OUT
            | pb::GuestActivationState::GUEST_ACTIVATION_STATE_LOST => {
                let _ = TokioCommand::new(&self.systemctl_path)
                    .arg("--no-pager")
                    .arg("reset-failed")
                    .arg(unit_name)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .await;
            }
            _ => {}
        }
        Ok(())
    }
}

fn parse_systemctl_show(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            Some((key.to_owned(), value.to_owned()))
        })
        .collect()
}

struct ActivationRuntime {
    manager: Arc<dyn ActivationUnitManager>,
    status_dir: PathBuf,
    max_timeout_ms: u64,
}

impl ActivationRuntime {
    fn new(
        manager: Arc<dyn ActivationUnitManager>,
        status_dir: PathBuf,
        max_timeout_ms: u64,
    ) -> Self {
        Self {
            manager,
            status_dir,
            max_timeout_ms: max_timeout_ms.clamp(1, ACTIVATION_MAX_TIMEOUT_MS),
        }
    }

    async fn start(
        &self,
        activation_id: &str,
        switch_script_path: &Path,
        mode: pb::GuestActivationMode,
        timeout_ms: u64,
    ) -> Result<ActivationStatusSnapshot, ActivationError> {
        validate_activation_id(activation_id)?;
        validate_switch_script_path_async(switch_script_path.to_path_buf()).await?;
        let mode_arg = activation_mode_arg(mode)?;
        let timeout_ms = timeout_ms.clamp(1, self.max_timeout_ms);
        self.validate_status_dir_async().await?;
        match self.status(activation_id).await {
            Ok(existing) => return Ok(existing),
            Err(ActivationError::NotFound) => {}
            Err(error) => return Err(error),
        }

        let unit_name = activation_unit_name(activation_id)?;
        let running = ActivationStatusSnapshot::running();
        self.write_record_async(activation_id, &running).await?;
        if let Err(error) = self
            .manager
            .start_unit(&unit_name, switch_script_path, mode_arg, timeout_ms)
            .await
        {
            let failed = match error {
                ActivationError::TimedOut => ActivationStatusSnapshot {
                    state: pb::GuestActivationState::GUEST_ACTIVATION_STATE_TIMED_OUT,
                    exit_code: None,
                    signal: None,
                    status_code: None,
                },
                _ => ActivationStatusSnapshot::failed_status(1),
            };
            let _ = self.write_record_async(activation_id, &failed).await;
            return Err(error);
        }
        Ok(running)
    }

    async fn status(
        &self,
        activation_id: &str,
    ) -> Result<ActivationStatusSnapshot, ActivationError> {
        validate_activation_id(activation_id)?;
        self.validate_status_dir_async().await?;
        let record = self.read_record_async(activation_id).await?;
        if record.state != pb::GuestActivationState::GUEST_ACTIVATION_STATE_RUNNING {
            return Ok(record);
        }
        let unit_name = activation_unit_name(activation_id)?;
        match self.manager.query_unit(&unit_name).await? {
            Some(snapshot)
                if snapshot.state == pb::GuestActivationState::GUEST_ACTIVATION_STATE_RUNNING =>
            {
                Ok(snapshot)
            }
            Some(snapshot) => {
                self.write_record_async(activation_id, &snapshot).await?;
                if snapshot.state != pb::GuestActivationState::GUEST_ACTIVATION_STATE_RUNNING {
                    let _ = self
                        .manager
                        .cleanup_terminal_unit(&unit_name, &snapshot)
                        .await;
                }
                Ok(snapshot)
            }
            None => {
                let lost = ActivationStatusSnapshot {
                    state: pb::GuestActivationState::GUEST_ACTIVATION_STATE_LOST,
                    exit_code: None,
                    signal: None,
                    status_code: None,
                };
                self.write_record_async(activation_id, &lost).await?;
                let _ = self.manager.cleanup_terminal_unit(&unit_name, &lost).await;
                Ok(lost)
            }
        }
    }

    async fn validate_status_dir_async(&self) -> Result<(), ActivationError> {
        let status_dir = self.status_dir.clone();
        tokio::task::spawn_blocking(move || validate_activation_status_dir(&status_dir))
            .await
            .map_err(|_| ActivationError::StatusUnavailable)?
    }

    async fn write_record_async(
        &self,
        activation_id: &str,
        snapshot: &ActivationStatusSnapshot,
    ) -> Result<(), ActivationError> {
        let status_dir = self.status_dir.clone();
        let activation_id = activation_id.to_owned();
        let snapshot = snapshot.clone();
        tokio::task::spawn_blocking(move || {
            write_activation_record_blocking(&status_dir, &activation_id, &snapshot)
        })
        .await
        .map_err(|_| ActivationError::StatusUnavailable)?
    }

    async fn read_record_async(
        &self,
        activation_id: &str,
    ) -> Result<ActivationStatusSnapshot, ActivationError> {
        let status_dir = self.status_dir.clone();
        let activation_id = activation_id.to_owned();
        tokio::task::spawn_blocking(move || {
            read_activation_record_blocking(&status_dir, &activation_id)
        })
        .await
        .map_err(|_| ActivationError::StatusUnavailable)?
    }
}

fn activation_record_path(
    status_dir: &Path,
    activation_id: &str,
) -> Result<PathBuf, ActivationError> {
    validate_activation_id(activation_id)?;
    Ok(status_dir.join(format!("{activation_id}.status")))
}

fn write_activation_record_blocking(
    status_dir: &Path,
    activation_id: &str,
    snapshot: &ActivationStatusSnapshot,
) -> Result<(), ActivationError> {
    let path = activation_record_path(status_dir, activation_id)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let tmp = status_dir.join(format!(
        ".{activation_id}.{}.{nonce}.tmp",
        std::process::id(),
    ));
    let data = format!(
        "state={}\nexit_code={}\nsignal={}\nstatus_code={}\n",
        activation_state_name(snapshot.state),
        snapshot.exit_code.map_or(String::new(), |v| v.to_string()),
        snapshot.signal.map_or(String::new(), |v| v.to_string()),
        snapshot
            .status_code
            .map_or(String::new(), |v| v.to_string()),
    );
    let write_result = (|| -> io::Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(data.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, &path)?;
        File::open(status_dir)?.sync_all()?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    write_result.map_err(|_| ActivationError::StatusUnavailable)
}

fn read_activation_record_blocking(
    status_dir: &Path,
    activation_id: &str,
) -> Result<ActivationStatusSnapshot, ActivationError> {
    let path = activation_record_path(status_dir, activation_id)?;
    let meta = fs::symlink_metadata(&path).map_err(|_| ActivationError::NotFound)?;
    if meta.file_type().is_symlink()
        || !meta.file_type().is_file()
        || meta.mode() & 0o077 != 0
        || !owner_is_safe(meta.uid())
    {
        return Err(ActivationError::StatusUnavailable);
    }
    let text = fs::read_to_string(&path).map_err(|_| ActivationError::StatusUnavailable)?;
    parse_activation_record(&text)
}

fn parse_activation_record(text: &str) -> Result<ActivationStatusSnapshot, ActivationError> {
    let fields = parse_systemctl_show(text);
    let state = fields
        .get("state")
        .and_then(|value| activation_state_from_name(value))
        .ok_or(ActivationError::StatusUnavailable)?;
    let optional_i32 = |key: &str| -> Result<Option<i32>, ActivationError> {
        match fields.get(key).map(String::as_str) {
            Some("") | None => Ok(None),
            Some(value) => value
                .parse::<i32>()
                .map(Some)
                .map_err(|_| ActivationError::StatusUnavailable),
        }
    };
    let signal = match fields.get("signal").map(String::as_str) {
        Some("") | None => None,
        Some(value) => Some(
            value
                .parse::<u32>()
                .map_err(|_| ActivationError::StatusUnavailable)?,
        ),
    };
    Ok(ActivationStatusSnapshot {
        state,
        exit_code: optional_i32("exit_code")?,
        signal,
        status_code: optional_i32("status_code")?,
    })
}

fn activation_unit_name(activation_id: &str) -> Result<String, ActivationError> {
    validate_activation_id(activation_id)?;
    Ok(format!("d2b-activation-{activation_id}.service"))
}

fn validate_activation_id(value: &str) -> Result<(), ActivationError> {
    let bytes = value.as_bytes();
    if bytes.len() != 36 || bytes.iter().any(|b| matches!(b, b'/' | b'.' | 0)) {
        return Err(ActivationError::InvalidId);
    }
    for (idx, byte) in bytes.iter().copied().enumerate() {
        let hyphen = matches!(idx, 8 | 13 | 18 | 23);
        if hyphen {
            if byte != b'-' {
                return Err(ActivationError::InvalidId);
            }
        } else if !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte) {
            return Err(ActivationError::InvalidId);
        }
    }
    if bytes[14] != b'4' || !matches!(bytes[19], b'8' | b'9' | b'a' | b'b') {
        return Err(ActivationError::InvalidId);
    }
    Ok(())
}

fn activation_mode_arg(mode: pb::GuestActivationMode) -> Result<&'static str, ActivationError> {
    match mode {
        pb::GuestActivationMode::GUEST_ACTIVATION_MODE_SWITCH => Ok("switch"),
        pb::GuestActivationMode::GUEST_ACTIVATION_MODE_BOOT => Ok("boot"),
        pb::GuestActivationMode::GUEST_ACTIVATION_MODE_TEST => Ok("test"),
        pb::GuestActivationMode::GUEST_ACTIVATION_MODE_DRY_ACTIVATE => Ok("dry-activate"),
        _ => Err(ActivationError::InvalidMode),
    }
}

async fn validate_switch_script_path_async(path: PathBuf) -> Result<(), ActivationError> {
    tokio::task::spawn_blocking(move || validate_switch_script_path(&path))
        .await
        .map_err(|_| ActivationError::InvalidPath)?
}

fn validate_switch_script_path(path: &Path) -> Result<(), ActivationError> {
    let value = path.to_str().ok_or(ActivationError::InvalidPath)?;
    if value.as_bytes().contains(&0) {
        return Err(ActivationError::InvalidPath);
    }
    let prefix = "/nix/store/";
    let suffix = "/bin/switch-to-configuration";
    let Some(rest) = value.strip_prefix(prefix) else {
        return Err(ActivationError::InvalidPath);
    };
    let Some(store_name) = rest.strip_suffix(suffix) else {
        return Err(ActivationError::InvalidPath);
    };
    if store_name.contains('/') || !strict_nix_store_basename(store_name) {
        return Err(ActivationError::InvalidPath);
    }
    let meta = fs::symlink_metadata(path).map_err(|_| ActivationError::InvalidPath)?;
    if meta.file_type().is_symlink() || !meta.file_type().is_file() || meta.mode() & 0o111 == 0 {
        return Err(ActivationError::InvalidPath);
    }
    Ok(())
}

fn strict_nix_store_basename(value: &str) -> bool {
    const NIX_BASE32: &[u8] = b"0123456789abcdfghijklmnpqrsvwxyz";
    let bytes = value.as_bytes();
    if bytes.len() < 34 || bytes[32] != b'-' {
        return false;
    }
    if !bytes[..32].iter().all(|byte| NIX_BASE32.contains(byte)) {
        return false;
    }
    bytes[33..].iter().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.' | b'_' | b'?' | b'=')
    })
}

fn validate_activation_status_dir(path: &Path) -> Result<(), ActivationError> {
    if !path.is_absolute() || path == Path::new("/nix/store") || path.starts_with("/nix/store/") {
        return Err(ActivationError::StatusUnavailable);
    }
    let meta = fs::symlink_metadata(path).map_err(|_| ActivationError::StatusUnavailable)?;
    if meta.file_type().is_symlink()
        || !meta.file_type().is_dir()
        || meta.mode() & 0o077 != 0
        || !owner_is_safe(meta.uid())
    {
        return Err(ActivationError::StatusUnavailable);
    }
    Ok(())
}

fn activation_state_name(state: pb::GuestActivationState) -> &'static str {
    match state {
        pb::GuestActivationState::GUEST_ACTIVATION_STATE_RUNNING => "running",
        pb::GuestActivationState::GUEST_ACTIVATION_STATE_SUCCEEDED => "succeeded",
        pb::GuestActivationState::GUEST_ACTIVATION_STATE_FAILED => "failed",
        pb::GuestActivationState::GUEST_ACTIVATION_STATE_TIMED_OUT => "timed-out",
        pb::GuestActivationState::GUEST_ACTIVATION_STATE_LOST => "lost",
        _ => "failed",
    }
}

fn activation_state_from_name(value: &str) -> Option<pb::GuestActivationState> {
    match value {
        "running" => Some(pb::GuestActivationState::GUEST_ACTIVATION_STATE_RUNNING),
        "succeeded" => Some(pb::GuestActivationState::GUEST_ACTIVATION_STATE_SUCCEEDED),
        "failed" => Some(pb::GuestActivationState::GUEST_ACTIVATION_STATE_FAILED),
        "timed-out" => Some(pb::GuestActivationState::GUEST_ACTIVATION_STATE_TIMED_OUT),
        "lost" => Some(pb::GuestActivationState::GUEST_ACTIVATION_STATE_LOST),
        _ => None,
    }
}

fn shell_policy_runtime_usable(
    policy: &ShellPolicy,
    exec_paths_present: bool,
    exec_uid: Option<u32>,
    mut path_is_file: impl FnMut(&Path) -> bool,
    mut shpool_service_usable: impl FnMut(&Path) -> bool,
) -> bool {
    policy.enabled
        && exec_paths_present
        && matches!(exec_uid, Some(uid) if uid != 0)
        && is_valid_shell_name(&policy.default_name)
        && (1..=256).contains(&policy.max_sessions)
        && (1..=64).contains(&policy.max_attached)
        && policy.max_attached <= policy.max_sessions
        && policy
            .runner_path
            .as_ref()
            .is_some_and(|path| path_is_file(path))
        && policy
            .systemctl_path
            .as_ref()
            .is_some_and(|path| shpool_service_usable(path))
}

fn is_valid_shell_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 || value == "." || value == ".." {
        return false;
    }
    let first = bytes[0];
    (first.is_ascii_alphanumeric() || first == b'_')
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
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

fn generate_protocol_instance_id(prefix: &str) -> Result<String, GuestdServiceError> {
    let mut bytes = [0_u8; 16];
    let mut rng = OsNonceRng;
    rng.fill_bytes(&mut bytes)
        .map_err(|_| GuestdServiceError::TokenUnavailable)?;
    let mut out = String::with_capacity(prefix.len() + 1 + bytes.len() * 2);
    out.push_str(prefix);
    out.push('-');
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    Ok(out)
}

pub async fn run_single_connection<S>(
    stream: S,
    runtime: ServiceRuntime,
    context: AuthConnectionContext,
) -> Result<(), GuestdServiceError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
{
    let cleanup = ConnectionCleanup::new(
        Arc::clone(&runtime.auth),
        Arc::clone(&runtime.exec),
        Arc::clone(&runtime.shell),
        context.clone(),
    );
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let wrapped = CleanupStream::new(stream, cleanup.clone(), done_tx);
    let listener = ttrpc::r#async::transport::Listener::new(stream::once(async move {
        Ok::<_, std::io::Error>(wrapped)
    }));
    let service = Arc::new(
        GuestControlService::new(runtime.auth, runtime.exec, runtime.detached, context)
            .with_shell_runtime(runtime.shell)
            .with_activation_runtime(runtime.activation)
            .with_guest_config_path(runtime.guest_config_path)
            .with_usbip_path(runtime.usbip_path)
            .with_audio_runtime(runtime.audio),
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
    shell: SharedShell,
    activation: SharedActivation,
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
    // Host-declared absolute path to the guest `usbip` binary. `None` => the
    // USBIP import capability is absent and the RPC returns UsbipUnavailable.
    usbip_path: Option<PathBuf>,
    // Active audio runtime: wpctl path + workload-user UID. `None` => audio is
    // not configured or the wpctl binary was not found at startup; the
    // AudioStatus and AudioSet capabilities are not advertised and handlers
    // return AudioPipeWireUnavailable fail-closed.
    audio: Option<Arc<AudioRuntime>>,
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
            shell: Arc::new(ShellRuntime::disabled()),
            activation: None,
            context,
            write_stdin_handlers: Arc::new(AtomicU64::new(0)),
            write_stdin_bytes: Arc::new(AtomicU64::new(0)),
            guest_config_path: None,
            usbip_path: None,
            audio: None,
        }
    }

    /// Attach the host-declared guest config working-copy path used to serve
    /// `ReadGuestFile { GuestConfig }`.
    pub fn with_guest_config_path(mut self, path: Option<PathBuf>) -> Self {
        self.guest_config_path = path;
        self
    }

    /// Attach the host-declared guest `usbip` binary path used to serve
    /// `UsbipImport`.
    pub fn with_usbip_path(mut self, path: Option<PathBuf>) -> Self {
        self.usbip_path = path;
        self
    }

    /// Attach the active audio runtime. When `Some`, the AudioStatus/AudioSet
    /// capabilities are advertised and RPCs target the workload user's PipeWire
    /// session via wpctl argv-only subprocesses.
    pub(crate) fn with_audio_runtime(mut self, runtime: Option<Arc<AudioRuntime>>) -> Self {
        self.audio = runtime;
        self
    }

    pub fn with_shell_runtime(mut self, shell: SharedShell) -> Self {
        self.shell = shell;
        self
    }

    fn with_activation_runtime(mut self, activation: SharedActivation) -> Self {
        self.activation = activation;
        self
    }

    fn lock_auth(&self) -> Result<MutexGuard<'_, RuntimeAuthCore>, ttrpc::Error> {
        self.auth
            .lock()
            .map_err(|_| rpc_status(ttrpc::Code::INTERNAL, "guest-control-internal-error"))
    }

    /// Resolve a `ReadGuestFile` enum key to the host-declared path and read it
    /// with the fail-closed safe-open algorithm on a blocking worker. Returns the
    /// file bytes or a typed `GuestControlErrorKind`. Only `GuestConfig` is
    /// supported; any other (or `Unspecified`/unknown) key maps to `PathUnsafe`
    /// because it names no safe target.
    async fn read_guest_file_inner_async(
        &self,
        file_id: pb::GuestFileId,
    ) -> Result<Vec<u8>, pb::GuestControlErrorKind> {
        use pb::GuestControlErrorKind as K;
        let path = match file_id {
            pb::GuestFileId::GUEST_FILE_ID_GUEST_CONFIG => self
                .guest_config_path
                .clone()
                .ok_or(K::GUEST_CONTROL_ERROR_KIND_READ_DENIED)?,
            _ => return Err(K::GUEST_CONTROL_ERROR_KIND_PATH_UNSAFE),
        };
        tokio::task::spawn_blocking(move || read_guest_file_safely(&path))
            .await
            .map_err(|_| K::GUEST_CONTROL_ERROR_KIND_READ_DENIED)?
    }

    async fn usbip_import_inner(
        &self,
        action: pb::UsbipImportAction,
        host: &str,
        bus_id: &str,
    ) -> Result<u32, pb::GuestControlErrorKind> {
        use pb::GuestControlErrorKind as K;
        let path = self
            .usbip_path
            .as_deref()
            .ok_or(K::GUEST_CONTROL_ERROR_KIND_USBIP_UNAVAILABLE)?;
        validate_guest_usbip_request(host, bus_id)?;
        match action {
            pb::UsbipImportAction::USBIP_IMPORT_ACTION_DETACH => {
                detach_guest_usbip_ports(path, host, bus_id).await
            }
            pb::UsbipImportAction::USBIP_IMPORT_ACTION_ATTACH => {
                let detached = detach_guest_usbip_ports(path, host, bus_id).await?;
                run_guest_usbip_command(path, &["attach", "-r", host, "-b", bus_id]).await?;
                Ok(detached)
            }
            _ => Err(K::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR),
        }
    }

    async fn usbip_status_inner(
        &self,
        host: Option<&str>,
        bus_id: Option<&str>,
    ) -> Result<Vec<GuestUsbipImportStatus>, pb::GuestControlErrorKind> {
        use pb::GuestControlErrorKind as K;
        let path = self
            .usbip_path
            .as_deref()
            .ok_or(K::GUEST_CONTROL_ERROR_KIND_USBIP_UNAVAILABLE)?;
        if let Some(host) = host {
            validate_guest_usbip_host(host)?;
        }
        if let Some(bus_id) = bus_id {
            validate_guest_usbip_bus_id(bus_id)?;
        }
        let output = run_guest_usbip_command(path, &["port"]).await?;
        let stdout = std::str::from_utf8(&output.stdout)
            .map_err(|_| K::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_OUTPUT)?;
        parse_guest_usbip_status(stdout, host, bus_id)
    }

    async fn activation_start_inner(
        &self,
        activation_id: &str,
        switch_script_path: &Path,
        mode: pb::GuestActivationMode,
        timeout_ms: u64,
    ) -> Result<ActivationStatusSnapshot, ActivationError> {
        let activation = self
            .activation
            .as_ref()
            .ok_or(ActivationError::StatusUnavailable)?;
        activation
            .start(activation_id, switch_script_path, mode, timeout_ms)
            .await
    }

    async fn activation_status_inner(
        &self,
        activation_id: &str,
    ) -> Result<ActivationStatusSnapshot, ActivationError> {
        let activation = self
            .activation
            .as_ref()
            .ok_or(ActivationError::StatusUnavailable)?;
        activation.status(activation_id).await
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

    fn validate_shell_terminal_metadata<'a>(
        &self,
        metadata: Option<&'a pb::TerminalRequestMetadata>,
    ) -> Result<(&'a str, String), ttrpc::Error> {
        let metadata = metadata.ok_or_else(|| {
            rpc_status(
                ttrpc::Code::INVALID_ARGUMENT,
                "guest-control-metadata-invalid",
            )
        })?;
        self.validate_metadata(metadata.common.as_ref())?;
        if metadata.session_id.is_empty()
            || metadata.guest_boot_id.is_empty()
            || metadata
                .kind
                .enum_value()
                .ok()
                .filter(|kind| *kind == pb::TerminalKind::TERMINAL_KIND_SHELL)
                .is_none()
        {
            return Err(rpc_status(
                ttrpc::Code::INVALID_ARGUMENT,
                "guest-control-metadata-invalid",
            ));
        }
        let expected_boot = self.shell.guest_boot_id().ok_or_else(|| {
            rpc_status(
                ttrpc::Code::FAILED_PRECONDITION,
                "guest-control-shell-disabled",
            )
        })?;
        if metadata.guest_boot_id != expected_boot {
            return Err(rpc_status(
                ttrpc::Code::FAILED_PRECONDITION,
                "guest-control-shell-stale-session",
            ));
        }
        Ok((&metadata.session_id, expected_boot))
    }

    async fn ensure_shell_daemon_ready(&self) -> Result<(), pb::GuestControlError> {
        let systemctl = self
            .shell
            .systemctl_path()
            .ok_or_else(shell_disabled_error)?;
        if systemctl.as_os_str().is_empty() {
            return Ok(());
        }
        let status = TokioCommand::new(systemctl)
            .arg("start")
            .arg("d2b-shpool-daemon.service")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map_err(|_| {
                guest_error(
                    pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_SHELL_POOL_UNAVAILABLE,
                )
            })?;
        if !status.success() {
            return Err(guest_error(
                pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_SHELL_POOL_UNAVAILABLE,
            ));
        }
        let deadline =
            tokio::time::Instant::now() + Duration::from_millis(SHELL_DAEMON_READY_TIMEOUT_MS);
        loop {
            if self.run_shell_helper("list", None).await.is_ok() {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(guest_error(
                    pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_SHELL_POOL_UNAVAILABLE,
                ));
            }
            tokio::time::sleep(Duration::from_millis(SHELL_DAEMON_READY_POLL_MS)).await;
        }
    }

    fn shell_exec_id(&self, session_id: &str) -> Result<String, pb::GuestControlError> {
        self.shell
            .terminal_exec_id(session_id)
            .map_err(|error| match error {
                ShellRuntimeError::NotFound => {
                    guest_error(pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_SHELL_NOT_FOUND)
                }
                _ => shell_disabled_error(),
            })
    }

    async fn run_shell_management_helper(
        &self,
        subcommand: &str,
        name: &str,
    ) -> Result<(), pb::GuestControlError> {
        self.run_shell_helper(subcommand, Some(name)).await
    }

    async fn run_shell_helper(
        &self,
        subcommand: &str,
        name: Option<&str>,
    ) -> Result<(), pb::GuestControlError> {
        let runner = self.shell.runner_path().ok_or_else(shell_disabled_error)?;
        let socket = self.shell.socket_path().ok_or_else(shell_disabled_error)?;
        let guest_boot_id = self
            .shell
            .guest_boot_id()
            .ok_or_else(shell_disabled_error)?;
        if runner.as_os_str().is_empty() || socket.as_os_str().is_empty() {
            return Err(shell_disabled_error());
        }
        let mut argv = vec![
            runner.to_string_lossy().into_owned(),
            subcommand.to_owned(),
            "--socket".to_owned(),
            socket.to_string_lossy().into_owned(),
        ];
        if let Some(name) = name {
            argv.extend(["--name".to_owned(), name.to_owned()]);
        }
        argv.push("--json".to_owned());
        let input = ExecCreateInput {
            argv,
            user: None,
            cwd: None,
            env: Vec::new(),
            tty: false,
            stdin_open: false,
            detached: false,
            has_terminal_size: false,
            max_chunk_bytes: HARD_MAX_CHUNK_BYTES,
            direct_workload_tty: false,
        };
        let (exec_id, snapshot) = self
            .exec
            .create(self.connection_key(), guest_boot_id.clone(), input)
            .await
            .map_err(guest_error_kind)?;
        let mut known_generation = Some(snapshot.state_generation);
        for _ in 0..20 {
            let (snapshot, timed_out) = self
                .exec
                .wait(
                    &self.connection_key(),
                    &exec_id,
                    &guest_boot_id,
                    known_generation,
                    250,
                )
                .await
                .map_err(guest_error_kind)?;
            if !timed_out {
                match snapshot.outcome {
                    Some(ExitOutcome::Exited(0)) => return Ok(()),
                    Some(_) => {
                        return Err(guest_error(
                            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_SHELL_POOL_UNAVAILABLE,
                        ));
                    }
                    None if !matches!(snapshot.state, RtExecState::Running) => {
                        return Err(guest_error(
                            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_SHELL_POOL_UNAVAILABLE,
                        ));
                    }
                    None => known_generation = Some(snapshot.state_generation),
                }
            }
        }
        Err(guest_error(
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_SHELL_POOL_UNAVAILABLE,
        ))
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

    /// Query both audio channels from the workload user's PipeWire session.
    async fn audio_status_inner(
        &self,
    ) -> Result<(pb::GuestAudioChannelState, pb::GuestAudioChannelState), pb::GuestControlErrorKind>
    {
        let runtime = self.audio.as_deref().ok_or(
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_AUDIO_PIPEWIRE_UNAVAILABLE,
        )?;
        let microphone = query_wpctl_channel_state(
            &runtime.wpctl_path,
            "@DEFAULT_SOURCE@",
            runtime.workload_uid,
        )
        .await?;
        let speaker =
            query_wpctl_channel_state(&runtime.wpctl_path, "@DEFAULT_SINK@", runtime.workload_uid)
                .await?;
        Ok((microphone, speaker))
    }

    /// Mutate one audio channel in the workload user's PipeWire session via
    /// wpctl argv-only subprocesses, then query and return the updated state.
    async fn audio_set_inner(
        &self,
        channel: protobuf::EnumOrUnknown<pb::AudioChannel>,
        kind: protobuf::EnumOrUnknown<pb::AudioSetKind>,
        grant_on: bool,
        level: u32,
    ) -> Result<pb::GuestAudioChannelState, pb::GuestControlErrorKind> {
        use pb::GuestControlErrorKind as K;
        let runtime = self
            .audio
            .as_deref()
            .ok_or(K::GUEST_CONTROL_ERROR_KIND_AUDIO_PIPEWIRE_UNAVAILABLE)?;
        let target = wpctl_channel_target(channel)?;
        match kind.enum_value() {
            Ok(pb::AudioSetKind::AUDIO_SET_KIND_GRANT) => {
                let mute_arg = if grant_on { "0" } else { "1" };
                run_wpctl_command(
                    &runtime.wpctl_path,
                    &["set-mute", target, mute_arg],
                    runtime.workload_uid,
                )
                .await?;
            }
            Ok(pb::AudioSetKind::AUDIO_SET_KIND_LEVEL) => {
                if level > 100 {
                    return Err(K::GUEST_CONTROL_ERROR_KIND_AUDIO_LEVEL_OUT_OF_RANGE);
                }
                let vol = format!("{:.4}", level as f64 / 100.0);
                run_wpctl_command(
                    &runtime.wpctl_path,
                    &["set-volume", target, &vol],
                    runtime.workload_uid,
                )
                .await?;
            }
            _ => return Err(K::GUEST_CONTROL_ERROR_KIND_AUDIO_CHANNEL_UNKNOWN),
        }
        // Return the updated state after the mutation.
        query_wpctl_channel_state(&runtime.wpctl_path, target, runtime.workload_uid).await
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

fn activation_error_kind(error: ActivationError) -> pb::GuestControlErrorKind {
    match error {
        ActivationError::InvalidId => {
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_ACTIVATION_INVALID_ID
        }
        ActivationError::InvalidPath => {
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_ACTIVATION_INVALID_PATH
        }
        ActivationError::InvalidMode => {
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_ACTIVATION_INVALID_MODE
        }
        ActivationError::NotFound => {
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_ACTIVATION_NOT_FOUND
        }
        ActivationError::StatusUnavailable => {
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_ACTIVATION_STATUS_UNAVAILABLE
        }
        ActivationError::TimedOut => {
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_ACTIVATION_TIMED_OUT
        }
        ActivationError::SpawnFailed => {
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_ACTIVATION_SPAWN_FAILED
        }
    }
}

fn activation_error(error: ActivationError) -> pb::GuestControlError {
    guest_error(activation_error_kind(error))
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

fn shell_disabled_error() -> pb::GuestControlError {
    guest_error(pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_GUEST_SHELL_DISABLED)
}

fn terminal_metadata_common(
    metadata: Option<&pb::TerminalRequestMetadata>,
) -> Option<&pb::RequestMetadata> {
    metadata.and_then(|metadata| metadata.common.as_ref())
}

fn terminal_metadata_is_shell(metadata: Option<&pb::TerminalRequestMetadata>) -> bool {
    metadata
        .and_then(|metadata| metadata.kind.enum_value().ok())
        .is_some_and(|kind| kind == pb::TerminalKind::TERMINAL_KIND_SHELL)
}

fn terminal_write_disabled_response(
    metadata: Option<&pb::TerminalRequestMetadata>,
) -> pb::WriteStdinResponse {
    if terminal_metadata_is_shell(metadata) {
        let mut response = pb::WriteStdinResponse::new();
        response.stdin_state = EnumOrUnknown::new(pb::StdinState::STDIN_STATE_OPEN);
        response.disposition = EnumOrUnknown::new(pb::WriteDisposition::WRITE_DISPOSITION_REJECTED);
        response.error = MessageField::some(shell_disabled_error());
        response
    } else {
        write_stdin_error(ExecError::ExecDisabled)
    }
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
    limits.detached_stdout_log_bytes = d2b_exec_runner::DETACHED_STREAM_LOG_BYTES;
    limits.detached_stderr_log_bytes = d2b_exec_runner::DETACHED_STREAM_LOG_BYTES;
    limits.long_poll_timeout_ms = 100;
    limits.slow_consumer_grace_ms = 30_000;
    limits.exec_sessions_per_vm = crate::exec::EXEC_SESSIONS_PER_VM as u32;
    limits.attached_sessions_per_vm = crate::exec::ATTACHED_SESSIONS_PER_VM as u32;
    limits.pending_read_output_waits_per_stream =
        crate::exec::PENDING_READ_OUTPUT_WAITS_PER_STREAM as u32;
    limits.pending_exec_waits_per_vm = crate::exec::PENDING_EXEC_WAITS_PER_VM as u32;
    limits.rpc_rate_per_connection_per_second = 200;
    limits.rpc_rate_per_vm_burst = 1_000;
    limits.shell_sessions_per_vm = 8;
    limits.shell_attached_sessions_per_vm = 1;
    limits
}

fn effective_limits_for_config(config: &CapabilitiesConfig) -> pb::GuestEffectiveLimits {
    let mut limits = effective_limits();
    limits.shell_sessions_per_vm = config.shell_sessions_per_vm;
    limits.shell_attached_sessions_per_vm = config.shell_attached_sessions_per_vm;
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
        let outcome = self.read_guest_file_inner_async(file_id).await;

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

    async fn usbip_import(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::UsbipImportRequest,
    ) -> ttrpc::Result<pb::UsbipImportResponse> {
        // Auth is enforced BEFORE any command execution or guest USBIP state
        // inspection. A caller without the VM token learns nothing about imported
        // ports.
        self.require_authenticated()?;
        self.validate_metadata(request.metadata.as_ref())?;

        let action = request.action.enum_value_or_default();
        let mut response = pb::UsbipImportResponse::new();
        response.action = EnumOrUnknown::new(action);
        response.bus_id = request.bus_id.clone();
        match self
            .usbip_import_inner(action, &request.host, &request.bus_id)
            .await
        {
            Ok(detached_ports) => response.detached_ports = detached_ports,
            Err(kind) => response.error = MessageField::some(guest_error(kind)),
        }
        Ok(response)
    }

    async fn usbip_status(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::UsbipStatusRequest,
    ) -> ttrpc::Result<pb::UsbipStatusResponse> {
        self.require_authenticated()?;
        self.validate_metadata(request.metadata.as_ref())?;

        let mut response = pb::UsbipStatusResponse::new();
        match self
            .usbip_status_inner(request.host.as_deref(), request.bus_id.as_deref())
            .await
        {
            Ok(entries) => {
                response.imports = entries
                    .into_iter()
                    .map(|entry| {
                        let mut wire = pb::UsbipStatusEntry::new();
                        wire.port = u32::from(entry.port);
                        wire.host = entry.host;
                        wire.tcp_port = u32::from(entry.tcp_port);
                        wire.bus_id = entry.bus_id;
                        wire
                    })
                    .collect();
            }
            Err(kind) => response.error = MessageField::some(guest_error(kind)),
        }
        Ok(response)
    }

    async fn activate_system_start(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::GuestActivationStartRequest,
    ) -> ttrpc::Result<pb::GuestActivationStartResponse> {
        self.require_authenticated()?;
        self.validate_metadata(request.metadata.as_ref())?;

        let activation_id = request.activation_id.clone();
        let mode = request.mode.enum_value_or_default();
        let mut response = pb::GuestActivationStartResponse::new();
        response.activation_id = activation_id.clone();
        match self
            .activation_start_inner(
                &activation_id,
                Path::new(&request.switch_script_path),
                mode,
                request.timeout_ms,
            )
            .await
        {
            Ok(snapshot) => response.state = EnumOrUnknown::new(snapshot.state),
            Err(error) => {
                response.state =
                    EnumOrUnknown::new(pb::GuestActivationState::GUEST_ACTIVATION_STATE_FAILED);
                if error == ActivationError::TimedOut {
                    response.state = EnumOrUnknown::new(
                        pb::GuestActivationState::GUEST_ACTIVATION_STATE_TIMED_OUT,
                    );
                }
                response.error = MessageField::some(activation_error(error));
            }
        }
        Ok(response)
    }

    async fn activate_system_status(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::GuestActivationStatusRequest,
    ) -> ttrpc::Result<pb::GuestActivationStatusResponse> {
        self.require_authenticated()?;
        self.validate_metadata(request.metadata.as_ref())?;

        let mut response = pb::GuestActivationStatusResponse::new();
        response.activation_id = request.activation_id.clone();
        match self.activation_status_inner(&request.activation_id).await {
            Ok(snapshot) => {
                response.state = EnumOrUnknown::new(snapshot.state);
                response.exit_code = snapshot.exit_code;
                response.signal = snapshot.signal;
                response.status_code = snapshot.status_code;
                if snapshot.state == pb::GuestActivationState::GUEST_ACTIVATION_STATE_TIMED_OUT {
                    response.error =
                        MessageField::some(activation_error(ActivationError::TimedOut));
                } else if snapshot.state == pb::GuestActivationState::GUEST_ACTIVATION_STATE_LOST {
                    response.error =
                        MessageField::some(activation_error(ActivationError::StatusUnavailable));
                }
            }
            Err(error) => {
                response.state =
                    EnumOrUnknown::new(pb::GuestActivationState::GUEST_ACTIVATION_STATE_LOST);
                response.error = MessageField::some(activation_error(error));
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
            direct_workload_tty: false,
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
            let initial_size = request.initial_terminal_size.as_ref().and_then(|size| {
                (size.rows > 0 && size.cols > 0).then_some((size.rows, size.cols))
            });
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

    async fn shell_attach(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::ShellAttachRequest,
    ) -> ttrpc::Result<pb::ShellAttachResponse> {
        self.require_authenticated()?;
        self.validate_metadata(request.metadata.as_ref())?;
        if let Err(error) = self.ensure_shell_daemon_ready().await {
            let mut response = pb::ShellAttachResponse::new();
            response.state = EnumOrUnknown::new(pb::ShellState::SHELL_STATE_FEATURE_DISABLED);
            response.error = MessageField::some(error);
            return Ok(response);
        }
        let initial_size = request
            .initial_terminal_size
            .as_ref()
            .filter(|size| size.rows > 0 && size.cols > 0)
            .map(|size| (size.rows, size.cols))
            .or(Some((24, 80)));
        let force = request.force;
        let Some(runner) = self.shell.runner_path() else {
            let mut response = pb::ShellAttachResponse::new();
            response.error = MessageField::some(shell_disabled_error());
            return Ok(response);
        };
        let Some(socket) = self.shell.socket_path() else {
            let mut response = pb::ShellAttachResponse::new();
            response.error = MessageField::some(shell_disabled_error());
            return Ok(response);
        };
        let Some(guest_boot_id) = self.shell.guest_boot_id() else {
            let mut response = pb::ShellAttachResponse::new();
            response.error = MessageField::some(shell_disabled_error());
            return Ok(response);
        };
        if runner.as_os_str().is_empty() || socket.as_os_str().is_empty() {
            let mut response = pb::ShellAttachResponse::new();
            response.error = MessageField::some(shell_disabled_error());
            return Ok(response);
        }
        let requested_name = request.name.clone();
        let resolved_for_rollback = match self.shell.resolve_name(requested_name.clone()) {
            Ok(name) => name,
            Err(_) => return Ok(self.shell.attach_with_owner(request, self.connection_key())),
        };
        let existed_before = self.shell.session_exists(&resolved_for_rollback);
        let attached_before = self.shell.attached_snapshot();
        let mut response = self.shell.attach_with_owner(request, self.connection_key());
        if response.error.is_some() {
            return Ok(response);
        }
        let Some(session_id) = response.session_id.clone() else {
            response.error = MessageField::some(guest_error(
                pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR,
            ));
            return Ok(response);
        };
        let mut argv = vec![
            runner.to_string_lossy().into_owned(),
            "attach".to_owned(),
            "--socket".to_owned(),
            socket.to_string_lossy().into_owned(),
            "--name".to_owned(),
            response.resolved_name.clone(),
        ];
        if force {
            argv.push("--force".to_owned());
        }
        let input = ExecCreateInput {
            argv,
            user: None,
            cwd: None,
            env: Vec::new(),
            tty: true,
            stdin_open: true,
            detached: false,
            has_terminal_size: true,
            max_chunk_bytes: HARD_MAX_CHUNK_BYTES,
            direct_workload_tty: true,
        };
        match self
            .exec
            .create_tty(self.connection_key(), guest_boot_id, input, initial_size)
            .await
        {
            Ok((exec_id, snapshot, control_seq)) => {
                if let Err(error) = self.shell.set_terminal_exec_id(&session_id, exec_id) {
                    response.error = MessageField::some(match error {
                        ShellRuntimeError::NotFound => guest_error(
                            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_SHELL_NOT_FOUND,
                        ),
                        _ => shell_disabled_error(),
                    });
                    return Ok(response);
                }
                response.control_seq = control_seq;
                response.output_cursor = snapshot.stdout_start_offset;
                Ok(response)
            }
            Err(error) => {
                eprintln!("d2b-guestd: shell attach helper spawn failed: {error:?}");
                self.shell.restore_failed_attach(
                    &resolved_for_rollback,
                    &session_id,
                    existed_before,
                    &attached_before,
                );
                response.error = MessageField::some(guest_error_kind(error));
                Ok(response)
            }
        }
    }

    async fn shell_list(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::ShellListRequest,
    ) -> ttrpc::Result<pb::ShellListResponse> {
        self.require_authenticated()?;
        self.validate_metadata(request.metadata.as_ref())?;
        Ok(self.shell.list())
    }

    async fn shell_detach(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::ShellDetachRequest,
    ) -> ttrpc::Result<pb::ShellDetachResponse> {
        self.require_authenticated()?;
        self.validate_metadata(request.metadata.as_ref())?;
        let resolved = match self.shell.resolve_name(request.name.clone()) {
            Ok(name) => name,
            Err(_) => return Ok(self.shell.detach(request.name)),
        };
        if self.shell.session_attached(&resolved)
            && let Err(error) = self.run_shell_management_helper("detach", &resolved).await
        {
            let mut response = pb::ShellDetachResponse::new();
            response.resolved_name = resolved;
            response.error = MessageField::some(error);
            return Ok(response);
        }
        Ok(self.shell.detach(Some(resolved)))
    }

    async fn shell_kill(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::ShellKillRequest,
    ) -> ttrpc::Result<pb::ShellKillResponse> {
        self.require_authenticated()?;
        self.validate_metadata(request.metadata.as_ref())?;
        let name = request.name.clone();
        if self.shell.session_exists(&name)
            && let Err(error) = self.run_shell_management_helper("kill", &name).await
        {
            let mut response = self.shell.kill(name);
            response.error = MessageField::some(error);
            return Ok(response);
        }
        Ok(self.shell.kill(request.name))
    }

    async fn shell_close_attach(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::ShellCloseAttachRequest,
    ) -> ttrpc::Result<pb::ShellDetachResponse> {
        self.require_authenticated()?;
        self.validate_metadata(terminal_metadata_common(request.metadata.as_ref()))?;
        let session_id = request
            .metadata
            .as_ref()
            .map(|metadata| metadata.session_id.as_str())
            .unwrap_or("");
        self.exec.close_connection(&self.connection_key());
        Ok(self.shell.close_attach(session_id))
    }

    async fn terminal_write_stdin(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::TerminalWriteStdinRequest,
    ) -> ttrpc::Result<pb::WriteStdinResponse> {
        self.require_authenticated()?;
        if terminal_metadata_is_shell(request.metadata.as_ref()) {
            let (session_id, guest_boot_id) =
                self.validate_shell_terminal_metadata(request.metadata.as_ref())?;
            let exec_id = match self.shell_exec_id(session_id) {
                Ok(exec_id) => exec_id,
                Err(error) => {
                    let mut response = pb::WriteStdinResponse::new();
                    response.stdin_state = EnumOrUnknown::new(pb::StdinState::STDIN_STATE_OPEN);
                    response.disposition =
                        EnumOrUnknown::new(pb::WriteDisposition::WRITE_DISPOSITION_REJECTED);
                    response.error = MessageField::some(error);
                    return Ok(response);
                }
            };
            let len = request.data.len() as u64;
            let _budget = match self.acquire_write_stdin_slot(len) {
                Ok(guard) => guard,
                Err(error) => return Ok(write_stdin_error(error)),
            };
            return match self
                .exec
                .write_stdin(
                    &self.connection_key(),
                    &exec_id,
                    &guest_boot_id,
                    request.offset,
                    &request.data,
                    request.close_after,
                )
                .await
            {
                Ok(out) => {
                    let mut response = pb::WriteStdinResponse::new();
                    response.accepted_offset = request.offset;
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
            };
        }
        self.validate_metadata(terminal_metadata_common(request.metadata.as_ref()))?;
        Ok(terminal_write_disabled_response(request.metadata.as_ref()))
    }

    async fn terminal_read_output(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::TerminalReadOutputRequest,
    ) -> ttrpc::Result<pb::ReadOutputResponse> {
        self.require_authenticated()?;
        if terminal_metadata_is_shell(request.metadata.as_ref()) {
            let (session_id, guest_boot_id) =
                self.validate_shell_terminal_metadata(request.metadata.as_ref())?;
            let exec_id = match self.shell_exec_id(session_id) {
                Ok(exec_id) => exec_id,
                Err(error) => {
                    let mut response = pb::ReadOutputResponse::new();
                    response.stream = request.stream;
                    response.error = MessageField::some(error);
                    return Ok(response);
                }
            };
            let stream = match request.stream.enum_value() {
                Ok(stream) => rt_stream(stream)?,
                Err(_) => {
                    return Err(rpc_status(
                        ttrpc::Code::INVALID_ARGUMENT,
                        "guest-control-stream-invalid",
                    ));
                }
            };
            return match self
                .exec
                .read_output(
                    &self.connection_key(),
                    &exec_id,
                    &guest_boot_id,
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
            };
        }
        self.validate_metadata(terminal_metadata_common(request.metadata.as_ref()))?;
        let mut response = pb::ReadOutputResponse::new();
        response.stream = request.stream;
        response.error =
            MessageField::some(if terminal_metadata_is_shell(request.metadata.as_ref()) {
                shell_disabled_error()
            } else {
                guest_error_kind(ExecError::ExecDisabled)
            });
        Ok(response)
    }

    async fn terminal_close_stdin(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::TerminalCloseStdinRequest,
    ) -> ttrpc::Result<pb::CloseStdinResponse> {
        self.require_authenticated()?;
        if terminal_metadata_is_shell(request.metadata.as_ref()) {
            let (session_id, guest_boot_id) =
                self.validate_shell_terminal_metadata(request.metadata.as_ref())?;
            let exec_id = match self.shell_exec_id(session_id) {
                Ok(exec_id) => exec_id,
                Err(error) => {
                    let mut response = pb::CloseStdinResponse::new();
                    response.disposition =
                        EnumOrUnknown::new(pb::WriteDisposition::WRITE_DISPOSITION_REJECTED);
                    response.error = MessageField::some(error);
                    return Ok(response);
                }
            };
            return match self
                .exec
                .close_stdin(
                    &self.connection_key(),
                    &exec_id,
                    &guest_boot_id,
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
            };
        }
        self.validate_metadata(terminal_metadata_common(request.metadata.as_ref()))?;
        let mut response = pb::CloseStdinResponse::new();
        response.disposition = EnumOrUnknown::new(pb::WriteDisposition::WRITE_DISPOSITION_REJECTED);
        response.error =
            MessageField::some(if terminal_metadata_is_shell(request.metadata.as_ref()) {
                shell_disabled_error()
            } else {
                guest_error_kind(ExecError::ExecDisabled)
            });
        Ok(response)
    }

    async fn terminal_tty_win_resize(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::TerminalTtyWinResizeRequest,
    ) -> ttrpc::Result<pb::ControlAck> {
        self.require_authenticated()?;
        if terminal_metadata_is_shell(request.metadata.as_ref()) {
            let (session_id, guest_boot_id) =
                self.validate_shell_terminal_metadata(request.metadata.as_ref())?;
            let exec_id = match self.shell_exec_id(session_id) {
                Ok(exec_id) => exec_id,
                Err(error) => {
                    let mut response = pb::ControlAck::new();
                    response.control_seq = request.control_seq;
                    response.error = MessageField::some(error);
                    return Ok(response);
                }
            };
            let mut ack = pb::ControlAck::new();
            ack.control_seq = request.control_seq;
            if let Err(error) = self.exec.tty_resize(
                &self.connection_key(),
                &exec_id,
                &guest_boot_id,
                request.control_seq,
                request.rows,
                request.cols,
            ) {
                ack.error = MessageField::some(guest_error_kind(error));
            }
            return Ok(ack);
        }
        self.validate_metadata(terminal_metadata_common(request.metadata.as_ref()))?;
        let mut response = pb::ControlAck::new();
        response.control_seq = request.control_seq;
        response.error =
            MessageField::some(if terminal_metadata_is_shell(request.metadata.as_ref()) {
                shell_disabled_error()
            } else {
                guest_error_kind(ExecError::ExecDisabled)
            });
        Ok(response)
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

    async fn audio_status(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::AudioStatusRequest,
    ) -> ttrpc::Result<pb::AudioStatusResponse> {
        self.require_authenticated()?;
        self.validate_metadata(request.metadata.as_ref())?;

        let mut response = pb::AudioStatusResponse::new();
        match self.audio_status_inner().await {
            Ok((microphone, speaker)) => {
                response.microphone = MessageField::some(microphone);
                response.speaker = MessageField::some(speaker);
            }
            Err(kind) => {
                response.error = MessageField::some(guest_error(kind));
            }
        }
        Ok(response)
    }

    async fn audio_set(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: pb::AudioSetRequest,
    ) -> ttrpc::Result<pb::AudioSetResponse> {
        self.require_authenticated()?;
        self.validate_metadata(request.metadata.as_ref())?;

        let mut response = pb::AudioSetResponse::new();
        match self
            .audio_set_inner(
                request.channel,
                request.kind,
                request.grant_on,
                request.level,
            )
            .await
        {
            Ok(state) => {
                response.state = MessageField::some(state);
            }
            Err(kind) => {
                response.error = MessageField::some(guest_error(kind));
            }
        }
        Ok(response)
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuestUsbipImportStatus {
    port: u16,
    host: String,
    tcp_port: u16,
    bus_id: String,
}

fn validate_guest_usbip_host(host: &str) -> Result<(), pb::GuestControlErrorKind> {
    use pb::GuestControlErrorKind as K;
    host.parse::<IpAddr>()
        .map(|_| ())
        .map_err(|_| K::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_HOST)
}

fn validate_guest_usbip_bus_id(bus_id: &str) -> Result<(), pb::GuestControlErrorKind> {
    use pb::GuestControlErrorKind as K;
    d2b_contracts::usbip::validate_bus_id(bus_id)
        .map_err(|_| K::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_BUS_ID)
}

fn validate_guest_usbip_request(host: &str, bus_id: &str) -> Result<(), pb::GuestControlErrorKind> {
    validate_guest_usbip_host(host)?;
    validate_guest_usbip_bus_id(bus_id)
}

async fn run_guest_usbip_command(
    usbip_path: &Path,
    args: &[&str],
) -> Result<std::process::Output, pb::GuestControlErrorKind> {
    run_guest_usbip_command_with_timeout(
        usbip_path,
        args,
        Duration::from_millis(USBIP_COMMAND_TIMEOUT_MS),
    )
    .await
}

async fn run_guest_usbip_command_with_timeout(
    usbip_path: &Path,
    args: &[&str],
    timeout: Duration,
) -> Result<std::process::Output, pb::GuestControlErrorKind> {
    use pb::GuestControlErrorKind as K;
    let mut command = TokioCommand::new(usbip_path);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(timeout, command.output())
        .await
        .map_err(|_| K::GUEST_CONTROL_ERROR_KIND_USBIP_COMMAND_TIMEOUT)?
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => K::GUEST_CONTROL_ERROR_KIND_USBIP_UNAVAILABLE,
            _ => K::GUEST_CONTROL_ERROR_KIND_USBIP_COMMAND_FAILED,
        })?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(K::GUEST_CONTROL_ERROR_KIND_USBIP_COMMAND_FAILED)
    }
}

async fn detach_guest_usbip_ports(
    usbip_path: &Path,
    host: &str,
    bus_id: &str,
) -> Result<u32, pb::GuestControlErrorKind> {
    let output = run_guest_usbip_command(usbip_path, &["port"]).await?;
    let stdout = std::str::from_utf8(&output.stdout)
        .map_err(|_| pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_OUTPUT)?;
    let ports = guest_usbip_ports_for_bus_id(stdout, host, bus_id)?;
    for port in &ports {
        run_guest_usbip_command(usbip_path, &["detach", "-p", &port.to_string()]).await?;
    }
    Ok(ports.len().try_into().unwrap_or(u32::MAX))
}

fn guest_usbip_ports_for_bus_id(
    port_output: &str,
    host: &str,
    bus_id: &str,
) -> Result<Vec<u16>, pb::GuestControlErrorKind> {
    Ok(
        parse_guest_usbip_status(port_output, Some(host), Some(bus_id))?
            .into_iter()
            .map(|entry| entry.port)
            .collect(),
    )
}

fn parse_guest_usbip_status(
    port_output: &str,
    host_filter: Option<&str>,
    bus_id_filter: Option<&str>,
) -> Result<Vec<GuestUsbipImportStatus>, pb::GuestControlErrorKind> {
    use pb::GuestControlErrorKind as K;
    let mut current_port: Option<u16> = None;
    let mut imports = Vec::new();

    for line in port_output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed == "Imported USB devices"
            || trimmed.chars().all(|ch| ch == '=')
            || trimmed.starts_with("-> remote bus/dev ")
        {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("Port ") {
            let (port, after_colon) = rest
                .split_once(':')
                .ok_or(K::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_OUTPUT)?;
            let port = port
                .trim()
                .parse::<u16>()
                .map_err(|_| K::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_OUTPUT)?;
            if after_colon.trim().is_empty() {
                return Err(K::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_OUTPUT);
            }
            current_port = Some(port);
            continue;
        }
        if let Some((_, uri)) = trimmed.split_once(" -> ") {
            if let Some(entry) = parse_guest_usbip_uri(
                current_port.ok_or(K::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_OUTPUT)?,
                uri.trim(),
            )?
            .filter(|entry| {
                host_filter.is_none_or(|host| host == entry.host)
                    && bus_id_filter.is_none_or(|bus_id| bus_id == entry.bus_id)
            }) {
                imports.push(entry);
            }
            continue;
        }
        if current_port.is_some() {
            continue;
        }
        return Err(K::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_OUTPUT);
    }

    if imports.len() > USBIP_STATUS_MAX_IMPORTS {
        return Err(K::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_OUTPUT);
    }
    imports.sort_by_key(|entry| entry.port);
    imports.dedup_by_key(|entry| entry.port);
    Ok(imports)
}

fn parse_guest_usbip_uri(
    port: u16,
    uri: &str,
) -> Result<Option<GuestUsbipImportStatus>, pb::GuestControlErrorKind> {
    use pb::GuestControlErrorKind as K;
    let Some(rest) = uri.strip_prefix("usbip://") else {
        return Ok(None);
    };
    let (host_port, bus_id) = rest
        .rsplit_once('/')
        .ok_or(K::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_OUTPUT)?;
    let (host, tcp_port) = host_port
        .rsplit_once(':')
        .ok_or(K::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_OUTPUT)?;
    validate_guest_usbip_host(host)
        .map_err(|_| K::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_OUTPUT)?;
    validate_guest_usbip_bus_id(bus_id)
        .map_err(|_| K::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_OUTPUT)?;
    let tcp_port = tcp_port
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or(K::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_OUTPUT)?;
    Ok(Some(GuestUsbipImportStatus {
        port,
        host: host.to_owned(),
        tcp_port,
        bus_id: bus_id.to_owned(),
    }))
}

// ── wpctl audio helpers ──────────────────────────────────────────────────────

/// Map a protobuf `AudioChannel` enum to the wpctl target string.
fn wpctl_channel_target(
    channel: protobuf::EnumOrUnknown<pb::AudioChannel>,
) -> Result<&'static str, pb::GuestControlErrorKind> {
    match channel.enum_value() {
        Ok(pb::AudioChannel::AUDIO_CHANNEL_SPEAKER) => Ok("@DEFAULT_SINK@"),
        Ok(pb::AudioChannel::AUDIO_CHANNEL_MICROPHONE) => Ok("@DEFAULT_SOURCE@"),
        _ => Err(pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_AUDIO_CHANNEL_UNKNOWN),
    }
}

/// Run a wpctl subcommand as an argv-only subprocess in the workload user's
/// PipeWire session. `PIPEWIRE_RUNTIME_DIR=/run/user/<workload_uid>` is set so
/// wpctl connects to the user's daemon, never to root's socket.
async fn run_wpctl_command(
    wpctl_path: &Path,
    args: &[&str],
    workload_uid: u32,
) -> Result<std::process::Output, pb::GuestControlErrorKind> {
    use pb::GuestControlErrorKind as K;
    let pipewire_runtime_dir = format!("/run/user/{workload_uid}");
    let mut command = TokioCommand::new(wpctl_path);
    command
        .args(args)
        .env("PIPEWIRE_RUNTIME_DIR", &pipewire_runtime_dir)
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(
        Duration::from_millis(WPCTL_COMMAND_TIMEOUT_MS),
        command.output(),
    )
    .await
    .map_err(|_| K::GUEST_CONTROL_ERROR_KIND_AUDIO_PIPEWIRE_UNAVAILABLE)?
    .map_err(|_| K::GUEST_CONTROL_ERROR_KIND_AUDIO_PIPEWIRE_UNAVAILABLE)?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(K::GUEST_CONTROL_ERROR_KIND_AUDIO_PIPEWIRE_UNAVAILABLE)
    }
}

/// Query the current state of one wpctl target (`@DEFAULT_SINK@` or
/// `@DEFAULT_SOURCE@`) from the workload user's PipeWire session.
async fn query_wpctl_channel_state(
    wpctl_path: &Path,
    target: &str,
    workload_uid: u32,
) -> Result<pb::GuestAudioChannelState, pb::GuestControlErrorKind> {
    let output = run_wpctl_command(wpctl_path, &["get-volume", target], workload_uid).await?;
    let stdout = std::str::from_utf8(&output.stdout).map_err(|_| {
        pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_AUDIO_PIPEWIRE_UNAVAILABLE
    })?;
    parse_wpctl_get_volume_output(stdout)
}

/// Parse `wpctl get-volume` stdout.
///
/// Expected formats:
/// - `Volume: 0.50`
/// - `Volume: 0.50 [MUTED]`
///
/// Returns a protobuf `GuestAudioChannelState` with `muted`, `level` (0–100),
/// and `level_known = true`. Returns `AudioPipeWireUnavailable` on any parse
/// failure so the caller fails closed.
fn parse_wpctl_get_volume_output(
    output: &str,
) -> Result<pb::GuestAudioChannelState, pb::GuestControlErrorKind> {
    use pb::GuestControlErrorKind as K;
    let line = output.lines().next().unwrap_or("").trim();
    let rest = line
        .strip_prefix("Volume: ")
        .ok_or(K::GUEST_CONTROL_ERROR_KIND_AUDIO_PIPEWIRE_UNAVAILABLE)?;
    let muted = rest.contains("[MUTED]");
    let vol_str = rest
        .split_whitespace()
        .next()
        .ok_or(K::GUEST_CONTROL_ERROR_KIND_AUDIO_PIPEWIRE_UNAVAILABLE)?;
    let vol: f64 = vol_str
        .parse()
        .map_err(|_| K::GUEST_CONTROL_ERROR_KIND_AUDIO_PIPEWIRE_UNAVAILABLE)?;
    // Clamp to [0, 100] even if wpctl reports > 1.0 (boosted volume).
    let level = (vol * 100.0).round().clamp(0.0, 100.0) as u32;
    let mut state = pb::GuestAudioChannelState::new();
    state.muted = muted;
    state.level = level;
    state.level_known = true;
    Ok(state)
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
        K::GUEST_CONTROL_ERROR_KIND_USBIP_UNAVAILABLE => {
            (R::HEALTH_REMEDIATION_CHECK_GUESTD_SERVICE, None)
        }
        K::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_BUS_ID
        | K::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_HOST => (R::HEALTH_REMEDIATION_NONE, None),
        K::GUEST_CONTROL_ERROR_KIND_USBIP_COMMAND_FAILED
        | K::GUEST_CONTROL_ERROR_KIND_USBIP_COMMAND_TIMEOUT => (R::HEALTH_REMEDIATION_RETRY, None),
        K::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_OUTPUT => {
            (R::HEALTH_REMEDIATION_CHECK_GUESTD_SERVICE, None)
        }
        K::GUEST_CONTROL_ERROR_KIND_GUEST_SHELL_DISABLED
        | K::GUEST_CONTROL_ERROR_KIND_SHELL_POOL_UNAVAILABLE
        | K::GUEST_CONTROL_ERROR_KIND_SHELL_DAEMON_EPOCH_MISMATCH => {
            (R::HEALTH_REMEDIATION_CHECK_GUESTD_SERVICE, None)
        }
        K::GUEST_CONTROL_ERROR_KIND_SHELL_CAPACITY_EXCEEDED
        | K::GUEST_CONTROL_ERROR_KIND_SHELL_ATTACH_CAPACITY_EXCEEDED => {
            (R::HEALTH_REMEDIATION_REDUCE_LOAD, None)
        }
        K::GUEST_CONTROL_ERROR_KIND_SHELL_INVALID_NAME
        | K::GUEST_CONTROL_ERROR_KIND_SHELL_NOT_FOUND
        | K::GUEST_CONTROL_ERROR_KIND_SHELL_ALREADY_ATTACHED
        | K::GUEST_CONTROL_ERROR_KIND_SHELL_OUTPUT_GAP => (R::HEALTH_REMEDIATION_NONE, None),
        K::GUEST_CONTROL_ERROR_KIND_ACTIVATION_INVALID_ID
        | K::GUEST_CONTROL_ERROR_KIND_ACTIVATION_INVALID_PATH
        | K::GUEST_CONTROL_ERROR_KIND_ACTIVATION_INVALID_MODE
        | K::GUEST_CONTROL_ERROR_KIND_ACTIVATION_NOT_FOUND => (R::HEALTH_REMEDIATION_NONE, None),
        K::GUEST_CONTROL_ERROR_KIND_ACTIVATION_STATUS_UNAVAILABLE
        | K::GUEST_CONTROL_ERROR_KIND_ACTIVATION_SPAWN_FAILED => {
            (R::HEALTH_REMEDIATION_CHECK_GUESTD_SERVICE, None)
        }
        K::GUEST_CONTROL_ERROR_KIND_ACTIVATION_TIMED_OUT => (R::HEALTH_REMEDIATION_RETRY, None),
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
        direct_workload_tty: false,
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
    shell: SharedShell,
    context: AuthConnectionContext,
    closed: Arc<AtomicBool>,
}

impl ConnectionCleanup {
    fn new(
        auth: SharedAuthCore,
        exec: SharedExec,
        shell: SharedShell,
        context: AuthConnectionContext,
    ) -> Self {
        Self {
            auth,
            exec,
            shell,
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
            self.shell
                .close_connection(&self.context.connection_instance);
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
        let limits = effective_limits_for_config(&config);

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
        if config.usbip_import {
            capabilities.capabilities.push(EnumOrUnknown::new(
                pb::GuestCapability::GUEST_CAPABILITY_USBIP_IMPORT,
            ));
        }
        if config.usbip_status {
            capabilities.capabilities.push(EnumOrUnknown::new(
                pb::GuestCapability::GUEST_CAPABILITY_USBIP_STATUS,
            ));
        }
        if config.system_activation {
            capabilities.capabilities.push(EnumOrUnknown::new(
                pb::GuestCapability::GUEST_CAPABILITY_SYSTEM_ACTIVATION,
            ));
        }
        if config.audio_status {
            capabilities.capabilities.push(EnumOrUnknown::new(
                pb::GuestCapability::GUEST_CAPABILITY_AUDIO_STATUS,
            ));
        }
        if config.audio_set {
            capabilities.capabilities.push(EnumOrUnknown::new(
                pb::GuestCapability::GUEST_CAPABILITY_AUDIO_SET,
            ));
        }
        if config.shell_attached {
            capabilities.capabilities.push(EnumOrUnknown::new(
                pb::GuestCapability::GUEST_CAPABILITY_SHELL_ATTACHED,
            ));
        }
        if config.shell_management {
            capabilities.capabilities.push(EnumOrUnknown::new(
                pb::GuestCapability::GUEST_CAPABILITY_SHELL_MANAGEMENT,
            ));
        }
        if config.shell_force_attach {
            capabilities.capabilities.push(EnumOrUnknown::new(
                pb::GuestCapability::GUEST_CAPABILITY_SHELL_FORCE_ATTACH,
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

    fn terminal_metadata(kind: pb::TerminalKind) -> pb::TerminalRequestMetadata {
        let mut metadata = pb::TerminalRequestMetadata::new();
        metadata.kind = EnumOrUnknown::new(kind);
        metadata
    }

    #[test]
    fn terminal_disabled_response_preserves_terminal_kind_boundary() {
        let shell = terminal_write_disabled_response(Some(&terminal_metadata(
            pb::TerminalKind::TERMINAL_KIND_SHELL,
        )));
        assert_eq!(
            shell
                .error
                .as_ref()
                .expect("shell error")
                .kind
                .enum_value()
                .expect("known shell kind"),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_GUEST_SHELL_DISABLED
        );

        let exec = terminal_write_disabled_response(Some(&terminal_metadata(
            pb::TerminalKind::TERMINAL_KIND_EXEC,
        )));
        assert_eq!(
            exec.error
                .as_ref()
                .expect("exec error")
                .kind
                .enum_value()
                .expect("known exec kind"),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_GUEST_EXEC_DISABLED
        );
    }

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
            direct_workload_tty: false,
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
            exec_runner_path: PathBuf::from("/run/current-system/sw/bin/d2b-exec-runner"),
            max_runtime_sec: 0,
        })
    }

    fn startup_config_with_shell(user: &str) -> GuestdServeConfig {
        startup_config(user).with_shell_policy(ShellPolicy {
            enabled: true,
            default_name: "default".to_owned(),
            max_sessions: 12,
            max_attached: 2,
            runner_path: Some(PathBuf::from(
                "/run/current-system/sw/bin/d2b-guest-shell-runner",
            )),
            systemctl_path: Some(PathBuf::from("/run/current-system/sw/bin/systemctl")),
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
                        for usbip_import in [false, true] {
                            for system_activation in [false, true] {
                                let cfg = derive_capabilities_config(
                                    exec_paths_present,
                                    exec_detached,
                                    exec_tty,
                                    read_guest_file,
                                    usbip_import,
                                    system_activation,
                                    None,
                                    false,
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
                                assert_eq!(cfg.usbip_import, usbip_import);
                                assert_eq!(cfg.usbip_status, usbip_import);
                                assert_eq!(cfg.system_activation, system_activation);
                                assert!(!cfg.shell_attached);
                                assert_eq!(cfg.shell_sessions_per_vm, 0);
                                assert_eq!(cfg.shell_attached_sessions_per_vm, 0);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn audio_caps_not_advertised_without_runtime() {
        // Without a usable audio runtime (audio_usable=false), AudioStatus and
        // AudioSet must not be advertised — handlers would return
        // AudioPipeWireUnavailable fail-closed which would cause d2bd to
        // incorrectly report HostAndGuest enforcement.
        let cfg = derive_capabilities_config(
            true,  // exec_paths_present
            false, // exec_detached
            false, // exec_tty
            false, // read_guest_file
            false, // usbip_import
            false, // system_activation
            None,  // shell_limits
            false, // audio_usable
        );
        assert!(
            !cfg.audio_status,
            "audio_status must not be advertised without a usable audio runtime"
        );
        assert!(
            !cfg.audio_set,
            "audio_set must not be advertised without a usable audio runtime"
        );
    }

    #[test]
    fn audio_caps_advertised_with_runtime() {
        // When audio_usable=true (wpctl binary present + workload user resolved),
        // both AudioStatus and AudioSet must be advertised.
        let cfg = derive_capabilities_config(
            true,  // exec_paths_present
            false, // exec_detached
            false, // exec_tty
            false, // read_guest_file
            false, // usbip_import
            false, // system_activation
            None,  // shell_limits
            true,  // audio_usable
        );
        assert!(
            cfg.audio_status,
            "audio_status must be advertised when audio runtime is usable"
        );
        assert!(
            cfg.audio_set,
            "audio_set must be advertised when audio runtime is usable"
        );
    }

    #[test]
    fn parse_wpctl_volume_unmuted() {
        let state = parse_wpctl_get_volume_output("Volume: 0.50\n").unwrap();
        assert!(!state.muted);
        assert_eq!(state.level, 50);
        assert!(state.level_known);
    }

    #[test]
    fn parse_wpctl_volume_muted() {
        let state = parse_wpctl_get_volume_output("Volume: 0.75 [MUTED]\n").unwrap();
        assert!(state.muted);
        assert_eq!(state.level, 75);
        assert!(state.level_known);
    }

    #[test]
    fn parse_wpctl_volume_zero() {
        let state = parse_wpctl_get_volume_output("Volume: 0.00\n").unwrap();
        assert!(!state.muted);
        assert_eq!(state.level, 0);
    }

    #[test]
    fn parse_wpctl_volume_full() {
        let state = parse_wpctl_get_volume_output("Volume: 1.00\n").unwrap();
        assert!(!state.muted);
        assert_eq!(state.level, 100);
    }

    #[test]
    fn parse_wpctl_volume_boosted_clamped_to_100() {
        // wpctl can report > 1.0 for boosted volumes; clamp to 100.
        let state = parse_wpctl_get_volume_output("Volume: 1.50\n").unwrap();
        assert_eq!(state.level, 100);
    }

    #[test]
    fn parse_wpctl_volume_malformed_returns_unavailable() {
        use pb::GuestControlErrorKind as K;
        let err = parse_wpctl_get_volume_output("unexpected output\n").unwrap_err();
        assert_eq!(
            err,
            K::GUEST_CONTROL_ERROR_KIND_AUDIO_PIPEWIRE_UNAVAILABLE,
            "malformed wpctl output must return AudioPipeWireUnavailable"
        );
    }

    #[test]
    fn parse_wpctl_volume_empty_returns_unavailable() {
        use pb::GuestControlErrorKind as K;
        let err = parse_wpctl_get_volume_output("").unwrap_err();
        assert_eq!(err, K::GUEST_CONTROL_ERROR_KIND_AUDIO_PIPEWIRE_UNAVAILABLE);
    }

    #[test]
    fn wpctl_channel_target_speaker() {
        let target = wpctl_channel_target(protobuf::EnumOrUnknown::new(
            pb::AudioChannel::AUDIO_CHANNEL_SPEAKER,
        ))
        .unwrap();
        assert_eq!(target, "@DEFAULT_SINK@");
    }

    #[test]
    fn wpctl_channel_target_microphone() {
        let target = wpctl_channel_target(protobuf::EnumOrUnknown::new(
            pb::AudioChannel::AUDIO_CHANNEL_MICROPHONE,
        ))
        .unwrap();
        assert_eq!(target, "@DEFAULT_SOURCE@");
    }

    #[test]
    fn wpctl_channel_target_unspecified_returns_unknown() {
        use pb::GuestControlErrorKind as K;
        let err = wpctl_channel_target(protobuf::EnumOrUnknown::new(
            pb::AudioChannel::AUDIO_CHANNEL_UNSPECIFIED,
        ))
        .unwrap_err();
        assert_eq!(err, K::GUEST_CONTROL_ERROR_KIND_AUDIO_CHANNEL_UNKNOWN);
    }

    #[tokio::test]
    async fn audio_status_unavailable_without_runtime() {
        // When no audio runtime is configured, audio_status_inner must return
        // AudioPipeWireUnavailable — never a success-shaped response.
        use pb::GuestControlErrorKind as K;
        // test_service has audio=None by default (no with_audio_runtime call).
        let service = test_service(220);
        let err = service.audio_status_inner().await.unwrap_err();
        assert_eq!(
            err,
            K::GUEST_CONTROL_ERROR_KIND_AUDIO_PIPEWIRE_UNAVAILABLE,
            "audio_status_inner must return AudioPipeWireUnavailable without runtime"
        );
    }

    #[tokio::test]
    async fn audio_set_unavailable_without_runtime() {
        // When no audio runtime is configured, audio_set_inner must return
        // AudioPipeWireUnavailable — never a success-shaped response.
        use pb::GuestControlErrorKind as K;
        let service = test_service(221);
        let err = service
            .audio_set_inner(
                protobuf::EnumOrUnknown::new(pb::AudioChannel::AUDIO_CHANNEL_SPEAKER),
                protobuf::EnumOrUnknown::new(pb::AudioSetKind::AUDIO_SET_KIND_GRANT),
                true,
                0,
            )
            .await
            .unwrap_err();
        assert_eq!(
            err,
            K::GUEST_CONTROL_ERROR_KIND_AUDIO_PIPEWIRE_UNAVAILABLE,
            "audio_set_inner must return AudioPipeWireUnavailable without runtime"
        );
    }

    #[tokio::test]
    async fn audio_set_level_out_of_range() {
        // Level > 100 must return AudioLevelOutOfRange before any wpctl call.
        use pb::GuestControlErrorKind as K;
        // Provide a fake wpctl path (non-existent) to ensure the level guard
        // fires before the subprocess attempt.
        let audio_rt = Arc::new(AudioRuntime {
            wpctl_path: PathBuf::from("/nix/store/fake-wpctl/bin/wpctl"),
            workload_uid: 1000,
        });
        let service = test_service(222).with_audio_runtime(Some(audio_rt));
        let err = service
            .audio_set_inner(
                protobuf::EnumOrUnknown::new(pb::AudioChannel::AUDIO_CHANNEL_SPEAKER),
                protobuf::EnumOrUnknown::new(pb::AudioSetKind::AUDIO_SET_KIND_LEVEL),
                false,
                101, // out of range
            )
            .await
            .unwrap_err();
        assert_eq!(
            err,
            K::GUEST_CONTROL_ERROR_KIND_AUDIO_LEVEL_OUT_OF_RANGE,
            "level 101 must return AudioLevelOutOfRange"
        );
    }

    #[test]
    fn shell_capability_gate_requires_exec_runtime_service_and_valid_limits() {
        let valid = ShellPolicy {
            enabled: true,
            default_name: "default".to_owned(),
            max_sessions: 8,
            max_attached: 1,
            runner_path: Some(PathBuf::from("/nix/store/runner")),
            systemctl_path: Some(PathBuf::from("/nix/store/systemctl")),
        };
        assert!(shell_policy_runtime_usable(
            &valid,
            true,
            Some(1000),
            |_| true,
            |_| true
        ));
        let invalid_limits = ShellPolicy {
            max_sessions: 1,
            max_attached: 2,
            ..valid.clone()
        };
        assert!(!shell_policy_runtime_usable(
            &invalid_limits,
            true,
            Some(1000),
            |_| true,
            |_| true
        ));
        assert!(!shell_policy_runtime_usable(
            &valid,
            false,
            Some(1000),
            |_| true,
            |_| true
        ));
        assert!(!shell_policy_runtime_usable(
            &valid,
            true,
            Some(0),
            |_| true,
            |_| true
        ));
        assert!(!shell_policy_runtime_usable(
            &valid,
            true,
            Some(1000),
            |_| true,
            |_| false
        ));
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

    #[tokio::test]
    async fn service_startup_advertises_shell_only_after_full_runtime_probe() {
        let runtime = prepare_service_runtime_with_probe(
            startup_config_with_shell("alice"),
            &TestStartupProbe {
                uid: crate::login_session::WorkloadUserUid::NonRoot(1000),
            },
        )
        .await
        .unwrap();
        let service = GuestControlService::new(
            runtime.auth,
            runtime.exec,
            runtime.detached,
            test_context(33),
        );
        authenticate(&service).await;
        let caps = service
            .capabilities(&ttrpc_context(), capabilities_request())
            .await
            .unwrap();
        let cap_values = caps
            .capabilities
            .iter()
            .map(|cap| cap.enum_value().unwrap())
            .collect::<Vec<_>>();
        assert!(cap_values.contains(&pb::GuestCapability::GUEST_CAPABILITY_SHELL_ATTACHED));
        assert_eq!(caps.limits.shell_sessions_per_vm, 12);
        assert_eq!(caps.limits.shell_attached_sessions_per_vm, 2);

        struct NoShpoolProbe;
        impl StartupProbe for NoShpoolProbe {
            fn classify_workload_user(&self, _user: &str) -> crate::login_session::WorkloadUserUid {
                crate::login_session::WorkloadUserUid::NonRoot(1000)
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

            fn shpool_service_usable(&self, _systemctl_path: &Path) -> bool {
                false
            }
        }

        let runtime =
            prepare_service_runtime_with_probe(startup_config_with_shell("alice"), &NoShpoolProbe)
                .await
                .unwrap();
        let service = GuestControlService::new(
            runtime.auth,
            runtime.exec,
            runtime.detached,
            test_context(34),
        );
        authenticate(&service).await;
        let caps = service
            .capabilities(&ttrpc_context(), capabilities_request())
            .await
            .unwrap();
        let cap_values = caps
            .capabilities
            .iter()
            .map(|cap| cap.enum_value().unwrap())
            .collect::<Vec<_>>();
        assert!(!cap_values.contains(&pb::GuestCapability::GUEST_CAPABILITY_SHELL_ATTACHED));
        assert_eq!(caps.limits.shell_sessions_per_vm, 0);
        assert_eq!(caps.limits.shell_attached_sessions_per_vm, 0);
    }

    #[tokio::test]
    async fn service_startup_advertises_usbip_status_with_import_capability() {
        let runtime = prepare_service_runtime_with_probe(
            startup_config("alice")
                .with_usbip_path(PathBuf::from("/run/current-system/sw/bin/usbip")),
            &TestStartupProbe {
                uid: crate::login_session::WorkloadUserUid::NonRoot(1000),
            },
        )
        .await
        .unwrap();
        let service = GuestControlService::new(
            runtime.auth,
            runtime.exec,
            runtime.detached,
            test_context(35),
        );
        authenticate(&service).await;
        let caps = service
            .capabilities(&ttrpc_context(), capabilities_request())
            .await
            .unwrap();
        let cap_values = caps
            .capabilities
            .iter()
            .map(|cap| cap.enum_value().unwrap())
            .collect::<Vec<_>>();
        assert!(cap_values.contains(&pb::GuestCapability::GUEST_CAPABILITY_USBIP_IMPORT));
        assert!(cap_values.contains(&pb::GuestCapability::GUEST_CAPABILITY_USBIP_STATUS));
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
        assert_unauthenticated(
            service
                .usbip_status(&ctx, pb::UsbipStatusRequest::new())
                .await,
        );
        assert_unauthenticated(
            service
                .activate_system_start(&ctx, pb::GuestActivationStartRequest::new())
                .await,
        );
        assert_unauthenticated(
            service
                .activate_system_status(&ctx, pb::GuestActivationStatusRequest::new())
                .await,
        );
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
            .join(format!("d2b-guestd-cred-test-{}", std::process::id()));
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
        assert!(!error.public_message().contains("d2b-guestd-cred-test"));
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
    use d2b_exec_runner::DETACHED_RETAINED_PER_VM;
    use d2b_exec_runner::filering::{FileRingError, RingChunk, StreamMeta};
    use d2b_exec_runner::paths::Stream as RunnerStream;
    use d2b_exec_runner::record::{DurableRecord, StatusPhase};
    use d2b_exec_runner::spec::ExecSpec;
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
            direct_workload_tty: false,
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
                paths: RunnerUnitPaths::new("/run/current-system/sw/bin/d2b-exec-runner"),
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
        // Keep scratch under the worktree; this session forbids /tmp writes.
        let base = std::env::current_dir()
            .unwrap()
            .join(".scratch-guestd-tests");
        let _ = std::fs::create_dir_all(&base);
        let dir = base.join(format!(
            "guestd-rgf-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }

    fn service_with_config(instance: u8, path: Option<PathBuf>) -> GuestControlService {
        GuestControlService::new(test_auth(), test_exec(), None, test_context(instance))
            .with_guest_config_path(path)
    }

    fn service_with_usbip(instance: u8, path: Option<PathBuf>) -> GuestControlService {
        GuestControlService::new(test_auth(), test_exec(), None, test_context(instance))
            .with_usbip_path(path)
    }

    type ActivationStartLog = Arc<Mutex<Vec<(String, String)>>>;

    #[derive(Clone)]
    struct FakeActivationUnits {
        query: Arc<Mutex<Option<ActivationStatusSnapshot>>>,
        starts: ActivationStartLog,
        fail_start: Option<ActivationError>,
    }

    impl FakeActivationUnits {
        fn with_query(query: Option<ActivationStatusSnapshot>) -> Self {
            Self {
                query: Arc::new(Mutex::new(query)),
                starts: Arc::new(Mutex::new(Vec::new())),
                fail_start: None,
            }
        }
    }

    #[async_trait]
    impl ActivationUnitManager for FakeActivationUnits {
        async fn start_unit(
            &self,
            unit_name: &str,
            _switch_script_path: &Path,
            mode_arg: &str,
            _timeout_ms: u64,
        ) -> Result<(), ActivationError> {
            self.starts
                .lock()
                .unwrap()
                .push((unit_name.to_owned(), mode_arg.to_owned()));
            if let Some(error) = self.fail_start {
                Err(error)
            } else {
                Ok(())
            }
        }

        async fn query_unit(
            &self,
            _unit_name: &str,
        ) -> Result<Option<ActivationStatusSnapshot>, ActivationError> {
            Ok(self.query.lock().unwrap().clone())
        }

        async fn cleanup_terminal_unit(
            &self,
            _unit_name: &str,
            _snapshot: &ActivationStatusSnapshot,
        ) -> Result<(), ActivationError> {
            Ok(())
        }
    }

    fn activation_runtime(
        dir: PathBuf,
        units: FakeActivationUnits,
    ) -> (Arc<ActivationRuntime>, ActivationStartLog) {
        let starts = Arc::clone(&units.starts);
        (
            Arc::new(ActivationRuntime::new(
                Arc::new(units),
                dir,
                ACTIVATION_MAX_TIMEOUT_MS,
            )),
            starts,
        )
    }

    fn service_with_activation(
        instance: u8,
        activation: Option<Arc<ActivationRuntime>>,
    ) -> GuestControlService {
        GuestControlService::new(test_auth(), test_exec(), None, test_context(instance))
            .with_activation_runtime(activation)
    }

    fn valid_activation_id() -> &'static str {
        "01234567-89ab-4def-8123-456789abcdef"
    }

    fn activation_start_request() -> pb::GuestActivationStartRequest {
        let mut request = pb::GuestActivationStartRequest::new();
        request.metadata = metadata();
        request.activation_id = valid_activation_id().to_owned();
        request.switch_script_path =
            "/nix/store/0123456789abcdfghijklmnpqrsvwxyz-nixos-system/bin/switch-to-configuration"
                .to_owned();
        request.mode = EnumOrUnknown::new(pb::GuestActivationMode::GUEST_ACTIVATION_MODE_SWITCH);
        request.timeout_ms = ACTIVATION_DEFAULT_TIMEOUT_MS;
        request
    }

    fn activation_status_request(id: &str) -> pb::GuestActivationStatusRequest {
        let mut request = pb::GuestActivationStatusRequest::new();
        request.metadata = metadata();
        request.activation_id = id.to_owned();
        request
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

    fn usbip_request(
        action: pb::UsbipImportAction,
        host: &str,
        bus_id: &str,
    ) -> pb::UsbipImportRequest {
        let mut request = pb::UsbipImportRequest::new();
        request.metadata = metadata();
        request.action = EnumOrUnknown::new(action);
        request.host = host.to_owned();
        request.bus_id = bus_id.to_owned();
        request
    }

    fn usbip_status_request(host: Option<&str>, bus_id: Option<&str>) -> pb::UsbipStatusRequest {
        let mut request = pb::UsbipStatusRequest::new();
        request.metadata = metadata();
        request.host = host.map(str::to_owned);
        request.bus_id = bus_id.map(str::to_owned);
        request
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
    async fn activation_handlers_require_authentication_before_validation() {
        let dir = scratch_dir("activation-auth");
        let (runtime, _starts) =
            activation_runtime(dir.clone(), FakeActivationUnits::with_query(None));
        let service = service_with_activation(70, Some(runtime));
        let ctx = ttrpc_context();
        assert_unauthenticated(
            service
                .activate_system_start(&ctx, pb::GuestActivationStartRequest::new())
                .await,
        );
        assert_unauthenticated(
            service
                .activate_system_status(&ctx, pb::GuestActivationStatusRequest::new())
                .await,
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn activation_rejects_invalid_id_path_and_mode_without_starting_unit() {
        let dir = scratch_dir("activation-invalid");
        let (runtime, starts) =
            activation_runtime(dir.clone(), FakeActivationUnits::with_query(None));
        let service = service_with_activation(71, Some(runtime));
        authenticate(&service).await;
        let ctx = ttrpc_context();

        let mut bad_id = activation_start_request();
        bad_id.activation_id = "../bad".to_owned();
        let response = service.activate_system_start(&ctx, bad_id).await.unwrap();
        assert_eq!(
            response.error.unwrap().kind.enum_value().unwrap(),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_ACTIVATION_INVALID_ID
        );

        let mut bad_path = activation_start_request();
        bad_path.switch_script_path = "/run/current-system/bin/switch-to-configuration".to_owned();
        let response = service.activate_system_start(&ctx, bad_path).await.unwrap();
        assert_eq!(
            response.error.unwrap().kind.enum_value().unwrap(),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_ACTIVATION_INVALID_PATH
        );

        assert_eq!(
            activation_error_kind(
                activation_mode_arg(pb::GuestActivationMode::GUEST_ACTIVATION_MODE_UNSPECIFIED,)
                    .unwrap_err()
            ),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_ACTIVATION_INVALID_MODE
        );
        assert!(starts.lock().unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn activation_status_rejoins_persisted_terminal_record() {
        let dir = scratch_dir("activation-rejoin");
        let (_runtime, _starts) =
            activation_runtime(dir.clone(), FakeActivationUnits::with_query(None));
        let done = ActivationStatusSnapshot {
            state: pb::GuestActivationState::GUEST_ACTIVATION_STATE_SUCCEEDED,
            exit_code: Some(0),
            signal: None,
            status_code: None,
        };
        write_activation_record_blocking(&dir, valid_activation_id(), &done).unwrap();

        let (runtime_after_restart, _starts) =
            activation_runtime(dir.clone(), FakeActivationUnits::with_query(None));
        let service = service_with_activation(72, Some(runtime_after_restart));
        authenticate(&service).await;
        let response = service
            .activate_system_status(
                &ttrpc_context(),
                activation_status_request(valid_activation_id()),
            )
            .await
            .unwrap();
        assert_eq!(
            response.state.enum_value().unwrap(),
            pb::GuestActivationState::GUEST_ACTIVATION_STATE_SUCCEEDED
        );
        assert_eq!(response.exit_code, Some(0));
        assert!(response.error.is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn activation_status_marks_stale_running_record_lost_when_unit_is_gone() {
        let dir = scratch_dir("activation-lost");
        let (runtime, _starts) =
            activation_runtime(dir.clone(), FakeActivationUnits::with_query(None));
        write_activation_record_blocking(
            &dir,
            valid_activation_id(),
            &ActivationStatusSnapshot::running(),
        )
        .unwrap();
        let service = service_with_activation(73, Some(runtime));
        authenticate(&service).await;
        let response = service
            .activate_system_status(
                &ttrpc_context(),
                activation_status_request(valid_activation_id()),
            )
            .await
            .unwrap();
        assert_eq!(
            response.state.enum_value().unwrap(),
            pb::GuestActivationState::GUEST_ACTIVATION_STATE_LOST
        );
        let persisted = read_activation_record_blocking(&dir, valid_activation_id()).unwrap();
        assert_eq!(
            persisted.state,
            pb::GuestActivationState::GUEST_ACTIVATION_STATE_LOST
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn usbip_import_requires_authentication() {
        let service =
            service_with_usbip(60, Some(PathBuf::from("/run/current-system/sw/bin/usbip")));
        let ctx = ttrpc_context();
        assert_unauthenticated(
            service
                .usbip_import(
                    &ctx,
                    usbip_request(
                        pb::UsbipImportAction::USBIP_IMPORT_ACTION_DETACH,
                        "192.168.100.1",
                        "1-2.1",
                    ),
                )
                .await,
        );
    }

    #[tokio::test]
    async fn usbip_status_requires_authentication() {
        let service =
            service_with_usbip(66, Some(PathBuf::from("/run/current-system/sw/bin/usbip")));
        let ctx = ttrpc_context();
        assert_unauthenticated(
            service
                .usbip_status(
                    &ctx,
                    usbip_status_request(Some("192.168.100.1"), Some("1-2.1")),
                )
                .await,
        );
    }

    #[tokio::test]
    async fn usbip_import_without_configured_path_is_unavailable() {
        let service = service_with_usbip(61, None);
        authenticate(&service).await;
        let ctx = ttrpc_context();
        let response = service
            .usbip_import(
                &ctx,
                usbip_request(
                    pb::UsbipImportAction::USBIP_IMPORT_ACTION_DETACH,
                    "192.168.100.1",
                    "1-2.1",
                ),
            )
            .await
            .unwrap();
        let error = response.error.as_ref().expect("usbip error set");
        assert_eq!(
            error.kind.enum_value().unwrap(),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_USBIP_UNAVAILABLE
        );
    }

    #[tokio::test]
    async fn usbip_status_without_configured_path_is_unavailable() {
        let service = service_with_usbip(65, None);
        authenticate(&service).await;
        let ctx = ttrpc_context();
        let response = service
            .usbip_status(
                &ctx,
                usbip_status_request(Some("192.168.100.1"), Some("1-2.1")),
            )
            .await
            .unwrap();
        let error = response.error.as_ref().expect("usbip error set");
        assert_eq!(
            error.kind.enum_value().unwrap(),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_USBIP_UNAVAILABLE
        );
        assert!(response.imports.is_empty());
    }

    #[tokio::test]
    async fn usbip_import_rejects_invalid_busid_before_command() {
        let service = service_with_usbip(62, Some(PathBuf::from("/does/not/need/to/exist")));
        authenticate(&service).await;
        let ctx = ttrpc_context();
        let response = service
            .usbip_import(
                &ctx,
                usbip_request(
                    pb::UsbipImportAction::USBIP_IMPORT_ACTION_DETACH,
                    "192.168.100.1",
                    "../bad",
                ),
            )
            .await
            .unwrap();
        let error = response.error.as_ref().expect("usbip error set");
        assert_eq!(
            error.kind.enum_value().unwrap(),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_BUS_ID
        );
    }

    #[tokio::test]
    async fn usbip_import_rejects_invalid_host_before_command() {
        let service = service_with_usbip(63, Some(PathBuf::from("/does/not/need/to/exist")));
        authenticate(&service).await;
        let ctx = ttrpc_context();
        let response = service
            .usbip_import(
                &ctx,
                usbip_request(
                    pb::UsbipImportAction::USBIP_IMPORT_ACTION_DETACH,
                    "not-an-ip",
                    "1-2.1",
                ),
            )
            .await
            .unwrap();
        let error = response.error.as_ref().expect("usbip error set");
        assert_eq!(
            error.kind.enum_value().unwrap(),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_HOST
        );
    }

    #[tokio::test]
    async fn usbip_command_spawn_permission_denied_is_command_failed() {
        let dir = scratch_dir("usbip-perm-denied");
        let path = dir.join("usbip");
        std::fs::write(&path, b"#!/bin/sh\nexit 0\n").unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&path, perms).unwrap();
        let err = run_guest_usbip_command(&path, &["port"])
            .await
            .expect_err("non-executable file must fail");
        assert_eq!(
            err,
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_USBIP_COMMAND_FAILED
        );
        let _ = std::fs::remove_dir_all(&dir);
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
            (encoded.len() as u64) < d2b_contracts::guest_wire::TTRPC_FRAME_CAP_BYTES,
            "encoded cap response {} must fit ttRPC frame cap {}",
            encoded.len(),
            d2b_contracts::guest_wire::TTRPC_FRAME_CAP_BYTES
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
    fn usbip_import_capability_advertised_only_when_usbip_configured() {
        let advertised = |caps: &[EnumOrUnknown<pb::GuestCapability>]| {
            caps.iter().any(|c| {
                c.enum_value().unwrap() == pb::GuestCapability::GUEST_CAPABILITY_USBIP_IMPORT
            })
        };
        let with = RuntimeCapabilitiesProvider::new(CapabilitiesConfig {
            usbip_import: true,
            ..CapabilitiesConfig::default()
        })
        .snapshot()
        .unwrap();
        assert!(advertised(&with.capabilities.capabilities));
        assert!(advertised(&with.health.capabilities));

        let without = RuntimeCapabilitiesProvider::new(CapabilitiesConfig::default())
            .snapshot()
            .unwrap();
        assert!(!advertised(&without.capabilities.capabilities));
        assert!(!advertised(&without.health.capabilities));
    }

    #[test]
    fn shell_capabilities_are_advertised_only_when_runtime_is_usable() {
        let advertised = |caps: &[EnumOrUnknown<pb::GuestCapability>], cap: pb::GuestCapability| {
            caps.iter().any(|c| c.enum_value().unwrap() == cap)
        };
        let shell_config = CapabilitiesConfig {
            shell_attached: true,
            shell_management: true,
            shell_force_attach: true,
            shell_sessions_per_vm: 12,
            shell_attached_sessions_per_vm: 2,
            ..CapabilitiesConfig::default()
        };
        let with = RuntimeCapabilitiesProvider::new(shell_config)
            .snapshot()
            .unwrap();
        for cap in [
            pb::GuestCapability::GUEST_CAPABILITY_SHELL_ATTACHED,
            pb::GuestCapability::GUEST_CAPABILITY_SHELL_MANAGEMENT,
            pb::GuestCapability::GUEST_CAPABILITY_SHELL_FORCE_ATTACH,
        ] {
            assert!(advertised(&with.capabilities.capabilities, cap));
            assert!(advertised(&with.health.capabilities, cap));
        }
        assert_eq!(with.capabilities.limits.shell_sessions_per_vm, 12);
        assert_eq!(with.capabilities.limits.shell_attached_sessions_per_vm, 2);
        assert!(
            constellation_shell_capability_set(&shell_config)
                .has(d2b_constellation_core::Capability::PersistentShell)
        );

        let without = RuntimeCapabilitiesProvider::new(CapabilitiesConfig::default())
            .snapshot()
            .unwrap();
        assert!(!advertised(
            &without.capabilities.capabilities,
            pb::GuestCapability::GUEST_CAPABILITY_SHELL_ATTACHED
        ));
        assert_eq!(without.capabilities.limits.shell_sessions_per_vm, 0);
        assert_eq!(
            without.capabilities.limits.shell_attached_sessions_per_vm,
            0
        );
        assert!(
            !constellation_shell_capability_set(&CapabilitiesConfig {
                shell_attached: true,
                shell_management: true,
                shell_force_attach: false,
                shell_sessions_per_vm: 12,
                shell_attached_sessions_per_vm: 2,
                ..CapabilitiesConfig::default()
            })
            .has(d2b_constellation_core::Capability::PersistentShell)
        );
    }

    #[tokio::test]
    async fn shell_attach_requires_configured_runtime_paths() {
        let runtime = Arc::new(ShellRuntime::enabled(ShellRuntimeConfig {
            default_name: "default".to_owned(),
            max_sessions: 2,
            max_attached: 1,
            workload_user: Some("alice".to_owned()),
            workload_uid: Some(1000),
            guest_boot_id: "boot-1".to_owned(),
            guestd_instance_id: "guestd-1".to_owned(),
            daemon_instance_id: "daemon-1".to_owned(),
            runner_path: PathBuf::new(),
            systemctl_path: PathBuf::new(),
            socket_path: PathBuf::new(),
        }));
        let service = test_service(61).with_shell_runtime(runtime);
        authenticate(&service).await;
        let ctx = ttrpc_context();

        let mut attach = pb::ShellAttachRequest::new();
        attach.metadata = metadata();
        let attached = service.shell_attach(&ctx, attach).await.unwrap();
        assert_shell_disabled(attached.error);

        let mut list = pb::ShellListRequest::new();
        list.metadata = metadata();
        let listed = service.shell_list(&ctx, list).await.unwrap();
        assert!(listed.error.is_none());
        assert_eq!(listed.default_name, "default");
        assert!(listed.sessions.is_empty());
    }

    #[tokio::test]
    async fn shell_terminal_rpc_is_not_disabled_when_runtime_enabled() {
        let runtime = Arc::new(ShellRuntime::enabled(ShellRuntimeConfig {
            default_name: "default".to_owned(),
            max_sessions: 2,
            max_attached: 1,
            workload_user: Some("alice".to_owned()),
            workload_uid: Some(1000),
            guest_boot_id: "boot-1".to_owned(),
            guestd_instance_id: "guestd-1".to_owned(),
            daemon_instance_id: "daemon-1".to_owned(),
            runner_path: PathBuf::new(),
            systemctl_path: PathBuf::new(),
            socket_path: PathBuf::new(),
        }));
        let attached = runtime.attach(pb::ShellAttachRequest::new());
        let session_id = attached.session_id.expect("session id");
        runtime
            .set_terminal_exec_id(&session_id, "missing-exec".to_owned())
            .expect("session exists");
        let service = test_service(64).with_shell_runtime(runtime);
        authenticate(&service).await;
        let ctx = ttrpc_context();

        let mut term = pb::TerminalRequestMetadata::new();
        term.common = metadata();
        term.session_id = session_id;
        term.guest_boot_id = "boot-1".to_owned();
        term.kind = EnumOrUnknown::new(pb::TerminalKind::TERMINAL_KIND_SHELL);
        let mut write = pb::TerminalWriteStdinRequest::new();
        write.metadata = MessageField::some(term);
        write.data = b"echo ready\n".to_vec();

        let response = service.terminal_write_stdin(&ctx, write).await.unwrap();
        let error = response.error.as_ref().expect("terminal error");
        assert_eq!(
            error.kind.enum_value().unwrap(),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_EXEC_NOT_FOUND
        );
    }

    #[tokio::test]
    async fn shell_rpcs_fail_closed_when_runtime_disabled() {
        let service = test_service(62);
        authenticate(&service).await;
        let ctx = ttrpc_context();

        let mut attach = pb::ShellAttachRequest::new();
        attach.metadata = metadata();
        assert_shell_disabled(service.shell_attach(&ctx, attach).await.unwrap().error);

        let mut list = pb::ShellListRequest::new();
        list.metadata = metadata();
        assert_shell_disabled(service.shell_list(&ctx, list).await.unwrap().error);

        let mut detach = pb::ShellDetachRequest::new();
        detach.metadata = metadata();
        assert_shell_disabled(service.shell_detach(&ctx, detach).await.unwrap().error);

        let mut kill = pb::ShellKillRequest::new();
        kill.metadata = metadata();
        kill.name = "default".to_owned();
        assert_shell_disabled(service.shell_kill(&ctx, kill).await.unwrap().error);

        let mut close = pb::ShellCloseAttachRequest::new();
        let mut term = pb::TerminalRequestMetadata::new();
        term.common = metadata();
        term.session_id = "shell-1".to_owned();
        term.guest_boot_id = "boot-1".to_owned();
        term.kind = EnumOrUnknown::new(pb::TerminalKind::TERMINAL_KIND_SHELL);
        close.metadata = MessageField::some(term);
        assert_shell_disabled(service.shell_close_attach(&ctx, close).await.unwrap().error);
    }

    #[tokio::test]
    async fn connection_cleanup_releases_shell_attachment() {
        let runtime = Arc::new(ShellRuntime::enabled(ShellRuntimeConfig {
            default_name: "default".to_owned(),
            max_sessions: 2,
            max_attached: 1,
            workload_user: Some("alice".to_owned()),
            workload_uid: Some(1000),
            guest_boot_id: "boot-1".to_owned(),
            guestd_instance_id: "guestd-1".to_owned(),
            daemon_instance_id: "daemon-1".to_owned(),
            runner_path: PathBuf::new(),
            systemctl_path: PathBuf::new(),
            socket_path: PathBuf::new(),
        }));
        let service = test_service(63).with_shell_runtime(Arc::clone(&runtime));
        authenticate(&service).await;
        let _response = runtime.attach_with_owner(
            pb::ShellAttachRequest::new(),
            [63; CONNECTION_INSTANCE_LEN].to_vec(),
        );

        let cleanup = ConnectionCleanup::new(test_auth(), test_exec(), runtime, test_context(63));
        cleanup.close();
        let listed = service.shell.list();
        assert_eq!(listed.sessions.len(), 1);
        assert!(!listed.sessions[0].attached);
    }

    fn assert_shell_disabled(error: MessageField<pb::GuestControlError>) {
        let error = error.as_ref().expect("shell error");
        assert_eq!(
            error.kind.enum_value().expect("known error"),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_GUEST_SHELL_DISABLED
        );
        assert_eq!(
            error.remediation.enum_value().expect("known remediation"),
            pb::HealthRemediation::HEALTH_REMEDIATION_CHECK_GUESTD_SERVICE
        );
    }

    #[test]
    fn guest_usbip_port_parser_matches_host_and_busid() {
        let output = r#"Imported USB devices
====================
Port 00: <Port in Use> at Full Speed(12Mbps)
       QinHeng Electronics : unknown product (1a86:fe0c)
       1-1 -> usbip://192.168.100.1:3240/1-2.2
           -> remote bus/dev 001/020
Port 01: <Port in Use> at High Speed(480Mbps)
       unknown vendor : unknown product (345f:2109)
       1-2 -> usbip://192.168.100.1:3240/1-2.1
           -> remote bus/dev 001/019
"#;
        assert_eq!(
            guest_usbip_ports_for_bus_id(output, "192.168.100.1", "1-2.1").unwrap(),
            vec![1]
        );
        assert_eq!(
            guest_usbip_ports_for_bus_id(output, "192.168.100.1", "1-2.2").unwrap(),
            vec![0]
        );
        assert!(
            guest_usbip_ports_for_bus_id(output, "192.168.100.2", "1-2.1")
                .unwrap()
                .is_empty()
        );
        let all = parse_guest_usbip_status(output, None, None).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].host, "192.168.100.1");
        assert_eq!(all[0].tcp_port, 3240);
        assert_eq!(all[0].bus_id, "1-2.2");
    }

    #[test]
    fn guest_usbip_status_parser_rejects_invalid_output() {
        for output in [
            "Port 00: <Port in Use>\n       1-1 -> usbip://not-an-ip:3240/1-2\n",
            "Port xx: <Port in Use>\n       1-1 -> usbip://192.168.100.1:3240/1-2\n",
            "Port 00: <Port in Use>\n       1-1 -> usbip://192.168.100.1:0/1-2\n",
            "Port 00: <Port in Use>\n       1-1 -> usbip://192.168.100.1:3240/1-/../../x\n",
            "unexpected header\n",
        ] {
            assert_eq!(
                parse_guest_usbip_status(output, None, None),
                Err(pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_OUTPUT),
                "{output:?}"
            );
        }
    }

    #[tokio::test]
    async fn guest_usbip_command_timeout_maps_to_closed_kind() {
        let shell = Path::new("/bin/sh");
        if !shell.exists() {
            return;
        }
        let result = run_guest_usbip_command_with_timeout(
            shell,
            &["-c", "sleep 1"],
            Duration::from_millis(1),
        )
        .await;
        assert!(matches!(
            result,
            Err(pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_USBIP_COMMAND_TIMEOUT)
        ));
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
