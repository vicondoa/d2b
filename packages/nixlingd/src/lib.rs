// TypedError is a large enum used throughout this crate as the canonical
// Result Err type. Boxing it would require pervasive API changes across
// hundreds of call sites; the size trade-off is intentional and tracked
// in plan.md §D-typed-error-boxing. Suppressed until that refactor lands.
#![allow(clippy::result_large_err)]

use std::collections::{BTreeMap, BTreeSet, HashMap, hash_map::Entry};
use std::fs::{self, File, OpenOptions};
use std::future::Future;
use std::io::{IoSliceMut, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::os::fd::{AsFd, AsRawFd, OwnedFd, RawFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use nix::cmsg_space;
use nix::fcntl::{FcntlArg, FdFlag, Flock, FlockArg, fcntl};
use nix::sys::socket::{
    AddressFamily, ControlMessageOwned, MsgFlags, SockFlag, SockType, UnixAddr, connect,
    getsockopt, recv, recvmsg, send, socket, sockopt::PeerCredentials,
};
use nix::unistd::{self, Gid, Group, Uid, User};
use nixling_constellation_core::TargetName;
use nixling_constellation_provider::provider::WorkloadProvider;
use nixling_core::bundle::Bundle;
use nixling_core::bundle_resolver::{
    BundleResolver, intent_id_activation, intent_id_gc_host, intent_id_hosts_host,
    intent_id_installer_host, intent_id_keys_rotate, intent_id_migrate_host, intent_id_nft_host,
    intent_id_nm_unmanaged_host, intent_id_rotate_known_host, intent_id_route_env,
    intent_id_runner, intent_id_sysctl, intent_id_trust, intent_id_usbip_firewall,
};
use nixling_core::closures::ClosureMetadata;
use nixling_core::error::BundleError;
use nixling_core::host::{HostJson, Ipv6SysctlEntry, QemuMediaSourceIntent};
use nixling_core::host_check;
use nixling_core::manifest_v04::{ManifestV04, VmEntry as ManifestVmEntry};
use nixling_core::processes::{ProcessNode, ProcessRole, ProcessesJson, ReadinessPredicate};
use nixling_gateway::{
    AgentHandle, AgentSpawnRequest, AppCommand, Clock, ContextSeed, DisplayListener,
    DisplaySessionContext, GatewayDeps, GatewayError, GatewayOrchestrator, GatewayWorkload,
    IdSource, LedgerLimits, ListenerHandle, NoopGatewayAudit, OpenSession, SECRET_LEN,
    SessionBinding, SessionSecret, TargetKey,
};
use nixling_gateway_runtime::{
    AcaGatewayWorkload, AgentBinaries, CredentialFilePolicy, GatewayCredential, RelayCoords,
    RelayDisplayListener, SealingKey, production_deps, relay_sas_token_snippet, system_now_fn,
    system_now_unix,
};
use nixling_host::ssh_keygen;
use nixling_ipc::{
    BROKER_SOCKET_PATH, KnownFeatureFlag,
    broker_wire::{
        ActivationMode as BrokerActivationMode, ApplyNftablesRequest as BrokerApplyNftablesRequest,
        ApplyNmUnmanagedRequest as BrokerApplyNmUnmanagedRequest,
        ApplyRouteRequest as BrokerApplyRouteRequest,
        ApplySysctlRequest as BrokerApplySysctlRequest, BrokerCallerRole, BrokerRequest,
        BrokerRequestEnvelope, BrokerResponse, DeregisterRunnerPidfdRequest,
        OpenPidfdRequest as BrokerOpenPidfdRequest,
        QemuMediaBootRequest as BrokerQemuMediaBootRequest,
        QemuMediaHotplugRequest as BrokerQemuMediaHotplugRequest,
        QemuMediaRefreshRegistryRequest as BrokerQemuMediaRefreshRegistryRequest,
        RunActivationRequest as BrokerRunActivationRequest, RunGcRequest as BrokerRunGcRequest,
        RunHostInstallRequest as BrokerRunHostInstallRequest,
        RunHostKeyTrustRequest as BrokerRunHostKeyTrustRequest,
        RunKeysRotateRequest as BrokerRunKeysRotateRequest,
        RunMigrateRequest as BrokerRunMigrateRequest,
        RunRotateKnownHostRequest as BrokerRunRotateKnownHostRequest, RunnerRole, RunnerSignal,
        SignalRunnerRequest, SpawnRunnerRequest as BrokerSpawnRunnerRequest,
        StoreVerifyRequest as BrokerStoreVerifyRequest,
        UpdateHostsFileRequest as BrokerUpdateHostsFileRequest,
        UsbipBindFirewallRuleRequest as BrokerUsbipBindFirewallRuleRequest,
        UsbipBindRequest as BrokerUsbipBindRequest,
        UsbipProxyReconcileRequest as BrokerUsbipProxyReconcileRequest,
        UsbipUnbindRequest as BrokerUsbipUnbindRequest,
    },
    guest_proto as pb,
    public_wire::{self, AuthRole, AuthStatusResponse, DeniedCommandHint, SocketReachability},
    types::{BundleClosureRef, BundleOpId, MediaRef, RoleId, ScopeId, VmId},
};
use nixling_provider_aca::{
    AcaConfig, AcaDiskImageSource, AcaSandboxDefaults, AcaWorkloadProvider,
};
use nixling_provider_relay::{LocalTarget, RelayEndpoint};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use socket2::{Domain, SockAddr, Socket, Type};
use supervisor::pidfd_table::{
    BrokerReapLog, PidfdEntry, PidfdRegistration, PidfdTable, PidfdTableError, WaitTermination,
};
use uzers::{get_user_by_uid, get_user_groups};

pub mod exec_detached;
pub mod exec_session;
pub mod exec_session_real;
pub mod guest_control_bridge;
pub mod guest_control_health;
pub mod guest_control_vsock;
pub mod supervisor;
pub mod terminal_session;
pub mod typed_error;
pub mod wire;
// `[pending restart]` machinery. Pure module + filesystem reader trait
// so the CLI can compute the daemon-level pending-restart signal
// post-restart without requiring /run live.
pub mod daemon_version;
// Per-VM state-directory ownership preflight invoked from
// `dispatch_broker_vm_start`. The pure enforcer lives in
// `nixling_host::ownership_matrix`.
pub mod ownership_preflight;
// Daemon-side replacement for the retired
// `nixling-known-hosts-refresh@<vm>.service` oneshot. Invoked from
// `dispatch_broker_vm_start` after the per-VM DAG
// reports `overall_ok` (i.e. the VM's readiness signal has
// fired). See `known_hosts_refresh` for the pure intent builder
// + side-effect wrapper.
pub mod known_hosts_refresh;
// Per-VM sshd host key posture preflight invoked from
// `dispatch_broker_vm_start` and from the host-prep DAG executor.
// The pure check lives in
// `crate::ssh_host_key_preflight`.
pub mod ssh_host_key_preflight;
// Refuses to start a `sys-<env>-net` VM when the on-disk dnsmasq.conf
// for that env diverges from the
// bundle's nft/route/hosts intent hash. Catches the case where the
// bundle was updated but the dnsmasq render step did not rerun.
// See `docs/reference/net-vm-bundle-gate.md`.
pub mod net_vm_bundle_gate;
// Daemon startup self-check that verifies the kernel-module matrix the
// running config requires is loaded; refuses to start on missing required
// modules and marks VMs as degraded on optional misses. See
// `docs/reference/kernel-module-check.md`.
pub mod kernel_module_check;
// Typed readiness gate that blocks `dispatch_broker_vm_start` from
// declaring the observability VM successful until the broker-spawned
// OtelHostBridge runner has registered its pidfd AND opened its
// obs vsock host socket. On timeout the daemon falls back to
// degraded mode (VM is up; observability annotated as broken).
// See `docs/reference/otel-host-bridge-readiness.md`.
pub mod otel_host_bridge_readiness;
// Daemon startup self-check that replaces the legacy
// `nixling-net-route-preflight.service` host singleton (retired in v1.0).
// Probes each env's LAN bridge, persists a small history, and engages an
// operator-only mode after N consecutive failures. Recovery is via the new
// `nixling host reconcile --network --apply` verb. See
// `docs/explanation/host-prepare.md`.
pub mod net_route_preflight;
// v1.1.1 runtime pidfs self-probe: hard-refuses daemon startup on
// kernels without pidfs (CONFIG_FS_PID stripped or kernel < 6.9).
// Defense-in-depth alongside the static `tests/v1.1-kernel-floor-eval.sh`
// gate. Per ADR 0008 + ADR 0018.
pub mod pidfs_probe;
// ADR 0034 startup contract check for generated storage/restart/sync artifacts.
pub mod storage_lifecycle;
// Contract for bringing autostart VMs up on daemon startup (net VMs
// first, concurrency cap, degraded-mode tolerant, idempotent). See
// docs/reference/daemon-autostart.md.
pub mod autostart;
// Daemon-side per-env usbipd autostart. Folds the transitional
// `nixling-sys-<env>-usbipd-{backend,proxy}.{service,socket}` units into
// broker `SpawnRunner` with `RunnerRole::Usbip`, keyed per-env on
// `vm_id = sys-<env>-usbipd` with role_ids `backend` / `proxy`.
pub mod usbipd_perenv_autostart;
// Prometheus scrape endpoint shape. Owns the canonical metric inventory (see
// `docs/reference/daemon-metrics.md`) and a minimal HTTP/1.1
// `GET /metrics` handler. The registry is process-local; serving is
// wired through the daemon's public socket accept loop.
pub mod metrics;
// Per-VM Cloud Hypervisor stats scraper folded into the daemon's
// `/metrics` endpoint. Replaces the host-side `nixling-ch-exporter.service`
// singleton (retired in v1.0). See `docs/reference/daemon-metrics.md` for
// the metric inventory.
pub mod ch_stats;
// In-daemon replacement for the
// `nixling-audit-check.{service,timer}` host singleton + timer that
// previously sanity-checked broker audit log shape on a daily cadence.
// Exposes `GET /health/audit-check` on the daemon's HTTP surface and
// a pure check function suitable for invocation from the supervisor
// event loop. See `docs/reference/daemon-audit-check.md`.
pub mod audit_check;
// Typed, per-busid USBIP state machine that pins the canonical bring-up
// order
// `modprobe → lock → withhold → firewall → backend → bind → proxy`
// (AGENTS.md "Critical subsystems"). Each step is a typed broker
// op or daemon-side action; failures are fail-fast and surface as
// `TypedError::UsbipStepFailed { busid, step, reason }`
// (exit code 67). See `docs/reference/usbip-state-machine.md`.
pub mod usbip_state_machine;
// Daemon-side JSONL audit events for transitions not covered by the
// broker's OpAuditRecord stream (e.g. api-ready timeout).
pub mod daemon_audit;

/// Accept-loop concurrency primitives: bounded in-flight admission
/// semaphore + per-VM / global op locks.
pub mod concurrency;

// ADR 0032: compile-only peer-module skeletons wiring the v2
// constellation provider/router trait surface. NOT called from the running
// daemon (zero behavior change); see the module docs.
pub mod constellation_stubs;

use typed_error::TypedError;

pub const DEFAULT_CONFIG_PATH: &str = "/etc/nixling/daemon-config.json";
pub const DEFAULT_GATEWAY_CONFIG_PATH: &str = "/etc/nixling/gateway.json";
pub const DEFAULT_SERVER_VERSION: &str = "0.4.0";
pub const DEFAULT_ACCEPTED_VERSION_RANGE: &str = ">=0.4.0, <0.5.0";
pub const DEFAULT_DAEMON_STATE_DIR: &str = "/var/lib/nixling/daemon-state";
const VM_RUNNER_ROLE_ID: &str = "ch-runner";
const VM_STOP_TIMEOUT: Duration = Duration::from_secs(30);
const GATEWAY_DISPLAY_SESSION_TTL: Duration = Duration::from_secs(3600);

/// Default cap on concurrent in-flight connection-handler threads.
/// Overridable at startup via `NIXLINGD_MAX_INFLIGHT_CONNECTIONS`.
const DEFAULT_MAX_INFLIGHT_CONNECTIONS: usize = 64;
/// Write deadline for the typed refusal frame (authz reject / busy) the
/// accept loop sends before closing — never block the accept loop on a
/// slow/abusive peer.
const ACCEPT_REFUSAL_WRITE_DEADLINE: Duration = Duration::from_secs(2);
/// Read deadline for the initial hello frame, so a connected-but-silent
/// peer cannot occupy a handler slot indefinitely.
const HELLO_READ_DEADLINE: Duration = Duration::from_secs(10);
/// Read deadline for each subsequent request frame on a persistent
/// connection. A timeout closes the connection gracefully and frees the
/// handler slot. Cleared before an exec handoff (the exec owner blocks
/// on the PTY indefinitely).
const REQUEST_READ_DEADLINE: Duration = Duration::from_secs(60);
/// Per-`recv` bound for draining a rejected peer's already-buffered input
/// before the socket is closed. Authz-first / busy refusals are written
/// BEFORE the peer's hello is read; closing a SEQPACKET socket with unread
/// input makes the kernel send RST, which the peer observes as a connection
/// reset instead of cleanly reading the rejection frame. Draining the
/// pending input first makes the close graceful.
///
/// This drain runs on the ACCEPT LOOP (the refusal is decided before a
/// handler thread is spawned), so the timeout MUST stay short: the refused
/// peer's hello is already buffered on the SEQPACKET socket, so a
/// cooperating peer drains in microseconds, and a silent/misbehaving peer is
/// bounded to a few `recv` timeouts (≈tens of ms) instead of stalling every
/// other client's `accept()`.
const REJECTION_DRAIN_DEADLINE: Duration = Duration::from_millis(10);

/// Resolve the in-flight connection cap from the environment, falling
/// back to [`DEFAULT_MAX_INFLIGHT_CONNECTIONS`]. A value of `0` or an
/// unparseable value uses the default; the semaphore itself clamps to a
/// minimum of one.
fn resolve_max_inflight_connections() -> usize {
    std::env::var("NIXLINGD_MAX_INFLIGHT_CONNECTIONS")
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .filter(|cap| *cap > 0)
        .unwrap_or(DEFAULT_MAX_INFLIGHT_CONNECTIONS)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactPaths {
    pub public_manifest_path: PathBuf,
    pub bundle_path: PathBuf,
    pub host_path: PathBuf,
    pub processes_path: PathBuf,
    pub closures_dir: PathBuf,
}

impl Default for ArtifactPaths {
    fn default() -> Self {
        Self {
            public_manifest_path: PathBuf::from("/run/current-system/sw/share/nixling/vms.json"),
            bundle_path: PathBuf::from("/etc/nixling/bundle.json"),
            host_path: PathBuf::from("/etc/nixling/host.json"),
            processes_path: PathBuf::from("/etc/nixling/processes.json"),
            closures_dir: PathBuf::from("/etc/nixling/closures"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DaemonConfig {
    pub public_socket_path: PathBuf,
    pub broker_socket_path: PathBuf,
    pub state_lock_path: PathBuf,
    pub locks_dir: PathBuf,
    pub daemon_user: String,
    pub daemon_group: String,
    pub public_socket_group: String,
    #[serde(default)]
    pub launcher_users: Vec<String>,
    #[serde(default)]
    pub admin_users: Vec<String>,
    #[serde(default = "default_server_version")]
    pub server_version: String,
    #[serde(default = "default_accepted_version_range")]
    pub accepted_client_version_range: String,
    #[serde(default)]
    pub artifacts: ArtifactPaths,
    #[serde(default = "default_gateway_config_path")]
    pub gateway_config_path: PathBuf,
    /// Concurrency cap for the autostart pass that runs on daemon
    /// startup. Default `3`.
    /// Mirrors `nixling.daemon.autostart.parallelism`.
    #[serde(default = "default_autostart_parallelism")]
    pub autostart_parallelism: usize,
}

fn default_autostart_parallelism() -> usize {
    autostart::DEFAULT_PARALLELISM
}

fn default_gateway_config_path() -> PathBuf {
    PathBuf::from(DEFAULT_GATEWAY_CONFIG_PATH)
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            public_socket_path: PathBuf::from("/run/nixling/public.sock"),
            broker_socket_path: PathBuf::from("/run/nixling/priv.sock"),
            state_lock_path: PathBuf::from("/run/nixling/daemon.lock"),
            locks_dir: PathBuf::from("/run/nixling/locks"),
            daemon_user: "nixlingd".to_owned(),
            daemon_group: "nixlingd".to_owned(),
            public_socket_group: "nixling".to_owned(),
            launcher_users: Vec::new(),
            admin_users: Vec::new(),
            server_version: default_server_version(),
            accepted_client_version_range: default_accepted_version_range(),
            artifacts: ArtifactPaths::default(),
            gateway_config_path: default_gateway_config_path(),
            autostart_parallelism: autostart::DEFAULT_PARALLELISM,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayFileConfig {
    gateway: String,
    realm: String,
    #[serde(default)]
    state_dir: Option<PathBuf>,
    #[serde(default)]
    credential_path: Option<PathBuf>,
    #[serde(default)]
    seal_key_path: Option<PathBuf>,
    #[serde(default)]
    allow_host_relay_credentials: bool,
    #[serde(default)]
    relay: GatewayRelayFileConfig,
    #[serde(default)]
    aca: GatewayAcaFileConfig,
    #[serde(default)]
    display: GatewayDisplayFileConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GatewayRelayFileConfig {
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    entity: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GatewayAcaFileConfig {
    #[serde(default)]
    endpoint: Option<String>,
    #[serde(default)]
    subscription: Option<String>,
    #[serde(default)]
    resource_group: Option<String>,
    #[serde(default)]
    sandbox_group: Option<String>,
    #[serde(default)]
    region: Option<String>,
    #[serde(default)]
    disk_image_id: Option<String>,
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    disk_name: Option<String>,
    #[serde(default)]
    managed_identity_resource_id: Option<String>,
    #[serde(default)]
    managed_identity_client_id: Option<String>,
    #[serde(default)]
    cpu: Option<String>,
    #[serde(default)]
    memory: Option<String>,
    #[serde(default)]
    auto_suspend_interval_secs: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GatewayDisplayFileConfig {
    #[serde(default)]
    vsock_port: Option<u32>,
    #[serde(default)]
    waypipe_compression: Option<String>,
    #[serde(default)]
    waypipe_socket: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ServeOptions {
    pub config_path: PathBuf,
    pub public_socket_path: Option<PathBuf>,
    pub broker_socket_path: Option<PathBuf>,
    pub state_lock_path: Option<PathBuf>,
    pub locks_dir: Option<PathBuf>,
    pub once: bool,
    pub allow_unprivileged_runtime_dir: bool,
    pub drop_privileges: bool,
    pub daemon_state_dir: Option<PathBuf>,
    pub test_state_restore_report_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct LockOnlyOptions {
    pub config_path: PathBuf,
    pub state_lock_path: Option<PathBuf>,
    pub allow_unprivileged_runtime_dir: bool,
    pub hold_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct TestClientOptions {
    pub socket_path: PathBuf,
    pub frame_json: Vec<String>,
}

#[derive(Debug, Clone)]
struct RuntimeIdentity {
    daemon_uid: Uid,
    daemon_gid: Gid,
    public_socket_gid: Gid,
    expect_root_owned_parent: bool,
}

#[derive(Debug, Clone)]
struct PeerIdentity {
    role: PeerRole,
    uid: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PeerRole {
    Launcher,
    Admin,
}

#[derive(Debug, Clone)]
struct ServerState {
    config: DaemonConfig,
    daemon_uid: u32,
    daemon_state_dir: PathBuf,
    pidfd_table: Arc<PidfdTable>,
    broker_reap_log: Arc<BrokerReapLog>,
    metrics_registry: Arc<metrics::Registry>,
    /// Daemon-side audit log for supervisor events (e.g. api-ready
    /// timeout) that are not emitted by the broker.
    daemon_audit: Arc<daemon_audit::DaemonAuditLog>,
    /// In-process exec session table (caps + opaque handles) for
    /// `nixling vm exec`. There is no per-VM unit and no broker op: a
    /// session is a daemon-held authenticated guest-control client owned by a
    /// spawned worker.
    exec_sessions: Arc<exec_session::SessionTable>,
    /// Gateway display orchestrator state. Persisted for the daemon lifetime so
    /// Open/List/Close share the same ledger and resource handles.
    gateway_display: Arc<GatewayDisplayRuntime>,
    /// Bounded admission gate for in-flight connection-handler threads.
    /// The accept loop performs a non-blocking try-acquire and refuses
    /// (typed-busy) on a miss rather than ever blocking `accept()`.
    conn_semaphore: concurrency::ConnSemaphore,
    /// Per-VM / global in-process op locks. Acquired once on the worker
    /// thread inside `dispatch_request` so a mutating lifecycle op cannot
    /// race another op on the same VM (or any per-VM op for a global op).
    op_locks: concurrency::OpLockManager,
}

struct GatewayDisplayRuntime {
    orchestrator: GatewayOrchestrator,
    sessions: Mutex<HashMap<String, GatewayDisplaySession>>,
    lifecycle: Box<dyn GatewayLifecycle>,
    preflight: Option<GatewayDisplayPreflight>,
}

impl std::fmt::Debug for GatewayDisplayRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GatewayDisplayRuntime(<state>)")
    }
}

#[derive(Debug, Clone)]
struct GatewayDisplaySession {
    target: String,
    principal: String,
    open: OpenSession,
    opened_at: Instant,
}

#[derive(Debug, Clone)]
struct GatewayDisplayPreflight {
    allow_host_relay_credentials: bool,
    waypipe_socket_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct ValidatedWaypipeSocket {
    uid: u32,
    mode: u32,
}

#[async_trait]
trait GatewayLifecycle: Send + Sync {
    async fn start(&self, target: &TargetName) -> Result<String, GatewayError>;
    async fn stop(&self, target: &TargetName) -> Result<String, GatewayError>;
}

fn new_gateway_display_runtime() -> Arc<GatewayDisplayRuntime> {
    Arc::new(GatewayDisplayRuntime {
        orchestrator: GatewayOrchestrator::new(
            unavailable_gateway_deps(),
            1,
            LedgerLimits::default(),
        ),
        sessions: Mutex::new(HashMap::new()),
        lifecycle: Box::new(UnavailableGatewayLifecycle),
        preflight: None,
    })
}

#[cfg(test)]
fn new_gateway_display_runtime_for_tests() -> Arc<GatewayDisplayRuntime> {
    Arc::new(GatewayDisplayRuntime {
        orchestrator: GatewayOrchestrator::new(daemon_gateway_deps(), 1, LedgerLimits::default()),
        sessions: Mutex::new(HashMap::new()),
        lifecycle: Box::new(DaemonGatewayLifecycle),
        preflight: None,
    })
}

#[cfg_attr(test, derive(Clone))]
struct PeerOverride {
    uid: u32,
    gid: u32,
    username: Option<String>,
    groups: Option<Vec<String>>,
}

fn default_server_version() -> String {
    DEFAULT_SERVER_VERSION.to_owned()
}

fn default_accepted_version_range() -> String {
    DEFAULT_ACCEPTED_VERSION_RANGE.to_owned()
}

fn effective_daemon_state_dir(options: &ServeOptions) -> PathBuf {
    options
        .daemon_state_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_DAEMON_STATE_DIR))
}

fn pidfd_table_state_path(daemon_state_dir: &Path) -> PathBuf {
    daemon_state_dir.join("pidfd-table.json")
}

/// Path of the persisted kernel-module-check report. `nixling host
/// doctor --read-only` reads this file to surface the kernel-module
/// matrix posture without re-running the bundle resolver in the CLI
/// process.
pub fn kernel_module_report_path(daemon_state_dir: &Path) -> PathBuf {
    daemon_state_dir.join("kernel-module-report.json")
}

/// Path of the persisted autostart-pass report (summary + per-VM
/// outcomes). `nixling host doctor --read-only` reads this file to report
/// degraded-VM count.
pub fn autostart_report_path(daemon_state_dir: &Path) -> PathBuf {
    daemon_state_dir.join("autostart-report.json")
}

/// Path of the persisted storage/restart/sync startup contract report.
pub fn storage_lifecycle_report_path(daemon_state_dir: &Path) -> PathBuf {
    daemon_state_dir.join("storage-lifecycle-report.json")
}

fn persist_json_report(path: &Path, json: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = File::create(&tmp)?;
        file.write_all(json)?;
        // host doctor is an unprivileged read-only CLI surface. Keep
        // diagnostic reports world-readable beneath the ACL-gated daemon-state
        // tree; they contain bounded posture data, not authority or secrets.
        file.set_permissions(fs::Permissions::from_mode(0o644))?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    if let Some(parent) = path.parent()
        && let Ok(parent_dir) = File::open(parent)
    {
        let _ = parent_dir.sync_all();
    }
    Ok(())
}

/// Persist the latest kernel-module-check report to
/// `kernel-module-report.json`. Best-effort: a write failure logs a
/// warning but does NOT abort daemon startup.
fn persist_kernel_module_report(
    daemon_state_dir: &Path,
    report: &kernel_module_check::ModuleCheckReport,
) {
    let path = kernel_module_report_path(daemon_state_dir);
    let json = match serde_json::to_vec_pretty(report) {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::warn!(error = %err, "kernel-module-check: serialize report failed");
            return;
        }
    };
    if let Err(err) = persist_json_report(&path, &json) {
        tracing::warn!(
            error = %err,
            path = %path.display(),
            "kernel-module-check: persist report failed",
        );
    }
}

/// Persist the latest autostart-pass report to
/// `autostart-report.json`. Best-effort: a write failure logs a
/// warning but does NOT abort daemon startup.
fn persist_autostart_report(daemon_state_dir: &Path, report: &autostart::AutostartReport) {
    let path = autostart_report_path(daemon_state_dir);
    let json = match serde_json::to_vec_pretty(report) {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::warn!(error = %err, "autostart: serialize report failed");
            return;
        }
    };
    if let Err(err) = persist_json_report(&path, &json) {
        tracing::warn!(
            error = %err,
            path = %path.display(),
            "autostart: persist report failed",
        );
    }
}

fn persist_storage_lifecycle_report(
    daemon_state_dir: &Path,
    report: &storage_lifecycle::StorageLifecycleReport,
) {
    let path = storage_lifecycle_report_path(daemon_state_dir);
    let json = match serde_json::to_vec_pretty(report) {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::warn!(error = %err, "storage-lifecycle: serialize report failed");
            return;
        }
    };
    if let Err(err) = persist_json_report(&path, &json) {
        tracing::warn!(
            error = %err,
            path = %path.display(),
            "storage-lifecycle: persist report failed",
        );
    }
}

pub fn banner() -> String {
    "nixlingd 0.0.0-bootstrap (bootstrap stub)".to_owned()
}

pub fn banner_note() -> String {
    "daemon skeleton: start with `nixlingd serve` or use hidden test modes for Layer-1 gates."
        .to_owned()
}

pub fn load_config(path: &Path) -> Result<DaemonConfig, TypedError> {
    if !path.exists() {
        return Ok(DaemonConfig::default());
    }
    let bytes = fs::read(path).map_err(|err| TypedError::InternalIo {
        context: format!("read config {}", path.display()),
        detail: err.to_string(),
    })?;
    serde_json::from_slice(&bytes).map_err(|err| TypedError::InternalConfig {
        detail: format!("{}: {err}", path.display()),
    })
}

fn load_gateway_file_config(path: &Path) -> Result<Option<GatewayFileConfig>, TypedError> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(|err| TypedError::InternalIo {
        context: format!("read gateway config {}", path.display()),
        detail: err.to_string(),
    })?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|err| TypedError::InternalConfig {
            detail: format!("{}: {err}", path.display()),
        })
}

fn new_gateway_display_runtime_from_config(
    config: GatewayFileConfig,
) -> Result<Arc<GatewayDisplayRuntime>, TypedError> {
    let preflight = gateway_display_preflight_from_config(&config)?;
    let deps = gateway_deps_from_config(&config)?;
    let provider =
        Arc::new(aca_provider_from_gateway_config(&config).map_err(gateway_error_to_typed)?);
    Ok(Arc::new(GatewayDisplayRuntime {
        orchestrator: GatewayOrchestrator::new(deps, 1, LedgerLimits::default()),
        sessions: Mutex::new(HashMap::new()),
        lifecycle: Box::new(AcaGatewayLifecycle { provider }),
        preflight: Some(preflight),
    }))
}

fn gateway_display_preflight_from_config(
    config: &GatewayFileConfig,
) -> Result<GatewayDisplayPreflight, TypedError> {
    let waypipe_socket_path = config
        .display
        .waypipe_socket
        .as_ref()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from);
    Ok(GatewayDisplayPreflight {
        allow_host_relay_credentials: config.allow_host_relay_credentials,
        waypipe_socket_path,
    })
}

fn validate_gateway_host_relay_transition_guard(
    config: &GatewayFileConfig,
) -> Result<(), TypedError> {
    if config.allow_host_relay_credentials {
        Err(gateway_display_config_error(
            "host-held gateway credentials and relay send-bearer minting are retired; enroll inside gateway then retry",
        ))
    } else {
        Ok(())
    }
}

fn validate_waypipe_receiver_socket(path: &Path) -> Result<ValidatedWaypipeSocket, TypedError> {
    if !path.is_absolute() {
        return Err(gateway_display_config_error(format!(
            "waypipeSocket {} must be an absolute Unix socket path",
            path.display()
        )));
    }
    reject_symlink_components(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|err| {
        gateway_display_config_error(format!(
            "waypipeSocket {} cannot be inspected without following symlinks: {err}; create an operator-owned Unix socket with mode 0600",
            path.display()
        ))
    })?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Err(gateway_display_config_error(format!(
            "waypipeSocket {} must point directly at an operator-owned Unix socket, not a symlink",
            path.display()
        )));
    }
    if !file_type.is_socket() {
        return Err(gateway_display_config_error(format!(
            "waypipeSocket {} must be a Unix socket owned by the operator with mode 0600",
            path.display()
        )));
    }
    let mode = metadata.permissions().mode() & 0o777;
    if mode != 0o600 {
        return Err(gateway_display_config_error(format!(
            "waypipeSocket {} must have mode 0600; current mode is {mode:#05o}; run chmod 0600 on the socket path",
            path.display()
        )));
    }
    let uid = metadata.uid();
    if uid == 0 {
        return Err(gateway_display_config_error(format!(
            "waypipeSocket {} must be owned by an unprivileged operator uid, not root",
            path.display()
        )));
    }
    Ok(ValidatedWaypipeSocket { uid, mode })
}

fn reject_symlink_components(path: &Path) -> Result<(), TypedError> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if current.parent().is_none() {
            continue;
        }
        let metadata = fs::symlink_metadata(&current).map_err(|err| {
            gateway_display_config_error(format!(
                "waypipeSocket component {} cannot be inspected without following symlinks: {err}",
                current.display()
            ))
        })?;
        if metadata.file_type().is_symlink() {
            return Err(gateway_display_config_error(format!(
                "waypipeSocket component {} must not be a symlink",
                current.display()
            )));
        }
    }
    Ok(())
}

fn gateway_display_config_error(detail: impl Into<String>) -> TypedError {
    TypedError::GatewayDisplayUnavailable {
        detail: detail.into(),
    }
}

pub async fn serve(options: ServeOptions) -> Result<(), TypedError> {
    let mut config = load_config(&options.config_path)?;
    apply_overrides(&mut config, &options);

    // v1.1.1 runtime pidfs self-probe: refuse startup on kernels
    // without pidfs support. Static `tests/v1.1-kernel-floor-eval.sh`
    // catches the easy case (operator flake declares < 6.9 kernel);
    // this probe catches the hard case (custom-built kernel at >= 6.9
    // that strips CONFIG_FS_PID). Soft-fail opt-in via the
    // `NIXLING_ALLOW_PIDFS_PROBE_SOFT_FAIL` env var (CI/dev hosts).
    {
        let outcome = pidfs_probe::probe_pidfs();
        pidfs_probe::enforce_probe_outcome(&outcome)?;
    }

    let runtime_identity =
        resolve_runtime_identity(&config, options.allow_unprivileged_runtime_dir)?;
    validate_lock_parent(&config.state_lock_path, &runtime_identity)?;
    ensure_locks_dir(&config.locks_dir, &runtime_identity)?;
    let _lock_file = acquire_state_lock(&config.state_lock_path, &runtime_identity)?;
    let listener = bind_public_socket(&config.public_socket_path, &runtime_identity)?;

    if options.drop_privileges {
        drop_privileges_if_root(&runtime_identity)?;
    }

    // Write /run/nixling/version on daemon startup so the CLI's
    // [pending restart] machinery has
    // an authoritative version + binary-path snapshot. Failures are
    // logged but non-fatal — operators can still drive the daemon
    // without the pending-restart signal.
    write_daemon_version_file(&config);
    maybe_write_state_restore_report(&options)?;

    let daemon_state_dir = effective_daemon_state_dir(&options);
    let pidfd_table_path = pidfd_table_state_path(&daemon_state_dir);
    let broker_reap_log = BrokerReapLog::new();
    let pidfd_table = Arc::new(PidfdTable::restore_from_disk(&pidfd_table_path).map_err(
        |err| TypedError::InternalIo {
            context: format!("restore pidfd table {}", pidfd_table_path.display()),
            detail: err.to_string(),
        },
    )?);
    pidfd_table.set_broker_reap_log(Arc::clone(&broker_reap_log));

    let gateway_display =
        if let Some(gateway_config) = load_gateway_file_config(&config.gateway_config_path)? {
            crate::new_gateway_display_runtime_from_config(gateway_config)?
        } else {
            crate::new_gateway_display_runtime()
        };

    let state = ServerState {
        daemon_uid: runtime_identity.daemon_uid.as_raw(),
        config,
        daemon_audit: Arc::new(daemon_audit::DaemonAuditLog::new(&daemon_state_dir)),
        daemon_state_dir,
        pidfd_table,
        broker_reap_log,
        metrics_registry: Arc::new(crate::metrics::Registry::new()),
        exec_sessions: Arc::new(crate::exec_session::SessionTable::new(
            crate::exec_session::ExecSessionCaps::default(),
        )),
        gateway_display,
        conn_semaphore: concurrency::ConnSemaphore::new(resolve_max_inflight_connections()),
        op_locks: crate::concurrency::OpLockManager::new(),
    };
    refresh_broker_reap_log(&state, "startup");

    match load_bundle_resolver(&state) {
        Ok(resolver) => {
            let report = storage_lifecycle::run_startup_contract_check(&resolver);
            let report_path = storage_lifecycle_report_path(&state.daemon_state_dir);
            if report.has_only_legacy_contract_issue() {
                tracing::info!(
                    bundle_version = resolver.bundle.bundle_version,
                    report_path = %report_path.display(),
                    "storage-lifecycle: legacy bundle lacks storage/sync contracts; rebuild host configuration to enable startup contract checks",
                );
            } else if report.is_degraded() {
                let issue_kinds = report.issue_kinds_csv();
                tracing::warn!(
                    issue_count = report.issues.len(),
                    issue_kinds = %issue_kinds,
                    path_count = report.path_count,
                    restart_policy_count = report.restart_policy_count,
                    lock_count = report.lock_count,
                    report_path = %report_path.display(),
                    "storage-lifecycle: startup contract check degraded",
                );
            } else {
                tracing::info!(
                    path_count = report.path_count,
                    restart_policy_count = report.restart_policy_count,
                    lock_count = report.lock_count,
                    report_path = %report_path.display(),
                    "storage-lifecycle: startup contract check clean",
                );
            }
            persist_storage_lifecycle_report(&state.daemon_state_dir, &report);
        }
        Err(error) => {
            let report_path = storage_lifecycle_report_path(&state.daemon_state_dir);
            // Fail closed for doctor/status consumers if the replacement
            // report cannot be written below: stale clean evidence is worse
            // than an absent report.
            match std::fs::remove_file(&report_path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        report_path = %report_path.display(),
                        "storage-lifecycle: remove stale report failed",
                    );
                }
            }
            let report = storage_lifecycle::bundle_resolver_unavailable_report();
            persist_storage_lifecycle_report(&state.daemon_state_dir, &report);
            let issue_kinds = report.issue_kinds_csv();
            tracing::warn!(
                error = %error.message(),
                issue_kinds = %issue_kinds,
                report_path = %report_path.display(),
                "storage-lifecycle: skipped (bundle resolver unavailable)",
            );
        }
    }
    adopt_orphaned_runners_on_startup(&state);

    // Startup self-check on the kernel-module matrix the bundle requires.
    // Fatal misses
    // refuse daemon start; optional misses are logged and the
    // affected VMs are skipped (Degraded) by the autostart pass.
    // If the bundle resolver itself is unavailable we skip the
    // gate — the autostart pass already logs and short-circuits
    // in that case, and the daemon must remain reachable for
    // diagnostic verbs (status / doctor / audit).
    let module_degraded_vms: BTreeSet<String> = match load_bundle_resolver(&state) {
        Ok(resolver) => {
            let report = kernel_module_check::run_kernel_module_check(&resolver);
            // NIXLING_SKIP_KERNEL_MODULE_CHECK converts the fatal check
            // into a logged warning. Real
            // deployments on hosts whose kernel has the GUEST-side
            // virtio modules built in (vs loadable) get false-
            // positives because the check uses `lsmod` and the
            // built-in modules don't appear there. The check
            // remains the default; the env-var is an explicit
            // operator override for the substrate-replaced v1.1
            // hosts where the historical module list is stale.
            let skip_kernel_check = std::env::var_os("NIXLING_SKIP_KERNEL_MODULE_CHECK").is_some();
            if report.is_fatal() && !skip_kernel_check {
                persist_kernel_module_report(&state.daemon_state_dir, &report);
                let err = kernel_module_check::fatal_typed_error(&report);
                tracing::error!(
                    kind = err.kind(),
                    missing = %report.missing_required_summary(),
                    present = ?report.present,
                    "kernel-module-check: refusing daemon startup; required modules missing",
                );
                return Err(err);
            }
            if report.is_fatal() && skip_kernel_check {
                tracing::warn!(
                    missing = %report.missing_required_summary(),
                    present = ?report.present,
                    "kernel-module-check: fatal misses bypassed via NIXLING_SKIP_KERNEL_MODULE_CHECK; affected VMs may fail at start",
                );
            }
            for row in &report.optional_missing {
                tracing::warn!(
                    module = %row.module,
                    affected_vms = ?row.affected_vms,
                    reason = %row.reason,
                    "kernel-module-check: optional module missing",
                );
            }
            let degraded = report.degraded_vms();
            persist_kernel_module_report(&state.daemon_state_dir, &report);
            degraded
        }
        Err(error) => {
            tracing::warn!(
                error = %error.message(),
                "kernel-module-check: skipped (bundle resolver unavailable)",
            );
            BTreeSet::new()
        }
    };
    // Daemon-side net-route preflight (replaces
    // `nixling-net-route-preflight.service`). For each env in the host
    // artifact, probe its LAN bridge.
    // Failed envs contribute their VMs to the pre-degraded set so
    // those VMs surface as `Outcome::Degraded` instead of failing
    // their unit. After N consecutive startup failures the daemon
    // enters operator-only mode: autostart is skipped entirely and
    // recovery is via `nixling host reconcile --network --apply`.
    let mut net_pre_degraded_vms: BTreeSet<String> = BTreeSet::new();
    let mut net_operator_only_mode = net_route_preflight::OperatorOnlyMode::Disengaged;
    let net_history = net_route_preflight::PreflightHistory::new(&state.daemon_state_dir);
    match load_host_artifact(&state) {
        Ok(host) => {
            let probe = net_route_preflight::SysClassNetProbe;
            let report = net_route_preflight::run_net_route_preflight(&host, &probe);
            let failed_envs = report.failed_envs();
            let record = net_route_preflight::PreflightHistoryRecord {
                ts: net_route_preflight::now_epoch_seconds(),
                ok: report.is_ok(),
                failed_envs: failed_envs.iter().cloned().collect(),
                source: "startup".to_owned(),
            };
            if let Err(err) = net_history.record(&record) {
                tracing::warn!(
                    path = %net_history.path().display(),
                    error = %err,
                    "net-route-preflight: failed to persist history record (continuing)",
                );
            }
            if !report.is_ok() {
                tracing::error!(
                    failed_envs = ?failed_envs,
                    summary = %report.summary(),
                    "net-route-preflight: one or more env bridges unhealthy; affected VMs will be marked Degraded",
                );
                if let Ok(resolver) = load_bundle_resolver(&state) {
                    for (name, vm) in &resolver.manifest.vms {
                        if let Some(env) = &vm.env
                            && failed_envs.contains(env)
                        {
                            net_pre_degraded_vms.insert(name.clone());
                        }
                    }
                }
            } else {
                tracing::info!(
                    summary = %report.summary(),
                    "net-route-preflight: all env bridges healthy",
                );
            }
            match net_history.consecutive_failures() {
                Ok(n) => {
                    net_operator_only_mode = net_route_preflight::OperatorOnlyMode::classify(
                        n,
                        net_route_preflight::DEFAULT_DEGRADED_MODE_THRESHOLD,
                    );
                    if net_operator_only_mode.is_engaged() {
                        tracing::error!(
                            consecutive_failures = n,
                            threshold = net_route_preflight::DEFAULT_DEGRADED_MODE_THRESHOLD,
                            failed_envs = ?failed_envs,
                            "net-route-preflight: OPERATOR-ONLY MODE ENGAGED — autostart skipped. Recovery: nixling host reconcile --network --apply",
                        );
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        path = %net_history.path().display(),
                        error = %err,
                        "net-route-preflight: failed to read history (assuming disengaged)",
                    );
                }
            }
        }
        Err(error) => {
            tracing::warn!(
                error = %error.message(),
                "net-route-preflight: skipped (host artifact unavailable)",
            );
        }
    }

    let mut combined_pre_degraded: BTreeSet<String> = module_degraded_vms.clone();
    combined_pre_degraded.extend(net_pre_degraded_vms.iter().cloned());

    if net_operator_only_mode.is_engaged() {
        tracing::warn!(
            "net-route-preflight: operator-only mode — skipping run_startup_autostart entirely",
        );
    } else {
        run_startup_autostart(&state, &combined_pre_degraded).await;
    }

    loop {
        let (stream, _) = listener.accept().map_err(|err| TypedError::InternalIo {
            context: "accept public seqpacket client".to_owned(),
            detail: err.to_string(),
        })?;

        // The `once` test path stays fully synchronous/inline so unit
        // tests can drive a single connection deterministically.
        if options.once {
            if let Err(error) = handle_connection(stream, &state, None) {
                eprintln!("{}", error.message());
            }
            break;
        }

        // Authz-first: resolve SO_PEERCRED immediately after accept, before
        // any blocking frame read, so an unauthorized or silent peer can
        // neither occupy a handler slot nor stall the accept loop.
        let peer = match authorize_peer(&stream, &state) {
            Ok(peer) => peer,
            Err(error) => {
                let _ = write_json_frame_deadlined(
                    &stream,
                    &wire::hello_rejected(&error),
                    ACCEPT_REFUSAL_WRITE_DEADLINE,
                );
                drain_rejected_peer_input(&stream);
                eprintln!("{}", error.message());
                continue;
            }
        };

        // Non-blocking admission: never block the accept loop. On a cap
        // miss refuse immediately with a typed-busy frame (deadlined).
        let permit = match state.conn_semaphore.try_acquire() {
            Some(permit) => permit,
            None => {
                let busy = TypedError::DaemonBusy;
                let _ = write_json_frame_deadlined(
                    &stream,
                    &wire::error_frame(&busy),
                    ACCEPT_REFUSAL_WRITE_DEADLINE,
                );
                drain_rejected_peer_input(&stream);
                continue;
            }
        };

        // Hand the connection to its own handler thread, moving the RAII
        // permit in so the in-flight slot is released when the handler —
        // not the accept loop — finishes. accept() returns immediately.
        let conn_state = state.clone();
        if let Err(err) = std::thread::Builder::new()
            .name("nixling-conn".to_owned())
            .spawn(move || {
                // `permit` (and, for an exec session, ownership of it) is
                // dropped when this handler returns.
                if let Err(error) =
                    handle_connection_authorized(stream, &conn_state, peer, Some(permit))
                {
                    eprintln!("{}", error.message());
                }
            })
        {
            // Spawn failure drops the moved closure (and its permit), so
            // the slot is released; log and keep serving.
            eprintln!(
                "{}",
                TypedError::InternalIo {
                    context: "spawn connection handler".to_owned(),
                    detail: err.to_string(),
                }
                .message()
            );
        }
    }

    Ok(())
}

pub async fn lock_only(options: LockOnlyOptions) -> Result<(), TypedError> {
    let mut config = load_config(&options.config_path)?;
    if let Some(path) = options.state_lock_path.clone() {
        config.state_lock_path = path;
    }
    let runtime_identity =
        resolve_runtime_identity(&config, options.allow_unprivileged_runtime_dir)?;
    validate_lock_parent(&config.state_lock_path, &runtime_identity)?;
    let _lock_file = acquire_state_lock(&config.state_lock_path, &runtime_identity)?;
    tokio::time::sleep(tokio::time::Duration::from_secs(options.hold_seconds)).await;
    Ok(())
}

pub fn run_test_client(options: TestClientOptions) -> Result<u8, TypedError> {
    let socket = connect_seqpacket(&options.socket_path)?;
    let mut exit_code = 0u8;
    for frame in &options.frame_json {
        let response = round_trip(&socket, frame)?;
        println!("{}", String::from_utf8_lossy(&response));
        if let Ok(value) = serde_json::from_slice::<Value>(&response)
            && let Some(code) = value
                .get("error")
                .and_then(|error| error.get("exitCode"))
                .and_then(Value::as_u64)
        {
            exit_code = code as u8;
        }
    }
    Ok(exit_code)
}

fn apply_overrides(config: &mut DaemonConfig, options: &ServeOptions) {
    if let Some(path) = &options.public_socket_path {
        config.public_socket_path = path.clone();
    }
    if let Some(path) = &options.broker_socket_path {
        config.broker_socket_path = path.clone();
    }
    if let Some(path) = &options.state_lock_path {
        config.state_lock_path = path.clone();
    }
    if let Some(path) = &options.locks_dir {
        config.locks_dir = path.clone();
    }
}

fn maybe_write_state_restore_report(options: &ServeOptions) -> Result<(), TypedError> {
    let Some(report_path) = options.test_state_restore_report_path.as_ref() else {
        return Ok(());
    };
    let daemon_state_dir = effective_daemon_state_dir(options);
    let state_dir = daemon_state_dir.as_path();

    let store = supervisor::state::FilesystemSnapshotStore::new(state_dir);
    let snapshots =
        supervisor::state::SnapshotStore::list(&store).map_err(|err| TypedError::InternalIo {
            context: "enumerate daemon state snapshots".to_owned(),
            detail: err.to_string(),
        })?;
    let report = supervisor::state::reconcile(&snapshots, &supervisor::state::SystemProcReader);
    let rendered = serde_json::to_vec_pretty(&report).map_err(|err| TypedError::InternalIo {
        context: "serialize daemon state report".to_owned(),
        detail: err.to_string(),
    })?;
    fs::write(report_path, rendered).map_err(|err| TypedError::InternalIo {
        context: "write daemon state report".to_owned(),
        detail: err.to_string(),
    })
}

struct BrokerPidfdOpener<'a> {
    state: &'a ServerState,
}

impl supervisor::state::PidfdOpener for BrokerPidfdOpener<'_> {
    fn open_pidfd(
        &self,
        vm: &str,
        role_id: &str,
        pid: i32,
        expected_start_time_ticks: u64,
    ) -> Result<OwnedFd, String> {
        match dispatch_broker_request_with_fds_timeout(
            self.state,
            BrokerRequest::OpenPidfd(BrokerOpenPidfdRequest {
                vm_id: VmId::new(vm),
                role_id: RoleId::new(role_id),
                pid,
                expected_start_time_ticks,
                tracing_span_id: None,
            }),
            Duration::from_secs(10),
        ) {
            Ok((BrokerResponse::OpenPidfd(response), received_fds)) => {
                let pidfd = duplicate_received_fd(
                    &received_fds,
                    response.pidfd_index,
                    "duplicate OpenPidfd pidfd",
                )
                .map_err(|error| error.message());
                close_received_fds(&received_fds);
                match pidfd {
                    Ok(pidfd) => {
                        if response.vm_id.as_str() != vm
                            || response.role_id.as_str() != role_id
                            || response.verified_start_time_ticks != expected_start_time_ticks
                        {
                            Err("broker-response-mismatch".to_owned())
                        } else {
                            Ok(pidfd)
                        }
                    }
                    Err(error) => Err(error),
                }
            }
            Ok((BrokerResponse::Error(error), received_fds)) => {
                close_received_fds(&received_fds);
                Err(format!("broker-error:{}", error.kind))
            }
            Ok((other, received_fds)) => {
                close_received_fds(&received_fds);
                Err(format!("broker-protocol:{}", broker_response_kind(&other)))
            }
            Err(error) => Err(format!("broker-dispatch:{}", error.message())),
        }
    }
}

fn adopt_orphaned_runners_on_startup(state: &ServerState) {
    let store = supervisor::state::FilesystemSnapshotStore::new(&state.daemon_state_dir);
    let proc_reader = supervisor::state::SystemProcReader;
    let opener = BrokerPidfdOpener { state };
    if let Err(error) = adopt_orphaned_runners_on_startup_with(state, &store, &proc_reader, &opener)
    {
        tracing::warn!(error = ?error, "startup orphan adoption failed");
    }
}

fn adopt_orphaned_runners_on_startup_with(
    state: &ServerState,
    store: &dyn supervisor::state::SnapshotStore,
    proc_reader: &dyn supervisor::state::ProcReader,
    opener: &dyn supervisor::state::PidfdOpener,
) -> Result<(), TypedError> {
    let snapshots =
        supervisor::state::SnapshotStore::list(store).map_err(|err| TypedError::InternalIo {
            context: "enumerate daemon runner snapshots".to_owned(),
            detail: err.to_string(),
        })?;
    if snapshots.is_empty() {
        return Ok(());
    }

    let snapshot_index: BTreeMap<(String, String), &supervisor::state::RunnerSnapshotRecord> =
        snapshots
            .iter()
            .map(|snapshot| ((snapshot.vm.clone(), snapshot.role_id.clone()), snapshot))
            .collect();

    for adopt in supervisor::state::reconcile_and_adopt(&snapshots, proc_reader, opener) {
        let key = (adopt.vm.clone(), adopt.role_id.clone());
        let Some(snapshot) = snapshot_index.get(&key) else {
            continue;
        };
        match adopt.outcome {
            supervisor::state::AdoptOutcome::Adopted(pidfd) => {
                if state.pidfd_table.contains(&adopt.vm, &adopt.role_id) {
                    tracing::info!(
                        vm = %adopt.vm,
                        role = %adopt.role_id,
                        "startup adoption skipped because pidfd table already restored the runner"
                    );
                    continue;
                }
                match state.pidfd_table.register(
                    adopt.vm.clone(),
                    adopt.role_id.clone(),
                    PidfdEntry {
                        pidfd,
                        pid: snapshot.pid,
                        start_time_ticks: snapshot.start_time_ticks,
                    },
                ) {
                    Ok(()) => {
                        tracing::info!(
                            vm = %adopt.vm,
                            role = %adopt.role_id,
                            pid = snapshot.pid,
                            start_time_ticks = snapshot.start_time_ticks,
                            "adopted runner snapshot into pidfd table"
                        );
                    }
                    Err(PidfdTableError::DuplicateRegistration { .. }) => {
                        tracing::info!(
                            vm = %adopt.vm,
                            role = %adopt.role_id,
                            "startup adoption observed a duplicate pidfd-table registration"
                        );
                    }
                    Err(error) => {
                        return Err(TypedError::InternalIo {
                            context: "register adopted pidfd".to_owned(),
                            detail: error.to_string(),
                        });
                    }
                }
            }
            supervisor::state::AdoptOutcome::Quarantine {
                observed_start_time_ticks,
            } => {
                tracing::warn!(
                    vm = %adopt.vm,
                    role = %adopt.role_id,
                    pid = snapshot.pid,
                    expected_start_time_ticks = snapshot.start_time_ticks,
                    observed_start_time_ticks,
                    "startup adoption quarantined runner snapshot"
                );
            }
            supervisor::state::AdoptOutcome::Missing => {
                supervisor::state::SnapshotStore::remove(store, &adopt.vm, &adopt.role_id)
                    .map_err(|err| TypedError::InternalIo {
                        context: "remove missing runner snapshot".to_owned(),
                        detail: err.to_string(),
                    })?;
                tracing::info!(
                    vm = %adopt.vm,
                    role = %adopt.role_id,
                    pid = snapshot.pid,
                    "startup adoption removed stale missing runner snapshot"
                );
            }
            supervisor::state::AdoptOutcome::AdoptRaced { detail } => {
                if transient_adoption_error(&detail) {
                    tracing::warn!(
                        vm = %adopt.vm,
                        role = %adopt.role_id,
                        pid = snapshot.pid,
                        detail = %detail,
                        "startup adoption could not reopen pidfd; leaving snapshot on disk"
                    );
                    continue;
                }
                supervisor::state::SnapshotStore::remove(store, &adopt.vm, &adopt.role_id)
                    .map_err(|err| TypedError::InternalIo {
                        context: "remove raced runner snapshot".to_owned(),
                        detail: err.to_string(),
                    })?;
                tracing::warn!(
                    vm = %adopt.vm,
                    role = %adopt.role_id,
                    pid = snapshot.pid,
                    detail = %detail,
                    "startup adoption dropped runner snapshot after pidfd reopen race"
                );
            }
            supervisor::state::AdoptOutcome::UnparseableProcStat { detail } => {
                tracing::warn!(
                    vm = %adopt.vm,
                    role = %adopt.role_id,
                    pid = snapshot.pid,
                    detail = %detail,
                    "startup adoption quarantined runner snapshot with unparseable proc stat"
                );
            }
        }
    }

    state
        .pidfd_table
        .snapshot()
        .map_err(|err| TypedError::InternalIo {
            context: "persist adopted pidfd table".to_owned(),
            detail: err.to_string(),
        })?;
    Ok(())
}

fn transient_adoption_error(detail: &str) -> bool {
    detail.starts_with("broker-")
}

// =====================================================================
// nixlingd autostart contract glue.
// The plan + executor live in `autostart`; this section wires the
// production starter (which dispatches into `dispatch_broker_vm_start`)
// and the startup invocation. See docs/reference/daemon-autostart.md.
// =====================================================================

/// Production [`autostart::VmStarter`] backed by the live broker
/// dispatch path. Wraps `ServerState` in an `Arc` so the autostart
/// `JoinSet` tasks can each hold a reference.
struct BrokerVmStarter {
    state: Arc<ServerState>,
}

impl autostart::VmStarter for BrokerVmStarter {
    fn is_running(&self, vm: &str) -> bool {
        // Idempotency check mirrors the duplicate-pidfd guard in
        // `dispatch_broker_vm_start`: if the ch-runner role is
        // already registered, the VM is supervised.
        self.state.pidfd_table.contains(vm, VM_RUNNER_ROLE_ID)
    }

    fn start(&self, vm: &str) -> Result<(), String> {
        let request = public_wire::VmLifecycleRequest {
            vm: vm.to_owned(),
            flags: public_wire::MutationFlags {
                apply: true,
                dry_run: false,
                json: true,
            },
            // Opt-IN to relaxed semantics so api-ready timeout (common
            // during cold boot of net VMs) does not
            // cascade-degrade every workload VM in the env. The
            // strict-default contract is preserved for explicit
            // `nixling vm start --apply` invocations.
            no_wait_api: true,
        };
        match dispatch_broker_vm_start(&self.state, request) {
            Ok(value) => {
                // dispatch_broker_vm_start returns a JSON envelope
                // even on logical failure (so the public verb can
                // surface it). For autostart we accept the
                // "applied" outcome regardless of api-ready state
                // (--no-wait-api means api-ready: pending is expected
                // during cold boot
                // and is NOT a failure).
                let outcome_ok = value
                    .get("outcome")
                    .and_then(|v| v.as_str())
                    .map(|s| s == "applied")
                    .unwrap_or(false);
                if outcome_ok {
                    Ok(())
                } else {
                    Err(value.to_string())
                }
            }
            Err(error) => Err(error.message()),
        }
    }
}

/// Drive the autostart plan against the live daemon on startup.
/// Logged outcomes are best-effort: any failure short-circuits to
/// a warning so the daemon's accept loop still comes up.
async fn run_startup_autostart(state: &ServerState, kernel_module_degraded: &BTreeSet<String>) {
    let resolver = match load_bundle_resolver(state) {
        Ok(r) => r,
        Err(error) => {
            tracing::warn!(
                error = %error.message(),
                "autostart skipped: bundle resolver unavailable",
            );
            return;
        }
    };
    let plan = autostart::build_autostart_plan(&resolver);
    if plan.vms.is_empty() {
        tracing::info!("autostart: nothing to do (empty plan)");
        return;
    }
    tracing::info!(
        net_vm_count = plan.net_vms().count(),
        workload_count = plan.workload_vms().count(),
        parallelism = state.config.autostart_parallelism,
        "autostart: dispatching plan",
    );
    let starter = Arc::new(BrokerVmStarter {
        state: Arc::new(state.clone()),
    });
    let config = autostart::AutostartConfig {
        parallelism: state.config.autostart_parallelism,
    };
    let report = autostart::execute_autostart_with_pre_degraded(
        &plan,
        starter,
        config,
        kernel_module_degraded,
    )
    .await;
    persist_autostart_report(&state.daemon_state_dir, &report);
    tracing::info!(
        started = report.started(),
        already_running = report.already_running(),
        failed = report.failed(),
        degraded = report.degraded(),
        "autostart: complete",
    );
    for outcome in &report.outcomes {
        match &outcome.outcome {
            autostart::Outcome::Started => {
                tracing::info!(vm = %outcome.vm, "autostart: started");
            }
            autostart::Outcome::AlreadyRunning => {
                tracing::info!(vm = %outcome.vm, "autostart: already-running (skipped)");
            }
            autostart::Outcome::NotAutostart => {
                tracing::debug!(vm = %outcome.vm, "autostart: vm not autostart-eligible");
            }
            autostart::Outcome::Failed { reason } => {
                tracing::warn!(vm = %outcome.vm, reason = %reason, "autostart: failed");
            }
            autostart::Outcome::Degraded { reason } => {
                tracing::warn!(vm = %outcome.vm, reason = %reason, "autostart: degraded");
            }
        }
    }

    // USBIP backend/proxy runners are attach-owned, not startup-owned:
    // `nixling usb attach --apply` first validates/binds/locks the
    // busid, then opens the firewall and starts the per-env runners.
    // Starting the listener here would expose TCP/3240 before the
    // per-busid ownership decision.
    let _ = resolver;
}

/// Drive the per-env usbipd spawn plan derived from the manifest.
/// Best-effort: any failure to dispatch a single env's spawn is
/// logged and the loop continues; the transitional NixOS units
/// remain in place to keep operators served while the daemon path
/// bakes in production.
#[allow(dead_code)]
async fn run_usbipd_perenv_autostart(
    state: &ServerState,
    resolver: &nixling_core::bundle_resolver::BundleResolver,
) {
    let specs = usbipd_perenv_autostart::derive_per_env_usbipd_specs(&resolver.manifest);
    if specs.is_empty() {
        tracing::debug!("usbipd-perenv autostart: no usbip-enabled envs in manifest");
        return;
    }
    tracing::info!(
        spec_count = specs.len(),
        env_count = specs.len() / 2,
        "usbipd-perenv autostart: dispatching per-env usbipd backend+proxy spawns",
    );
    let state_arc = Arc::new(state.clone());
    let report = tokio::task::spawn_blocking(move || {
        let spawner = BrokerPerEnvUsbipdSpawner {
            state: Arc::clone(&state_arc),
        };
        usbipd_perenv_autostart::execute_usbipd_perenv_autostart(&specs, &spawner)
    })
    .await
    .unwrap_or_else(|join_err| {
        tracing::warn!(error = ?join_err, "usbipd-perenv autostart: join task failed");
        usbipd_perenv_autostart::PerEnvUsbipdAutostartReport::default()
    });
    tracing::info!(
        spawned = report.spawned(),
        already_running = report.already_running(),
        skipped_pending_bundle = report.skipped_pending_bundle(),
        failed = report.failed(),
        "usbipd-perenv autostart: complete",
    );
    for entry in &report.specs {
        match &entry.outcome {
            usbipd_perenv_autostart::PerEnvUsbipdOutcome::Spawned => {
                tracing::info!(
                    env = %entry.env, role = ?entry.role, port = entry.backend_port,
                    "usbipd-perenv: spawned"
                );
            }
            usbipd_perenv_autostart::PerEnvUsbipdOutcome::AlreadyRunning => {
                tracing::info!(env = %entry.env, role = ?entry.role, "usbipd-perenv: already-running (idempotent)");
            }
            usbipd_perenv_autostart::PerEnvUsbipdOutcome::SkippedPendingBundle => {
                tracing::info!(
                    env = %entry.env, role = ?entry.role,
                    "usbipd-perenv: skipped — bundle has no sys-<env>-usbipd runner intent yet (transitional NixOS unit serves this env)"
                );
            }
            usbipd_perenv_autostart::PerEnvUsbipdOutcome::Failed { reason } => {
                tracing::warn!(env = %entry.env, role = ?entry.role, reason = %reason, "usbipd-perenv: failed");
            }
        }
    }
}

/// Live broker adapter for the per-env usbipd spawner trait.
/// Translates `BundleIntentMissing` into `SkippedPendingBundle` per the
/// trait contract so the transitional window does not fail-closed before
/// `processes-json.nix` grows the new DAGs.
struct BrokerPerEnvUsbipdSpawner {
    state: Arc<ServerState>,
}

impl usbipd_perenv_autostart::PerEnvUsbipdSpawner for BrokerPerEnvUsbipdSpawner {
    fn is_running(&self, vm_id: &str, role_id: &str) -> bool {
        if let Err(error) = self.state.pidfd_table.prune_dead_entries() {
            tracing::warn!(
                vm = %vm_id,
                role = %role_id,
                error = %error,
                "usbipd-perenv: failed to prune stale pidfd entries before is_running check"
            );
        }
        self.state.pidfd_table.contains(vm_id, role_id)
    }

    fn spawn(
        &self,
        spec: &usbipd_perenv_autostart::PerEnvUsbipdSpec,
    ) -> usbipd_perenv_autostart::PerEnvUsbipdOutcome {
        use usbipd_perenv_autostart::PerEnvUsbipdOutcome;
        let request = BrokerRequest::SpawnRunner(BrokerSpawnRunnerRequest {
            vm_id: VmId::new(spec.vm_id.clone()),
            role_id: RoleId::new(spec.role.role_id().to_owned()),
            role: usbipd_perenv_autostart::spawn_runner_role(spec),
            bundle_runner_intent_ref: BundleOpId::new(spec.intent_id()),
            runtime_allocations: vec![],
            tracing_span_id: None,
        });
        match dispatch_broker_request_with_fds_timeout(
            &self.state,
            request,
            Duration::from_secs(10),
        ) {
            Ok((BrokerResponse::SpawnRunner(response), received_fds)) => {
                let pidfd = match duplicate_received_fd(
                    &received_fds,
                    response.pidfd_index,
                    "duplicate per-env usbipd SpawnRunner pidfd",
                ) {
                    Ok(fd) => fd,
                    Err(error) => {
                        close_received_fds(&received_fds);
                        return PerEnvUsbipdOutcome::Failed {
                            reason: format!("pidfd-duplicate:{}", error.kind()),
                        };
                    }
                };
                if let Err(error) = self.state.pidfd_table.register(
                    spec.vm_id.clone(),
                    spec.role.role_id().to_owned(),
                    PidfdEntry {
                        pidfd,
                        pid: response.pid,
                        start_time_ticks: response.start_time_ticks,
                    },
                ) {
                    close_received_fds(&received_fds);
                    return PerEnvUsbipdOutcome::Failed {
                        reason: format!("pidfd-register:{error}"),
                    };
                }
                if let Err(error) = self.state.pidfd_table.snapshot() {
                    let _ = self
                        .state
                        .pidfd_table
                        .deregister(&spec.vm_id, spec.role.role_id());
                    close_received_fds(&received_fds);
                    return PerEnvUsbipdOutcome::Failed {
                        reason: format!("pidfd-snapshot:{error}"),
                    };
                }
                if let Err(error) = write_runner_snapshot(
                    &self.state,
                    &spec.vm_id,
                    spec.role.role_id(),
                    usbipd_perenv_autostart::spawn_runner_role(spec),
                    response.pid,
                    response.start_time_ticks,
                ) {
                    cleanup_vm_start_registration(&self.state, &spec.vm_id, spec.role.role_id());
                    close_received_fds(&received_fds);
                    return PerEnvUsbipdOutcome::Failed { reason: error };
                }
                close_received_fds(&received_fds);
                PerEnvUsbipdOutcome::Spawned
            }
            Ok((BrokerResponse::Error(error), received_fds)) => {
                close_received_fds(&received_fds);
                if error.kind == "bundle-intent-missing" {
                    PerEnvUsbipdOutcome::SkippedPendingBundle
                } else {
                    PerEnvUsbipdOutcome::Failed {
                        reason: format!("broker-error:{}", error.kind),
                    }
                }
            }
            Ok((other, received_fds)) => {
                close_received_fds(&received_fds);
                PerEnvUsbipdOutcome::Failed {
                    reason: format!("broker-protocol:{}", broker_response_kind(&other)),
                }
            }
            Err(error) => PerEnvUsbipdOutcome::Failed {
                reason: format!("broker-dispatch:{}", error.message()),
            },
        }
    }
}

fn resolve_runtime_identity(
    config: &DaemonConfig,
    allow_unprivileged_runtime_dir: bool,
) -> Result<RuntimeIdentity, TypedError> {
    if allow_unprivileged_runtime_dir {
        let daemon_uid = User::from_name(&config.daemon_user)
            .ok()
            .flatten()
            .map(|user| user.uid)
            .unwrap_or_else(unistd::getuid);
        let daemon_gid = Group::from_name(&config.daemon_group)
            .ok()
            .flatten()
            .map(|group| group.gid)
            .unwrap_or_else(unistd::getgid);
        return Ok(RuntimeIdentity {
            daemon_uid,
            daemon_gid,
            public_socket_gid: unistd::getgid(),
            expect_root_owned_parent: false,
        });
    }
    let daemon_user = User::from_name(&config.daemon_user)
        .map_err(io_wrap("lookup daemon user"))?
        .ok_or_else(|| TypedError::InternalConfig {
            detail: format!("daemon user {} does not exist", config.daemon_user),
        })?;
    let daemon_group = Group::from_name(&config.daemon_group)
        .map_err(io_wrap("lookup daemon group"))?
        .ok_or_else(|| TypedError::InternalConfig {
            detail: format!("daemon group {} does not exist", config.daemon_group),
        })?;
    let public_group = Group::from_name(&config.public_socket_group)
        .map_err(io_wrap("lookup public socket group"))?
        .ok_or_else(|| TypedError::InternalConfig {
            detail: format!(
                "public socket group {} does not exist",
                config.public_socket_group
            ),
        })?;
    Ok(RuntimeIdentity {
        daemon_uid: daemon_user.uid,
        daemon_gid: daemon_group.gid,
        public_socket_gid: public_group.gid,
        expect_root_owned_parent: true,
    })
}

fn validate_lock_parent(lock_path: &Path, identity: &RuntimeIdentity) -> Result<(), TypedError> {
    let parent = lock_path
        .parent()
        .ok_or_else(|| TypedError::InternalLockParentInvalid {
            path: lock_path.to_path_buf(),
            detail: "lock path has no parent directory".to_owned(),
        })?;
    let metadata =
        fs::symlink_metadata(parent).map_err(|err| TypedError::InternalLockParentInvalid {
            path: parent.to_path_buf(),
            detail: err.to_string(),
        })?;
    if metadata.file_type().is_symlink() {
        return Err(TypedError::InternalLockParentInvalid {
            path: parent.to_path_buf(),
            detail: "parent directory must not be a symlink".to_owned(),
        });
    }
    if !metadata.is_dir() {
        return Err(TypedError::InternalLockParentInvalid {
            path: parent.to_path_buf(),
            detail: "parent path is not a directory".to_owned(),
        });
    }
    // The production tmpfile rule installs /run/nixling as
    // `nixlingd:nixling 0750` so launcher users (members of `nixling`)
    // can traverse the directory to reach `/run/nixling/public.sock`
    // (mode 0660, group nixling). The previous validation expected the
    // root-owned 0755 shape; under the non-root daemon it would have
    // refused to start. The expected shape now matches the systemd
    // tmpfile contract: owner =
    // daemon_uid, group = public_socket_gid, mode = 0750. The
    // `--allow-unprivileged-runtime-dir` test flag still permits
    // running under the invoking user's uid/gid (and accepts either
    // 0755 or 0750 to keep ad-hoc `cargo test` scratch dirs valid).
    let (expected_uid, expected_gid, mode_acceptable): (u32, u32, fn(u32) -> bool) =
        if identity.expect_root_owned_parent {
            (
                identity.daemon_uid.as_raw(),
                identity.public_socket_gid.as_raw(),
                |m| m == 0o750,
            )
        } else {
            (unistd::getuid().as_raw(), unistd::getgid().as_raw(), |m| {
                m == 0o755 || m == 0o750
            })
        };
    let mode = metadata.permissions().mode() & 0o777;
    if metadata.uid() != expected_uid || metadata.gid() != expected_gid || !mode_acceptable(mode) {
        return Err(TypedError::InternalLockParentInvalid {
            path: parent.to_path_buf(),
            detail: format!(
                "expected uid:gid {}:{} mode 0750 (production) or 0755/0750 (test), got {}:{} mode {:04o}",
                expected_uid,
                expected_gid,
                metadata.uid(),
                metadata.gid(),
                mode
            ),
        });
    }
    Ok(())
}

fn ensure_locks_dir(path: &Path, identity: &RuntimeIdentity) -> Result<(), TypedError> {
    fs::create_dir_all(path).map_err(|err| TypedError::InternalIo {
        context: format!("create locks dir {}", path.display()),
        detail: err.to_string(),
    })?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o750)).map_err(|err| {
        TypedError::InternalIo {
            context: format!("chmod locks dir {}", path.display()),
            detail: err.to_string(),
        }
    })?;
    if identity.expect_root_owned_parent && unistd::geteuid().is_root() {
        unistd::chown(path, Some(Uid::from_raw(0)), Some(identity.daemon_gid))
            .map_err(io_wrap("chown locks dir"))?;
    }
    Ok(())
}

fn acquire_state_lock(path: &Path, identity: &RuntimeIdentity) -> Result<File, TypedError> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|err| TypedError::InternalIo {
            context: format!("open daemon lock {}", path.display()),
            detail: err.to_string(),
        })?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o640)).map_err(|err| {
        TypedError::InternalIo {
            context: format!("chmod daemon lock {}", path.display()),
            detail: err.to_string(),
        }
    })?;
    if identity.expect_root_owned_parent && unistd::geteuid().is_root() {
        unistd::chown(path, Some(Uid::from_raw(0)), Some(identity.daemon_gid))
            .map_err(io_wrap("chown daemon lock"))?;
    }

    let lock = libc::flock {
        l_type: libc::F_WRLCK as i16,
        l_whence: libc::SEEK_SET as i16,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };
    match fcntl(file.as_raw_fd(), FcntlArg::F_OFD_SETLK(&lock)) {
        Ok(_) => Ok(file),
        Err(nix::errno::Errno::EAGAIN) | Err(nix::errno::Errno::EACCES) => {
            Err(TypedError::InternalAlreadyRunning {
                path: path.to_path_buf(),
            })
        }
        Err(err) => Err(TypedError::InternalIo {
            context: format!("acquire OFD lock {}", path.display()),
            detail: err.to_string(),
        }),
    }
}

fn bind_public_socket(path: &Path, identity: &RuntimeIdentity) -> Result<Socket, TypedError> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_socket() {
            fs::remove_file(path).map_err(|err| TypedError::InternalIo {
                context: format!("remove stale socket {}", path.display()),
                detail: err.to_string(),
            })?;
        } else {
            return Err(TypedError::InternalIo {
                context: format!("bind public socket {}", path.display()),
                detail: "existing path is not a socket".to_owned(),
            });
        }
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| TypedError::InternalIo {
            context: format!("create public socket parent {}", parent.display()),
            detail: err.to_string(),
        })?;
    }

    let socket =
        Socket::new(Domain::UNIX, Type::from(libc::SOCK_SEQPACKET), None).map_err(|err| {
            TypedError::InternalIo {
                context: format!("create public seqpacket socket {}", path.display()),
                detail: err.to_string(),
            }
        })?;
    let address = SockAddr::unix(path).map_err(|err| TypedError::InternalIo {
        context: format!("encode public socket path {}", path.display()),
        detail: err.to_string(),
    })?;
    socket
        .bind(&address)
        .map_err(|err| TypedError::InternalIo {
            context: format!("bind public socket {}", path.display()),
            detail: err.to_string(),
        })?;
    socket.listen(128).map_err(|err| TypedError::InternalIo {
        context: format!("listen on public socket {}", path.display()),
        detail: err.to_string(),
    })?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o660)).map_err(|err| {
        TypedError::InternalIo {
            context: format!("chmod public socket {}", path.display()),
            detail: err.to_string(),
        }
    })?;
    // Always chgrp the socket to `public_socket_gid` (i.e. `nixling` in
    // production). The previous `geteuid().is_root()` gate meant the
    // non-root systemd unit (User=nixlingd, SupplementaryGroups=nixling)
    // left the socket with group `nixlingd`, which made launcher users
    // unable to connect even though they have a seat in
    // the supplementary group. `chown(path, None, Some(group))` is
    // permitted for the file owner whenever the target gid is one of
    // the caller's groups (real, effective, or supplementary), which
    // is exactly the production case. The test path still works:
    // `expect_root_owned_parent` is false, so we skip the chown there
    // and the socket inherits the caller's primary gid.
    if identity.expect_root_owned_parent {
        unistd::chown(path, None, Some(identity.public_socket_gid))
            .map_err(io_wrap("chown public socket"))?;
    }
    Ok(socket)
}

fn drop_privileges_if_root(identity: &RuntimeIdentity) -> Result<(), TypedError> {
    if !identity.expect_root_owned_parent || !unistd::geteuid().is_root() {
        return Ok(());
    }
    unistd::setgroups(&[identity.daemon_gid]).map_err(io_wrap("setgroups"))?;
    unistd::setgid(identity.daemon_gid).map_err(io_wrap("setgid"))?;
    unistd::setuid(identity.daemon_uid).map_err(io_wrap("setuid"))?;
    Ok(())
}

/// Write the daemon's canonicalized binary path + version + start-time
/// to the runtime `version` file on startup. The production public socket
/// lives in `/run/nixling`, so production writes `/run/nixling/version`; test
/// listeners write beside their redirected public socket.
/// This lets the CLI's `daemon_version::compute_restart_status` compute the
/// `[pending restart]` signal post-restart. Failures are logged
/// to stderr and non-fatal — the absence of the version file
/// surfaces in the CLI as `DaemonRestartStatus::DaemonNotRunning`,
/// which is a reasonable degraded shape.
fn write_daemon_version_file(config: &DaemonConfig) {
    let binary_path = match std::env::current_exe().and_then(std::fs::canonicalize) {
        Ok(p) => p.to_string_lossy().into_owned(),
        Err(err) => {
            eprintln!("nixlingd: could not canonicalize daemon binary path: {err}");
            return;
        }
    };
    let started_at = chrono_like_rfc3339();
    let payload = daemon_version::DaemonVersionFile {
        server_version: DEFAULT_SERVER_VERSION.to_owned(),
        binary_path,
        started_at,
        protocol_version: nixling_ipc::PROTOCOL_VERSION,
    };
    let json = match serde_json::to_vec_pretty(&payload) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("nixlingd: could not serialize daemon version: {err}");
            return;
        }
    };
    let path = daemon_version_file_path(config);
    if let Some(parent) = path.parent()
        && let Err(err) = std::fs::create_dir_all(parent)
    {
        eprintln!(
            "nixlingd: could not create {} for version file: {err}",
            parent.display()
        );
        return;
    }
    let tmp = path.with_extension("version.tmp");
    if let Err(err) = std::fs::write(&tmp, &json) {
        eprintln!("nixlingd: could not write {}: {err}", tmp.display());
        return;
    }
    if let Err(err) = std::fs::rename(&tmp, path) {
        eprintln!("nixlingd: could not rename version file into place: {err}");
    }
}

fn daemon_version_file_path(config: &DaemonConfig) -> std::path::PathBuf {
    config
        .public_socket_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("/run/nixling"))
        .join("version")
}

/// Tiny RFC-3339 UTC formatter (`YYYY-MM-DDTHH:MM:SSZ`) so we can
/// stamp `DaemonVersionFile.started_at` without pulling in `chrono`
/// as a new top-level dependency. The daemon's startup is the only
/// caller; precision to the second is sufficient.
fn chrono_like_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Civil-time inverse via Howard Hinnant's days-from-civil.
    let days = (secs / 86_400) as i64;
    let secs_of_day = (secs % 86_400) as u32;
    let (y, m, d) = days_to_ymd(days);
    let h = secs_of_day / 3600;
    let mi = (secs_of_day % 3600) / 60;
    let s = secs_of_day % 60;
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Howard Hinnant's `civil_from_days`: given a days-since-1970-01-01
/// integer, return `(year, month, day)` in the proleptic Gregorian
/// calendar. Adapted for u32 → tuple.
fn days_to_ymd(days_since_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if m <= 2 { (y + 1) as i32 } else { y as i32 };
    (year, m, d)
}

/// Test-only seam letting an accept-loop test substitute the exec owner body so
/// it can HOLD the owner session open. Without it, `run_exec_owner` fails fast
/// at `load_bundle_resolver` under default test state and returns immediately,
/// so a test could not distinguish an off-loop spawn from an inline call. The
/// hook is consulted at the top of `run_exec_owner` (the owner body itself), so
/// an inline `handle_connection` would block in it on the accept-loop thread; it
/// is `None` in every build except a test that installs it, and production
/// builds compile it out entirely.
#[cfg(test)]
mod exec_owner_test_hook {
    use std::sync::{Arc, Mutex, OnceLock};

    pub(crate) type Hook = Arc<dyn Fn() + Send + Sync>;

    fn slot() -> &'static Mutex<Option<Hook>> {
        static HOOK: OnceLock<Mutex<Option<Hook>>> = OnceLock::new();
        HOOK.get_or_init(|| Mutex::new(None))
    }

    pub(crate) fn set(hook: Hook) {
        *slot().lock().expect("exec owner hook lock") = Some(hook);
    }

    pub(crate) fn clear() {
        *slot().lock().expect("exec owner hook lock") = None;
    }

    pub(crate) fn active() -> Option<Hook> {
        slot().lock().expect("exec owner hook lock").clone()
    }
}

/// Thin wrapper used by the `options.once` test path and by direct
/// unit-test callers: authorizes the peer (SO_PEERCRED), then runs the
/// authorized connection body. The production accept loop authorizes the
/// peer itself (before admission) and calls
/// [`handle_connection_authorized`] directly.
fn handle_connection(
    stream: Socket,
    state: &ServerState,
    permit: Option<concurrency::ConnPermit>,
) -> Result<(), TypedError> {
    let peer = match authorize_peer(&stream, state) {
        Ok(peer) => peer,
        Err(error) => {
            let _ = write_json_frame(&stream, &wire::hello_rejected(&error));
            // Authz ran before the hello read; drain the unread hello so the
            // close is graceful and the peer receives the rejection frame.
            drain_rejected_peer_input(&stream);
            return Err(error);
        }
    };
    handle_connection_authorized(stream, state, peer, permit)
}

/// Connection body for an already-authorized peer. Reads the hello frame
/// (deadlined), negotiates the wire version, then serves requests on a
/// deadlined per-frame read loop. An attached `Exec::Start` takes over
/// the connection on a spawned owner thread, moving the admission
/// `permit` with it so the in-flight slot is held until the exec session
/// (owner) terminates.
fn handle_connection_authorized(
    stream: Socket,
    state: &ServerState,
    peer: PeerIdentity,
    permit: Option<concurrency::ConnPermit>,
) -> Result<(), TypedError> {
    // Bound the wait for the initial hello so a connected-but-silent peer
    // cannot occupy a handler slot indefinitely.
    set_frame_read_deadline(&stream, Some(HELLO_READ_DEADLINE));
    let hello_bytes = match read_frame(&stream) {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = write_json_frame(&stream, &wire::hello_rejected(&error));
            return Err(error);
        }
    };
    let hello = wire::parse_hello(&hello_bytes).inspect_err(|error| {
        let _ = write_json_frame(&stream, &wire::hello_rejected(error));
    })?;
    let selected_version = match wire::negotiate_version(
        hello.client_version.as_str(),
        &state.config.accepted_client_version_range,
        &state.config.server_version,
    ) {
        Ok(version) => version,
        Err(error) => {
            let _ = write_json_frame(&stream, &wire::hello_rejected(&error));
            return Err(error);
        }
    };
    let capabilities = vec![
        KnownFeatureFlag::TypedErrors.wire_value(),
        KnownFeatureFlag::StatusCheckBridges.wire_value(),
        KnownFeatureFlag::ExportBrokerAudit.wire_value(),
    ];
    let hello_ok = wire::hello_ok(
        &state.config.server_version,
        &selected_version,
        &capabilities,
    )?;
    write_json_frame(&stream, &hello_ok)?;

    loop {
        // Bound each request read so a half-open / slow-loris peer frees
        // its handler slot instead of pinning it forever.
        set_frame_read_deadline(&stream, Some(REQUEST_READ_DEADLINE));
        let frame = match read_frame(&stream) {
            Ok(bytes) => bytes,
            Err(TypedError::InternalIo { .. }) => return Ok(()),
            Err(error) => {
                let _ = write_json_frame(&stream, &wire::error_frame(&error));
                return Err(error);
            }
        };
        let request = match wire::parse_request(&frame) {
            Ok(request) => request,
            Err(error) => {
                let _ = write_json_frame(&stream, &wire::error_frame(&error));
                continue;
            }
        };
        // Exec takes over the connection as the long-lived owner connection.
        // Admin (SO_PEERCRED) is verified here, BEFORE any session work; then
        // the connection + a cheap ServerState clone move to a SPAWNED owner
        // handler. The admission permit moves with it so the in-flight slot
        // is held for the lifetime of the exec session, not just the read.
        if let wire::Request::Exec(op) = &request {
            if !matches!(peer.role, PeerRole::Admin) {
                let error = TypedError::AuthzNotAdmin {
                    verb: "exec".to_owned(),
                };
                let _ = write_json_frame(&stream, &wire::error_frame(&error));
                continue;
            }
            if matches!(op, public_wire::ExecOp::Start(_)) {
                // Recover the establishing op's envelope `opId` so the Start
                // reply (and any establish error) echoes it for client
                // correlation. Detached Start is also handled by the owner body,
                // but returns after one ExecCreate instead of reserving a
                // session slot or entering the attached FSM.
                let first_op_id = wire::exec_op_id(&frame);
                let owner_state = state.clone();
                let owner_peer = peer.clone();
                let op = op.clone();
                // The exec owner blocks on the PTY indefinitely; clear the
                // request read deadline before handing off the socket.
                set_frame_read_deadline(&stream, None);
                let owner_permit = permit;
                match std::thread::Builder::new()
                    .name("nixling-exec-owner".to_owned())
                    .spawn(move || {
                        run_exec_owner(
                            stream,
                            owner_state,
                            owner_peer,
                            first_op_id,
                            op,
                            owner_permit,
                        );
                    }) {
                    Ok(_) => return Ok(()),
                    Err(err) => {
                        return Err(TypedError::InternalIo {
                            context: "spawn exec owner handler".to_owned(),
                            detail: err.to_string(),
                        });
                    }
                }
            }
        }
        if let wire::Request::Shell(op) = &request {
            if !matches!(peer.role, PeerRole::Admin) {
                let error = TypedError::AuthzNotAdmin {
                    verb: "shell".to_owned(),
                };
                let _ = write_json_frame(&stream, &wire::error_frame(&error));
                continue;
            }
            if matches!(op, public_wire::ShellOp::Attach(_)) {
                let first_op_id = wire::shell_op_id(&frame);
                let owner_state = state.clone();
                let owner_peer = peer.clone();
                let op = op.clone();
                set_frame_read_deadline(&stream, None);
                let owner_permit = permit;
                match std::thread::Builder::new()
                    .name("nixling-shell-owner".to_owned())
                    .spawn(move || {
                        run_shell_owner(
                            stream,
                            owner_state,
                            owner_peer,
                            first_op_id,
                            op,
                            owner_permit,
                        );
                    }) {
                    Ok(_) => return Ok(()),
                    Err(err) => {
                        return Err(TypedError::InternalIo {
                            context: "spawn shell owner handler".to_owned(),
                            detail: err.to_string(),
                        });
                    }
                }
            }
        }
        // Gateway display operations can perform provider/relay orchestration.
        // Hand them off the serial accept loop just like exec owner sessions.
        if let wire::Request::GatewayDisplay(op) = &request {
            if gateway_display_op_requires_admin(op) && !matches!(peer.role, PeerRole::Admin) {
                let error = TypedError::AuthzNotAdmin {
                    verb: "gatewayDisplay".to_owned(),
                };
                let _ = write_json_frame(&stream, &wire::error_frame(&error));
                continue;
            }
            let owner_state = state.clone();
            let owner_peer = peer.clone();
            let op = op.clone();
            match std::thread::Builder::new()
                .name("nixling-gateway-display".to_owned())
                .spawn(move || {
                    run_gateway_display_owner(stream, owner_state, owner_peer, op);
                }) {
                Ok(_) => return Ok(()),
                Err(err) => {
                    return Err(TypedError::InternalIo {
                        context: "spawn gateway display handler".to_owned(),
                        detail: err.to_string(),
                    });
                }
            }
        }
        let response = match dispatch_request(state, &peer, request) {
            Ok(value) => value,
            Err(error) => serde_json::to_value(wire::error_frame(&error)).map_err(|err| {
                TypedError::InternalIo {
                    context: "serialize error response".to_owned(),
                    detail: err.to_string(),
                }
            })?,
        };
        write_json_frame(&stream, &response)?;
    }
}

fn run_gateway_display_owner(
    stream: Socket,
    state: ServerState,
    peer: PeerIdentity,
    op: public_wire::GatewayDisplayOp,
) {
    let response = match dispatch_request(&state, &peer, wire::Request::GatewayDisplay(op)) {
        Ok(value) => value,
        Err(error) => serde_json::to_value(wire::error_frame(&error))
            .unwrap_or_else(|_| json!({ "type": "error" })),
    };
    let _ = write_json_frame(&stream, &response);
}

fn authorize_peer(stream: &Socket, state: &ServerState) -> Result<PeerIdentity, TypedError> {
    // Peer-identity resolution order:
    //   1. the `#[cfg(test)]` in-process injection slot (lib unit tests that
    //      drive `handle_connection` directly),
    //   2. the `NIXLINGD_TEST_PEER_*` env vars (integration tests that spawn
    //      the real daemon binary and pass them via `Command::env`; reading
    //      env is safe under edition 2024),
    //   3. the real `SO_PEERCRED` of the connected socket (production).
    let peer_override = match peer_override_injected() {
        Some(peer) => peer,
        None => match peer_override_from_env()? {
            Some(peer) => peer,
            None => {
                let peer =
                    getsockopt(stream, PeerCredentials).map_err(io_wrap("read SO_PEERCRED"))?;
                PeerOverride {
                    uid: peer.uid() as u32,
                    gid: peer.gid() as u32,
                    username: None,
                    groups: None,
                }
            }
        },
    };
    let uid = peer_override.uid;
    let _gid = peer_override.gid;
    let username = peer_override
        .username
        .or_else(|| get_user_by_uid(uid).map(|user| user.name().to_string_lossy().into_owned()));
    let _supplementary_groups = if let Some(groups) = peer_override.groups {
        groups
    } else if let Some(user) = get_user_by_uid(uid) {
        get_user_groups(user.name(), user.primary_group_id())
            .into_iter()
            .flatten()
            .map(|group| group.name().to_string_lossy().into_owned())
            .collect()
    } else {
        Vec::new()
    };

    if uid == state.daemon_uid {
        return Err(TypedError::AuthzNotALauncher { peer_uid: uid });
    }

    let is_launcher = username
        .as_ref()
        .map(|name| {
            state
                .config
                .launcher_users
                .iter()
                .any(|launcher| launcher == name)
        })
        .unwrap_or(false);
    if !is_launcher {
        return Err(TypedError::AuthzNotALauncher { peer_uid: uid });
    }

    let role = if username
        .as_ref()
        .map(|name| state.config.admin_users.iter().any(|admin| admin == name))
        .unwrap_or(false)
    {
        PeerRole::Admin
    } else {
        PeerRole::Launcher
    };

    Ok(PeerIdentity { role, uid })
}

fn verb_requires_admin(verb: &str) -> bool {
    matches!(
        verb,
        "vmStart"
            | "vmStop"
            | "vmRestart"
            | "switch"
            | "boot"
            | "test"
            | "rollback"
            | "gc"
            | "keysRotate"
            | "trust"
            | "rotateKnownHost"
            | "usbipBind"
            | "usbipUnbind"
            | "storeVerify"
            | "migrate"
            | "hostPrepare"
            | "hostDestroy"
            | "hostInstall"
            | "hostReconcile"
            | "readGuestConfig"
            | "exec"
            | "shell"
    )
}

fn gateway_display_op_requires_admin(op: &public_wire::GatewayDisplayOp) -> bool {
    matches!(
        op,
        public_wire::GatewayDisplayOp::Start(_) | public_wire::GatewayDisplayOp::Stop(_)
    )
}

fn gateway_display_peer_principal(peer: &PeerIdentity) -> nixling_constellation_core::PrincipalId {
    nixling_constellation_core::PrincipalId::parse(format!("uid-{}", peer.uid))
        .expect("trusted display principal derived from numeric uid is valid")
}

fn gateway_display_peer_principal_string(peer: &PeerIdentity) -> String {
    gateway_display_peer_principal(peer).to_string()
}

fn dispatch_request(
    state: &ServerState,
    peer: &PeerIdentity,
    request: wire::Request,
) -> Result<Value, TypedError> {
    let verb = request.verb_name();
    if verb_requires_admin(verb) && !matches!(peer.role, PeerRole::Admin) {
        return Err(TypedError::AuthzNotAdmin {
            verb: verb.to_owned(),
        });
    }
    // Acquire the op lock for this verb ONCE, here at the dispatch
    // boundary on the worker thread (never the accept loop). Read-only
    // verbs take no lock; per-VM mutating verbs serialize on the VM; a
    // global verb is mutually exclusive with all per-VM ops. The guard is
    // held across the whole op (DAG + rollback + cleanup); inner
    // stop/start helpers invoked by restart/rollback do NOT re-acquire it,
    // so there is no nested self-deadlock.
    let _op_lock = state.op_locks.acquire(&request.lock_class());
    dispatch_request_locked(state, peer, request)
}

/// Verb dispatch body executed under the op lock already acquired by
/// [`dispatch_request`]. Restart/rollback paths call the inner
/// `dispatch_broker_*` helpers directly (never re-entering
/// `dispatch_request`), so the lock is held exactly once for the op.
fn dispatch_request_locked(
    state: &ServerState,
    peer: &PeerIdentity,
    request: wire::Request,
) -> Result<Value, TypedError> {
    match request {
        wire::Request::List(request) => dispatch_list(state, request),
        wire::Request::Status(request) => dispatch_status(state, request),
        wire::Request::Audit(request) => dispatch_audit(state, peer, request),
        wire::Request::HostCheck(request) => dispatch_host_check(state, request),
        wire::Request::AuthStatus => Ok(dispatch_auth_status(state, peer)),
        wire::Request::KeysList => dispatch_keys_list(state),
        wire::Request::KeysShow(request) => dispatch_keys_show(state, request),
        // Mutating-verb apply dispatch is now fully direct. The backlog
        // verbs route from these request arms straight to their
        // `dispatch_broker_<verb>` helpers, and the HostInstall/Migrate
        // paths stay on their dedicated broker helpers.
        //
        // The old shared `dispatch_mutating_verb` split no longer
        // applies in nixlingd; only `mutating_verb_preflight` remains
        // to emit the typed InvalidRequest / dry-run-planned envelope
        // before apply dispatch runs.
        wire::Request::VmStart(req) => dispatch_broker_vm_start(state, req),
        wire::Request::VmStop(req) => {
            dispatch_broker_vm_stop_as(state, req, broker_caller_role_for_peer(peer))
        }
        wire::Request::VmRestart(req) => {
            dispatch_broker_vm_restart_as(state, req, broker_caller_role_for_peer(peer))
        }
        wire::Request::Switch(req) => dispatch_broker_switch(state, req),
        wire::Request::Boot(req) => dispatch_broker_boot(state, req),
        wire::Request::Test(req) => dispatch_broker_test(state, req),
        wire::Request::Rollback(req) => dispatch_broker_rollback(state, req),
        wire::Request::Gc(req) => dispatch_broker_gc(state, req),
        wire::Request::KeysRotate(req) => dispatch_broker_keys_rotate(state, req),
        wire::Request::Trust(req) => dispatch_broker_trust(state, req),
        wire::Request::RotateKnownHost(req) => dispatch_broker_rotate_known_host(state, req),
        wire::Request::UsbipBind(req) => dispatch_broker_usbip_bind(state, req),
        wire::Request::UsbipUnbind(req) => dispatch_broker_usbip_unbind(state, req),
        wire::Request::UsbipProbe => dispatch_broker_usbip_probe(state),
        wire::Request::StoreVerify(req) => dispatch_broker_store_verify(state, req),
        wire::Request::Migrate(req) => dispatch_broker_run_migrate(state, req),
        wire::Request::HostPrepare(req) => dispatch_broker_host_prepare(state, req),
        wire::Request::HostDestroy(req) => dispatch_broker_host_destroy(state, req),
        wire::Request::HostInstall(req) => dispatch_broker_run_host_install(state, req),
        wire::Request::HostReconcile(req) => dispatch_broker_host_reconcile(state, req),
        wire::Request::ReadGuestConfig(req) => dispatch_read_guest_config(state, req),
        // Attached Exec::Start is intercepted in `handle_connection` (it takes
        // over the connection as the owner connection and is handled on a
        // spawned worker off the serial accept loop). Detached management ops
        // are ordinary one-shot requests.
        wire::Request::Exec(op) => dispatch_exec_management(state, peer, op),
        wire::Request::Shell(op) => dispatch_shell_management(state, op),
        wire::Request::GatewayDisplay(op) => dispatch_gateway_display(state, peer, op),
    }
}

fn dispatch_gateway_display(
    state: &ServerState,
    peer: &PeerIdentity,
    op: public_wire::GatewayDisplayOp,
) -> Result<Value, TypedError> {
    let response = match op {
        public_wire::GatewayDisplayOp::Start(args) => {
            if !matches!(peer.role, PeerRole::Admin) {
                return Err(TypedError::AuthzNotAdmin {
                    verb: "gatewayDisplay".to_owned(),
                });
            }
            let target = parse_gateway_display_lifecycle_target(
                &args.target,
                &args.operation_id,
                &args.principal,
            )?;
            let lifecycle_state = block_on_future(state.gateway_display.lifecycle.start(&target))
                .map_err(gateway_error_to_typed)?;
            public_wire::GatewayDisplayOpResponse::Start(public_wire::GatewayDisplayStartResult {
                target: args.target,
                state: lifecycle_state,
            })
        }
        public_wire::GatewayDisplayOp::Stop(args) => {
            if !matches!(peer.role, PeerRole::Admin) {
                return Err(TypedError::AuthzNotAdmin {
                    verb: "gatewayDisplay".to_owned(),
                });
            }
            let target = parse_gateway_display_lifecycle_target(
                &args.target,
                &args.operation_id,
                &args.principal,
            )?;
            close_gateway_sessions_for_target(state, &args.target)?;
            let lifecycle_state = block_on_future(state.gateway_display.lifecycle.stop(&target))
                .map_err(gateway_error_to_typed)?;
            public_wire::GatewayDisplayOpResponse::Stop(public_wire::GatewayDisplayStopResult {
                target: args.target,
                state: lifecycle_state,
            })
        }
        public_wire::GatewayDisplayOp::Open(args) => {
            let target_text = args.target.clone();
            let target =
                TargetName::parse(&args.target).map_err(|err| TypedError::WireInvalidFrame {
                    detail: format!("gatewayDisplay target parse failed: {err}"),
                })?;
            let operation_id = nixling_constellation_core::OperationId::parse(args.operation_id)
                .map_err(|err| TypedError::WireInvalidFrame {
                    detail: format!("gatewayDisplay operation_id invalid: {err}"),
                })?;
            let principal = gateway_display_peer_principal(peer);
            let app =
                AppCommand::new(args.app_argv).ok_or_else(|| TypedError::WireInvalidFrame {
                    detail:
                        "gatewayDisplay app_argv must be non-empty and contain no empty arguments"
                            .to_owned(),
                })?;
            validate_gateway_display_open_preflight(state)?;
            let target_key = TargetKey {
                realm: target.realm.target_form(),
                workload: target.workload.as_str().to_owned(),
            };
            let seed = ContextSeed {
                realm: target.realm,
                operation_id,
                principal,
                node: target.node,
                workload: target.workload,
            };
            let owner_principal = seed.principal.to_string();
            gateway_display_gc(state);
            let open = block_on_future(state.gateway_display.orchestrator.open(
                target_key,
                seed,
                app,
                args.request_hash,
            ))
            .map_err(gateway_error_to_typed)?;
            let session_id = open.session_id.to_string();
            let mut sessions =
                state
                    .gateway_display
                    .sessions
                    .lock()
                    .map_err(|_| TypedError::InternalIo {
                        context: "lock gateway display sessions".to_owned(),
                        detail: "mutex poisoned".to_owned(),
                    })?;
            match sessions.entry(session_id.clone()) {
                Entry::Occupied(_) => {}
                Entry::Vacant(slot) => {
                    slot.insert(GatewayDisplaySession {
                        target: target_text,
                        principal: owner_principal,
                        open,
                        opened_at: Instant::now(),
                    });
                }
            }
            public_wire::GatewayDisplayOpResponse::Open(public_wire::GatewayDisplayOpenResult {
                session_id,
                state: "running".to_owned(),
            })
        }
        public_wire::GatewayDisplayOp::Close(args) => {
            gateway_display_gc(state);
            let peer_principal = gateway_display_peer_principal_string(peer);
            let session =
                {
                    let mut sessions = state.gateway_display.sessions.lock().map_err(|_| {
                        TypedError::InternalIo {
                            context: "lock gateway display sessions".to_owned(),
                            detail: "mutex poisoned".to_owned(),
                        }
                    })?;
                    let unauthorized = sessions.get(&args.session_id).is_some_and(|session| {
                        !matches!(peer.role, PeerRole::Admin) && session.principal != peer_principal
                    });
                    if unauthorized {
                        return Err(TypedError::AuthzNotAdmin {
                            verb: "gatewayDisplay close".to_owned(),
                        });
                    }
                    sessions.remove(&args.session_id)
                };
            let closed = if let Some(session) = session {
                if let Err(err) =
                    block_on_future(state.gateway_display.orchestrator.close(&session.open))
                {
                    tracing::warn!(error = %err, session_id = %args.session_id, "gateway display close cleanup failed");
                }
                true
            } else {
                false
            };
            public_wire::GatewayDisplayOpResponse::Close(public_wire::GatewayDisplayCloseResult {
                closed,
            })
        }
        public_wire::GatewayDisplayOp::List(args) => {
            gateway_display_gc(state);
            let peer_principal = gateway_display_peer_principal_string(peer);
            let target_by_id: HashMap<String, String> = state
                .gateway_display
                .sessions
                .lock()
                .map_err(|_| TypedError::InternalIo {
                    context: "lock gateway display sessions".to_owned(),
                    detail: "mutex poisoned".to_owned(),
                })?
                .values()
                .map(|session| (session.open.session_id.to_string(), session.target.clone()))
                .collect();
            let sessions = state
                .gateway_display
                .orchestrator
                .list_sessions()
                .map_err(gateway_error_to_typed)?
                .into_iter()
                .filter_map(|summary| {
                    let session_id = summary.session_id.to_string();
                    let target = target_by_id.get(&session_id)?.clone();
                    if args.target.as_ref().is_some_and(|wanted| wanted != &target) {
                        return None;
                    }
                    if !matches!(peer.role, PeerRole::Admin)
                        && summary.peer_principal.as_str() != peer_principal.as_str()
                    {
                        return None;
                    }
                    let state = format!("{:?}", summary.state).to_ascii_lowercase();
                    Some(public_wire::GatewayDisplaySessionSummary {
                        session_id,
                        target,
                        state,
                    })
                })
                .collect();
            public_wire::GatewayDisplayOpResponse::List(public_wire::GatewayDisplayListResult {
                sessions,
            })
        }
        public_wire::GatewayDisplayOp::ListDetailed(args) => {
            gateway_display_gc(state);
            let peer_principal = gateway_display_peer_principal_string(peer);
            let target_by_id: HashMap<String, String> = state
                .gateway_display
                .sessions
                .lock()
                .map_err(|_| TypedError::InternalIo {
                    context: "lock gateway display sessions".to_owned(),
                    detail: "mutex poisoned".to_owned(),
                })?
                .values()
                .map(|session| (session.open.session_id.to_string(), session.target.clone()))
                .collect();
            let sessions = state
                .gateway_display
                .orchestrator
                .list_sessions()
                .map_err(gateway_error_to_typed)?
                .into_iter()
                .filter_map(|summary| {
                    let session_id = summary.session_id.to_string();
                    let target = target_by_id.get(&session_id)?.clone();
                    if args.target.as_ref().is_some_and(|wanted| wanted != &target) {
                        return None;
                    }
                    if !matches!(peer.role, PeerRole::Admin)
                        && summary.peer_principal.as_str() != peer_principal.as_str()
                    {
                        return None;
                    }
                    let state = format!("{:?}", summary.state).to_ascii_lowercase();
                    Some(public_wire::GatewayDisplaySessionDetail {
                        session_id,
                        target,
                        state,
                        operation_id: summary.operation_id.to_string(),
                        principal: summary.peer_principal.to_string(),
                    })
                })
                .collect();
            public_wire::GatewayDisplayOpResponse::ListDetailed(
                public_wire::GatewayDisplayListDetailedResult { sessions },
            )
        }
    };
    let mut value = serde_json::to_value(response).map_err(|err| TypedError::InternalIo {
        context: "serialize gatewayDisplay response".to_owned(),
        detail: err.to_string(),
    })?;
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "type".to_owned(),
            Value::String("gatewayDisplayResponse".to_owned()),
        );
    }
    Ok(value)
}

fn parse_gateway_display_lifecycle_target(
    target: &str,
    operation_id: &str,
    principal: &str,
) -> Result<TargetName, TypedError> {
    let target = TargetName::parse(target).map_err(|err| TypedError::WireInvalidFrame {
        detail: format!("gatewayDisplay target parse failed: {err}"),
    })?;
    let _operation_id =
        nixling_constellation_core::OperationId::parse(operation_id).map_err(|err| {
            TypedError::WireInvalidFrame {
                detail: format!("gatewayDisplay operation_id invalid: {err}"),
            }
        })?;
    let _principal = nixling_constellation_core::PrincipalId::parse(principal).map_err(|err| {
        TypedError::WireInvalidFrame {
            detail: format!("gatewayDisplay principal invalid: {err}"),
        }
    })?;
    Ok(target)
}

fn validate_gateway_display_open_preflight(state: &ServerState) -> Result<(), TypedError> {
    if let Some(preflight) = state.gateway_display.preflight.as_ref() {
        let guard_config = GatewayFileConfig {
            gateway: String::new(),
            realm: String::new(),
            state_dir: None,
            credential_path: None,
            seal_key_path: None,
            allow_host_relay_credentials: preflight.allow_host_relay_credentials,
            relay: GatewayRelayFileConfig::default(),
            aca: GatewayAcaFileConfig::default(),
            display: GatewayDisplayFileConfig::default(),
        };
        validate_gateway_host_relay_transition_guard(&guard_config)?;
        let waypipe_socket_path = preflight.waypipe_socket_path.as_ref().ok_or_else(|| {
            gateway_display_config_error(
                "gateway config field display.waypipeSocket is required; set it to the operator Waypipe receiver Unix socket path",
            )
        })?;
        validate_waypipe_receiver_socket(waypipe_socket_path)?;
    }
    Ok(())
}

fn close_gateway_sessions_for_target(state: &ServerState, target: &str) -> Result<(), TypedError> {
    gateway_display_gc(state);
    let sessions: Vec<(String, GatewayDisplaySession)> = state
        .gateway_display
        .sessions
        .lock()
        .map_err(|_| TypedError::InternalIo {
            context: "lock gateway display sessions".to_owned(),
            detail: "mutex poisoned".to_owned(),
        })?
        .extract_if(|_, session| session.target == target)
        .collect();
    for (id, session) in sessions {
        if let Err(err) = block_on_future(state.gateway_display.orchestrator.close(&session.open)) {
            tracing::warn!(error = %err, session_id = %id, target = %target, "gateway display target cleanup failed");
        }
    }
    Ok(())
}

fn gateway_error_to_typed(error: GatewayError) -> TypedError {
    tracing::warn!(
        gateway_error = error.slug(),
        "gateway display request failed"
    );
    TypedError::GatewayDisplayUnavailable {
        detail: error.slug().to_owned(),
    }
}

fn gateway_display_gc(state: &ServerState) {
    let now = Instant::now();
    let expired: Vec<(String, GatewayDisplaySession)> = match state.gateway_display.sessions.lock()
    {
        Ok(mut sessions) => sessions
            .extract_if(|_, session| {
                now.duration_since(session.opened_at) >= GATEWAY_DISPLAY_SESSION_TTL
            })
            .collect(),
        Err(_) => {
            tracing::warn!("gateway display GC skipped because session mutex was poisoned");
            return;
        }
    };
    for (id, session) in expired {
        if let Err(err) = block_on_future(state.gateway_display.orchestrator.close(&session.open)) {
            tracing::warn!(error = %err, session_id = %id, "gateway display GC cleanup failed");
        }
    }
}

fn gateway_deps_from_config(config: &GatewayFileConfig) -> Result<GatewayDeps, TypedError> {
    Ok(production_deps(
        Box::new(ConfiguredGatewayWorkload {
            config: config.clone(),
        }),
        Box::new(ConfiguredDisplayListener {
            config: config.clone(),
            listeners: Mutex::new(HashMap::new()),
        }),
    ))
}

struct ConfiguredGatewayWorkload {
    config: GatewayFileConfig,
}

#[async_trait]
impl GatewayWorkload for ConfiguredGatewayWorkload {
    async fn spawn_agent(&self, req: &AgentSpawnRequest) -> Result<AgentHandle, GatewayError> {
        let provider = aca_provider_from_gateway_config(&self.config)?;
        let workload = AcaGatewayWorkload::for_workload_labels(
            provider,
            relay_coords_from_config(&self.config)?,
        )
        .with_binaries(agent_bins_from_config(&self.config))
        .with_relay_auth_snippet(relay_auth_snippet_from_config(&self.config)?);
        workload.spawn_agent(req).await
    }

    async fn cleanup(&self, handle: &AgentHandle) -> Result<(), GatewayError> {
        let provider = aca_provider_from_gateway_config(&self.config)?;
        let workload = AcaGatewayWorkload::for_workload_labels(
            provider,
            relay_coords_from_config(&self.config)?,
        )
        .with_binaries(agent_bins_from_config(&self.config));
        workload.cleanup(handle).await
    }
}

struct ConfiguredDisplayListener {
    config: GatewayFileConfig,
    listeners: Mutex<HashMap<String, Arc<RelayDisplayListener>>>,
}

#[async_trait]
impl DisplayListener for ConfiguredDisplayListener {
    async fn arm(
        &self,
        ctx: &DisplaySessionContext,
        binding: &SessionBinding,
        secret: &SessionSecret,
    ) -> Result<ListenerHandle, GatewayError> {
        let listener = Arc::new(display_listener_from_config(&self.config)?);
        let handle = listener.arm(ctx, binding, secret).await?;
        // Azure Relay listener registration is asynchronous after the control
        // channel is spawned. Give the listener a short head start before the
        // sandbox sender dials; otherwise the service can reset the sender
        // rendezvous instead of returning the retryable 404.
        tokio::time::sleep(Duration::from_secs(2)).await;
        self.listeners
            .lock()
            .map_err(|_| GatewayError::ProviderAllocationFailed)?
            .insert(handle.0.clone(), listener);
        Ok(handle)
    }

    async fn await_handshake(&self, handle: &ListenerHandle) -> Result<(), GatewayError> {
        let listener = self
            .listeners
            .lock()
            .map_err(|_| GatewayError::ProviderAllocationFailed)?
            .get(&handle.0)
            .cloned()
            .ok_or(GatewayError::ProviderAllocationFailed)?;
        listener.await_handshake(handle).await
    }

    async fn close(&self, handle: &ListenerHandle) -> Result<(), GatewayError> {
        let listener = self
            .listeners
            .lock()
            .map_err(|_| GatewayError::ProviderAllocationFailed)?
            .remove(&handle.0);
        if let Some(listener) = listener {
            listener.close(handle).await?;
        }
        Ok(())
    }
}

fn relay_coords_from_config(config: &GatewayFileConfig) -> Result<RelayCoords, GatewayError> {
    Ok(RelayCoords {
        namespace: required_gateway_field(&config.relay.namespace)?,
        entity: required_gateway_field(&config.relay.entity)?,
        ca_file: Some("/etc/ssl/certs/adc-egress-proxy-ca.crt".to_owned()),
        managed_identity_client_id: config.aca.managed_identity_client_id.clone(),
    })
}

fn relay_endpoint_from_config(config: &GatewayFileConfig) -> Result<RelayEndpoint, GatewayError> {
    Ok(RelayEndpoint {
        namespace: required_gateway_field(&config.relay.namespace)?,
        entity: required_gateway_field(&config.relay.entity)?,
    })
}

fn agent_bins_from_config(config: &GatewayFileConfig) -> AgentBinaries {
    let mut bins = AgentBinaries::default();
    if let Some(compression) = config
        .display
        .waypipe_compression
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        bins.compression = compression.clone();
    }
    bins
}

fn gateway_credential_from_config(
    config: &GatewayFileConfig,
) -> Result<GatewayCredential, GatewayError> {
    let credential_path = config
        .credential_path
        .as_ref()
        .ok_or(GatewayError::ProviderAllocationFailed)?;
    let seal_key_path = config
        .seal_key_path
        .as_ref()
        .ok_or(GatewayError::ProviderAllocationFailed)?;
    let policy = CredentialFilePolicy::default();
    let key = SealingKey::load(seal_key_path, &policy)
        .map_err(|_| GatewayError::ProviderAllocationFailed)?;
    GatewayCredential::load_sealed(credential_path, &key, &policy, system_now_unix())
        .map_err(|_| GatewayError::ProviderAllocationFailed)
}

fn relay_auth_snippet_from_config(config: &GatewayFileConfig) -> Result<String, GatewayError> {
    let credential = gateway_credential_from_config(config)?;
    let token = credential
        .mint_send_token(
            &relay_endpoint_from_config(config)?,
            nixling_provider_relay::DEFAULT_SAS_TTL_SECS,
        )
        .map_err(|_| GatewayError::ProviderAllocationFailed)?;
    Ok(relay_sas_token_snippet(token.expose()))
}

fn display_listener_from_config(
    config: &GatewayFileConfig,
) -> Result<RelayDisplayListener, GatewayError> {
    let credential = gateway_credential_from_config(config)?;
    let waypipe_socket = required_gateway_field(&config.display.waypipe_socket)?;
    let validated = match validate_waypipe_receiver_socket(Path::new(&waypipe_socket)) {
        Ok(validated) => validated,
        Err(error) => {
            tracing::warn!(
                error = %error.message(),
                "gateway display waypipe receiver socket rejected",
            );
            return Err(GatewayError::ProviderAllocationFailed);
        }
    };
    Ok(RelayDisplayListener::new(
        relay_endpoint_from_config(config)?,
        credential.listener_credential(),
        LocalTarget::UnixConnectChecked {
            path: waypipe_socket,
            uid: validated.uid,
            mode: validated.mode,
        },
        nixling_provider_relay::DEFAULT_SAS_TTL_SECS,
        None,
        system_now_fn(),
    ))
}

fn unavailable_gateway_deps() -> GatewayDeps {
    GatewayDeps {
        workload: Box::new(UnavailableGatewayWorkload),
        listener: Box::new(UnavailableDisplayListener),
        clock: Box::new(DaemonGatewayClock),
        ids: Box::new(DaemonGatewayIds),
        audit: Box::new(NoopGatewayAudit),
    }
}

#[cfg(test)]
fn daemon_gateway_deps() -> GatewayDeps {
    GatewayDeps {
        workload: Box::new(DaemonGatewayWorkload),
        listener: Box::new(DaemonDisplayListener),
        clock: Box::new(DaemonGatewayClock),
        ids: Box::new(DaemonGatewayIds),
        audit: Box::new(NoopGatewayAudit),
    }
}

struct UnavailableGatewayLifecycle;

#[async_trait]
impl GatewayLifecycle for UnavailableGatewayLifecycle {
    async fn start(&self, _target: &TargetName) -> Result<String, GatewayError> {
        Err(GatewayError::ProviderAllocationFailed)
    }

    async fn stop(&self, _target: &TargetName) -> Result<String, GatewayError> {
        Err(GatewayError::ProviderAllocationFailed)
    }
}

struct UnavailableGatewayWorkload;

#[async_trait]
impl GatewayWorkload for UnavailableGatewayWorkload {
    async fn spawn_agent(&self, _req: &AgentSpawnRequest) -> Result<AgentHandle, GatewayError> {
        Err(GatewayError::ProviderAllocationFailed)
    }

    async fn cleanup(&self, _handle: &AgentHandle) -> Result<(), GatewayError> {
        Ok(())
    }
}

struct UnavailableDisplayListener;

#[async_trait]
impl DisplayListener for UnavailableDisplayListener {
    async fn arm(
        &self,
        _ctx: &DisplaySessionContext,
        _binding: &SessionBinding,
        _secret: &SessionSecret,
    ) -> Result<ListenerHandle, GatewayError> {
        Err(GatewayError::ProviderAllocationFailed)
    }

    async fn await_handshake(&self, _handle: &ListenerHandle) -> Result<(), GatewayError> {
        Err(GatewayError::ProviderAllocationFailed)
    }

    async fn close(&self, _handle: &ListenerHandle) -> Result<(), GatewayError> {
        Ok(())
    }
}

#[cfg(test)]
struct DaemonGatewayLifecycle;

#[cfg(test)]
#[async_trait]
impl GatewayLifecycle for DaemonGatewayLifecycle {
    async fn start(&self, _target: &TargetName) -> Result<String, GatewayError> {
        Ok("ready".to_owned())
    }

    async fn stop(&self, _target: &TargetName) -> Result<String, GatewayError> {
        Ok("stopped".to_owned())
    }
}

struct AcaGatewayLifecycle {
    provider: Arc<AcaWorkloadProvider>,
}

#[async_trait]
impl GatewayLifecycle for AcaGatewayLifecycle {
    async fn start(&self, target: &TargetName) -> Result<String, GatewayError> {
        self.provider
            .start(target.workload.clone())
            .await
            .map_err(|err| {
                tracing::warn!(error = %err, target = %target, "aca gateway lifecycle start failed");
                GatewayError::ProviderAllocationFailed
            })?;
        Ok("running".to_owned())
    }

    async fn stop(&self, target: &TargetName) -> Result<String, GatewayError> {
        self.provider
            .stop(target.workload.clone())
            .await
            .map_err(|err| {
                tracing::warn!(error = %err, target = %target, "aca gateway lifecycle stop failed");
                GatewayError::ProviderAllocationFailed
            })?;
        Ok("stopped".to_owned())
    }
}

fn required_gateway_field(value: &Option<String>) -> Result<String, GatewayError> {
    value
        .as_ref()
        .filter(|s| !s.trim().is_empty())
        .cloned()
        .ok_or(GatewayError::ProviderAllocationFailed)
}

fn aca_provider_from_gateway_config(
    config: &GatewayFileConfig,
) -> Result<AcaWorkloadProvider, GatewayError> {
    let aca = &config.aca;
    let provider_config = AcaConfig {
        subscription: required_gateway_field(&aca.subscription)?,
        resource_group: required_gateway_field(&aca.resource_group)?,
        sandbox_group: required_gateway_field(&aca.sandbox_group)?,
        region: required_gateway_field(&aca.region)?,
        endpoint: aca.endpoint.clone(),
        managed_identity_client_id: aca.managed_identity_client_id.clone(),
    };
    let disk_image = if let Some(id) = aca.disk_image_id.as_ref().filter(|s| !s.trim().is_empty()) {
        AcaDiskImageSource::ExistingDiskId(id.clone())
    } else {
        AcaDiskImageSource::ContainerImage {
            image: required_gateway_field(&aca.image)?,
            name: aca
                .disk_name
                .as_ref()
                .filter(|s| !s.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| format!("nixling-{}-wayland", config.gateway)),
            managed_identity_resource_id: aca.managed_identity_resource_id.clone(),
            labels: BTreeMap::new(),
        }
    };
    let mut defaults = AcaSandboxDefaults::new(disk_image);
    defaults.cpu = aca.cpu.clone().unwrap_or_else(|| "1000m".to_owned());
    defaults.memory = aca.memory.clone().unwrap_or_else(|| "2048Mi".to_owned());
    defaults.auto_suspend_interval_secs = aca.auto_suspend_interval_secs.unwrap_or(600);
    defaults.managed_identity_resource_id = aca.managed_identity_resource_id.clone();
    defaults
        .labels
        .insert("nixling-realm".to_owned(), config.realm.clone());
    let provider = AcaWorkloadProvider::new(
        provider_config,
        nixling_constellation_core::NodeId::parse("gateway")
            .map_err(|_| GatewayError::ProviderAllocationFailed)?,
    )
    .map_err(|err| {
        tracing::warn!(error = %err, "aca gateway provider initialization failed");
        GatewayError::ProviderAllocationFailed
    })?
    .with_sandbox_defaults(defaults);
    Ok(provider)
}

#[cfg(test)]
struct DaemonGatewayWorkload;

#[cfg(test)]
#[async_trait]
impl GatewayWorkload for DaemonGatewayWorkload {
    async fn spawn_agent(&self, req: &AgentSpawnRequest) -> Result<AgentHandle, GatewayError> {
        Ok(AgentHandle(format!("daemon-agent-{}", req.ctx.session_id)))
    }

    async fn cleanup(&self, _handle: &AgentHandle) -> Result<(), GatewayError> {
        Ok(())
    }
}

#[cfg(test)]
struct DaemonDisplayListener;

#[cfg(test)]
#[async_trait]
impl DisplayListener for DaemonDisplayListener {
    async fn arm(
        &self,
        ctx: &DisplaySessionContext,
        _binding: &SessionBinding,
        _secret: &SessionSecret,
    ) -> Result<ListenerHandle, GatewayError> {
        Ok(ListenerHandle(format!(
            "daemon-listener-{}",
            ctx.session_id
        )))
    }

    async fn await_handshake(&self, _handle: &ListenerHandle) -> Result<(), GatewayError> {
        Ok(())
    }

    async fn close(&self, _handle: &ListenerHandle) -> Result<(), GatewayError> {
        Ok(())
    }
}

struct DaemonGatewayClock;

impl Clock for DaemonGatewayClock {
    fn now_unix(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

struct DaemonGatewayIds;

impl IdSource for DaemonGatewayIds {
    fn new_session_id(&self) -> nixling_gateway::DisplaySessionId {
        let mut raw = [0u8; 16];
        getrandom::getrandom(&mut raw).unwrap_or(());
        nixling_gateway::DisplaySessionId::new(format!("gw-{}", hex_bytes(&raw)))
    }

    fn new_secret(&self) -> SessionSecret {
        let mut raw = [0u8; SECRET_LEN];
        getrandom::getrandom(&mut raw).unwrap_or(());
        SessionSecret::from_bytes(raw)
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
    }
    s
}

fn dispatch_exec_management(
    state: &ServerState,
    peer: &PeerIdentity,
    op: public_wire::ExecOp,
) -> Result<Value, TypedError> {
    if let Some(vm) = exec_op_vm(&op) {
        ensure_vm_runtime_capability(state, vm, RuntimeCapabilityGate::Exec, "exec")?;
    }
    let response = match op {
        public_wire::ExecOp::List(args) => {
            public_wire::ExecOpResponse::List(exec_detached::list(state, &args)?)
        }
        public_wire::ExecOp::Logs(args) => {
            public_wire::ExecOpResponse::Logs(exec_detached::logs(state, &args)?)
        }
        public_wire::ExecOp::Status(args) => {
            public_wire::ExecOpResponse::Status(exec_detached::status(state, &args)?)
        }
        public_wire::ExecOp::Kill(args) => {
            let result = exec_detached::kill(state, &args);
            emit_detached_kill_audit(state, peer.uid, &args.vm, result.as_ref());
            public_wire::ExecOpResponse::Kill(result?)
        }
        public_wire::ExecOp::Start(_)
        | public_wire::ExecOp::WriteStdin(_)
        | public_wire::ExecOp::ReadOutput(_)
        | public_wire::ExecOp::Signal(_)
        | public_wire::ExecOp::Resize(_)
        | public_wire::ExecOp::Wait(_)
        | public_wire::ExecOp::Close(_) => {
            return Err(TypedError::GuestControlExecFailed {
                kind: crate::typed_error::GuestControlExecErrorKind::Protocol,
            });
        }
    };
    Ok(wire::exec_response(&response))
}

fn exec_op_vm(op: &public_wire::ExecOp) -> Option<&str> {
    match op {
        public_wire::ExecOp::Start(args) => Some(args.vm.as_str()),
        public_wire::ExecOp::List(args) => Some(args.vm.as_str()),
        public_wire::ExecOp::Logs(args) => Some(args.vm.as_str()),
        public_wire::ExecOp::Status(args) => Some(args.vm.as_str()),
        public_wire::ExecOp::Kill(args) => Some(args.vm.as_str()),
        public_wire::ExecOp::WriteStdin(_)
        | public_wire::ExecOp::ReadOutput(_)
        | public_wire::ExecOp::Signal(_)
        | public_wire::ExecOp::Resize(_)
        | public_wire::ExecOp::Wait(_)
        | public_wire::ExecOp::Close(_) => None,
    }
}

fn dispatch_keys_list(state: &ServerState) -> Result<Value, TypedError> {
    let bundle: Bundle = load_json(&state.config.artifacts.bundle_path)?;
    let manifest: ManifestV04 = load_json(&state.config.artifacts.public_manifest_path)?;
    let ssh_keygen_binary = PathBuf::from("/run/current-system/sw/bin/ssh-keygen");
    let entries = manifest
        .vms
        .iter()
        .filter(|(_, entry)| entry.runtime.capabilities.keys)
        .map(|(vm, entry)| build_key_entry(&bundle, &ssh_keygen_binary, vm, entry))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(wire::keys_list_response(public_wire::KeysListResponse {
        entries,
    }))
}

fn dispatch_keys_show(
    state: &ServerState,
    request: public_wire::KeysShowRequest,
) -> Result<Value, TypedError> {
    let bundle: Bundle = load_json(&state.config.artifacts.bundle_path)?;
    let manifest: ManifestV04 = load_json(&state.config.artifacts.public_manifest_path)?;
    ensure_manifest_entry_runtime_capability(
        manifest.vms.get(&request.vm),
        &request.vm,
        RuntimeCapabilityGate::Keys,
        "keys show",
    )?;
    let ssh_keygen_binary = PathBuf::from("/run/current-system/sw/bin/ssh-keygen");
    let entry = manifest
        .vms
        .get(&request.vm)
        .ok_or_else(|| TypedError::InternalIo {
            context: format!("keys show {}", request.vm),
            detail: "VM not present in public manifest".to_owned(),
        })?;
    let key_entry = build_key_entry(&bundle, &ssh_keygen_binary, &request.vm, entry)?;
    let public_key = read_trimmed_file(
        &bundle.managed_keys.public_key_path(&request.vm),
        &format!("read {} public key", request.vm),
    )?;
    Ok(wire::keys_show_response(public_wire::KeysShowResponse {
        vm: key_entry.vm,
        env: key_entry.env,
        managed_key_path: key_entry.managed_key_path,
        public_key,
        fingerprint: key_entry.fingerprint,
        known_hosts_entry: key_entry.known_hosts_entry,
    }))
}

fn build_key_entry(
    bundle: &Bundle,
    ssh_keygen_binary: &Path,
    vm: &str,
    entry: &ManifestVmEntry,
) -> Result<public_wire::KeyEntry, TypedError> {
    let managed_key_path = bundle.managed_keys.effective_key_path(vm);
    let public_key_path = bundle.managed_keys.public_key_path(vm);
    let fingerprint = ssh_keygen::probe_fingerprint(ssh_keygen_binary, &public_key_path)
        .map_err(|err| TypedError::InternalIo {
            context: format!("ssh-keygen -lf {}", public_key_path.display()),
            detail: err.to_string(),
        })?
        .fingerprint;
    Ok(public_wire::KeyEntry {
        vm: vm.to_owned(),
        env: entry.env.clone(),
        managed_key_path: managed_key_path.display().to_string(),
        fingerprint,
        known_hosts_entry: build_known_hosts_entry(entry)?,
    })
}

fn build_known_hosts_entry(entry: &ManifestVmEntry) -> Result<Option<String>, TypedError> {
    let Some(static_ip) = entry.static_ip.as_ref() else {
        return Ok(None);
    };
    let host_public_key = read_trimmed_file(
        &PathBuf::from(&entry.state_dir)
            .join("sshd-host-keys")
            .join("ssh_host_ed25519_key.pub"),
        &format!("read {} host public key", entry.name),
    )?;
    if host_public_key.is_empty() {
        return Ok(None);
    }
    Ok(Some(format!("{static_ip} {host_public_key}")))
}

fn read_trimmed_file(path: &Path, context: &str) -> Result<String, TypedError> {
    fs::read_to_string(path)
        .map(|content| content.trim().to_owned())
        .map_err(|err| TypedError::InternalIo {
            context: context.to_owned(),
            detail: err.to_string(),
        })
}

fn dispatch_broker_usbip_bind(
    state: &ServerState,
    request: public_wire::UsbipBindCliRequest,
) -> Result<Value, TypedError> {
    const VERB: &str = "usb attach";
    if let Some(response) = mutating_verb_preflight(VERB, &request.flags, Some(request.vm.as_str()))
    {
        return Ok(response);
    }
    let resolver = load_bundle_resolver(state)?;
    ensure_manifest_entry_runtime_capability(
        resolver.manifest.vms.get(&request.vm),
        &request.vm,
        RuntimeCapabilityGate::UsbHotplug,
        VERB,
    )?;
    if vm_is_qemu_media(&resolver, &request.vm)? {
        return dispatch_broker_qemu_media_attach(state, request);
    }
    if let Err(response) = validate_usbip_bus_id_for_daemon(VERB, &request.bus_id) {
        return Ok(response);
    }
    if let Err(summary) = run_guest_usbip_import(
        state,
        &resolver,
        &request.vm,
        &request.bus_id,
        guest_control_health::GuestUsbipAction::Detach,
    ) {
        return Ok(daemon_failure_response(VERB, summary));
    }
    if let Err(response) = dispatch_broker_ack_request(
        state,
        VERB,
        "UsbipBind",
        BrokerRequest::UsbipBind(BrokerUsbipBindRequest {
            bus_id: request.bus_id.clone(),
            vm_id: VmId::new(request.vm.clone()),
        }),
    ) {
        return Ok(response);
    }
    if let Err(response) =
        ensure_usbipd_env_ready_for_attach(state, &resolver, &request.vm, &request.bus_id, VERB)
    {
        compensate_usbip_bind_failure(state, &request.vm, &request.bus_id, VERB);
        return Ok(response);
    }
    if let Err(response) = dispatch_broker_ack_request(
        state,
        VERB,
        "UsbipProxyReconcile",
        BrokerRequest::UsbipProxyReconcile(BrokerUsbipProxyReconcileRequest {
            scope_id: ScopeId::new(format!("vm:{}", request.vm)),
        }),
    ) {
        compensate_usbip_bind_failure(state, &request.vm, &request.bus_id, VERB);
        return Ok(response);
    }
    if let Err(summary) = run_guest_usbip_import(
        state,
        &resolver,
        &request.vm,
        &request.bus_id,
        guest_control_health::GuestUsbipAction::Attach,
    ) {
        compensate_usbip_bind_failure(state, &request.vm, &request.bus_id, VERB);
        return Ok(daemon_failure_response(VERB, summary));
    }
    Ok(applied_response(
        VERB,
        format!(
            "nixling usb attach --apply: bound busid '{}' for vm '{}' and imported it via guestd",
            request.bus_id, request.vm
        ),
    ))
}

fn validate_usbip_bus_id_for_daemon(verb: &str, bus_id: &str) -> Result<(), Value> {
    nixling_ipc::usbip::validate_bus_id(bus_id).map_err(|_| {
        daemon_failure_response(
            verb,
            format!("USBIP busid '{bus_id}' does not match the canonical sysfs bus-id shape"),
        )
    })
}

fn run_guest_usbip_import(
    state: &ServerState,
    resolver: &BundleResolver,
    vm: &str,
    bus_id: &str,
    action: guest_control_health::GuestUsbipAction,
) -> Result<guest_control_health::GuestUsbipImportResult, String> {
    let Some(entry) = resolver.manifest.vms.get(vm) else {
        return Err(format!("VM '{vm}' is not present in the trusted manifest"));
    };
    let Some(host) = entry.usbipd_host_ip.as_deref() else {
        return Err(format!(
            "VM '{vm}' has no per-env USBIP host IP in the trusted manifest"
        ));
    };
    let params = resolve_guest_control_probe_params(state, resolver, vm).map_err(|detail| {
        tracing::warn!(
            kind = "critical",
            subsystem = "guest-control-usbip",
            error_kind = "transport-io",
            "guest-control USBIP import: probe params unresolved: {detail}"
        );
        format!(
            "guest-control USBIP import for vm '{vm}' could not resolve guest-control transport"
        )
    })?;
    guest_control_bridge::run_usbip_import_on_dedicated_thread(
        params,
        broker_socket_path(state),
        action,
        host.to_owned(),
        bus_id.to_owned(),
        guest_control_bridge::GUEST_CONTROL_USBIP_IMPORT_TIMEOUT,
    )
    .map_err(|error| guest_usbip_import_error_summary(vm, error))
}

fn guest_usbip_import_error_summary(
    vm: &str,
    error: guest_control_health::GuestUsbipImportError,
) -> String {
    use guest_control_health::{GuestControlHealthError as H, GuestUsbipImportError as E};
    let detail = match error {
        E::Probe(H::TransportIo) | E::Probe(H::Signer) | E::Probe(H::Ttrpc) => {
            "guest-control transport unavailable"
        }
        E::Probe(H::Timeout) => "guest-control USBIP import timed out",
        E::Probe(H::AuthFailed) | E::Probe(H::StaleSession) => {
            "guest-control authentication failed"
        }
        E::Probe(H::Protocol) | E::Protocol => "guest-control USBIP protocol error",
        E::CapabilityUnavailable => "guest does not advertise USBIP import capability",
        E::UsbipUnavailable => "guestd has no usable usbip binary",
        E::InvalidBusId => "guestd rejected the USBIP busid",
        E::InvalidHost => "guestd rejected the USBIP backend host",
        E::CommandFailed => "guest usbip command failed",
    };
    format!("guest-control USBIP import failed for vm '{vm}': {detail}")
}

fn compensate_usbip_bind_failure(state: &ServerState, vm: &str, bus_id: &str, verb: &str) {
    if let Err(response) = dispatch_broker_ack_request(
        state,
        verb,
        "UsbipUnbind",
        BrokerRequest::UsbipUnbind(BrokerUsbipUnbindRequest {
            bus_id: bus_id.to_owned(),
        }),
    ) {
        tracing::warn!(vm = %vm, bus_id = %bus_id, response = %response, "USBIP attach compensation unbind failed");
    }
    if let Err(response) = dispatch_broker_ack_request(
        state,
        verb,
        "UsbipProxyReconcile",
        BrokerRequest::UsbipProxyReconcile(BrokerUsbipProxyReconcileRequest {
            scope_id: ScopeId::new(format!("vm:{vm}")),
        }),
    ) {
        tracing::warn!(vm = %vm, bus_id = %bus_id, response = %response, "USBIP attach compensation proxy reconcile failed");
    }
}

fn ensure_usbipd_env_ready_for_attach(
    state: &ServerState,
    resolver: &BundleResolver,
    vm: &str,
    bus_id: &str,
    verb: &str,
) -> Result<(), Value> {
    let Some(entry) = resolver.manifest.vms.get(vm) else {
        return Err(daemon_failure_response(
            verb,
            format!("VM '{vm}' is not present in the trusted manifest"),
        ));
    };
    let Some(env) = entry.env.as_deref() else {
        return Err(daemon_failure_response(
            verb,
            format!("VM '{vm}' is not attached to a nixling env"),
        ));
    };
    let Some(host_ip) = entry.usbipd_host_ip.as_deref() else {
        return Err(daemon_failure_response(
            verb,
            format!("VM '{vm}' has no per-env USBIP host IP in the trusted manifest"),
        ));
    };

    let specs: Vec<_> = usbipd_perenv_autostart::derive_per_env_usbipd_specs(&resolver.manifest)
        .into_iter()
        .filter(|spec| spec.env == env)
        .collect();
    if specs.len() != 2 {
        return Err(daemon_failure_response(
            verb,
            format!("trusted bundle has no complete per-env USBIP runner plan for env '{env}'"),
        ));
    }

    dispatch_broker_ack_request(
        state,
        verb,
        "UsbipBindFirewallRule",
        BrokerRequest::UsbipBindFirewallRule(BrokerUsbipBindFirewallRuleRequest {
            bundle_usbip_firewall_intent_ref: BundleOpId::new(intent_id_usbip_firewall(
                env, bus_id,
            )),
            tracing_span_id: None,
        }),
    )?;

    let spawner = BrokerPerEnvUsbipdSpawner {
        state: Arc::new(state.clone()),
    };
    let report = usbipd_perenv_autostart::execute_usbipd_perenv_autostart(&specs, &spawner);
    let failed: Vec<String> = report
        .specs
        .iter()
        .filter_map(|entry| match &entry.outcome {
            usbipd_perenv_autostart::PerEnvUsbipdOutcome::Failed { reason } => {
                Some(format!("{}:{reason}", entry.role.role_id()))
            }
            usbipd_perenv_autostart::PerEnvUsbipdOutcome::SkippedPendingBundle => {
                Some(format!("{}:bundle-intent-missing", entry.role.role_id()))
            }
            usbipd_perenv_autostart::PerEnvUsbipdOutcome::Spawned
            | usbipd_perenv_autostart::PerEnvUsbipdOutcome::AlreadyRunning => None,
        })
        .collect();
    if !failed.is_empty() {
        return Err(daemon_failure_response(
            verb,
            format!(
                "per-env USBIP runners for env '{env}' did not start cleanly: {}",
                failed.join(", ")
            ),
        ));
    }

    let backend_port = specs[0].backend_port;
    wait_for_tcp_port("127.0.0.1", backend_port, Duration::from_secs(10))
        .and_then(|_| wait_for_tcp_port(host_ip, 3240, Duration::from_secs(10)))
        .map_err(|reason| {
            daemon_failure_response(
                verb,
                format!("per-env USBIP runners for env '{env}' did not become ready: {reason}"),
            )
        })
}

fn dispatch_broker_usbip_unbind(
    state: &ServerState,
    request: public_wire::UsbipUnbindCliRequest,
) -> Result<Value, TypedError> {
    const VERB: &str = "usb detach";
    if let Some(response) = mutating_verb_preflight(VERB, &request.flags, Some(request.vm.as_str()))
    {
        return Ok(response);
    }
    let resolver = load_bundle_resolver(state)?;
    ensure_manifest_entry_runtime_capability(
        resolver.manifest.vms.get(&request.vm),
        &request.vm,
        RuntimeCapabilityGate::UsbHotplug,
        VERB,
    )?;
    if vm_is_qemu_media(&resolver, &request.vm)? {
        return dispatch_broker_qemu_media_detach(state, request);
    }
    if let Err(response) = validate_usbip_bus_id_for_daemon(VERB, &request.bus_id) {
        return Ok(response);
    }
    let guest_cleanup_failure = run_guest_usbip_import(
        state,
        &resolver,
        &request.vm,
        &request.bus_id,
        guest_control_health::GuestUsbipAction::Detach,
    )
    .err();
    if let Err(response) = dispatch_broker_ack_request(
        state,
        VERB,
        "UsbipUnbind",
        BrokerRequest::UsbipUnbind(BrokerUsbipUnbindRequest {
            bus_id: request.bus_id.clone(),
        }),
    ) {
        return Ok(response);
    }
    if let Err(response) = dispatch_broker_ack_request(
        state,
        VERB,
        "UsbipProxyReconcile",
        BrokerRequest::UsbipProxyReconcile(BrokerUsbipProxyReconcileRequest {
            scope_id: ScopeId::new(format!("vm:{}", request.vm)),
        }),
    ) {
        return Ok(response);
    }
    if let Some(summary) = guest_cleanup_failure {
        return Ok(daemon_failure_response(
            VERB,
            format!("host USBIP unbind completed, but {summary}"),
        ));
    }
    Ok(applied_response(
        VERB,
        format!(
            "nixling usb detach --apply: detached guest import and unbound busid '{}' for vm '{}'",
            request.bus_id, request.vm
        ),
    ))
}

fn vm_is_qemu_media(resolver: &BundleResolver, vm: &str) -> Result<bool, TypedError> {
    let Some(entry) = resolver.find_manifest_vm(vm) else {
        return Ok(false);
    };
    Ok(!vm_requires_nixos_state_preflights(entry))
}

fn vm_requires_nixos_state_preflights(entry: &ManifestVmEntry) -> bool {
    entry.runtime.kind != nixling_core::runtime::RuntimeKind::QemuMedia
}

fn refresh_qemu_media_registry_index_if_needed(
    state: &ServerState,
    resolver: &BundleResolver,
) -> Result<(), TypedError> {
    if resolver.host.qemu_media.is_none() {
        return Ok(());
    }
    match dispatch_broker_request(
        state,
        BrokerRequest::QemuMediaRefreshRegistry(BrokerQemuMediaRefreshRegistryRequest {
            tracing_span_id: None,
        }),
    )? {
        BrokerResponse::QemuMediaRefreshRegistry(_) => Ok(()),
        BrokerResponse::Error(error) => Err(TypedError::InternalIo {
            context: "refresh qemu-media registry index".to_owned(),
            detail: format!("{}:{}", error.operation, error.kind),
        }),
        other => Err(TypedError::InternalIo {
            context: "refresh qemu-media registry index".to_owned(),
            detail: format!(
                "unexpected broker response {}",
                broker_response_kind(&other)
            ),
        }),
    }
}

fn dispatch_broker_qemu_media_attach(
    state: &ServerState,
    request: public_wire::UsbipBindCliRequest,
) -> Result<Value, TypedError> {
    const VERB: &str = "usb attach";
    if let Err(err) = nixling_host::media::validate_usb_busid(&request.bus_id) {
        return Ok(invalid_request_response(
            VERB,
            format!("invalid USB busid selector: {err}"),
        ));
    }
    match dispatch_broker_request(
        state,
        BrokerRequest::QemuMediaAttach(BrokerQemuMediaHotplugRequest {
            vm_id: VmId::new(request.vm.clone()),
            bus_id: request.bus_id,
            tracing_span_id: None,
        }),
    ) {
        Ok(BrokerResponse::QemuMediaAttach(response)) => Ok(applied_response(
            VERB,
            qemu_media_hotplug_summary(VERB, "attached", &response),
        )),
        Ok(BrokerResponse::Error(error)) => broker_error_for_qemu_media_hotplug(VERB, error),
        Ok(other) => {
            tracing::warn!(
                broker_response_kind = %broker_response_kind(&other),
                "broker returned unexpected qemu-media attach response"
            );
            let (summary, remediation) =
                redact_broker_error_for_launcher("QemuMediaAttach", None, "Broker.Protocol");
            Ok(broker_failure_response(VERB, summary, remediation, None))
        }
        Err(error) => {
            tracing::warn!(error = ?error, "qemu-media attach broker dispatch failed");
            let (summary, remediation) =
                redact_broker_dispatch_failure_for_launcher("QemuMediaAttach");
            Ok(broker_failure_response(VERB, summary, remediation, None))
        }
    }
}

fn dispatch_broker_qemu_media_detach(
    state: &ServerState,
    request: public_wire::UsbipUnbindCliRequest,
) -> Result<Value, TypedError> {
    const VERB: &str = "usb detach";
    if let Err(err) = nixling_host::media::validate_usb_busid(&request.bus_id) {
        return Ok(invalid_request_response(
            VERB,
            format!("invalid USB busid selector: {err}"),
        ));
    }
    match dispatch_broker_request(
        state,
        BrokerRequest::QemuMediaDetach(BrokerQemuMediaHotplugRequest {
            vm_id: VmId::new(request.vm.clone()),
            bus_id: request.bus_id,
            tracing_span_id: None,
        }),
    ) {
        Ok(BrokerResponse::QemuMediaDetach(response)) => Ok(applied_response(
            VERB,
            qemu_media_hotplug_summary(VERB, "detached", &response),
        )),
        Ok(BrokerResponse::Error(error)) => broker_error_for_qemu_media_hotplug(VERB, error),
        Ok(other) => {
            tracing::warn!(
                broker_response_kind = %broker_response_kind(&other),
                "broker returned unexpected qemu-media detach response"
            );
            let (summary, remediation) =
                redact_broker_error_for_launcher("QemuMediaDetach", None, "Broker.Protocol");
            Ok(broker_failure_response(VERB, summary, remediation, None))
        }
        Err(error) => {
            tracing::warn!(error = ?error, "qemu-media detach broker dispatch failed");
            let (summary, remediation) =
                redact_broker_dispatch_failure_for_launcher("QemuMediaDetach");
            Ok(broker_failure_response(VERB, summary, remediation, None))
        }
    }
}

fn qemu_media_hotplug_summary(
    verb: &str,
    action: &str,
    response: &nixling_ipc::broker_wire::QemuMediaHotplugResponse,
) -> String {
    format!(
        "nixling {verb} --apply: qemu-media {action} ref '{}' in slot '{}' for vm '{}' via QMP (commands={})",
        response.media_ref.as_str(),
        response.slot,
        response.vm_id.as_str(),
        response.qmp_commands.join(",")
    )
}

fn broker_error_for_qemu_media_hotplug(
    verb: &str,
    error: nixling_ipc::broker_wire::BrokerErrorResponse,
) -> Result<Value, TypedError> {
    let (summary, remediation) = redact_broker_error_for_launcher(
        error.operation.as_str(),
        error.target_wave.as_deref(),
        &error.kind,
    );
    Ok(broker_failure_response(
        verb,
        summary,
        remediation,
        error.target_wave,
    ))
}

fn dispatch_broker_usbip_probe(state: &ServerState) -> Result<Value, TypedError> {
    const VERB: &str = "usb probe";
    let resolver =
        BundleResolver::load(&state.config.artifacts.bundle_path).map_err(|err| match err {
            nixling_core::error::Error::Bundle(BundleError::Tampered { path, reason }) => {
                TypedError::BundleTampered { path, reason }
            }
            other => TypedError::InternalIo {
                context: "load bundle resolver".to_owned(),
                detail: other.to_string(),
            },
        })?;
    refresh_qemu_media_registry_index_if_needed(state, &resolver)?;
    let usbip_intent_ids: Vec<_> = resolver
        .usbip_bind_intent_ids()
        .map(str::to_owned)
        .collect();
    if !usbip_intent_ids.is_empty()
        && let Err(response) = dispatch_broker_ack_request(
            state,
            VERB,
            "UsbipProxyReconcile",
            BrokerRequest::UsbipProxyReconcile(BrokerUsbipProxyReconcileRequest {
                scope_id: ScopeId::new("host"),
            }),
        )
    {
        return Ok(response);
    }
    let mut entries: Vec<_> = usbip_intent_ids
        .iter()
        .map(String::as_str)
        .filter_map(|intent_id| resolver.find_usbip_bind_intent(intent_id))
        .map(|intent| {
            let owner_vm = fs::read_to_string(&intent.lock_path)
                .ok()
                .map(|content| content.trim().to_owned())
                .filter(|owner| !owner.is_empty());
            public_wire::UsbipProbeEntry {
                vm: intent.vm_name.clone(),
                env: intent.env.clone(),
                bus_id: intent.bus_id.clone(),
                lock_path: intent.lock_path.display().to_string(),
                status: if owner_vm.is_some() {
                    public_wire::UsbipProbeStatus::Bound
                } else {
                    public_wire::UsbipProbeStatus::Unbound
                },
                owner_vm,
                kind: public_wire::UsbProbeEntryKind::Usbip,
                slot: None,
                media_ref: None,
                source_kind: None,
                candidate_bus_ids: Vec::new(),
                follow_up_command: None,
            }
        })
        .collect();
    entries.extend(qemu_media_probe_entries(&resolver));
    Ok(wire::usbip_probe_response(
        public_wire::UsbipProbeResponse { entries },
    ))
}

fn qemu_media_probe_entries(resolver: &BundleResolver) -> Vec<public_wire::UsbipProbeEntry> {
    let Some(qemu_media) = resolver.host.qemu_media.as_ref() else {
        return Vec::new();
    };
    const MAX_QEMU_MEDIA_PROBE_CANDIDATES: usize = 16;
    let candidates = nixling_host::media::safe_usb_block_candidates(
        Path::new("/sys"),
        Path::new("/dev/disk/by-id"),
    );
    let candidate_bus_ids: Vec<String> = candidates
        .iter()
        .take(MAX_QEMU_MEDIA_PROBE_CANDIDATES)
        .map(|candidate| candidate.bus_id.clone())
        .collect();
    let registry_records = qemu_media_probe_registry_records();
    let mut entries = Vec::new();
    for source in &qemu_media.sources {
        let source_kind = match source.source_kind {
            nixling_core::host::QemuMediaSourceKind::PhysicalUsb => "physical-usb",
            nixling_core::host::QemuMediaSourceKind::ImageFile => "image-file",
        }
        .to_owned();
        if !matches!(
            source.source_kind,
            nixling_core::host::QemuMediaSourceKind::PhysicalUsb
        ) {
            let env = resolver
                .find_manifest_vm(&source.vm)
                .and_then(|vm| vm.env.as_deref())
                .unwrap_or("-");
            entries.push(qemu_media_probe_entry(
                source,
                env,
                "-",
                &[],
                source_kind,
                public_wire::UsbipProbeStatus::DirectConfig,
                None,
            ));
            continue;
        }
        if candidate_bus_ids.is_empty() {
            let env = resolver
                .find_manifest_vm(&source.vm)
                .and_then(|vm| vm.env.as_deref())
                .unwrap_or("-");
            let has_registry_record = registry_records
                .iter()
                .any(|record| record.vm == source.vm && record.media_ref == source.media_ref);
            entries.push(qemu_media_probe_entry(
                source,
                env,
                "-",
                &[],
                source_kind,
                if has_registry_record {
                    public_wire::UsbipProbeStatus::Stale
                } else {
                    public_wire::UsbipProbeStatus::Unbound
                },
                None,
            ));
            continue;
        }
        let env = resolver
            .find_manifest_vm(&source.vm)
            .and_then(|vm| vm.env.as_deref())
            .unwrap_or("-");
        let matching_records: Vec<_> = registry_records
            .iter()
            .filter(|record| record.vm == source.vm && record.media_ref == source.media_ref)
            .collect();
        let duplicate_identity = matching_records.iter().any(|record| {
            registry_records.iter().any(|other| {
                other.vm == source.vm
                    && other.media_ref != source.media_ref
                    && other.identity_hash == record.identity_hash
            })
        });
        let mut any_enrolled_candidate = false;
        let mut candidate_refs = candidates.iter().collect::<Vec<_>>();
        candidate_refs.sort_by_key(|candidate| {
            let enrolled = matching_records
                .iter()
                .any(|record| qemu_media_probe_candidate_matches_record(candidate, record));
            let enrolled_elsewhere = !enrolled
                && registry_records.iter().any(|record| {
                    record.vm == source.vm
                        && record.media_ref != source.media_ref
                        && qemu_media_probe_candidate_matches_record(candidate, record)
                });
            (!enrolled, !enrolled_elsewhere, candidate.bus_id.clone())
        });
        for candidate in candidate_refs
            .into_iter()
            .take(MAX_QEMU_MEDIA_PROBE_CANDIDATES)
        {
            let enrolled = matching_records
                .iter()
                .any(|record| qemu_media_probe_candidate_matches_record(candidate, record));
            let enrolled_elsewhere = !enrolled
                && registry_records.iter().any(|record| {
                    record.vm == source.vm
                        && record.media_ref != source.media_ref
                        && qemu_media_probe_candidate_matches_record(candidate, record)
                });
            any_enrolled_candidate |= enrolled;
            let bus_id = candidate.bus_id.as_str();
            entries.push(qemu_media_probe_entry(
                source,
                env,
                bus_id,
                std::slice::from_ref(&candidate.bus_id),
                source_kind.clone(),
                if duplicate_identity && (enrolled || enrolled_elsewhere) {
                    public_wire::UsbipProbeStatus::Stale
                } else if enrolled {
                    public_wire::UsbipProbeStatus::Enrolled
                } else if enrolled_elsewhere {
                    public_wire::UsbipProbeStatus::Stale
                } else {
                    public_wire::UsbipProbeStatus::Enrollable
                },
                if duplicate_identity && (enrolled || enrolled_elsewhere) {
                    None
                } else if enrolled {
                    Some(format!(
                        "nixling usb attach {} {} --apply",
                        source.vm, bus_id
                    ))
                } else if enrolled_elsewhere {
                    None
                } else {
                    Some(format!(
                        "update qemu-media config for vm '{}' and ref '{}', then run `nixling usb probe`; when the VM is running, hotplug this selector with `nixling usb attach {} {} --apply`",
                        source.vm, source.media_ref, source.vm, bus_id
                    ))
                },
            ));
        }
        if !matching_records.is_empty() && (duplicate_identity || !any_enrolled_candidate) {
            entries.push(qemu_media_probe_entry(
                source,
                env,
                "-",
                &[],
                source_kind,
                public_wire::UsbipProbeStatus::Stale,
                None,
            ));
        }
    }
    entries
}

const QEMU_MEDIA_REDACTED_INDEX_PATH: &str = "/run/nixling/qemu-media-registry-index.json";

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct QemuMediaProbeRegistryRecord {
    vm: String,
    media_ref: String,
    source_kind: String,
    format: String,
    read_only: bool,
    identity_hash: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct QemuMediaProbeRegistryIndex {
    records: Vec<QemuMediaProbeRegistryRecord>,
}

fn qemu_media_probe_registry_records() -> Vec<QemuMediaProbeRegistryRecord> {
    let Ok(bytes) = fs::read(QEMU_MEDIA_REDACTED_INDEX_PATH) else {
        return Vec::new();
    };
    serde_json::from_slice::<QemuMediaProbeRegistryIndex>(&bytes)
        .map(|index| index.records)
        .unwrap_or_default()
}

fn qemu_media_probe_candidate_matches_record(
    candidate: &nixling_host::media::SafeUsbCandidate,
    record: &QemuMediaProbeRegistryRecord,
) -> bool {
    qemu_media_identity_hash(&candidate.by_id_names) == record.identity_hash
}

fn qemu_media_identity_hash(by_id_names: &[String]) -> String {
    let mut names = by_id_names.to_vec();
    names.sort();
    names.dedup();
    let mut hasher = Sha256::new();
    for name in names {
        hasher.update(name.as_bytes());
        hasher.update([0]);
    }
    format!("{:x}", hasher.finalize())
}

fn qemu_media_probe_entry(
    source: &nixling_core::host::QemuMediaSourceIntent,
    env: &str,
    bus_id: &str,
    candidate_bus_ids: &[String],
    source_kind: String,
    status: public_wire::UsbipProbeStatus,
    follow_up_command: Option<String>,
) -> public_wire::UsbipProbeEntry {
    public_wire::UsbipProbeEntry {
        kind: public_wire::UsbProbeEntryKind::QemuMediaSlot,
        vm: source.vm.clone(),
        env: env.to_owned(),
        bus_id: bus_id.to_owned(),
        lock_path: String::new(),
        status,
        owner_vm: None,
        slot: Some(source.slot.clone()),
        media_ref: Some(MediaRef::new(source.media_ref.clone())),
        source_kind: Some(source_kind),
        candidate_bus_ids: candidate_bus_ids.to_vec(),
        follow_up_command,
    }
}

fn mutating_verb_preflight(
    verb: &str,
    flags: &nixling_ipc::public_wire::MutationFlags,
    target_vm: Option<&str>,
) -> Option<Value> {
    use nixling_ipc::public_wire::{MutatingVerbOutcome, MutatingVerbResponse};

    if !flags.dry_run && !flags.apply {
        return Some(wire::mutating_verb_response(MutatingVerbResponse {
            verb: verb.to_owned(),
            outcome: MutatingVerbOutcome::InvalidRequest,
            target_wave: None,
            summary: None,
            remediation: Some(format!(
                "nixling {verb} requires either --dry-run or --apply"
            )),
            api_ready: None,
        }));
    }

    if flags.dry_run {
        let summary = match target_vm {
            Some(vm) => format!("nixling {verb} --dry-run: daemon-side plan for vm '{vm}'"),
            None => format!("nixling {verb} --dry-run: daemon-side plan"),
        };
        return Some(wire::mutating_verb_response(MutatingVerbResponse {
            verb: verb.to_owned(),
            outcome: MutatingVerbOutcome::DryRunPlanned,
            target_wave: None,
            summary: Some(summary),
            remediation: None,
            api_ready: None,
        }));
    }

    None
}

fn broker_socket_path(state: &ServerState) -> PathBuf {
    if state.config.broker_socket_path.as_os_str().is_empty() {
        PathBuf::from(BROKER_SOCKET_PATH)
    } else {
        state.config.broker_socket_path.clone()
    }
}

/// Canonical group owning the per-VM state directory `<stateDir>/vms/<vm>`.
/// The tmpfiles seed line uses `microvm:kvm`, but host activation chowns the
/// per-VM directory to `<daemon-user>:users` mode 2770, so the runtime owner
/// is `nixlingd:users`. Existing-code-is-canon: the probe reconciles its
/// `expected_state_root_uid/gid` to the actual runtime owner; the tmpfiles vs
/// activation divergence is to be confirmed against a live host.
const GUEST_CONTROL_STATE_ROOT_GROUP: &str = "users";

/// Resolve the canonical runtime owner `(uid, gid)` of the per-VM state
/// directory for the guest-control probe's `expected_state_root_uid/gid`.
/// Derived independently of the peer-credential identity. Fails CLOSED if
/// either identity is unresolvable.
fn resolve_guest_control_state_root_owner(state: &ServerState) -> Result<(u32, u32), String> {
    let uid = User::from_name(&state.config.daemon_user)
        .ok()
        .flatten()
        .map(|user| user.uid.as_raw())
        .ok_or_else(|| {
            format!(
                "guest-control-probe:unresolved-state-root-user:{}",
                state.config.daemon_user
            )
        })?;
    let gid = Group::from_name(GUEST_CONTROL_STATE_ROOT_GROUP)
        .ok()
        .flatten()
        .map(|group| group.gid.as_raw())
        .ok_or_else(|| {
            format!(
                "guest-control-probe:unresolved-state-root-group:{GUEST_CONTROL_STATE_ROOT_GROUP}"
            )
        })?;
    Ok((uid, gid))
}

/// Extract the cloud-hypervisor `--vsock socket=<path>` argument from a CH
/// runner argv. Returns the resolved socket path, or `None` if absent.
fn cloud_hypervisor_vsock_socket(argv: &[String]) -> Option<PathBuf> {
    argv.windows(2).find_map(|pair| {
        if pair[0] != "--vsock" {
            return None;
        }
        pair[1]
            .split(',')
            .find_map(|field| field.strip_prefix("socket=").map(PathBuf::from))
    })
}

/// Resolve the guest-control probe parameters for `vm` from the trusted
/// bundle: the per-VM vsock socket path + its parent state-root, the
/// cloud-hypervisor runner's peer credentials (principal
/// `nixling-<vm>-runner`), and the canonical state-root owner. Fails CLOSED if
/// any identity is unresolvable.
fn resolve_guest_control_probe_params(
    state: &ServerState,
    resolver: &BundleResolver,
    vm: &str,
) -> Result<guest_control_bridge::ProbeParams, String> {
    let dag = resolver
        .find_process_vm(vm)
        .ok_or_else(|| "guest-control-probe:no-process-dag".to_owned())?;
    let ch = dag
        .nodes
        .iter()
        .find(|node| node.role == ProcessRole::CloudHypervisorRunner)
        .ok_or_else(|| "guest-control-probe:no-cloud-hypervisor-node".to_owned())?;
    let socket_path = cloud_hypervisor_vsock_socket(&ch.argv)
        .ok_or_else(|| "guest-control-probe:no-vsock-socket".to_owned())?;
    let state_root = socket_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "guest-control-probe:vsock-socket-no-parent".to_owned())?;
    let (expected_state_root_uid, expected_state_root_gid) =
        resolve_guest_control_state_root_owner(state)?;
    Ok(guest_control_bridge::ProbeParams {
        vm_id: vm.to_owned(),
        socket_path,
        state_root,
        expected_state_root_uid,
        expected_state_root_gid,
        expected_peer_uid: ch.profile.uid,
        expected_peer_gid: ch.profile.gid,
    })
}

/// Map a guest-control config read error to the closed-enum daemon error kind.
/// The mapping is exhaustive and never embeds a path, byte, or guest string.
fn map_guest_file_read_error(error: guest_control_health::GuestFileReadError) -> TypedError {
    use crate::typed_error::GuestControlReadErrorKind as K;
    use guest_control_health::{GuestControlHealthError as H, GuestFileReadError as E};
    let kind = match error {
        E::Probe(H::TransportIo) | E::Probe(H::Signer) => K::Transport,
        E::Probe(H::Ttrpc) => K::Transport,
        E::Probe(H::Timeout) => K::Timeout,
        E::Probe(H::AuthFailed) => K::AuthFailed,
        E::Probe(H::StaleSession) => K::AuthFailed,
        E::Probe(H::Protocol) => K::Protocol,
        E::CapabilityUnavailable => K::CapabilityUnavailable,
        E::FileNotFound => K::FileNotFound,
        E::FileTooLarge => K::FileTooLarge,
        E::PathUnsafe => K::PathUnsafe,
        E::ReadDenied => K::ReadDenied,
        E::Protocol => K::Protocol,
    };
    TypedError::GuestControlReadFailed { kind }
}

/// ADMIN-ONLY public.sock verb: read the editable guest config working copy of
/// `vm` over the authenticated guest-control bridge and return it as a base64
/// string. The admin authorization gate runs in `dispatch_request` BEFORE this
/// handler. The orchestration runs on a dedicated OS thread (the
/// synchronous-verb runtime boundary). The encoded payload is bounded so it fits both transport frames;
/// any guest content is never echoed into an error.
fn dispatch_read_guest_config(
    state: &ServerState,
    request: public_wire::ReadGuestConfigRequest,
) -> Result<Value, TypedError> {
    ensure_vm_runtime_capability(
        state,
        &request.vm,
        RuntimeCapabilityGate::ConfigSync,
        "config sync",
    )?;
    ensure_vm_runtime_capability(
        state,
        &request.vm,
        RuntimeCapabilityGate::GuestControl,
        "read guest config",
    )?;
    let resolver = load_bundle_resolver(state)?;
    let params =
        resolve_guest_control_probe_params(state, &resolver, &request.vm).map_err(|detail| {
            tracing::warn!(
                kind = "critical",
                subsystem = "guest-control-health",
                error_kind = "transport-io",
                "guest-control config read: probe params unresolved: {detail}"
            );
            TypedError::GuestControlReadFailed {
                kind: crate::typed_error::GuestControlReadErrorKind::Transport,
            }
        })?;
    let broker_path = broker_socket_path(state);
    let bytes = guest_control_bridge::run_config_read_on_dedicated_thread(
        params,
        broker_path,
        guest_control_bridge::GUEST_CONTROL_CONFIG_READ_TIMEOUT,
    )
    .map_err(map_guest_file_read_error)?;
    let encoded = nixling_core::base64_codec::encode(&bytes);
    if !nixling_ipc::guest_wire::guest_config_encoded_within_frame_caps(encoded.len()) {
        return Err(TypedError::GuestControlReadFailed {
            kind: crate::typed_error::GuestControlReadErrorKind::FileTooLarge,
        });
    }
    Ok(wire::read_guest_config_response(
        public_wire::ReadGuestConfigResponse {
            content_base64: encoded,
        },
    ))
}

const SHELL_MANAGEMENT_TIMEOUT: Duration = Duration::from_secs(12);

fn dispatch_shell_management(
    state: &ServerState,
    op: public_wire::ShellOp,
) -> Result<Value, TypedError> {
    let response = match op {
        public_wire::ShellOp::List(args) => {
            let result =
                run_guest_shell_management(state, args.vm.as_str(), |client, metadata| {
                    let mut request = pb::ShellListRequest::new();
                    request.metadata = protobuf::MessageField::some(metadata);
                    async move {
                        let response: pb::ShellListResponse = client
                            .unary_with_timeout("ShellList", request, SHELL_MANAGEMENT_TIMEOUT)
                            .await
                            .map_err(|_| TypedError::GuestShellDisabled)?;
                        shell_error_to_typed(response.error.as_ref())?;
                        map_shell_list_response(response)
                    }
                })?;
            public_wire::ShellOpResponse::List(result)
        }
        public_wire::ShellOp::Detach(args) => {
            let result =
                run_guest_shell_management(state, args.vm.as_str(), |client, metadata| {
                    let mut request = pb::ShellDetachRequest::new();
                    request.metadata = protobuf::MessageField::some(metadata);
                    request.name = args.name.map(|name| name.as_str().to_owned());
                    async move {
                        let response: pb::ShellDetachResponse = client
                            .unary_with_timeout("ShellDetach", request, SHELL_MANAGEMENT_TIMEOUT)
                            .await
                            .map_err(|_| TypedError::GuestShellDisabled)?;
                        shell_error_to_typed(response.error.as_ref())?;
                        map_shell_detach_response(response)
                    }
                })?;
            public_wire::ShellOpResponse::Detach(result)
        }
        public_wire::ShellOp::Kill(args) => {
            let result =
                run_guest_shell_management(state, args.vm.as_str(), |client, metadata| {
                    let mut request = pb::ShellKillRequest::new();
                    request.metadata = protobuf::MessageField::some(metadata);
                    request.name = args.name.as_str().to_owned();
                    async move {
                        let response: pb::ShellKillResponse = client
                            .unary_with_timeout("ShellKill", request, SHELL_MANAGEMENT_TIMEOUT)
                            .await
                            .map_err(|_| TypedError::GuestShellDisabled)?;
                        shell_error_to_typed(response.error.as_ref())?;
                        map_shell_kill_response(response)
                    }
                })?;
            public_wire::ShellOpResponse::Kill(result)
        }
        public_wire::ShellOp::Attach(_)
        | public_wire::ShellOp::WriteStdin(_)
        | public_wire::ShellOp::ReadOutput(_)
        | public_wire::ShellOp::Resize(_)
        | public_wire::ShellOp::Wait(_)
        | public_wire::ShellOp::CloseStdin(_)
        | public_wire::ShellOp::CloseAttach(_) => return Err(TypedError::GuestShellDisabled),
    };
    Ok(wire::shell_response(&response))
}

fn run_shell_owner(
    stream: Socket,
    state: ServerState,
    _peer: PeerIdentity,
    first_op_id: u64,
    first_op: public_wire::ShellOp,
    _conn_permit: Option<concurrency::ConnPermit>,
) {
    let public_wire::ShellOp::Attach(attach) = first_op else {
        let _ = write_json_frame(
            &stream,
            &wire::error_frame_with_id(first_op_id, &TypedError::GuestShellDisabled),
        );
        return;
    };
    let (client, guest_boot_id, attach_response) = match establish_shell_owner(&state, &attach) {
        Ok(value) => value,
        Err(error) => {
            let _ = write_json_frame(&stream, &wire::error_frame_with_id(first_op_id, &error));
            return;
        }
    };
    let session_id = match attach_response.session_id.clone() {
        Some(session) => session,
        None => {
            let _ = write_json_frame(
                &stream,
                &wire::error_frame_with_id(first_op_id, &TypedError::GuestShellDisabled),
            );
            return;
        }
    };
    let initial_control_seq = attach_response.control_seq;
    let public_attach = match map_shell_attach_response(attach_response) {
        Ok(value) => public_wire::ShellOpResponse::Attach(value),
        Err(error) => {
            let _ = write_json_frame(&stream, &wire::error_frame_with_id(first_op_id, &error));
            return;
        }
    };
    if write_json_frame(
        &stream,
        &wire::shell_response_with_id(first_op_id, &public_attach),
    )
    .is_err()
    {
        shell_close_attach_best_effort(&client, &attach.vm, &session_id, &guest_boot_id);
        return;
    }

    let mut control_seq = initial_control_seq;
    while let Ok(frame) = read_frame(&stream) {
        let op_id = wire::shell_op_id(&frame);
        let response = match wire::parse_shell_op(&frame) {
            Ok((_, op)) => handle_shell_owner_op(
                &client,
                &attach.vm,
                &session_id,
                &guest_boot_id,
                &mut control_seq,
                op,
            ),
            Err(error) => Err(error),
        };
        match response {
            Ok(Some(response)) => {
                if write_json_frame(&stream, &wire::shell_response_with_id(op_id, &response))
                    .is_err()
                {
                    break;
                }
            }
            Ok(None) => break,
            Err(error) => {
                if write_json_frame(&stream, &wire::error_frame_with_id(op_id, &error)).is_err() {
                    break;
                }
            }
        }
    }
    shell_close_attach_best_effort(&client, &attach.vm, &session_id, &guest_boot_id);
}

fn establish_shell_owner(
    state: &ServerState,
    attach: &public_wire::ShellAttachArgs,
) -> Result<
    (
        Arc<guest_control_health::TtrpcGuestControlClient>,
        String,
        pb::ShellAttachResponse,
    ),
    TypedError,
> {
    ensure_vm_runtime_capability(
        state,
        &attach.vm,
        RuntimeCapabilityGate::GuestControl,
        "shell",
    )?;
    let resolver = load_bundle_resolver(state)?;
    let params = resolve_guest_control_probe_params(state, &resolver, &attach.vm)
        .map_err(|_| TypedError::GuestShellDisabled)?;
    let broker_path = broker_socket_path(state);
    block_on_future(async move {
        let budget = guest_control_health::AttemptBudget::from_now(
            SHELL_MANAGEMENT_TIMEOUT,
            guest_control_bridge::GUEST_CONTROL_ATTEMPT_CAP,
        );
        let signer = guest_control_bridge::BrokerSigner::new(broker_path, budget);
        let nonce =
            guest_control_bridge::host_nonce().map_err(|_| TypedError::GuestShellDisabled)?;
        let client = guest_control_bridge::connect_and_build_client(&params, budget)
            .map_err(|_| TypedError::GuestShellDisabled)?;
        let evidence = guest_control_health::probe_guest_control_health(
            &params.vm_id,
            Some(guest_control_bridge::VMADDR_CID_HOST),
            nonce,
            &client,
            &signer,
        )
        .await
        .map_err(|_| TypedError::GuestShellDisabled)?;
        if !guest_advertises_capability(
            &evidence.health.capabilities,
            pb::GuestCapability::GUEST_CAPABILITY_SHELL_ATTACHED,
        ) {
            return Err(TypedError::GuestShellDisabled);
        }
        if attach.force
            && !guest_advertises_capability(
                &evidence.health.capabilities,
                pb::GuestCapability::GUEST_CAPABILITY_SHELL_FORCE_ATTACH,
            )
        {
            return Err(TypedError::GuestShellDisabled);
        }
        let mut request = pb::ShellAttachRequest::new();
        request.metadata = protobuf::MessageField::some(shell_request_metadata(&params.vm_id));
        request.name = attach.name.as_ref().map(|name| name.as_str().to_owned());
        request.force = attach.force;
        let mut size = pb::TerminalSize::new();
        size.rows = attach.initial_terminal_size.rows;
        size.cols = attach.initial_terminal_size.cols;
        request.initial_terminal_size = protobuf::MessageField::some(size);
        let response: pb::ShellAttachResponse = client
            .unary_with_timeout("ShellAttach", request, SHELL_MANAGEMENT_TIMEOUT)
            .await
            .map_err(|_| TypedError::GuestShellDisabled)?;
        shell_error_to_typed(response.error.as_ref())?;
        Ok((Arc::new(client), evidence.guest_boot_id, response))
    })
}

fn shell_request_metadata(vm: &str) -> pb::RequestMetadata {
    let mut metadata = pb::RequestMetadata::new();
    metadata.vm_id = vm.to_owned();
    metadata.request_id = "guest-control-shell".to_owned();
    metadata.protocol_version = nixling_ipc::guest_wire::GUEST_CONTROL_PROTOCOL_VERSION;
    metadata
}

fn shell_terminal_metadata(
    vm: &str,
    session_id: &str,
    guest_boot_id: &str,
) -> pb::TerminalRequestMetadata {
    let mut metadata = pb::TerminalRequestMetadata::new();
    metadata.common = protobuf::MessageField::some(shell_request_metadata(vm));
    metadata.session_id = session_id.to_owned();
    metadata.guest_boot_id = guest_boot_id.to_owned();
    metadata.kind = protobuf::EnumOrUnknown::new(pb::TerminalKind::TERMINAL_KIND_SHELL);
    metadata
}

fn handle_shell_owner_op(
    client: &guest_control_health::TtrpcGuestControlClient,
    vm: &str,
    session_id: &str,
    guest_boot_id: &str,
    control_seq: &mut u64,
    op: public_wire::ShellOp,
) -> Result<Option<public_wire::ShellOpResponse>, TypedError> {
    use nixling_ipc::terminal_wire as tw;
    match op {
        public_wire::ShellOp::WriteStdin(args) => {
            if args.session != session_id {
                return Err(TypedError::GuestShellDisabled);
            }
            let data = nixling_core::base64_codec::decode(&args.chunk_base64)
                .map_err(|_| TypedError::GuestShellDisabled)?;
            let mut request = pb::TerminalWriteStdinRequest::new();
            request.metadata = protobuf::MessageField::some(shell_terminal_metadata(
                vm,
                session_id,
                guest_boot_id,
            ));
            request.offset = args.offset;
            request.data = data;
            request.close_after = args.eof;
            let response: pb::WriteStdinResponse = block_on_future(client.unary_with_timeout(
                "TerminalWriteStdin",
                request,
                SHELL_MANAGEMENT_TIMEOUT,
            ))
            .map_err(|_| TypedError::GuestShellDisabled)?;
            shell_error_to_typed(response.error.as_ref())?;
            Ok(Some(public_wire::ShellOpResponse::WriteStdin(
                tw::TerminalWriteStdinResult {
                    accepted_len: response.accepted_len,
                    next_offset: response.next_offset,
                    backpressured: response.blocked_ms > 0,
                    stdin_closed: matches!(
                        response.stdin_state.enum_value(),
                        Ok(pb::StdinState::STDIN_STATE_CLOSED
                            | pb::StdinState::STDIN_STATE_CLOSED_BY_PROCESS
                            | pb::StdinState::STDIN_STATE_CLOSING)
                    ),
                },
            )))
        }
        public_wire::ShellOp::ReadOutput(args) => {
            if args.session != session_id {
                return Err(TypedError::GuestShellDisabled);
            }
            let mut request = pb::TerminalReadOutputRequest::new();
            request.metadata = protobuf::MessageField::some(shell_terminal_metadata(
                vm,
                session_id,
                guest_boot_id,
            ));
            request.stream = protobuf::EnumOrUnknown::new(match args.stream {
                tw::TerminalStream::Stdout => pb::OutputStream::OUTPUT_STREAM_STDOUT,
                tw::TerminalStream::Stderr => pb::OutputStream::OUTPUT_STREAM_STDERR,
            });
            request.offset = args.offset;
            request.max_len = args
                .max_len
                .min(nixling_ipc::public_wire::EXEC_MAX_CHUNK_BYTES);
            request.wait = args.wait;
            request.timeout_ms = args.timeout_ms;
            let response: pb::ReadOutputResponse = block_on_future(client.unary_with_timeout(
                "TerminalReadOutput",
                request,
                SHELL_MANAGEMENT_TIMEOUT,
            ))
            .map_err(|_| TypedError::GuestShellDisabled)?;
            shell_error_to_typed(response.error.as_ref())?;
            Ok(Some(public_wire::ShellOpResponse::ReadOutput(
                tw::TerminalReadOutputChunk {
                    data_base64: nixling_core::base64_codec::encode(&response.data),
                    next_offset: response.next_offset,
                    eof: response.eof,
                    dropped_bytes: response.dropped_bytes,
                    truncated: response.truncated,
                    timed_out: response.timed_out,
                },
            )))
        }
        public_wire::ShellOp::Resize(args) => {
            if args.session != session_id {
                return Err(TypedError::GuestShellDisabled);
            }
            *control_seq = control_seq.saturating_add(1);
            let mut request = pb::TerminalTtyWinResizeRequest::new();
            request.metadata = protobuf::MessageField::some(shell_terminal_metadata(
                vm,
                session_id,
                guest_boot_id,
            ));
            request.control_seq = *control_seq;
            request.rows = args.rows;
            request.cols = args.cols;
            let response: pb::ControlAck = block_on_future(client.unary_with_timeout(
                "TerminalTtyWinResize",
                request,
                SHELL_MANAGEMENT_TIMEOUT,
            ))
            .map_err(|_| TypedError::GuestShellDisabled)?;
            shell_error_to_typed(response.error.as_ref())?;
            Ok(Some(public_wire::ShellOpResponse::Resize(
                tw::TerminalControlResult { delivered: true },
            )))
        }
        public_wire::ShellOp::CloseStdin(args) => {
            if args.session != session_id {
                return Err(TypedError::GuestShellDisabled);
            }
            let mut request = pb::TerminalCloseStdinRequest::new();
            request.metadata = protobuf::MessageField::some(shell_terminal_metadata(
                vm,
                session_id,
                guest_boot_id,
            ));
            let response: pb::CloseStdinResponse = block_on_future(client.unary_with_timeout(
                "TerminalCloseStdin",
                request,
                SHELL_MANAGEMENT_TIMEOUT,
            ))
            .map_err(|_| TypedError::GuestShellDisabled)?;
            shell_error_to_typed(response.error.as_ref())?;
            Ok(Some(public_wire::ShellOpResponse::CloseStdin(
                tw::TerminalCloseResult { stdin_closed: true },
            )))
        }
        public_wire::ShellOp::CloseAttach(args) => {
            if args.session != session_id {
                return Err(TypedError::GuestShellDisabled);
            }
            let response = shell_close_attach(client, vm, session_id, guest_boot_id)?;
            Ok(Some(public_wire::ShellOpResponse::CloseAttach(response)))
        }
        public_wire::ShellOp::Wait(args) => {
            if args.session != session_id {
                return Err(TypedError::GuestShellDisabled);
            }
            Ok(Some(public_wire::ShellOpResponse::Wait(
                tw::TerminalWaitResult {
                    running: true,
                    terminal_status: None,
                },
            )))
        }
        public_wire::ShellOp::Attach(_)
        | public_wire::ShellOp::List(_)
        | public_wire::ShellOp::Detach(_)
        | public_wire::ShellOp::Kill(_) => Err(TypedError::GuestShellDisabled),
    }
}

fn shell_close_attach(
    client: &guest_control_health::TtrpcGuestControlClient,
    vm: &str,
    session_id: &str,
    guest_boot_id: &str,
) -> Result<public_wire::ShellDetachResult, TypedError> {
    let mut request = pb::ShellCloseAttachRequest::new();
    request.metadata =
        protobuf::MessageField::some(shell_terminal_metadata(vm, session_id, guest_boot_id));
    let response: pb::ShellDetachResponse = block_on_future(client.unary_with_timeout(
        "ShellCloseAttach",
        request,
        SHELL_MANAGEMENT_TIMEOUT,
    ))
    .map_err(|_| TypedError::GuestShellDisabled)?;
    shell_error_to_typed(response.error.as_ref())?;
    map_shell_detach_response(response)
}

fn shell_close_attach_best_effort(
    client: &guest_control_health::TtrpcGuestControlClient,
    vm: &str,
    session_id: &str,
    guest_boot_id: &str,
) {
    let _ = shell_close_attach(client, vm, session_id, guest_boot_id);
}

fn run_guest_shell_management<F, Fut, T>(
    state: &ServerState,
    vm: &str,
    f: F,
) -> Result<T, TypedError>
where
    F: FnOnce(Arc<guest_control_health::TtrpcGuestControlClient>, pb::RequestMetadata) -> Fut,
    Fut: Future<Output = Result<T, TypedError>>,
{
    ensure_vm_runtime_capability(state, vm, RuntimeCapabilityGate::GuestControl, "shell")?;
    let resolver = load_bundle_resolver(state)?;
    let params = resolve_guest_control_probe_params(state, &resolver, vm).map_err(|detail| {
        tracing::warn!(
            kind = "critical",
            subsystem = "guest-control-shell",
            error_kind = "transport-io",
            "guest-control shell: probe params unresolved: {detail}"
        );
        TypedError::GuestShellDisabled
    })?;
    let broker_path = broker_socket_path(state);
    block_on_future(async move {
        let budget = guest_control_health::AttemptBudget::from_now(
            SHELL_MANAGEMENT_TIMEOUT,
            guest_control_bridge::GUEST_CONTROL_ATTEMPT_CAP,
        );
        let signer = guest_control_bridge::BrokerSigner::new(broker_path, budget);
        let nonce =
            guest_control_bridge::host_nonce().map_err(|_| TypedError::GuestShellDisabled)?;
        let client = guest_control_bridge::connect_and_build_client(&params, budget)
            .map_err(|_| TypedError::GuestShellDisabled)?;
        let evidence = guest_control_health::probe_guest_control_health(
            &params.vm_id,
            Some(guest_control_bridge::VMADDR_CID_HOST),
            nonce,
            &client,
            &signer,
        )
        .await
        .map_err(|_| TypedError::GuestShellDisabled)?;
        if !guest_advertises_capability(
            &evidence.health.capabilities,
            pb::GuestCapability::GUEST_CAPABILITY_SHELL_MANAGEMENT,
        ) {
            return Err(TypedError::GuestShellDisabled);
        }
        let mut metadata = pb::RequestMetadata::new();
        metadata.vm_id = params.vm_id.clone();
        metadata.request_id = "guest-control-shell".to_owned();
        metadata.protocol_version = nixling_ipc::guest_wire::GUEST_CONTROL_PROTOCOL_VERSION;
        f(Arc::new(client), metadata).await
    })
}

fn guest_advertises_capability(
    capabilities: &[protobuf::EnumOrUnknown<pb::GuestCapability>],
    cap: pb::GuestCapability,
) -> bool {
    capabilities
        .iter()
        .filter_map(|value| value.enum_value().ok())
        .any(|value| value == cap)
}

fn shell_error_to_typed(error: Option<&pb::GuestControlError>) -> Result<(), TypedError> {
    if let Some(error) = error
        && !exec_session_real::is_unspecified(error.kind)
    {
        return Err(TypedError::GuestShellDisabled);
    }
    Ok(())
}

fn shell_name_from_guest(value: String) -> Result<public_wire::ShellName, TypedError> {
    public_wire::ShellName::new(value).map_err(|_| TypedError::WireInvalidFrame {
        detail: "guest shell response carried an invalid shell name".to_owned(),
    })
}

fn map_shell_state(
    state: protobuf::EnumOrUnknown<pb::ShellState>,
) -> public_wire::ShellSessionState {
    match state
        .enum_value()
        .unwrap_or(pb::ShellState::SHELL_STATE_UNSPECIFIED)
    {
        pb::ShellState::SHELL_STATE_ATTACHED => public_wire::ShellSessionState::Attached,
        pb::ShellState::SHELL_STATE_DETACHED => public_wire::ShellSessionState::Detached,
        pb::ShellState::SHELL_STATE_KILLED => public_wire::ShellSessionState::Killed,
        pb::ShellState::SHELL_STATE_POOL_UNAVAILABLE => {
            public_wire::ShellSessionState::PoolUnavailable
        }
        pb::ShellState::SHELL_STATE_FEATURE_DISABLED => {
            public_wire::ShellSessionState::FeatureDisabled
        }
        pb::ShellState::SHELL_STATE_OUTPUT_GAP => public_wire::ShellSessionState::OutputGap,
        pb::ShellState::SHELL_STATE_UNSPECIFIED => public_wire::ShellSessionState::Detached,
    }
}

fn map_shell_close_cause(
    cause: protobuf::EnumOrUnknown<pb::ShellCloseCause>,
) -> Option<public_wire::ShellCloseCause> {
    match cause
        .enum_value()
        .unwrap_or(pb::ShellCloseCause::SHELL_CLOSE_CAUSE_UNSPECIFIED)
    {
        pb::ShellCloseCause::SHELL_CLOSE_CAUSE_CLIENT_DETACH => {
            Some(public_wire::ShellCloseCause::ClientDetach)
        }
        pb::ShellCloseCause::SHELL_CLOSE_CAUSE_EVICTED_BY_FORCE => {
            Some(public_wire::ShellCloseCause::EvictedByForce)
        }
        pb::ShellCloseCause::SHELL_CLOSE_CAUSE_EVICTED_BY_ADMIN_DETACH => {
            Some(public_wire::ShellCloseCause::EvictedByAdminDetach)
        }
        pb::ShellCloseCause::SHELL_CLOSE_CAUSE_KILLED_BY_ADMIN => {
            Some(public_wire::ShellCloseCause::KilledByAdmin)
        }
        pb::ShellCloseCause::SHELL_CLOSE_CAUSE_POOL_UNAVAILABLE => {
            Some(public_wire::ShellCloseCause::PoolUnavailable)
        }
        pb::ShellCloseCause::SHELL_CLOSE_CAUSE_OUTPUT_GAP => {
            Some(public_wire::ShellCloseCause::OutputGap)
        }
        pb::ShellCloseCause::SHELL_CLOSE_CAUSE_UNSPECIFIED => None,
    }
}

fn map_shell_list_response(
    response: pb::ShellListResponse,
) -> Result<public_wire::ShellListResult, TypedError> {
    Ok(public_wire::ShellListResult {
        default_name: shell_name_from_guest(response.default_name)?,
        sessions: response
            .sessions
            .into_iter()
            .map(|entry| {
                Ok(public_wire::ShellListEntry {
                    name: shell_name_from_guest(entry.name)?,
                    state: map_shell_state(entry.state),
                    attached: entry.attached,
                    is_default: entry.is_default,
                })
            })
            .collect::<Result<Vec<_>, TypedError>>()?,
    })
}

fn map_shell_attach_response(
    response: pb::ShellAttachResponse,
) -> Result<public_wire::ShellAttachResult, TypedError> {
    Ok(public_wire::ShellAttachResult {
        session: response.session_id.ok_or(TypedError::GuestShellDisabled)?,
        resolved_name: shell_name_from_guest(response.resolved_name)?,
        state: map_shell_state(response.state),
        force_evicted: response.force_evicted,
    })
}

fn map_shell_detach_response(
    response: pb::ShellDetachResponse,
) -> Result<public_wire::ShellDetachResult, TypedError> {
    Ok(public_wire::ShellDetachResult {
        resolved_name: shell_name_from_guest(response.resolved_name)?,
        detached: response.detached,
        cause: map_shell_close_cause(response.cause),
    })
}

fn map_shell_kill_response(
    response: pb::ShellKillResponse,
) -> Result<public_wire::ShellKillResult, TypedError> {
    Ok(public_wire::ShellKillResult {
        name: shell_name_from_guest(response.name)?,
        killed: response.killed,
        state: map_shell_state(response.state),
    })
}

const EXEC_METRIC: &str = "nixling_daemon_guest_control_exec_total";
const EXEC_SUBSYSTEM: &str = "guest-control-exec";

/// Closed allowlist of `outcome` label values for the exec metric. Any value
/// emitted outside this set is a hard bug (caught by `debug_assert` below and
/// the metric-label allowlist test) — `outcome` MUST stay a bounded enum so the
/// `nixling_daemon_guest_control_exec_total` series cannot explode in
/// cardinality or leak a free-form string.
const EXEC_OUTCOME_LABELS: &[&str] = &["established", "closed", "error", "op-error"];

/// Closed allowlist of `error_kind` label values for the exec metric. Mirrors
/// the literals at the `exec_metric` call sites plus every value
/// [`exec_error_kind_label`] can return. `none` is the sentinel for a
/// non-error outcome (establish/close); `inflight-cap-exceeded` is the
/// owner-reader signal emitted when a session is closed for exceeding the
/// per-connection in-flight op cap.
const EXEC_ERROR_KIND_LABELS: &[&str] = &[
    "none",
    "transport",
    "auth",
    "protocol",
    "timeout",
    "old-generation",
    "capability",
    "detached-unavailable",
    "session-capacity",
    "rate-limited",
    "stale-session",
    "exec-not-found",
    "exec-expired",
    "invalid-program",
    "guest",
    "internal",
    "inflight-cap-exceeded",
];

/// Increment the closed-label exec outcome counter. `outcome` and `error_kind`
/// are the only labels besides the constant subsystem; all three are drawn
/// from a hard allowlist (no vm/uid/handle/argv ever becomes a label).
fn exec_metric(state: &ServerState, outcome: &'static str, error_kind: &'static str) {
    exec_metric_into(&state.metrics_registry, outcome, error_kind);
}

/// Increment the exec outcome counter against a metrics registry directly. Used
/// by the owner writer, which holds only the registry (not the full
/// `ServerState`) so it is hermetically testable.
fn exec_metric_into(registry: &metrics::Registry, outcome: &'static str, error_kind: &'static str) {
    // Fail-closed in debug/test builds: the only labels are the constant
    // subsystem plus a closed `outcome` / `error_kind` enum. A stray value
    // (e.g. a vm name, session handle, or op id mistakenly threaded through)
    // would be caught here and by `exec_metric_labels_are_closed_enum`.
    debug_assert!(
        EXEC_OUTCOME_LABELS.contains(&outcome),
        "exec metric outcome {outcome:?} is not in the closed allowlist"
    );
    debug_assert!(
        EXEC_ERROR_KIND_LABELS.contains(&error_kind),
        "exec metric error_kind {error_kind:?} is not in the closed allowlist"
    );
    registry.counter_inc(
        EXEC_METRIC,
        &[
            ("subsystem", EXEC_SUBSYSTEM),
            ("outcome", outcome),
            ("error_kind", error_kind),
        ],
    );
}

/// The opaque session handle carried by a non-`Start` exec op, or `None` for
/// `Start` (which has no session yet).
fn exec_op_session(op: &public_wire::ExecOp) -> Option<&str> {
    match op {
        public_wire::ExecOp::Start(_) => None,
        public_wire::ExecOp::WriteStdin(args) => Some(&args.session),
        public_wire::ExecOp::ReadOutput(args) => Some(&args.session),
        public_wire::ExecOp::Signal(args) => Some(&args.session),
        public_wire::ExecOp::Resize(args) => Some(&args.session),
        public_wire::ExecOp::Wait(args) => Some(&args.session),
        public_wire::ExecOp::Close(args) => Some(&args.session),
        public_wire::ExecOp::List(_)
        | public_wire::ExecOp::Logs(_)
        | public_wire::ExecOp::Status(_)
        | public_wire::ExecOp::Kill(_) => None,
    }
}

fn map_exec_establish_error(error: exec_session::ExecEstablishError) -> TypedError {
    use crate::typed_error::GuestControlExecErrorKind as K;
    use exec_session::ExecEstablishError as E;
    let kind = match error {
        E::Transport => K::Transport,
        E::Auth => K::Auth,
        E::Protocol => K::Protocol,
        E::Timeout => K::Timeout,
        E::OldGeneration => K::OldGeneration,
        E::Capability => K::Capability,
        E::Guest(inner) => map_guest_exec_error_kind(inner),
    };
    TypedError::GuestControlExecFailed { kind }
}

fn map_guest_exec_error_kind(
    error: exec_session::GuestOpError,
) -> crate::typed_error::GuestControlExecErrorKind {
    use crate::typed_error::GuestControlExecErrorKind as K;
    use exec_session::GuestOpError as E;
    match error {
        E::ExecNotFound => K::ExecNotFound,
        E::ExecExpired => K::ExecExpired,
        E::InvalidProgram => K::InvalidProgram,
        E::Protocol => K::Protocol,
        _ => K::GuestError,
    }
}

fn map_exec_op_error(error: exec_session::ExecOpError) -> TypedError {
    use crate::typed_error::GuestControlExecErrorKind as K;
    use exec_session::ExecOpError as E;
    let kind = match error {
        E::Transport => K::Transport,
        E::Auth => K::Auth,
        E::StaleSession => K::StaleSession,
        E::Protocol => K::Protocol,
        E::Timeout => K::Timeout,
        E::OldGeneration => K::OldGeneration,
        E::Capability => K::Capability,
        E::DetachedUnavailable => K::DetachedUnavailable,
        E::Guest(inner) => map_guest_exec_error_kind(inner),
    };
    TypedError::GuestControlExecFailed { kind }
}

fn map_exec_reserve_error(error: exec_session::SessionReserveError) -> TypedError {
    use crate::typed_error::GuestControlExecErrorKind as K;
    use exec_session::SessionReserveError as E;
    let kind = match error {
        E::RateLimited => K::RateLimited,
        _ => K::SessionCapacity,
    };
    TypedError::GuestControlExecFailed { kind }
}

/// The `error_kind` metric label for a typed exec failure (closed allowlist).
fn exec_error_kind_label(error: &TypedError) -> &'static str {
    use crate::typed_error::GuestControlExecErrorKind as K;
    match error {
        TypedError::GuestControlExecFailed { kind } => match kind {
            K::Transport => "transport",
            K::Auth => "auth",
            K::Protocol => "protocol",
            K::Timeout => "timeout",
            K::OldGeneration => "old-generation",
            K::Capability => "capability",
            K::DetachedUnavailable => "detached-unavailable",
            K::SessionCapacity => "session-capacity",
            K::RateLimited => "rate-limited",
            K::StaleSession => "stale-session",
            K::ExecNotFound => "exec-not-found",
            K::ExecExpired => "exec-expired",
            K::InvalidProgram => "invalid-program",
            K::GuestError => "guest",
            K::Internal => "internal",
        },
        _ => "internal",
    }
}

/// Emit the single kind=critical exec session-establishment event. Kept as a
/// free function so the redaction-safe field set can be asserted by a tracing
/// capture test: it accepts ONLY the leak-safe identifiers (vm name, peer uid,
/// negotiated tty). The opaque session handle is deliberately NOT included —
/// per AGENTS, session handles must never reach a span, log, audit, or
/// metric. argv/env/cwd/output bytes are never passed here either.
fn emit_exec_established_event(vm: &str, peer_uid: u32, tty: bool) {
    tracing::info!(
        kind = "critical",
        subsystem = EXEC_SUBSYSTEM,
        vm = %vm,
        peer_uid = peer_uid,
        tty = tty,
        "guest-control exec session established"
    );
}

fn emit_detached_create_audit(state: &ServerState, peer_uid: u32, vm: &str, exec_id: &str) {
    if let Err(err) =
        state
            .daemon_audit
            .write_event(&daemon_audit::DaemonEvent::GuestControlExecDetachedCreate {
                vm: vm.to_owned(),
                peer_uid,
                action: daemon_audit::DetachedExecAuditAction::Create,
                result: daemon_audit::DetachedExecAuditResult::Created,
                exec_id: exec_id.to_owned(),
            })
    {
        tracing::warn!(
            error = %err,
            "failed to write detached exec create daemon audit event"
        );
    }
}

fn emit_detached_kill_audit(
    state: &ServerState,
    peer_uid: u32,
    vm: &str,
    result: Result<&public_wire::ExecDetachedKillResult, &TypedError>,
) {
    let (audit_result, exec_id) = match result {
        Ok(kill) => (
            match kill.result {
                public_wire::ExecDetachedKillOutcome::Cancelling => {
                    daemon_audit::DetachedExecAuditResult::Cancelling
                }
                public_wire::ExecDetachedKillOutcome::AlreadyTerminal => {
                    daemon_audit::DetachedExecAuditResult::AlreadyTerminal
                }
            },
            kill.exec_id.as_str(),
        ),
        Err(_) => (
            daemon_audit::DetachedExecAuditResult::Error,
            "<redacted-on-error>",
        ),
    };
    if let Err(err) =
        state
            .daemon_audit
            .write_event(&daemon_audit::DaemonEvent::GuestControlExecDetachedKill {
                vm: vm.to_owned(),
                peer_uid,
                action: daemon_audit::DetachedExecAuditAction::Cancel,
                result: audit_result,
                exec_id: exec_id.to_owned(),
            })
    {
        tracing::warn!(
            error = %err,
            "failed to write detached exec kill daemon audit event"
        );
    }
}

/// Owner-connection handler for an exec session. Runs on a SPAWNED thread off
/// the serial accept loop: the public.sock accept loop never blocks for
/// the lifetime of an exec. SO_PEERCRED admin was verified before the spawn.
///
/// Lifecycle (non-detached): reserve a session slot (cap-checked BEFORE any
/// connect/auth/ExecCreate), spawn the per-session worker, relay the establish
/// reply, then proxy one op per frame. The connection's EOF/POLLHUP closes the
/// command channel, which returns the worker, drops the runtime, and drops the
/// authenticated client — prompting the guest `close_connection` and PTY
/// teardown. The slot is released when its RAII guard drops on return.
fn run_exec_owner(
    stream: Socket,
    state: ServerState,
    peer: PeerIdentity,
    first_op_id: u64,
    first_op: public_wire::ExecOp,
    // Admission permit held for the lifetime of the exec session so the
    // in-flight connection slot is released only on owner termination.
    _conn_permit: Option<concurrency::ConnPermit>,
) {
    // Test seam: when an accept-loop test installs the owner-body hook, run it
    // (holding `stream` — and thus the owner session — open for as long as the
    // hook blocks) and return without touching real bundle/guest state. Placing
    // the hook HERE, in the owner body itself, is what lets a test distinguish
    // an off-loop spawn from an inline call: a hypothetical inline
    // `handle_connection` would run this body — and block in the hook — on the
    // accept-loop thread, so its caller would never observe a prompt return.
    #[cfg(test)]
    {
        if let Some(hook) = exec_owner_test_hook::active() {
            hook();
            drop(stream);
            return;
        }
    }
    // The owner socket is read by the reader (this thread) and written by a
    // dedicated writer thread concurrently; SOCK_SEQPACKET send/recv on the
    // same fd from two threads is safe, so share it behind an `Arc`.
    let stream = Arc::new(stream);
    let public_wire::ExecOp::Start(start) = first_op else {
        // A non-`Start` first op on a fresh owner connection has no session.
        let error = TypedError::GuestControlExecFailed {
            kind: crate::typed_error::GuestControlExecErrorKind::Protocol,
        };
        let _ = write_json_frame(
            stream.as_ref(),
            &wire::error_frame_with_id(first_op_id, &error),
        );
        exec_metric(&state, "error", "protocol");
        return;
    };

    if let Err(error) =
        ensure_vm_runtime_capability(&state, &start.vm, RuntimeCapabilityGate::Exec, "exec")
    {
        let _ = write_json_frame(
            stream.as_ref(),
            &wire::error_frame_with_id(first_op_id, &error),
        );
        exec_metric(&state, "error", error.kind());
        return;
    }

    if start.argv.is_empty() {
        let error = TypedError::GuestControlExecFailed {
            kind: crate::typed_error::GuestControlExecErrorKind::Protocol,
        };
        let _ = write_json_frame(
            stream.as_ref(),
            &wire::error_frame_with_id(first_op_id, &error),
        );
        exec_metric(&state, "error", "protocol");
        return;
    }
    if start.detached {
        match exec_detached::create(&state, &start) {
            Ok(result) => {
                emit_detached_create_audit(&state, peer.uid, &start.vm, &result.exec_id);
                let response = public_wire::ExecOpResponse::DetachedCreate(result);
                if write_json_frame(
                    stream.as_ref(),
                    &wire::exec_response_with_id(first_op_id, &response),
                )
                .is_err()
                {
                    exec_metric(&state, "error", "transport");
                    return;
                }
                exec_metric(&state, "established", "none");
            }
            Err(error) => {
                let kind = exec_error_kind_label(&error);
                let _ = write_json_frame(
                    stream.as_ref(),
                    &wire::error_frame_with_id(first_op_id, &error),
                );
                exec_metric(&state, "error", kind);
            }
        }
        return;
    }

    let resolver = match load_bundle_resolver(&state) {
        Ok(resolver) => resolver,
        Err(_) => {
            let error = TypedError::GuestControlExecFailed {
                kind: crate::typed_error::GuestControlExecErrorKind::Transport,
            };
            let _ = write_json_frame(
                stream.as_ref(),
                &wire::error_frame_with_id(first_op_id, &error),
            );
            exec_metric(&state, "error", "transport");
            return;
        }
    };
    let params = match resolve_guest_control_probe_params(&state, &resolver, &start.vm) {
        Ok(params) => params,
        Err(_) => {
            // No process DAG / no vsock socket: the VM is not a guest-control
            // generation. Fail closed (old-generation), never SSH.
            let error = TypedError::GuestControlExecFailed {
                kind: crate::typed_error::GuestControlExecErrorKind::OldGeneration,
            };
            let _ = write_json_frame(
                stream.as_ref(),
                &wire::error_frame_with_id(first_op_id, &error),
            );
            exec_metric(&state, "error", "old-generation");
            return;
        }
    };

    // Reserve a session slot BEFORE any connect/auth/ExecCreate. The
    // guard releases the slot on every return path below.
    let slot = match state.exec_sessions.reserve(peer.uid, &start.vm) {
        Ok(slot) => slot,
        Err(reserve_error) => {
            let error = map_exec_reserve_error(reserve_error);
            let kind = exec_error_kind_label(&error);
            let _ = write_json_frame(
                stream.as_ref(),
                &wire::error_frame_with_id(first_op_id, &error),
            );
            exec_metric(&state, "error", kind);
            return;
        }
    };
    let handle = slot.handle().to_owned();

    let spec = exec_session::ExecStartSpec {
        vm: start.vm.clone(),
        argv: start.argv.clone(),
        tty: start.tty,
        detached: start.detached,
        env: start
            .env
            .unwrap_or_default()
            .into_iter()
            .map(|var| (var.key, var.value))
            .collect(),
        cwd: start.cwd.clone(),
        term_size: start.term_size.map(|size| (size.rows, size.cols)),
    };

    let deadlines = exec_session::ExecOpDeadlines::default();
    let connector: Arc<dyn exec_session::ExecGuestConnector> = Arc::new(
        exec_session_real::RealExecConnector::new(params, broker_socket_path(&state), deadlines),
    );

    // The terminal-cleanup reaper shuts down the owner socket so a
    // stalled owner that never closes after the command goes terminal does not
    // pin the session slot. It only fires AFTER the command is terminal, never
    // while the command is live.
    let owner_reaper: Arc<dyn exec_session::OwnerReaper> =
        Arc::new(SocketShutdownReaper::new(stream.as_raw_fd()));

    let (control_tx, control_rx) = tokio::sync::mpsc::channel::<exec_session::WorkerCommand>(16);
    let (establish_tx, establish_rx) = tokio::sync::oneshot::channel();
    let worker = exec_session::spawn_session_worker(exec_session::WorkerSpawn {
        connector,
        spec,
        deadlines,
        establish_tx,
        control_rx,
        terminal_ttl: exec_session::EXEC_TERMINAL_CLEANUP_TTL,
        clock: Arc::new(exec_session::SystemClock),
        owner_reaper,
    });

    let info = match establish_rx.blocking_recv() {
        Ok(Ok(info)) => info,
        Ok(Err(establish_error)) => {
            let error = map_exec_establish_error(establish_error);
            let kind = exec_error_kind_label(&error);
            let _ = write_json_frame(
                stream.as_ref(),
                &wire::error_frame_with_id(first_op_id, &error),
            );
            exec_metric(&state, "error", kind);
            drop(control_tx);
            let _ = worker.join();
            return;
        }
        Err(_) => {
            // Worker thread vanished before replying (panic / runtime build
            // failure already mapped to Transport). Surface an internal error.
            let error = TypedError::GuestControlExecFailed {
                kind: crate::typed_error::GuestControlExecErrorKind::Internal,
            };
            let _ = write_json_frame(
                stream.as_ref(),
                &wire::error_frame_with_id(first_op_id, &error),
            );
            exec_metric(&state, "error", "internal");
            let _ = worker.join();
            return;
        }
    };

    // One kind=critical session-establishment span (NO per-op span/audit).
    emit_exec_established_event(&start.vm, peer.uid, info.tty);
    exec_metric(&state, "established", "none");
    // Bounded lifecycle audit (leak-safe: vm + admin uid + tty only).
    let _ =
        state
            .daemon_audit
            .write_event(&daemon_audit::DaemonEvent::GuestControlExecEstablished {
                vm: start.vm.clone(),
                peer_uid: peer.uid,
                tty: info.tty,
            });

    // Spawn the owner writer thread BEFORE committing the session with a start
    // response. An OS thread-spawn failure must surface as a typed internal
    // error for the establishing op rather than panic the handler (a process
    // exhausting its thread limit is a recoverable resource failure, not a
    // daemon-fatal bug). With the writer up first, that failure cleanly replaces
    // the start response.
    let (writer, item_tx, inflight) =
        match spawn_exec_owner_writer(&stream, &state.metrics_registry) {
            Ok(parts) => parts,
            Err(_) => {
                let error = TypedError::GuestControlExecFailed {
                    kind: crate::typed_error::GuestControlExecErrorKind::Internal,
                };
                let _ = write_json_frame(
                    stream.as_ref(),
                    &wire::error_frame_with_id(first_op_id, &error),
                );
                exec_metric(&state, "error", "internal");
                drop(control_tx);
                let _ = worker.join();
                exec_metric(&state, "closed", "none");
                let _ = state.daemon_audit.write_event(
                    &daemon_audit::DaemonEvent::GuestControlExecTerminated {
                        vm: start.vm.clone(),
                        peer_uid: peer.uid,
                    },
                );
                return;
            }
        };

    let start_response = exec_session::start_response(&handle, &info);
    if write_json_frame(
        stream.as_ref(),
        &wire::exec_response_with_id(first_op_id, &start_response),
    )
    .is_err()
    {
        drop(item_tx);
        drop(control_tx);
        let _ = writer.join();
        let _ = worker.join();
        exec_metric(&state, "closed", "none");
        let _ = state.daemon_audit.write_event(
            &daemon_audit::DaemonEvent::GuestControlExecTerminated {
                vm: start.vm.clone(),
                peer_uid: peer.uid,
            },
        );
        return;
    }

    // Drive the owner connection: a reader (this thread) dispatches frames to
    // the worker WITHOUT blocking on each reply, and the writer thread drains
    // op-id-tagged replies back to the socket. `control_tx` is moved in and
    // dropped after the reader returns, so an owner disconnect during a
    // long-poll tears the worker down (cancelling the poll) promptly.
    run_exec_owner_io(
        &stream,
        control_tx,
        item_tx,
        inflight,
        writer,
        &state.metrics_registry,
        &handle,
    );

    let _ = worker.join();
    drop(slot);
    exec_metric(&state, "closed", "none");
    let _ =
        state
            .daemon_audit
            .write_event(&daemon_audit::DaemonEvent::GuestControlExecTerminated {
                vm: start.vm.clone(),
                peer_uid: peer.uid,
            });
}

/// A reply frame the owner writer must emit, carried from the per-op awaiter to
/// the single socket-writing task so all owner-socket sends happen on one task.
enum ExecOwnerFrame {
    Response(Box<public_wire::ExecOpResponse>),
    Error {
        error: Box<TypedError>,
        metric_kind: &'static str,
    },
}

/// One item handed from the owner reader to the owner writer. `Pending` carries
/// the worker reply receiver (awaited concurrently so multiple ops, including a
/// long-poll plus an urgent control op, are in flight at once and matched by
/// `op_id`). `Immediate` is a reader-resolved error (parse / session-binding /
/// non-exec frame) that needs no worker round trip. Both variants carry the
/// owned [`InflightPermit`] for the op so the writer releases it only after the
/// reply frame is written (or on teardown), enforcing the real in-flight cap.
enum ExecWriterItem {
    Pending {
        op_id: u64,
        reply_rx: tokio::sync::oneshot::Receiver<
            Result<public_wire::ExecOpResponse, exec_session::ExecOpError>,
        >,
        permit: InflightPermit,
    },
    Immediate {
        op_id: u64,
        error: Box<TypedError>,
        metric_kind: &'static str,
        permit: InflightPermit,
    },
}

/// Bound on owner-connection ops concurrently in flight. This is a HARD
/// per-connection limit on the number of ops dispatched-but-not-yet-replied —
/// including long-polls (`ReadOutput`/`Wait`) that each pin a guest RPC. A
/// backpressure-aware owner (the real CLI is strictly sequential — one op,
/// await its reply, then the next) stays at 1–2 in flight and never approaches
/// this cap; a flooding/pipelining owner that exceeds it has its session closed
/// promptly (the reader never blocks acquiring a permit).
const EXEC_OWNER_INFLIGHT_CAP: usize = 64;

/// Bounded grace for the owner writer to flush its last resolved replies (e.g. a
/// final exit-status `Wait`) during teardown before the owner socket is
/// force–shut-down. A healthy writer exits in microseconds; this only bounds the
/// wait for a writer wedged on a blocking `send` to an owner that stopped
/// reading, after which the socket is shut down so the send fails and the writer
/// can exit (otherwise `join()` would hang and strand the owner thread + slot).
const EXEC_OWNER_WRITER_DRAIN_GRACE: Duration = Duration::from_millis(250);
const EXEC_OWNER_WRITER_DRAIN_POLL: Duration = Duration::from_millis(5);

/// A non-blocking counting semaphore bounding the owner connection's actual
/// concurrent in-flight ops. The earlier design only bounded the
/// reader→writer channel, but the worker immediately spawns each long-poll and
/// the writer immediately spawns each awaiter, so both channels drained as fast
/// as the reader filled them — the reader was never bounded and a pipelining
/// owner could open unbounded concurrent long-polls/guest RPCs. Here a permit
/// is taken just before an op is dispatched and HELD until its reply frame is
/// written (or the op is torn down), so the cap hard-bounds the number of ops
/// genuinely in flight. A permit is taken with the NON-BLOCKING
/// [`InflightSemaphore::try_acquire`]: a well-behaved (backpressure-aware) owner
/// stays far below the cap, and an owner that exceeds it has its session closed
/// promptly rather than the reader BLOCKING on a permit (which would delay
/// observing owner EOF/POLLHUP under saturation). The reader runs on a plain OS
/// thread (no tokio runtime), so a plain `Mutex<usize>` is used rather than
/// `tokio::sync::Semaphore`.
struct InflightSemaphore {
    available: std::sync::Mutex<usize>,
}

impl InflightSemaphore {
    fn new(permits: usize) -> Arc<Self> {
        Arc::new(Self {
            available: std::sync::Mutex::new(permits),
        })
    }

    /// Take a permit WITHOUT blocking. Returns `Some(permit)` iff one is free,
    /// else `None` (the cap is saturated). The returned guard releases the
    /// permit on drop (reply written, immediate error, or teardown). Never
    /// blocks: the reader must always be free to return to `read_frame` and
    /// observe owner EOF/POLLHUP. Mutex poison is recovered rather than
    /// propagated as a panic — the critical section only increments/decrements
    /// a counter and cannot leave broken invariants.
    fn try_acquire(self: &Arc<Self>) -> Option<InflightPermit> {
        let mut available = self
            .available
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if *available == 0 {
            return None;
        }
        *available -= 1;
        Some(InflightPermit {
            semaphore: Arc::clone(self),
        })
    }
}

/// RAII permit for [`InflightSemaphore`]. Releasing on drop covers every
/// teardown path (reply written, immediate error frame written, owner
/// disconnect, worker teardown dropping the reply oneshot).
struct InflightPermit {
    semaphore: Arc<InflightSemaphore>,
}

impl Drop for InflightPermit {
    fn drop(&mut self) {
        let mut available = self
            .semaphore
            .available
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        *available += 1;
    }
}

/// Parts produced by [`spawn_exec_owner_writer`]: the writer join handle, the
/// reader→writer item channel sender, and the shared in-flight semaphore.
type ExecOwnerWriterParts = (
    std::thread::JoinHandle<()>,
    tokio::sync::mpsc::Sender<ExecWriterItem>,
    Arc<InflightSemaphore>,
);

/// Spawn the owner-connection writer thread and return the channel + semaphore
/// the reader drives it with. Returns the OS-thread-spawn error to the caller
/// (instead of panicking) so a resource-exhausted host fails the session with a
/// typed internal error rather than crashing the handler.
fn spawn_exec_owner_writer(
    stream: &Arc<Socket>,
    metrics: &Arc<metrics::Registry>,
) -> std::io::Result<ExecOwnerWriterParts> {
    let (item_tx, item_rx) = tokio::sync::mpsc::channel::<ExecWriterItem>(EXEC_OWNER_INFLIGHT_CAP);
    let inflight = InflightSemaphore::new(EXEC_OWNER_INFLIGHT_CAP);
    let writer_stream = Arc::clone(stream);
    let writer_metrics = Arc::clone(metrics);
    let writer = std::thread::Builder::new()
        .name("nixling-exec-writer".to_owned())
        .spawn(move || exec_owner_writer(writer_stream, writer_metrics, item_rx))?;
    Ok((writer, item_tx, inflight))
}

/// Emit the closed-allowlist observability signal for an owner connection that
/// exceeded the in-flight op cap and is being closed. A leak-safe metric
/// (closed `outcome`/`error_kind` labels — no vm/handle/uid/argv) plus a
/// rate-bounded structured log carrying only the constant cap. No wire frame is
/// written from the reader thread (the writer thread is the sole socket
/// writer).
fn signal_owner_inflight_cap_exceeded(metrics: &metrics::Registry) {
    exec_metric_into(metrics, "op-error", "inflight-cap-exceeded");
    tracing::warn!(
        kind = "critical",
        subsystem = EXEC_SUBSYSTEM,
        error_kind = "inflight-cap-exceeded",
        cap = EXEC_OWNER_INFLIGHT_CAP,
        "guest-control-exec: owner connection exceeded the in-flight op cap; closing the session",
    );
}

/// Drive the owner connection's reader loop (this function runs on the owner
/// thread). Each frame is parsed into `(op_id, op)`, bound to `handle`, and
/// dispatched to the worker over `control_tx` WITHOUT waiting for the reply;
/// the reply receiver is forwarded to the writer thread, which matches replies
/// to ops by `op_id` and writes them out of order. A permit from `inflight` is
/// taken (NON-BLOCKING) before EACH op is queued and travels with it to the
/// writer. The reader NEVER blocks on a permit: a well-behaved owner stays far
/// below the cap, and an owner that exceeds `EXEC_OWNER_INFLIGHT_CAP` ops in
/// flight has its session closed through the single teardown path below
/// (after emitting an observability signal). Because the reader never blocks
/// acquiring a permit, owner EOF/POLLHUP is always observed promptly — even
/// when the cap is fully saturated by parked long-polls.
/// On reader EOF/POLLHUP (owner disconnect) or over-cap close the loop returns,
/// `control_tx` is dropped (tearing the worker down and cancelling any
/// in-flight long-poll), then the writer is joined.
fn run_exec_owner_io(
    stream: &Arc<Socket>,
    control_tx: tokio::sync::mpsc::Sender<exec_session::WorkerCommand>,
    item_tx: tokio::sync::mpsc::Sender<ExecWriterItem>,
    inflight: Arc<InflightSemaphore>,
    writer: std::thread::JoinHandle<()>,
    metrics: &Arc<metrics::Registry>,
    handle: &str,
) {
    // EOF / POLLHUP / shutdown / any read error closes the connection and ends
    // the loop, triggering the teardown below.
    while let Ok(frame) = read_frame(stream.as_ref()) {
        let op_id = wire::exec_op_id(&frame);
        let op = match wire::parse_exec_op(&frame) {
            Ok((_, op)) => op,
            Err(error) => {
                // Take a permit even for an immediate error so a flood of
                // malformed frames is bounded by the same in-flight cap; it is
                // released once the error frame is written. Over-cap closes the
                // session (the reader never blocks).
                let Some(permit) = inflight.try_acquire() else {
                    signal_owner_inflight_cap_exceeded(metrics);
                    break;
                };
                if item_tx
                    .blocking_send(ExecWriterItem::Immediate {
                        op_id,
                        error: Box::new(error),
                        metric_kind: "protocol",
                        permit,
                    })
                    .is_err()
                {
                    break;
                }
                continue;
            }
        };
        // Bind every op to THIS session handle (peer-uid binding is implicit:
        // the handle was minted for this connection's admin peer).
        if exec_op_session(&op) != Some(handle) {
            let error = TypedError::GuestControlExecFailed {
                kind: crate::typed_error::GuestControlExecErrorKind::Protocol,
            };
            let Some(permit) = inflight.try_acquire() else {
                signal_owner_inflight_cap_exceeded(metrics);
                break;
            };
            if item_tx
                .blocking_send(ExecWriterItem::Immediate {
                    op_id,
                    error: Box::new(error),
                    metric_kind: "protocol",
                    permit,
                })
                .is_err()
            {
                break;
            }
            continue;
        }

        // Take the in-flight permit BEFORE handing the op to the worker. When
        // the cap is reached this does NOT block: it closes the session (a
        // backpressure-aware owner never reaches the cap), so the reader is
        // always free to observe owner EOF/POLLHUP promptly.
        let Some(permit) = inflight.try_acquire() else {
            signal_owner_inflight_cap_exceeded(metrics);
            break;
        };
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let command = exec_session::WorkerCommand {
            op,
            reply: reply_tx,
        };
        if control_tx.blocking_send(command).is_err() {
            // Worker gone (terminal cleanup / teardown): close the connection.
            // `permit` drops here, releasing it.
            break;
        }
        if item_tx
            .blocking_send(ExecWriterItem::Pending {
                op_id,
                reply_rx,
                permit,
            })
            .is_err()
        {
            break;
        }
    }

    // Reader done. Drop `control_tx` FIRST so the worker returns and resolves
    // every pending reply oneshot (cancelling in-flight long-polls without
    // waiting for their deadline), THEN drop `item_tx` so the writer drains the
    // resolved replies and exits. Joining the writer guarantees no further
    // socket writes after this returns.
    drop(control_tx);
    drop(item_tx);
    // Give the writer a brief, bounded grace to flush its last resolved replies
    // and exit on its own, then force teardown: a misbehaving owner that stopped
    // reading and filled its socket receive buffer would wedge the writer's
    // blocking `send`, so `join()` would hang forever and strand the owner
    // thread + session slot. Shutting the owner socket down makes the wedged
    // send fail promptly so the writer can exit. A healthy writer finishes well
    // within the grace and never reaches the shutdown.
    let drain_deadline = Instant::now() + EXEC_OWNER_WRITER_DRAIN_GRACE;
    while !writer.is_finished() && Instant::now() < drain_deadline {
        std::thread::sleep(EXEC_OWNER_WRITER_DRAIN_POLL);
    }
    if !writer.is_finished() {
        let _ = nix::sys::socket::shutdown(
            stream.as_ref().as_raw_fd(),
            nix::sys::socket::Shutdown::Both,
        );
    }
    let _ = writer.join();
}

/// The owner-connection writer: a current-thread tokio runtime that awaits each
/// op's worker reply concurrently and writes op-id-tagged frames back to the
/// socket from a single drain task (so the socket has exactly one writer).
fn exec_owner_writer(
    stream: Arc<Socket>,
    metrics: Arc<metrics::Registry>,
    mut item_rx: tokio::sync::mpsc::Receiver<ExecWriterItem>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return,
    };
    runtime.block_on(async move {
        // The frame channel carries the op's in-flight permit alongside the
        // frame so the permit is released only AFTER the reply is written to the
        // socket (in the drain task), which is what makes the cap bound ACTUAL
        // in-flight ops rather than just the reader→writer channel depth.
        let (frame_tx, mut frame_rx) =
            tokio::sync::mpsc::unbounded_channel::<(u64, ExecOwnerFrame, InflightPermit)>();
        let drain_stream = Arc::clone(&stream);
        let drain_metrics = Arc::clone(&metrics);
        let drain = tokio::spawn(async move {
            while let Some((op_id, frame, permit)) = frame_rx.recv().await {
                let value = match &frame {
                    ExecOwnerFrame::Response(response) => {
                        wire::exec_response_with_id(op_id, response)
                    }
                    ExecOwnerFrame::Error { error, .. } => wire::error_frame_with_id(op_id, error),
                };
                let write_result = write_json_frame(drain_stream.as_ref(), &value);
                if let ExecOwnerFrame::Error { metric_kind, .. } = &frame
                    && write_result.is_ok()
                {
                    exec_metric_into(&drain_metrics, "op-error", metric_kind);
                }
                // Release the in-flight permit only now that this op's reply has
                // left the writer (or failed to). Explicit for clarity; `permit`
                // would drop at the end of the iteration regardless.
                drop(permit);
                if write_result.is_err() {
                    break;
                }
            }
        });

        while let Some(item) = item_rx.recv().await {
            match item {
                ExecWriterItem::Pending {
                    op_id,
                    reply_rx,
                    permit,
                } => {
                    let frame_tx = frame_tx.clone();
                    tokio::spawn(async move {
                        let frame = match reply_rx.await {
                            Ok(Ok(response)) => ExecOwnerFrame::Response(Box::new(response)),
                            Ok(Err(op_error)) => {
                                let error = map_exec_op_error(op_error);
                                let metric_kind = exec_error_kind_label(&error);
                                ExecOwnerFrame::Error {
                                    error: Box::new(error),
                                    metric_kind,
                                }
                            }
                            // Worker dropped the reply (teardown). The owner is
                            // going away; emit nothing for this op. `permit`
                            // drops here, releasing it.
                            Err(_) => return,
                        };
                        let _ = frame_tx.send((op_id, frame, permit));
                    });
                }
                ExecWriterItem::Immediate {
                    op_id,
                    error,
                    metric_kind,
                    permit,
                } => {
                    let _ = frame_tx.send((
                        op_id,
                        ExecOwnerFrame::Error { error, metric_kind },
                        permit,
                    ));
                }
            }
        }
        // Reader closed the item channel. Drop this task's frame sender so the
        // drain finishes once the still-pending awaiters resolve (they resolve
        // promptly: worker teardown drops their reply oneshots).
        drop(frame_tx);
        let _ = drain.await;
    });
}

/// Owner-socket teardown seam for the terminal-cleanup reaper. Shutting
/// down the socket unblocks the owner reader (`read_frame` returns), which
/// releases the session slot. Idempotent: a second shutdown is a harmless
/// `ENOTCONN`.
struct SocketShutdownReaper {
    fd: RawFd,
}

impl SocketShutdownReaper {
    fn new(fd: RawFd) -> Self {
        Self { fd }
    }
}

impl exec_session::OwnerReaper for SocketShutdownReaper {
    fn reap(&self) {
        let _ = nix::sys::socket::shutdown(self.fd, nix::sys::socket::Shutdown::Both);
    }
}

#[cfg(test)]
mod exec_metric_tests {
    //! The exec metric `nixling_daemon_guest_control_exec_total` is
    //! a HARD closed-label series. Its only labels are the constant
    //! `subsystem` plus a bounded `outcome` / `error_kind` enum — never a vm
    //! name, session handle, op id, peer uid, or argv hash. These tests assert
    //! the descriptor shape, the closed value sets, and that a rendered series
    //! carries nothing else.

    use super::{
        EXEC_ERROR_KIND_LABELS, EXEC_METRIC, EXEC_OUTCOME_LABELS, EXEC_SUBSYSTEM,
        exec_error_kind_label, exec_metric_into, metrics,
    };
    use crate::typed_error::{GuestControlExecErrorKind, TypedError};

    /// Every `GuestControlExecErrorKind` the daemon can surface (closed enum).
    const ALL_EXEC_ERROR_KINDS: &[GuestControlExecErrorKind] = &[
        GuestControlExecErrorKind::Transport,
        GuestControlExecErrorKind::Auth,
        GuestControlExecErrorKind::Protocol,
        GuestControlExecErrorKind::Timeout,
        GuestControlExecErrorKind::OldGeneration,
        GuestControlExecErrorKind::Capability,
        GuestControlExecErrorKind::DetachedUnavailable,
        GuestControlExecErrorKind::SessionCapacity,
        GuestControlExecErrorKind::RateLimited,
        GuestControlExecErrorKind::StaleSession,
        GuestControlExecErrorKind::ExecNotFound,
        GuestControlExecErrorKind::ExecExpired,
        GuestControlExecErrorKind::InvalidProgram,
        GuestControlExecErrorKind::GuestError,
        GuestControlExecErrorKind::Internal,
    ];

    #[test]
    fn exec_metric_descriptor_has_only_three_closed_labels() {
        // The inventory descriptor for the exec metric must declare EXACTLY
        // the three closed keys — adding `vm`, `session`, `op_id`, or any
        // per-session identifier here is the regression this guards.
        let descriptor = metrics::descriptor(EXEC_METRIC).expect("exec metric is in the inventory");
        assert_eq!(
            descriptor.labels,
            &["subsystem", "outcome", "error_kind"],
            "exec metric must carry only the closed subsystem/outcome/error_kind labels"
        );
    }

    #[test]
    fn exec_error_kind_label_is_within_closed_allowlist() {
        // Every typed exec error maps to a label inside the closed set, so the
        // `error_kind` cardinality can never exceed the enum.
        for kind in ALL_EXEC_ERROR_KINDS {
            let error = TypedError::GuestControlExecFailed { kind: *kind };
            let label = exec_error_kind_label(&error);
            assert!(
                EXEC_ERROR_KIND_LABELS.contains(&label),
                "exec_error_kind_label returned {label:?} which is outside the closed allowlist"
            );
        }
        // A non-exec TypedError defaults to the `internal` bucket (still closed).
        assert_eq!(
            exec_error_kind_label(&TypedError::AuthzNotAdmin {
                verb: "exec".to_owned()
            }),
            "internal"
        );
    }

    #[test]
    fn exec_metric_labels_are_closed_enum() {
        // Emit one sample for EVERY (outcome, error_kind) pair in the closed
        // sets, render, and assert the rendered exec series carries only the
        // three approved keys, the constant subsystem, and closed values —
        // and never a forbidden per-session identifier.
        let registry = metrics::Registry::new();
        for &outcome in EXEC_OUTCOME_LABELS {
            for &error_kind in EXEC_ERROR_KIND_LABELS {
                exec_metric_into(&registry, outcome, error_kind);
            }
        }
        let body = registry.render();

        let mut saw_exec_series = false;
        for line in body.lines() {
            if !line.starts_with(EXEC_METRIC) {
                continue;
            }
            let (Some(open), Some(close)) = (line.find('{'), line.find('}')) else {
                continue;
            };
            saw_exec_series = true;
            let inner = &line[open + 1..close];
            for pair in inner.split(',') {
                let mut kv = pair.splitn(2, '=');
                let key = kv.next().unwrap_or("").trim();
                let value = kv.next().unwrap_or("").trim().trim_matches('"');
                match key {
                    "subsystem" => assert_eq!(
                        value, EXEC_SUBSYSTEM,
                        "exec subsystem label must be the constant guest-control-exec"
                    ),
                    "outcome" => assert!(
                        EXEC_OUTCOME_LABELS.contains(&value),
                        "exec outcome label {value:?} is outside the closed allowlist"
                    ),
                    "error_kind" => assert!(
                        EXEC_ERROR_KIND_LABELS.contains(&value),
                        "exec error_kind label {value:?} is outside the closed allowlist"
                    ),
                    other => panic!("exec metric leaked an unapproved label key {other:?}: {line}"),
                }
            }
        }
        assert!(
            saw_exec_series,
            "expected the exec metric to render a series"
        );

        // Belt-and-suspenders: no per-session identifier may ever appear as a
        // label key on the exec metric.
        for forbidden in [
            "vm=\"",
            "session=\"",
            "handle=\"",
            "op_id=\"",
            "op-id=\"",
            "peer_uid=\"",
            "uid=\"",
            "argv=\"",
            "argv_hash=\"",
        ] {
            for line in body.lines().filter(|l| l.starts_with(EXEC_METRIC)) {
                assert!(
                    !line.contains(forbidden),
                    "exec metric leaked forbidden label {forbidden:?}: {line}"
                );
            }
        }
    }
}

#[cfg(test)]
mod exec_owner_io_tests {
    //! Hermetic coverage for the owner reader/writer: the owner
    //! connection dispatches frames to the worker WITHOUT blocking on each
    //! reply, so (a) an urgent control op is serviced while a long-poll is in
    //! flight (no head-of-line), and (b) owner disconnect tears the session
    //! down promptly (the in-flight long-poll is cancelled, not awaited).

    use super::{
        EXEC_METRIC, EXEC_OWNER_INFLIGHT_CAP, EXEC_OWNER_WRITER_DRAIN_GRACE, Socket, exec_session,
        metrics, read_frame, run_exec_owner_io, spawn_exec_owner_writer, write_frame,
    };
    use nixling_ipc::public_wire::{
        ExecCloseArgs, ExecCloseResult, ExecControlResult, ExecOp, ExecOpResponse,
        ExecReadOutputResult, ExecSignalArgs, ExecWaitArgs,
    };
    use serde_json::{Value, json};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tokio::sync::oneshot;

    const HANDLE: &str = "h-test-owner";

    fn seqpacket_pair() -> (Socket, Socket) {
        use nix::sys::socket::{AddressFamily, SockFlag, SockType, socketpair};
        let (a, b) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::empty(),
        )
        .expect("seqpacket socketpair");
        (Socket::from(a), Socket::from(b))
    }

    fn exec_frame(op_id: u64, op: &ExecOp) -> Vec<u8> {
        let mut value = serde_json::to_value(op).expect("encode exec op");
        let object = value.as_object_mut().expect("exec op object");
        object.insert("type".to_owned(), json!("exec"));
        object.insert("opId".to_owned(), json!(op_id));
        serde_json::to_vec(&value).expect("serialize exec frame")
    }

    fn send_op(socket: &Socket, op_id: u64, op: &ExecOp) {
        write_frame(socket, &exec_frame(op_id, op)).expect("client sends exec frame");
    }

    fn recv_reply(socket: &Socket) -> Value {
        let bytes = read_frame(socket).expect("client reads reply");
        serde_json::from_slice(&bytes).expect("reply is JSON")
    }

    /// A fake worker that replies to fast control ops immediately but STASHES a
    /// long-poll (`Wait`/`ReadOutput`) reply sender so the poll stays in flight.
    /// On channel close (owner teardown) every stashed reply sender is dropped,
    /// modelling the production worker dropping its in-flight oneshots.
    fn spawn_fake_worker(
        mut control_rx: tokio::sync::mpsc::Receiver<exec_session::WorkerCommand>,
        longpoll_seen: std::sync::mpsc::Sender<()>,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let mut stashed: Vec<
                oneshot::Sender<Result<ExecOpResponse, exec_session::ExecOpError>>,
            > = Vec::new();
            while let Some(exec_session::WorkerCommand { op, reply }) = control_rx.blocking_recv() {
                match op {
                    ExecOp::Wait(_) | ExecOp::ReadOutput(_) => {
                        stashed.push(reply);
                        let _ = longpoll_seen.send(());
                    }
                    ExecOp::Signal(_) => {
                        let _ = reply.send(Ok(ExecOpResponse::Signal(ExecControlResult {
                            delivered: true,
                        })));
                    }
                    ExecOp::Close(_) => {
                        let _ = reply.send(Ok(ExecOpResponse::Close(ExecCloseResult {
                            stdin_closed: true,
                        })));
                    }
                    _ => {
                        let _ = reply.send(Err(exec_session::ExecOpError::Protocol));
                    }
                }
            }
            // Owner teardown: drop the stashed long-poll reply senders so the
            // writer's awaiters resolve `Err` (the poll is cancelled, never
            // awaited to its deadline).
            drop(stashed);
        })
    }

    fn wait_op() -> ExecOp {
        ExecOp::Wait(ExecWaitArgs {
            session: HANDLE.to_owned(),
            timeout_ms: 60_000,
        })
    }

    fn signal_op() -> ExecOp {
        ExecOp::Signal(ExecSignalArgs {
            session: HANDLE.to_owned(),
            signo: 2,
            op_id: 0,
        })
    }

    fn close_op() -> ExecOp {
        ExecOp::Close(ExecCloseArgs {
            session: HANDLE.to_owned(),
        })
    }

    #[test]
    fn control_op_is_serviced_while_a_long_poll_is_in_flight() {
        let (daemon, client) = seqpacket_pair();
        let daemon = Arc::new(daemon);
        let metrics = Arc::new(metrics::Registry::new());
        let (control_tx, control_rx) = tokio::sync::mpsc::channel(16);
        let (seen_tx, seen_rx) = std::sync::mpsc::channel();
        let worker = spawn_fake_worker(control_rx, seen_tx);

        let io_daemon = Arc::clone(&daemon);
        let io_metrics = Arc::clone(&metrics);
        let io = std::thread::spawn(move || {
            let (writer, item_tx, inflight) =
                spawn_exec_owner_writer(&io_daemon, &io_metrics).expect("writer thread spawns");
            run_exec_owner_io(
                &io_daemon,
                control_tx,
                item_tx,
                inflight,
                writer,
                &io_metrics,
                HANDLE,
            );
        });

        // A normal op completes (owner-open + unrelated request proceeds).
        send_op(&client, 1, &close_op());
        let close_reply = recv_reply(&client);
        assert_eq!(close_reply["type"], "execResponse");
        assert_eq!(close_reply["opId"], 1);
        assert_eq!(close_reply["op"], "close");

        // Park a long-poll, then send an urgent control op. The control reply
        // must come back (out of order, by op-id) BEFORE the parked poll — proof
        // the owner socket read is not serialized behind the long-poll reply.
        send_op(&client, 10, &wait_op());
        seen_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("worker observed the parked long-poll");
        send_op(&client, 11, &signal_op());
        let signal_reply = recv_reply(&client);
        assert_eq!(
            signal_reply["opId"], 11,
            "control reply must be serviced first"
        );
        assert_eq!(signal_reply["op"], "signal");

        // Teardown: closing the client unblocks the reader; the parked poll is
        // cancelled (never replied), and the io thread returns promptly.
        drop(client);
        let start = Instant::now();
        io.join().expect("owner io thread joins");
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "owner io did not tear down promptly after disconnect"
        );
        worker.join().expect("fake worker joins");
    }

    #[test]
    fn disconnect_during_long_poll_tears_down_without_awaiting_the_deadline() {
        let (daemon, client) = seqpacket_pair();
        let daemon = Arc::new(daemon);
        let metrics = Arc::new(metrics::Registry::new());
        let (control_tx, control_rx) = tokio::sync::mpsc::channel(16);
        let (seen_tx, seen_rx) = std::sync::mpsc::channel();
        let worker = spawn_fake_worker(control_rx, seen_tx);

        let io_daemon = Arc::clone(&daemon);
        let io_metrics = Arc::clone(&metrics);
        let io = std::thread::spawn(move || {
            let (writer, item_tx, inflight) =
                spawn_exec_owner_writer(&io_daemon, &io_metrics).expect("writer thread spawns");
            run_exec_owner_io(
                &io_daemon,
                control_tx,
                item_tx,
                inflight,
                writer,
                &io_metrics,
                HANDLE,
            );
        });

        // Park a 60s long-poll, then disconnect. Teardown must NOT wait for the
        // poll's deadline.
        send_op(&client, 10, &wait_op());
        seen_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("worker observed the parked long-poll");
        drop(client);

        let start = Instant::now();
        io.join().expect("owner io thread joins");
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "teardown blocked on the long-poll deadline"
        );
        worker.join().expect("fake worker joins");
    }

    /// The in-flight cap must bound the number of ops ACTUALLY
    /// dispatched-but-unanswered (each pins a guest RPC), not merely the depth
    /// of a channel the worker/writer drain as fast as the reader fills it. A
    /// pipelining owner that floods `cap + N` long-polls must see at most `cap`
    /// reach the worker concurrently; the `(cap + 1)`-th op finds NO free permit
    /// and — crucially — the reader does NOT block on it. Instead the session is
    /// closed PROMPTLY (the over-cap observability signal is emitted and the
    /// reader returns through the single teardown path). This proves both that
    /// the cap hard-bounds concurrent in-flight work AND that the reader never
    /// parks acquiring a permit, so owner EOF/POLLHUP is always observable.
    #[test]
    fn concurrent_inflight_ops_are_bounded_by_the_cap() {
        use std::sync::Mutex;

        let cap = EXEC_OWNER_INFLIGHT_CAP;
        let (daemon, client) = seqpacket_pair();
        let daemon = Arc::new(daemon);
        let metrics = Arc::new(metrics::Registry::new());
        let (control_tx, mut control_rx) = tokio::sync::mpsc::channel(16);
        let (seen_tx, seen_rx) = std::sync::mpsc::channel::<()>();

        // A counting fake worker: every long-poll reply sender is parked in a
        // shared stash (never answered) so the op holds its in-flight permit;
        // each receipt signals `seen_tx`. The stash is also held by the test so
        // it can release the parked senders to let the writer awaiters resolve
        // for a clean teardown.
        type Stash =
            Arc<Mutex<Vec<oneshot::Sender<Result<ExecOpResponse, exec_session::ExecOpError>>>>>;
        let stash: Stash = Arc::new(Mutex::new(Vec::new()));
        let worker_stash = Arc::clone(&stash);
        let worker = std::thread::spawn(move || {
            while let Some(exec_session::WorkerCommand { op, reply }) = control_rx.blocking_recv() {
                match op {
                    ExecOp::Wait(_) | ExecOp::ReadOutput(_) => {
                        worker_stash.lock().expect("stash lock").push(reply);
                        let _ = seen_tx.send(());
                    }
                    _ => {
                        let _ = reply.send(Err(exec_session::ExecOpError::Protocol));
                    }
                }
            }
        });

        let io_daemon = Arc::clone(&daemon);
        let io_metrics = Arc::clone(&metrics);
        let io = std::thread::spawn(move || {
            let (writer, item_tx, inflight) =
                spawn_exec_owner_writer(&io_daemon, &io_metrics).expect("writer thread spawns");
            run_exec_owner_io(
                &io_daemon,
                control_tx,
                item_tx,
                inflight,
                writer,
                &io_metrics,
                HANDLE,
            );
        });

        // Pipeline well beyond the cap. The frames are tiny and fit in the
        // socket buffer, so these client writes do not block even though the
        // reader closes the session after the cap is exceeded.
        let total = cap + 8;
        for op_id in 0..total {
            send_op(&client, op_id as u64, &wait_op());
        }

        // Exactly `cap` long-polls reach the worker. The `(cap + 1)`-th op finds
        // no permit and the reader closes the session rather than dispatching it
        // — so no more than `cap` are ever seen.
        for _ in 0..cap {
            seen_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("worker observes parked long-polls up to the cap");
        }
        assert!(
            seen_rx.recv_timeout(Duration::from_millis(750)).is_err(),
            "more than the cap of {cap} ops were dispatched concurrently to the worker",
        );

        // The over-cap close path emits the closed-allowlist observability
        // signal. Poll briefly: the reader breaks just after the cap-th
        // dispatch, so the metric appears within a short window.
        let mut saw_signal = false;
        for _ in 0..50 {
            if metrics.render().lines().any(|line| {
                line.starts_with(EXEC_METRIC)
                    && line.contains("outcome=\"op-error\"")
                    && line.contains("error_kind=\"inflight-cap-exceeded\"")
            }) {
                saw_signal = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            saw_signal,
            "over-cap close must emit the inflight-cap-exceeded exec metric",
        );

        // The reader did NOT block on the over-cap permit: the session is
        // already closing on its own. Release the parked replies so the held
        // permits free and the writer awaiters resolve, then the io thread joins
        // PROMPTLY — without the test ever dropping the client.
        stash.lock().expect("stash lock").clear();
        let start = Instant::now();
        io.join().expect("owner io thread joins");
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "owner io did not close the session promptly after exceeding the cap",
        );
        drop(client);
        worker.join().expect("fake worker joins");
    }

    /// Prompt teardown under saturation: when the in-flight cap is FULLY held
    /// (every permit taken by a parked long-poll), an owner disconnect must
    /// still tear the session down promptly. The reader is parked in
    /// `read_frame` (never in a permit acquisition), so owner EOF is observed at
    /// once: `control_tx` is dropped, the worker cancels its parked polls, and
    /// the io thread returns without waiting for any poll deadline.
    #[test]
    fn disconnect_while_inflight_cap_saturated_tears_down_promptly() {
        let cap = EXEC_OWNER_INFLIGHT_CAP;
        let (daemon, client) = seqpacket_pair();
        let daemon = Arc::new(daemon);
        let metrics = Arc::new(metrics::Registry::new());
        let (control_tx, mut control_rx) = tokio::sync::mpsc::channel(16);
        let (seen_tx, seen_rx) = std::sync::mpsc::channel::<()>();

        // Parked long-poll senders live in a stash owned by the worker thread;
        // dropping them on worker-loop exit (owner teardown) models the
        // production worker dropping its in-flight oneshots so the polls are
        // cancelled rather than awaited.
        let worker = std::thread::spawn(move || {
            let mut stashed: Vec<
                oneshot::Sender<Result<ExecOpResponse, exec_session::ExecOpError>>,
            > = Vec::new();
            while let Some(exec_session::WorkerCommand { op, reply }) = control_rx.blocking_recv() {
                match op {
                    ExecOp::Wait(_) | ExecOp::ReadOutput(_) => {
                        stashed.push(reply);
                        let _ = seen_tx.send(());
                    }
                    _ => {
                        let _ = reply.send(Err(exec_session::ExecOpError::Protocol));
                    }
                }
            }
            drop(stashed);
        });

        let io_daemon = Arc::clone(&daemon);
        let io_metrics = Arc::clone(&metrics);
        let io = std::thread::spawn(move || {
            let (writer, item_tx, inflight) =
                spawn_exec_owner_writer(&io_daemon, &io_metrics).expect("writer thread spawns");
            run_exec_owner_io(
                &io_daemon,
                control_tx,
                item_tx,
                inflight,
                writer,
                &io_metrics,
                HANDLE,
            );
        });

        // Saturate EXACTLY to the cap (do not exceed it): all `cap` permits are
        // taken by parked long-polls, and the reader is now parked in
        // `read_frame` awaiting the next frame.
        for op_id in 0..cap {
            send_op(&client, op_id as u64, &wait_op());
        }
        for _ in 0..cap {
            seen_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("worker observes parked long-polls up to the cap");
        }

        // Disconnect while fully saturated. The reader (parked in read_frame,
        // NOT in a permit acquisition) observes EOF immediately and tears down.
        drop(client);
        let start = Instant::now();
        io.join().expect("owner io thread joins");
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "saturated owner disconnect did not tear down promptly (reader blocked?)",
        );
        let _ = &metrics;
        worker.join().expect("fake worker joins");
    }

    #[test]
    fn over_cap_teardown_completes_when_owner_stops_reading() {
        // A misbehaving owner that pipelines past the cap while NEVER reading
        // replies fills the daemon's owner-socket send buffer, wedging the
        // writer's blocking `send`. Teardown must still complete: the bounded
        // drain grace elapses, the owner socket is shut down to unblock the
        // wedged send, and the io thread joins — instead of hanging forever and
        // stranding the owner thread + session slot.
        let cap = EXEC_OWNER_INFLIGHT_CAP;
        let (daemon, client) = seqpacket_pair();
        // Squeeze both buffers so a couple of unread ~1 KiB replies fill the pipe
        // and wedge the writer well before the cap is reached.
        let _ = daemon.set_send_buffer_size(1024);
        let _ = client.set_recv_buffer_size(1024);
        let daemon = Arc::new(daemon);
        let metrics = Arc::new(metrics::Registry::new());
        let (control_tx, mut control_rx) = tokio::sync::mpsc::channel(16);

        // Resolve EVERY long-poll immediately with a ~1 KiB reply so the writer
        // actively sends; with the owner never reading, the send buffer backs up
        // and the writer's blocking `send` wedges.
        let worker = std::thread::spawn(move || {
            let payload = "x".repeat(1024);
            while let Some(exec_session::WorkerCommand { op, reply }) = control_rx.blocking_recv() {
                match op {
                    ExecOp::Wait(_) | ExecOp::ReadOutput(_) => {
                        let _ = reply.send(Ok(ExecOpResponse::ReadOutput(ExecReadOutputResult {
                            data_base64: payload.clone(),
                            next_offset: 0,
                            eof: false,
                            dropped_bytes: 0,
                            truncated: false,
                            timed_out: false,
                        })));
                    }
                    _ => {
                        let _ = reply.send(Err(exec_session::ExecOpError::Protocol));
                    }
                }
            }
        });

        let io_daemon = Arc::clone(&daemon);
        let io_metrics = Arc::clone(&metrics);
        let started = Instant::now();
        let io = std::thread::spawn(move || {
            let (writer, item_tx, inflight) =
                spawn_exec_owner_writer(&io_daemon, &io_metrics).expect("writer spawns");
            run_exec_owner_io(
                &io_daemon,
                control_tx,
                item_tx,
                inflight,
                writer,
                &io_metrics,
                HANDLE,
            );
        });

        // Pipeline far past the cap WITHOUT ever reading a reply, from a thread so
        // a backed-up client send (once the reader over-caps and stops reading)
        // cannot wedge the test itself. Hold the client socket OPEN so teardown is
        // an over-cap close, NOT an EOF (which would unblock the writer for free).
        let sender = std::thread::spawn(move || {
            for op_id in 0..(cap * 2) {
                if write_frame(&client, &exec_frame(op_id as u64, &wait_op())).is_err() {
                    break;
                }
            }
            client
        });

        // The io thread must JOIN — proving teardown did not hang on the wedged
        // send. Poll up to 10s (the bounded grace is sub-second).
        let mut joined = false;
        for _ in 0..400 {
            if io.is_finished() {
                joined = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(
            joined,
            "owner teardown hung: the wedged writer's send was never unblocked",
        );
        io.join().expect("io thread joins");

        // The teardown waited the full bounded grace, proving the writer was
        // genuinely wedged (a healthy writer exits in microseconds, far under the
        // grace) and the shutdown path — not a free EOF — released it.
        assert!(
            started.elapsed() >= EXEC_OWNER_WRITER_DRAIN_GRACE,
            "expected the bounded drain grace to elapse on a wedged writer; got {:?}",
            started.elapsed(),
        );

        let client = sender.join().expect("sender thread joins");
        drop(client);
        worker.join().expect("fake worker joins");
    }
}

fn dispatch_broker_request(
    state: &ServerState,
    request: BrokerRequest,
) -> Result<BrokerResponse, TypedError> {
    dispatch_broker_request_as(state, request, Default::default())
}

fn dispatch_broker_request_as(
    state: &ServerState,
    request: BrokerRequest,
    caller_role: BrokerCallerRole,
) -> Result<BrokerResponse, TypedError> {
    let socket_path = broker_socket_path(state);
    let socket = connect_seqpacket(&socket_path)?;
    write_json_frame(
        &socket,
        &BrokerRequestEnvelope {
            request,
            caller_role,
            test_peer_uid: None,
        },
    )?;
    let response = read_frame(&socket)?;
    serde_json::from_slice(&response).map_err(|err| TypedError::InternalBrokerUnavailable {
        path: socket_path,
        detail: err.to_string(),
    })
}

fn broker_caller_role_for_peer(peer: &PeerIdentity) -> BrokerCallerRole {
    match peer.role {
        PeerRole::Admin => BrokerCallerRole::AdminUid { uid: peer.uid },
        PeerRole::Launcher => BrokerCallerRole::LauncherUid { uid: peer.uid },
    }
}

fn dispatch_broker_request_with_timeout(
    state: &ServerState,
    request: BrokerRequest,
    timeout: Duration,
) -> Result<BrokerResponse, TypedError> {
    let socket_path = broker_socket_path(state);
    dispatch_broker_request_to_socket(&socket_path, request, Default::default(), Some(timeout))
}

/// Dispatch a single broker request over a freshly-connected seqpacket
/// socket identified only by its path. Unlike the `ServerState`-based
/// dispatchers this borrows nothing from the daemon and so can be
/// invoked from an owned-data worker (e.g. the guest-control
/// `BrokerSigner`, which holds only the broker socket path so it stays
/// `Send + Sync` across a `spawn_blocking` boundary).
///
/// `timeout`, when set, bounds the ENTIRE connect + write + read round
/// trip by a SINGLE absolute deadline (`now + timeout`). Applying
/// `timeout` independently to connect, write, and read would let one
/// round trip run up to ~3x `timeout`, which defeats the caller's
/// per-attempt budget and pins worker threads / fds past a stalled or
/// backlogged broker. We recompute the remaining budget before each
/// blocking op and fail closed with [`TypedError::InternalBrokerTimeout`]
/// the moment the deadline is reached (whether before an op or while one
/// op blocked past it), so a genuine deadline exhaustion surfaces as a
/// timeout end to end rather than as a generic transport failure.
pub(crate) fn dispatch_broker_request_to_socket(
    socket_path: &Path,
    request: BrokerRequest,
    caller_role: BrokerCallerRole,
    timeout: Option<Duration>,
) -> Result<BrokerResponse, TypedError> {
    let envelope = BrokerRequestEnvelope {
        request,
        caller_role,
        test_peer_uid: None,
    };
    let Some(timeout) = timeout else {
        // No deadline: plain blocking connect + round trip (unchanged).
        let socket = Socket::from(connect_seqpacket(socket_path)?);
        write_json_frame(&socket, &envelope)?;
        let response = read_frame(&socket)?;
        return serde_json::from_slice(&response).map_err(|err| {
            TypedError::InternalBrokerUnavailable {
                path: socket_path.to_path_buf(),
                detail: err.to_string(),
            }
        });
    };

    let deadline = Instant::now() + timeout;
    let result = broker_round_trip_within_deadline(socket_path, &envelope, deadline);
    match result {
        Ok(response) => Ok(response),
        // Any failure that lands at or past the single round-trip
        // deadline is a genuine timeout (connect/write/read blocked the
        // whole remaining budget, or the deadline lapsed between ops),
        // not a fast broker-unavailable failure.
        Err(_) if Instant::now() >= deadline => Err(TypedError::InternalBrokerTimeout {
            path: socket_path.to_path_buf(),
        }),
        Err(err) => Err(err),
    }
}

/// Remaining budget until `deadline`, or [`TypedError::InternalBrokerTimeout`]
/// if the deadline has already been reached before issuing the next op.
fn broker_remaining_before_op(
    deadline: Instant,
    socket_path: &Path,
) -> Result<Duration, TypedError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(TypedError::InternalBrokerTimeout {
            path: socket_path.to_path_buf(),
        });
    }
    Ok(remaining)
}

/// Run connect + write + read so that each blocking op is bounded by the
/// budget remaining until the shared `deadline`. Returns early with a
/// timeout if the deadline lapses before any op.
fn broker_round_trip_within_deadline(
    socket_path: &Path,
    envelope: &BrokerRequestEnvelope,
    deadline: Instant,
) -> Result<BrokerResponse, TypedError> {
    let remaining = broker_remaining_before_op(deadline, socket_path)?;
    let socket = Socket::from(connect_seqpacket_with_timeout(
        socket_path,
        Some(remaining),
    )?);

    let remaining = broker_remaining_before_op(deadline, socket_path)?;
    socket
        .set_write_timeout(Some(remaining))
        .map_err(|err| TypedError::InternalIo {
            context: format!("set broker write timeout to {remaining:?}"),
            detail: err.to_string(),
        })?;
    write_json_frame(&socket, envelope)?;

    let remaining = broker_remaining_before_op(deadline, socket_path)?;
    socket
        .set_read_timeout(Some(remaining))
        .map_err(|err| TypedError::InternalIo {
            context: format!("set broker read timeout to {remaining:?}"),
            detail: err.to_string(),
        })?;
    let response = read_frame(&socket)?;
    serde_json::from_slice(&response).map_err(|err| TypedError::InternalBrokerUnavailable {
        path: socket_path.to_path_buf(),
        detail: err.to_string(),
    })
}

fn poll_broker_child_reaped(state: &ServerState) -> Result<usize, TypedError> {
    let response = dispatch_broker_request(state, BrokerRequest::PollChildReaped)?;
    match response {
        BrokerResponse::PollChildReaped(response) => {
            let count = response.notifications.len();
            for notification in response.notifications {
                state.broker_reap_log.insert(notification);
            }
            Ok(count)
        }
        BrokerResponse::Error(error) => Err(TypedError::InternalBrokerUnavailable {
            path: broker_socket_path(state),
            detail: format!(
                "PollChildReaped rejected by broker: {} ({})",
                error.message, error.kind
            ),
        }),
        other => Err(TypedError::InternalBrokerUnavailable {
            path: broker_socket_path(state),
            detail: format!("PollChildReaped returned unexpected response: {other:?}"),
        }),
    }
}

fn refresh_broker_reap_log(state: &ServerState, context: &str) {
    match poll_broker_child_reaped(state) {
        Ok(0) => {}
        Ok(count) => tracing::debug!(count, context, "broker child reap log refreshed"),
        Err(err) => tracing::debug!(error = ?err, context, "broker child reap log refresh skipped"),
    }
}

fn dispatch_broker_request_with_fds_timeout(
    state: &ServerState,
    request: BrokerRequest,
    timeout: Duration,
) -> Result<(BrokerResponse, Vec<RawFd>), TypedError> {
    let socket_path = broker_socket_path(state);
    let socket = Socket::from(connect_seqpacket_with_timeout(&socket_path, Some(timeout))?);
    socket
        .set_read_timeout(Some(timeout))
        .map_err(|err| TypedError::InternalIo {
            context: format!("set broker read timeout to {timeout:?}"),
            detail: err.to_string(),
        })?;
    socket
        .set_write_timeout(Some(timeout))
        .map_err(|err| TypedError::InternalIo {
            context: format!("set broker write timeout to {timeout:?}"),
            detail: err.to_string(),
        })?;
    write_json_frame(
        &socket,
        &BrokerRequestEnvelope {
            request,
            caller_role: Default::default(),
            test_peer_uid: None,
        },
    )?;
    let (response, received_fds) = read_frame_with_fds(&socket)?;
    let decoded = serde_json::from_slice(&response).map_err(|err| {
        close_received_fds(&received_fds);
        TypedError::InternalBrokerUnavailable {
            path: socket_path,
            detail: err.to_string(),
        }
    })?;
    Ok((decoded, received_fds))
}

fn broker_response_kind(response: &BrokerResponse) -> String {
    serde_json::to_value(response)
        .ok()
        .and_then(|value| {
            value
                .get("kind")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

fn redact_broker_error_for_launcher(
    op_name: &str,
    target_wave: Option<&str>,
    broker_error_kind: &str,
) -> (String, String) {
    let _ = target_wave;
    let summary = format!("{op_name} failed");
    let remediation = match broker_error_kind {
        "Broker.BundleResolverUnavailable" => {
            "broker is starting up / bundle not yet loaded; retry shortly. Admin: confirm the bundle path is populated.".to_owned()
        }
        "Broker.BundleIntentMissing" => format!(
            "{op_name} references a bundle intent that the broker did not find. Admin: ask `journalctl -u nixling-priv-broker` for the intent id."
        ),
        "Broker.StoreViewFilesystemMismatch" => format!(
            "{op_name} refused: the per-VM store view is not on the same filesystem as /nix/store. Admin: check the VM state dir layout and retry."
        ),
        "Broker.StoreViewMarkerMissing" => format!(
            "{op_name} refused: the prepared store-view generation is missing its marker. Admin: rebuild the store view and retry."
        ),
        "Broker.LiveHandlerFailed" => format!(
            "{op_name} failed at the broker live handler. Admin: inspect `journalctl -u nixling-priv-broker` for the underlying syscall/exit code."
        ),
        "Broker.CoexistenceRefused" => "{op_name} refused: another firewall manager owns the table per FirewallCoexistencePolicy. Admin: check nixling.site.firewallCoexistencePolicy."
            .replace("{op_name}", op_name),
        "Broker.NftScriptParseFailed" => "{op_name} failed: bundle nft script could not be parsed. Admin: inspect `journalctl -u nixling-priv-broker` for the parse error."
            .replace("{op_name}", op_name),
        "Broker.CarveoutOrderingViolation" => "{op_name} refused: USBIP firewall carve-out rules are out of order relative to broad allow/drop. Admin: inspect the bundle's nft batch ordering."
            .replace("{op_name}", op_name),
        "Broker.NftablesDriftDetected" => "{op_name} refused: the live nft table hash differs from the bundle's expected hash; someone modified the table out-of-band. Admin: investigate before reapplying."
            .replace("{op_name}", op_name),
        "Broker.ValidateBundleFailed" => {
            "trusted bundle validation failed; Admin: re-render the bundle and retry.".to_owned()
        }
        "Broker.Protocol" => {
            "broker protocol error; retry after admin checks broker logs".to_owned()
        }
        "Broker.Unimplemented" => {
            "broker operation is not implemented in this build; Admin: use the supported fallback path for this wave.".to_owned()
        }
        "unknown-operation" => {
            "broker rejected an unknown operation; Admin: verify daemon and broker versions match.".to_owned()
        }
        "authz-audit-requires-admin" => {
            "broker audit export requires an authorized admin user.".to_owned()
        }
        _ => format!(
            "{op_name} failed; admin should inspect `journalctl -u nixling-priv-broker` for details"
        ),
    };
    (summary, remediation)
}

fn redact_broker_dispatch_failure_for_launcher(op_name: &str) -> (String, String) {
    (
        format!("{op_name} failed"),
        format!(
            "{op_name} could not reach the broker. Admin: inspect `journalctl -u nixlingd` for the daemon-side diagnostic."
        ),
    )
}

fn broker_failure_response(
    verb: &str,
    summary: String,
    remediation: String,
    target_wave: Option<String>,
) -> Value {
    use nixling_ipc::public_wire::{MutatingVerbOutcome, MutatingVerbResponse};

    wire::mutating_verb_response(MutatingVerbResponse {
        verb: verb.to_owned(),
        outcome: MutatingVerbOutcome::BrokerError,
        target_wave,
        summary: Some(summary),
        remediation: Some(remediation),
        api_ready: None,
    })
}

fn invalid_request_response(verb: &str, remediation: String) -> Value {
    use nixling_ipc::public_wire::{MutatingVerbOutcome, MutatingVerbResponse};

    wire::mutating_verb_response(MutatingVerbResponse {
        verb: verb.to_owned(),
        outcome: MutatingVerbOutcome::InvalidRequest,
        target_wave: None,
        summary: None,
        remediation: Some(remediation),
        api_ready: None,
    })
}

fn daemon_failure_response(verb: &str, summary: String) -> Value {
    broker_failure_response(
        verb,
        summary,
        "Admin: inspect `journalctl -u nixlingd` for the daemon-side diagnostic.".to_owned(),
        None,
    )
}

fn applied_response(verb: &str, summary: String) -> Value {
    use nixling_ipc::public_wire::{MutatingVerbOutcome, MutatingVerbResponse};

    wire::mutating_verb_response(MutatingVerbResponse {
        verb: verb.to_owned(),
        outcome: MutatingVerbOutcome::Applied,
        target_wave: None,
        summary: Some(summary),
        remediation: None,
        api_ready: None,
    })
}

fn api_ready_timeout_response(verb: &str, summary: String) -> Value {
    use nixling_ipc::public_wire::{MutatingVerbOutcome, MutatingVerbResponse};

    wire::mutating_verb_response(MutatingVerbResponse {
        verb: verb.to_owned(),
        outcome: MutatingVerbOutcome::ApiReadyTimeout,
        target_wave: None,
        summary: Some(summary),
        remediation: None,
        api_ready: Some("timeout".to_owned()),
    })
}

fn response_outcome(value: &Value) -> Option<&str> {
    value.get("outcome").and_then(Value::as_str)
}

fn response_summary(value: &Value) -> Option<&str> {
    value.get("summary").and_then(Value::as_str)
}

fn response_remediation(value: &Value) -> Option<&str> {
    value.get("remediation").and_then(Value::as_str)
}

fn response_target_wave(value: &Value) -> Option<String> {
    value
        .get("targetWave")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn retarget_mutating_response(value: &Value, verb: &str) -> Value {
    match response_outcome(value) {
        Some("applied") => {
            applied_response(verb, response_summary(value).unwrap_or_default().to_owned())
        }
        Some("broker-error") => broker_failure_response(
            verb,
            response_summary(value).unwrap_or_default().to_owned(),
            response_remediation(value).unwrap_or_default().to_owned(),
            response_target_wave(value),
        ),
        Some("api-ready-timeout") => {
            let mut retargeted = value.clone();
            if let Some(object) = retargeted.as_object_mut() {
                object.insert("verb".to_owned(), Value::String(verb.to_owned()));
            }
            retargeted
        }
        _ => value.clone(),
    }
}

fn load_host_artifact(state: &ServerState) -> Result<HostJson, TypedError> {
    load_json(&state.config.artifacts.host_path)
}

fn ipv6_sysctl_short_keys(_entry: &Ipv6SysctlEntry) -> [&'static str; 5] {
    [
        "disable_ipv6",
        "accept_ra",
        "autoconf",
        "addr_gen_mode",
        "arp_ignore",
    ]
}

fn dispatch_broker_ack_request(
    state: &ServerState,
    verb: &str,
    op_name: &str,
    request: BrokerRequest,
) -> Result<(), Value> {
    match dispatch_broker_request(state, request) {
        Ok(BrokerResponse::Ack(ack)) if ack.accepted && ack.operation == op_name => Ok(()),
        Ok(BrokerResponse::Ack(ack)) => {
            tracing::warn!(
                op_name = op_name,
                broker_ack_operation = %ack.operation,
                broker_ack_accepted = ack.accepted,
                "broker returned unexpected ack payload"
            );
            let (summary, remediation) =
                redact_broker_error_for_launcher(op_name, None, "Broker.Protocol");
            Err(broker_failure_response(verb, summary, remediation, None))
        }
        Ok(BrokerResponse::Error(error)) => {
            tracing::warn!(
                broker_kind = %error.kind,
                broker_operation = %error.operation,
                broker_target_wave = error.target_wave.as_deref().unwrap_or("none"),
                broker_message = %error.message,
                broker_action = %error.action,
                "broker live op failed"
            );
            let (summary, remediation) = redact_broker_error_for_launcher(
                op_name,
                error.target_wave.as_deref(),
                &error.kind,
            );
            Err(broker_failure_response(
                verb,
                summary,
                remediation,
                error.target_wave,
            ))
        }
        Ok(other) => {
            tracing::warn!(
                op_name = op_name,
                broker_response_kind = %broker_response_kind(&other),
                "broker returned unexpected response kind"
            );
            let (summary, remediation) =
                redact_broker_error_for_launcher(op_name, None, "Broker.Protocol");
            Err(broker_failure_response(verb, summary, remediation, None))
        }
        Err(error) => {
            tracing::warn!(op_name = op_name, error = ?error, "broker dispatch failed");
            let (summary, remediation) = redact_broker_dispatch_failure_for_launcher(op_name);
            Err(broker_failure_response(verb, summary, remediation, None))
        }
    }
}

fn load_bundle_resolver(state: &ServerState) -> Result<BundleResolver, TypedError> {
    #[cfg(test)]
    let loaded = BundleResolver::load_with_policy(
        &state.config.artifacts.bundle_path,
        &nixling_core::bundle_resolver::BundleVerifyPolicy::for_tests(),
    );
    #[cfg(not(test))]
    let loaded = BundleResolver::load(&state.config.artifacts.bundle_path);

    loaded.map_err(|err| match err {
        nixling_core::error::Error::Bundle(BundleError::Tampered { path, reason }) => {
            TypedError::BundleTampered { path, reason }
        }
        other => TypedError::InternalIo {
            context: "load bundle resolver".to_owned(),
            detail: other.to_string(),
        },
    })
}

fn duplicate_received_fd(
    received_fds: &[RawFd],
    fd_index: u32,
    context: &str,
) -> Result<OwnedFd, TypedError> {
    let Some(fd_slot) = usize::try_from(fd_index)
        .ok()
        .filter(|index| *index < received_fds.len())
    else {
        return Err(TypedError::InternalIo {
            context: context.to_owned(),
            detail: format!("missing SCM_RIGHTS fd at index {fd_index}"),
        });
    };
    duplicate_fd_cloexec(received_fds[fd_slot], context)
}

fn block_on_future<T>(future: impl Future<Output = T>) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(future)),
        Err(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build temporary tokio runtime")
            .block_on(future),
    }
}

fn acquire_vm_start_lock(state: &ServerState, vm: &str) -> Result<Flock<File>, TypedError> {
    let path = state.config.locks_dir.join(format!("vm-start-{vm}.lock"));
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|err| TypedError::InternalIo {
            context: format!("open VM start lock {}", path.display()),
            detail: err.to_string(),
        })?;
    Flock::lock(file, FlockArg::LockExclusive).map_err(|(_file, err)| TypedError::InternalIo {
        context: format!("lock VM start lock {}", path.display()),
        detail: err.to_string(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VmStartNodeMode {
    ReadinessOnly,
    OneShot(RunnerRole),
    LongLived(RunnerRole),
}

fn vm_start_node_mode(role: &ProcessRole) -> VmStartNodeMode {
    match role {
        ProcessRole::SwtpmPreStartFlush => VmStartNodeMode::OneShot(RunnerRole::SwtpmFlush),
        ProcessRole::Swtpm => VmStartNodeMode::LongLived(RunnerRole::Swtpm),
        ProcessRole::Virtiofsd => VmStartNodeMode::LongLived(RunnerRole::Virtiofsd),
        ProcessRole::CloudHypervisorRunner => {
            VmStartNodeMode::LongLived(RunnerRole::CloudHypervisor)
        }
        ProcessRole::QemuMediaRunner => VmStartNodeMode::LongLived(RunnerRole::QemuMedia),
        ProcessRole::Gpu | ProcessRole::GpuRenderNode => {
            VmStartNodeMode::LongLived(RunnerRole::Gpu)
        }
        ProcessRole::Audio => VmStartNodeMode::LongLived(RunnerRole::Audio),
        ProcessRole::Video => VmStartNodeMode::LongLived(RunnerRole::Video),
        ProcessRole::VsockRelay => VmStartNodeMode::LongLived(RunnerRole::VsockRelay),
        ProcessRole::OtelHostBridge => VmStartNodeMode::LongLived(RunnerRole::OtelHostBridge),
        ProcessRole::Usbip => VmStartNodeMode::LongLived(RunnerRole::Usbip),
        ProcessRole::WaylandProxy => VmStartNodeMode::LongLived(RunnerRole::WaylandProxy),
        ProcessRole::HostReconcile
        | ProcessRole::StoreVirtiofsPreflight
        | ProcessRole::GuestSshReadiness
        | ProcessRole::GuestControlHealth => VmStartNodeMode::ReadinessOnly,
    }
}

fn tracked_role_id(node: &ProcessNode) -> String {
    match node.role {
        ProcessRole::CloudHypervisorRunner => VM_RUNNER_ROLE_ID.to_owned(),
        _ => node.id.0.clone(),
    }
}

/// v1.2fu46/fu53: pure decision predicate for whether the daemon
/// must dispatch `BrokerRequest::DiskInit` BEFORE `SpawnRunner` for
/// the given node.
///
/// Extracted as a free function so the dispatch-order regression
/// has hermetic unit-test coverage (the broker IPC itself requires
/// integration testing).  See `node_requires_disk_init_dispatch_*`
/// tests at the bottom of this module.
fn node_requires_disk_init_dispatch(node: &ProcessNode) -> bool {
    use nixling_core::processes::SpawnRunnerPlanOp;
    node.plan_ops
        .iter()
        .any(|op| matches!(op, SpawnRunnerPlanOp::DiskInit { .. }))
}

struct VmStartRunner<'a> {
    state: &'a ServerState,
    resolver: &'a BundleResolver,
}

fn resolve_store_view_intent_for_vm<'a>(
    resolver: &'a BundleResolver,
    vm: &str,
) -> Result<&'a nixling_core::bundle_resolver::ResolvedStoreViewIntent, String> {
    resolver
        .find_store_view_intent(vm)
        .ok_or_else(|| "bundle-intent-missing:store-view".to_owned())
}

/// Emit the guest-control readiness observation as a structured tracing
/// event. Every field is drawn from the closed-enum / numeric projection
/// in [`guest_control_bridge::ReadinessObservation`]; by construction the
/// event can never carry guest content, store/socket/state-dir paths,
/// nonces, tokens, auth tags, `guest_boot_id`, or `capabilities_hash`.
/// Kept as a free function so the leak-safe field set can be asserted by
/// a tracing-capture test without driving the full supervisor.
fn emit_guest_control_readiness_event(
    obs: &guest_control_bridge::ReadinessObservation,
    ready: bool,
) {
    if ready {
        tracing::info!(
            kind = "critical",
            subsystem = obs.subsystem,
            outcome = obs.outcome,
            health_state = obs.health_state,
            health_reason = obs.health_reason,
            attempt_count = obs.attempt_count,
            duration_ms = obs.duration_ms,
            "guest-control readiness probe completed"
        );
    } else {
        tracing::warn!(
            kind = "critical",
            subsystem = obs.subsystem,
            outcome = obs.outcome,
            error_kind = obs.error_kind,
            health_state = obs.health_state,
            health_reason = obs.health_reason,
            attempt_count = obs.attempt_count,
            duration_ms = obs.duration_ms,
            "guest-control readiness probe failed"
        );
    }
}

impl VmStartRunner<'_> {
    fn sync_store_view(&self, vm: &str) -> Result<(), String> {
        let intent = resolve_store_view_intent_for_vm(self.resolver, vm)?;
        match dispatch_broker_request(
            self.state,
            BrokerRequest::StoreSync(nixling_ipc::broker_wire::StoreSyncRequest {
                vm_id: VmId::new(vm),
                bundle_closure_ref: BundleClosureRef::new(intent.intent_id.clone()),
                generation_token: u32::try_from(intent.generation)
                    .map_err(|_| "store-view-generation-overflow".to_owned())?,
                tracing_span_id: None,
            }),
        ) {
            Ok(BrokerResponse::StoreSync(_)) => Ok(()),
            Ok(BrokerResponse::Error(error)) => {
                tracing::warn!(
                    vm = %vm,
                    broker_kind = %error.kind,
                    broker_operation = %error.operation,
                    broker_message = %error.message,
                    broker_action = %error.action,
                    "StoreSync preflight dispatch failed"
                );
                Err(format!("broker-error:StoreSync:{}", error.kind))
            }
            Ok(other) => {
                tracing::warn!(
                    vm = %vm,
                    broker_response_kind = %broker_response_kind(&other),
                    "StoreSync preflight returned unexpected broker response"
                );
                Err("broker-protocol:StoreSync".to_owned())
            }
            Err(error) => {
                tracing::warn!(
                    vm = %vm,
                    error = ?error,
                    "StoreSync preflight dispatch failed"
                );
                Err("broker-dispatch:StoreSync".to_owned())
            }
        }
    }

    fn spawn_runner(
        &self,
        vm: &str,
        node: &ProcessNode,
        runner_role: RunnerRole,
        timeout: Duration,
    ) -> Result<nixling_ipc::broker_wire::SpawnRunnerResponse, String> {
        let intent_id = intent_id_runner(vm, &node.id.0);
        let intent = self
            .resolver
            .find_runner_intent(&intent_id)
            .ok_or_else(|| "bundle-intent-missing".to_owned())?;
        let role_id = tracked_role_id(node);
        // v1.2fu46/fu53: D9 close-the-loop — if the ProcessNode
        // declares any DiskInit plan-ops (e.g. for
        // writableStoreOverlay), dispatch BrokerRequest::DiskInit
        // BEFORE SpawnRunner.  The broker resolves all plan-ops
        // from the trusted bundle by vm_id and creates the disk
        // images.  Without this dispatch the manifest emits the
        // plan-op but the broker never runs it, so CH boots with
        // no overlay file and fatals with `NotFound`.
        //
        // Decision logic is extracted into
        // `node_requires_disk_init_dispatch` for hermetic unit
        // testing — the regression covered by panel-test R1 #2.
        if node_requires_disk_init_dispatch(node) {
            match dispatch_broker_request(
                self.state,
                BrokerRequest::DiskInit(nixling_ipc::broker_wire::DiskInitRequest {
                    vm_id: VmId::new(vm),
                    tracing_span_id: None,
                }),
            ) {
                Ok(BrokerResponse::Ack(_)) => {
                    tracing::info!(
                        vm = %vm,
                        node = %node.id.0,
                        plan_op_count = node.plan_ops.len(),
                        "v1.2fu46: DiskInit plan-ops applied before SpawnRunner"
                    );
                }
                Ok(BrokerResponse::Error(error)) => {
                    tracing::warn!(
                        vm = %vm,
                        node = %node.id.0,
                        broker_kind = %error.kind,
                        broker_operation = %error.operation,
                        broker_target_wave = error.target_wave.as_deref().unwrap_or("none"),
                        broker_message = %error.message,
                        broker_action = %error.action,
                        "v1.2fu46: DiskInit pre-SpawnRunner dispatch failed"
                    );
                    return Err(format!("broker-error:DiskInit:{}", error.kind));
                }
                Ok(other) => {
                    tracing::warn!(
                        vm = %vm,
                        node = %node.id.0,
                        broker_response_kind = %broker_response_kind(&other),
                        "v1.2fu46: DiskInit returned unexpected broker response"
                    );
                    return Err("broker-protocol:DiskInit".to_owned());
                }
                Err(error) => {
                    tracing::warn!(
                        vm = %vm,
                        node = %node.id.0,
                        error = ?error,
                        "v1.2fu46: DiskInit dispatch failed"
                    );
                    return Err("broker-dispatch:DiskInit".to_owned());
                }
            }
        }
        match dispatch_broker_request_with_fds_timeout(
            self.state,
            BrokerRequest::SpawnRunner(BrokerSpawnRunnerRequest {
                vm_id: VmId::new(vm),
                role_id: RoleId::new(role_id.clone()),
                role: runner_role,
                bundle_runner_intent_ref: BundleOpId::new(intent.intent_id.clone()),
                runtime_allocations: vec![],
                tracing_span_id: None,
            }),
            timeout,
        ) {
            Ok((BrokerResponse::SpawnRunner(response), received_fds)) => {
                if let Err(error) =
                    self.register_node_pidfd(vm, node, runner_role, &response, &received_fds)
                {
                    stop_unregistered_spawned_runner(
                        self.state,
                        vm,
                        &role_id,
                        &response,
                        &received_fds,
                    );
                    close_received_fds(&received_fds);
                    return Err(error);
                }
                close_received_fds(&received_fds);
                Ok(response)
            }
            Ok((BrokerResponse::Error(error), received_fds)) => {
                close_received_fds(&received_fds);
                tracing::warn!(
                    node = %node.id.0,
                    broker_kind = %error.kind,
                    broker_operation = %error.operation,
                    broker_target_wave = error.target_wave.as_deref().unwrap_or("none"),
                    broker_message = %error.message,
                    broker_action = %error.action,
                    "vm start node spawn failed"
                );
                Err(format!("broker-error:{}", error.kind))
            }
            Ok((other, received_fds)) => {
                close_received_fds(&received_fds);
                tracing::warn!(
                    node = %node.id.0,
                    broker_response_kind = %broker_response_kind(&other),
                    "vm start node received unexpected broker response"
                );
                Err("broker-protocol".to_owned())
            }
            Err(error) => {
                tracing::warn!(node = %node.id.0, error = ?error, "vm start node dispatch failed");
                Err("broker-dispatch".to_owned())
            }
        }
    }

    fn register_node_pidfd(
        &self,
        vm: &str,
        node: &ProcessNode,
        runner_role: RunnerRole,
        response: &nixling_ipc::broker_wire::SpawnRunnerResponse,
        received_fds: &[RawFd],
    ) -> Result<(), String> {
        let VmStartNodeMode::LongLived(_) = vm_start_node_mode(&node.role) else {
            return Ok(());
        };
        let pidfd = duplicate_received_fd(
            received_fds,
            response.pidfd_index,
            "duplicate SpawnRunner pidfd",
        )
        .map_err(|error| error.message())?;
        let role_id = tracked_role_id(node);
        // Serialize register + snapshot as one unit so a concurrent
        // different-VM op cannot persist a stale snapshot that drops this
        // entry (register A, snapshot A reads {A}, register B, snapshot B
        // writes {A,B}, delayed snapshot A overwrites with {A} — losing B).
        let _mguard = self.state.pidfd_table.mutation_guard();
        self.state
            .pidfd_table
            .register(
                vm.to_owned(),
                role_id.clone(),
                PidfdEntry {
                    pidfd,
                    pid: response.pid,
                    start_time_ticks: response.start_time_ticks,
                },
            )
            .map_err(|error| format!("pidfd-register:{error}"))?;
        if let Err(error) = self.state.pidfd_table.snapshot() {
            let _ = self.state.pidfd_table.deregister(vm, &role_id);
            return Err(format!("pidfd-snapshot:{error}"));
        }
        // The pidfd-table register + snapshot sequence is now complete and
        // consistent, so release the serialization guard BEFORE
        // `write_runner_snapshot` (which writes a separate runner-snapshot
        // file and never touches the pidfd table). Holding it across the
        // failure path below would self-deadlock: `cleanup_vm_start_registration`
        // re-acquires this same non-reentrant guard.
        drop(_mguard);
        if let Err(error) = write_runner_snapshot(
            self.state,
            vm,
            &role_id,
            runner_role,
            response.pid,
            response.start_time_ticks,
        ) {
            cleanup_vm_start_registration(self.state, vm, &role_id);
            return Err(error);
        }
        Ok(())
    }

    fn boot_qemu_media(
        &self,
        vm: &str,
        node: &ProcessNode,
        timeout: Duration,
    ) -> Result<(), String> {
        match dispatch_broker_request_with_timeout(
            self.state,
            BrokerRequest::QemuMediaBoot(BrokerQemuMediaBootRequest {
                vm_id: VmId::new(vm),
                tracing_span_id: None,
            }),
            timeout,
        ) {
            Ok(BrokerResponse::QemuMediaBoot(response)) => {
                tracing::info!(
                    vm = %vm,
                    node = %node.id.0,
                    role_id = %tracked_role_id(node),
                    media_ref = %response.media_ref.as_str(),
                    slot = %response.slot,
                    qmp_commands = ?response.qmp_commands,
                    "qemu-media boot source attached and runner continued"
                );
                Ok(())
            }
            Ok(BrokerResponse::Error(error)) => {
                tracing::warn!(
                    vm = %vm,
                    node = %node.id.0,
                    broker_kind = %error.kind,
                    broker_operation = %error.operation,
                    broker_message = %error.message,
                    broker_action = %error.action,
                    "qemu-media boot transaction failed"
                );
                Err(format!("broker-error:QemuMediaBoot:{}", error.kind))
            }
            Ok(other) => {
                tracing::warn!(
                    vm = %vm,
                    node = %node.id.0,
                    broker_response_kind = %broker_response_kind(&other),
                    "qemu-media boot transaction returned unexpected response"
                );
                Err("broker-protocol:QemuMediaBoot".to_owned())
            }
            Err(error) => {
                tracing::warn!(
                    vm = %vm,
                    node = %node.id.0,
                    error = ?error,
                    "qemu-media boot transaction dispatch failed"
                );
                Err("broker-dispatch:QemuMediaBoot".to_owned())
            }
        }
    }

    /// State-aware readiness for a `GuestControlHealth` node. Resolves the
    /// per-VM probe parameters from the trusted bundle and runs the
    /// authenticated Health probe on a dedicated current-thread runtime
    /// inside `spawn_blocking` (a strict runtime boundary: no `Handle::current`,
    /// `block_in_place`, or nested runtime; nothing borrowed from
    /// `ServerState` crosses the boundary). The retry loop is bounded by
    /// `budget.readiness`; `guest_control_health_ready` decides ready.
    async fn wait_for_guest_control_health(
        &self,
        vm: &str,
        node: &ProcessNode,
        budget: supervisor::dag::NodeBudget,
    ) -> Result<(), String> {
        let params = resolve_guest_control_probe_params(self.state, self.resolver, vm)?;
        let broker_path = broker_socket_path(self.state);
        let deadline = budget.readiness;
        let node_id = node.id.0.clone();
        let run = tokio::task::spawn_blocking(move || {
            let probe = guest_control_bridge::RealGuestControlProbe::new(broker_path);
            let clock = guest_control_bridge::RealProbeClock::new();
            guest_control_bridge::run_guest_control_readiness_loop(
                &probe,
                &params,
                deadline,
                guest_control_bridge::GUEST_CONTROL_ATTEMPT_CAP,
                guest_control_bridge::GUEST_CONTROL_RETRY_BACKOFF,
                &clock,
            )
        })
        .await
        .map_err(|error| format!("guest-control-readiness-join:{error}"))?;

        let ready = guest_control_health::guest_control_health_ready(&run.outcome);
        let obs = guest_control_bridge::ReadinessObservation::from_run(&run);
        emit_guest_control_readiness_event(&obs, ready);

        if ready {
            tracing::info!(
                vm = %vm,
                node = %node_id,
                role_id = %tracked_role_id(node),
                "guest-control-health node ready"
            );
            Ok(())
        } else {
            Err(format!("guest-control-health-not-ready:{node_id}"))
        }
    }
}

#[async_trait::async_trait]
impl supervisor::dag::NodeRunner for VmStartRunner<'_> {
    async fn spawn_and_wait_ready(
        &self,
        vm: &str,
        node: &ProcessNode,
        readiness: &[ReadinessPredicate],
        budget: supervisor::dag::NodeBudget,
    ) -> Result<(), String> {
        // The authenticated guest-control Health node is readiness-only but
        // needs daemon state (per-VM vsock socket, peer credentials, the
        // broker-backed signer), so it cannot go through the stateless
        // `wait_for_readiness` path. Intercept it here; this also covers the
        // `spawn_and_check_process_alive` fall-through, which delegates here.
        if node.role == ProcessRole::GuestControlHealth {
            return self.wait_for_guest_control_health(vm, node, budget).await;
        }
        match vm_start_node_mode(&node.role) {
            VmStartNodeMode::ReadinessOnly => {
                if node.role == ProcessRole::StoreVirtiofsPreflight {
                    self.sync_store_view(vm)?;
                }
                // ReadinessOnly nodes spawn no long-lived runner, so there
                // is no daemon-held pidfd to observe — no liveness probe.
                wait_for_readiness(node, readiness, budget.readiness, None)
            }
            VmStartNodeMode::OneShot(runner_role) => {
                let response = self.spawn_runner(vm, node, runner_role, budget.spawn)?;
                wait_for_one_shot_exit(response.pid, response.start_time_ticks, budget.readiness)
            }
            VmStartNodeMode::LongLived(runner_role) => {
                let response = self.spawn_runner(vm, node, runner_role, budget.spawn)?;
                let liveness = supervisor::readiness_liveness::PidfdLivenessProbe::new(
                    &self.state.pidfd_table,
                    &self.state.broker_reap_log,
                    vm,
                    tracked_role_id(node),
                );
                wait_for_readiness(node, readiness, budget.readiness, Some(&liveness))?;
                if node.role == ProcessRole::QemuMediaRunner {
                    self.boot_qemu_media(vm, node, budget.readiness)?;
                }
                tracing::info!(
                    vm = %vm,
                    node = %node.id.0,
                    role_id = %tracked_role_id(node),
                    pid = response.pid,
                    start_time_ticks = response.start_time_ticks,
                    "vm start node registered and ready"
                );
                Ok(())
            }
        }
    }

    async fn spawn_and_check_process_alive(
        &self,
        vm: &str,
        node: &ProcessNode,
        budget: supervisor::dag::NodeBudget,
    ) -> Result<(), String> {
        match vm_start_node_mode(&node.role) {
            VmStartNodeMode::LongLived(runner_role) => {
                let response = self.spawn_runner(vm, node, runner_role, budget.spawn)?;
                tracing::info!(
                    vm = %vm,
                    node = %node.id.0,
                    role_id = %tracked_role_id(node),
                    pid = response.pid,
                    start_time_ticks = response.start_time_ticks,
                    "vm start node registered and process-alive"
                );
                Ok(())
            }
            _ => self.spawn_and_wait_ready(vm, node, &[], budget).await,
        }
    }

    async fn probe_api_ready(
        &self,
        vm: &str,
        node: &ProcessNode,
        readiness: &[ReadinessPredicate],
        timeout: Duration,
    ) -> supervisor::dag::ApiReadyState {
        // The split-readiness api-ready phase observes the already-spawned
        // long-lived runner, so wire in the liveness probe: a runner that
        // dies before the api-ready socket appears surfaces as an Error
        // (runner-exited / runner-reused) instead of a full-budget Timeout.
        let liveness = supervisor::readiness_liveness::PidfdLivenessProbe::new(
            &self.state.pidfd_table,
            &self.state.broker_reap_log,
            vm,
            tracked_role_id(node),
        );
        match wait_for_readiness(node, readiness, timeout, Some(&liveness)) {
            Ok(()) => supervisor::dag::ApiReadyState::Yes,
            Err(error) if error == format!("readiness-timeout:{}", node.id.0) => {
                supervisor::dag::ApiReadyState::Timeout
            }
            Err(reason) => supervisor::dag::ApiReadyState::Error { reason },
        }
    }
}

fn wait_for_readiness(
    node: &ProcessNode,
    readiness: &[ReadinessPredicate],
    timeout: Duration,
    liveness: Option<&dyn supervisor::readiness_liveness::LivenessProbe>,
) -> Result<(), String> {
    use supervisor::readiness_liveness::RunnerLiveness;

    // Map a terminal liveness verdict to the fast-fail error string. The
    // readiness loop returns this immediately instead of blocking to the
    // readiness deadline when the spawned runner dies (or its PID is
    // reused) before its readiness signal fires.
    fn terminal_liveness_error(
        node: &ProcessNode,
        liveness: Option<&dyn supervisor::readiness_liveness::LivenessProbe>,
    ) -> Option<String> {
        match liveness?.probe() {
            RunnerLiveness::Exited(_) => Some(format!("runner-exited:{}", node.id.0)),
            RunnerLiveness::Reused => Some(format!("runner-reused:{}", node.id.0)),
            RunnerLiveness::Alive | RunnerLiveness::Unknown => None,
        }
    }

    if readiness.is_empty() {
        return Ok(());
    }
    let deadline = Instant::now() + timeout;
    loop {
        // Liveness BEFORE readiness: a runner that already exited must
        // fast-fail rather than spin to the deadline.
        if let Some(error) = terminal_liveness_error(node, liveness) {
            return Err(error);
        }
        let mut all_ready = true;
        for predicate in readiness {
            if !readiness_predicate_ready(predicate)? {
                all_ready = false;
                break;
            }
        }
        if all_ready {
            // Re-confirm liveness BEFORE declaring ready so a stale
            // listening socket left behind by an exited runner cannot
            // yield a false-ready.
            if let Some(error) = terminal_liveness_error(node, liveness) {
                return Err(error);
            }
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("readiness-timeout:{}", node.id.0));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn readiness_predicate_ready(predicate: &ReadinessPredicate) -> Result<bool, String> {
    match predicate {
        ReadinessPredicate::ApiSocketInfo(path) => Ok(api_socket_info_ready(path)),
        ReadinessPredicate::VsockNotify(value) => Ok(Path::new(value).exists()),
        ReadinessPredicate::UnixSocketExists(path) => Ok(unix_socket_exists(path)),
        ReadinessPredicate::UnixSocketListening(path) => Ok(unix_socket_listening(path)),
        ReadinessPredicate::TcpPort { host, port } => Ok(tcp_port_ready(host, *port)),
        ReadinessPredicate::Command(command) => command_ready(command),
        ReadinessPredicate::ComponentSpecific(_) => Ok(true),
        // The authenticated guest-control Health probe is evaluated through a
        // daemon-state-aware path (it needs the per-VM vsock socket, peer
        // credentials, and a broker-backed signer that this stateless helper
        // cannot reach). The live readiness path intercepts
        // `GuestControlHealth` nodes in `VmStartRunner::spawn_and_wait_ready`
        // before this generic evaluation is reached, so hitting this arm means
        // the state-aware routing regressed. Fail LOUD rather than silently
        // never-ready so the regression surfaces immediately.
        ReadinessPredicate::GuestControlHealth { .. } => {
            Err("guest-control-health-needs-state-aware-path".to_owned())
        }
    }
}

fn api_socket_info_ready(path: &str) -> bool {
    if !unix_socket_exists(path) {
        return false;
    }
    let Ok(mut socket) = UnixStream::connect(path) else {
        return false;
    };
    let _ = socket.set_read_timeout(Some(Duration::from_millis(250)));
    let _ = socket.set_write_timeout(Some(Duration::from_millis(250)));
    if socket
        .write_all(b"GET /api/v1/vm.info HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut buffer = [0_u8; 4096];
    let Ok(read) = socket.read(&mut buffer) else {
        return false;
    };
    if read == 0 {
        return false;
    }
    let response = String::from_utf8_lossy(&buffer[..read]);
    response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200")
}

fn unix_socket_exists(path: &str) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.file_type().is_socket())
        .unwrap_or(false)
}

fn unix_socket_listening(path: &str) -> bool {
    const SO_ACCEPTCON: u64 = 0x0001_0000;
    let Ok(contents) = fs::read_to_string("/proc/net/unix") else {
        return false;
    };
    contents.lines().skip(1).any(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 8 {
            return false;
        }
        let flags = u64::from_str_radix(fields[3], 16).unwrap_or(0);
        let socket_type = fields[4];
        let socket_path = fields[7];
        socket_path == path && socket_type == "0001" && (flags & SO_ACCEPTCON) != 0
    })
}

fn tcp_port_ready(host: &str, port: u16) -> bool {
    let Ok(addrs) = format!("{host}:{port}").to_socket_addrs() else {
        return false;
    };
    addrs
        .into_iter()
        .any(|addr| TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok())
}

fn wait_for_tcp_port(host: &str, port: u16, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if tcp_port_ready(host, port) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("tcp-readiness-timeout:{host}:{port}"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn command_ready(command: &[String]) -> Result<bool, String> {
    let Some(program) = command.first() else {
        return Err("command-readiness-empty".to_owned());
    };
    Command::new(program)
        .args(&command[1..])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .map_err(|_| "command-readiness-exec-failed".to_owned())
}

/// v1.1.2-final-R1 (panel-software + panel-test HIGH): explicit
/// process-state outcomes from `/proc/<pid>/stat`. The previous
/// `Ok(None)` return conflated three different scenarios — file
/// missing (process gone), file unreadable (transient race),
/// and file present-but-unparseable (kernel format regression).
/// Callers can now distinguish these and decide whether to retry,
/// fail-fast, or treat as terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcState {
    /// The process is alive in the given state character (e.g.
    /// 'S' sleeping, 'R' running, 'D' uninterruptible sleep,
    /// 'Z' zombie awaiting reap, 'X' dead).
    Alive(char),
    /// `/proc/<pid>/stat` does not exist — process has been
    /// reaped (no parent holding pidfd) or never existed.
    Gone,
    /// `/proc/<pid>/stat` is present but unparseable. This is
    /// either a transient mid-write race or a kernel-format
    /// regression. Callers may log + retry; treating it as
    /// `Alive` would risk spinning, treating it as `Gone` would
    /// risk false-positive termination.
    ParseFailed,
}

fn wait_for_one_shot_exit(
    pid: i32,
    start_time_ticks: u64,
    timeout: Duration,
) -> Result<(), String> {
    let proc_reader = supervisor::state::SystemProcReader;
    let deadline = Instant::now() + timeout;
    let mut parse_fail_warned = false;
    loop {
        match supervisor::state::ProcReader::proc_starttime(&proc_reader, pid) {
            Ok(Some(observed)) if observed == start_time_ticks => {
                // v1.1.2fu34: the broker holds the pidfd as the spawn parent
                // but never explicitly reaps via waitid; the child becomes a
                // zombie which still has /proc/<pid>/stat returning the same
                // starttime. Treat process-state 'Z' (zombie) or 'X' (dead)
                // as terminated so OneShot DAG nodes don't spin until the
                // polling timeout.
                match read_proc_state(pid) {
                    Ok(ProcState::Alive('Z')) | Ok(ProcState::Alive('X')) => {
                        return Ok(());
                    }
                    Ok(ProcState::Alive(_)) => {} // keep polling
                    Ok(ProcState::Gone) => return Ok(()),
                    Ok(ProcState::ParseFailed) => {
                        if !parse_fail_warned {
                            tracing::warn!(
                                pid,
                                "wait_for_one_shot_exit: /proc/<pid>/stat unparseable; \
                                 continuing to poll (will surface as oneshot-timeout if persistent)"
                            );
                            parse_fail_warned = true;
                        }
                    }
                    Err(err) => {
                        tracing::debug!(pid, %err, "read_proc_state I/O error; continuing to poll");
                    }
                }
                if Instant::now() >= deadline {
                    return Err(format!("oneshot-timeout:{pid}"));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Ok(Some(_)) => return Err(format!("oneshot-starttime-drift:{pid}")),
            Ok(None) => return Ok(()),
            Err(_) => return Err(format!("oneshot-proc-read-failed:{pid}")),
        }
    }
}

/// Parse /proc/<pid>/stat to extract the process-state field (field
/// 3, single character). Uses `rfind(')')` to correctly handle
/// comm fields containing `)` (the kernel emits `<pid> (<comm>)
/// <state> ...` and the LAST `)` always closes the comm field).
///
/// Returns:
/// - `Ok(ProcState::Alive(c))` when stat is readable and parses
/// - `Ok(ProcState::Gone)` when `/proc/<pid>/stat` is missing (ENOENT)
/// - `Ok(ProcState::ParseFailed)` when stat is readable but malformed
/// - `Err(io::Error)` for any other I/O error (permission, etc.)
fn read_proc_state(pid: i32) -> Result<ProcState, std::io::Error> {
    let path = format!("/proc/{pid}/stat");
    let data = match fs::read_to_string(&path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ProcState::Gone),
        Err(e) => return Err(e),
    };
    if let Some(close) = data.rfind(')') {
        let after = &data[close + 1..];
        let mut chars = after.split_whitespace();
        if let Some(state_str) = chars.next()
            && let Some(c) = state_str.chars().next()
        {
            return Ok(ProcState::Alive(c));
        }
    }
    Ok(ProcState::ParseFailed)
}

#[cfg(test)]
mod proc_state_tests {
    // v1.1.2-final-R1 (panel-test HIGH): explicit coverage of
    // /proc/<pid>/stat parsing. Each case exercises the parser
    // with a synthetic stat-format string to ensure the
    // `rfind(')')` correctly handles comm names containing `)`
    // and that malformed input maps to `ParseFailed`, not
    // `Alive`.
    use super::*;

    fn parse(data: &str) -> ProcState {
        if let Some(close) = data.rfind(')') {
            let after = &data[close + 1..];
            let mut chars = after.split_whitespace();
            if let Some(state_str) = chars.next()
                && let Some(c) = state_str.chars().next()
            {
                return ProcState::Alive(c);
            }
        }
        ProcState::ParseFailed
    }

    #[test]
    fn simple_zombie() {
        assert_eq!(parse("1234 (sh) Z 1 1234 ..."), ProcState::Alive('Z'));
    }

    #[test]
    fn simple_running() {
        assert_eq!(parse("99 (bash) R 1 99 99 ..."), ProcState::Alive('R'));
    }

    #[test]
    fn comm_with_paren() {
        // Process comm contains ')' — rfind correctly picks the
        // OUTER closing paren that ends the comm field.
        assert_eq!(parse("42 (foo) bar) Z 1 42 ..."), ProcState::Alive('Z'));
    }

    #[test]
    fn comm_with_spaces_and_paren() {
        assert_eq!(parse("7 (cmd (in jail)) S 1 7 ..."), ProcState::Alive('S'));
    }

    #[test]
    fn truncated_stat() {
        // Comm present but no state field after — ParseFailed.
        assert_eq!(parse("1234 (sh)"), ProcState::ParseFailed);
    }

    #[test]
    fn no_paren_at_all() {
        // Garbage input without comm parens — ParseFailed.
        assert_eq!(parse("not a stat line at all"), ProcState::ParseFailed);
    }

    #[test]
    fn empty_input() {
        assert_eq!(parse(""), ProcState::ParseFailed);
    }

    #[test]
    fn dead_process() {
        assert_eq!(parse("88 (init) X 1 88 ..."), ProcState::Alive('X'));
    }
}

#[cfg(test)]
#[cfg(target_os = "linux")]
mod unix_socket_readiness_tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[test]
    fn unix_socket_listening_detects_listening_stream_socket_without_connecting() {
        let path = std::env::temp_dir().join(format!("nl-usl-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let path_str = path.to_string_lossy().to_string();

        assert!(!unix_socket_listening(&path_str));
        let listener = UnixListener::bind(&path).expect("bind unix listener");
        assert!(unix_socket_exists(&path_str));
        assert!(unix_socket_listening(&path_str));

        drop(listener);
        let _ = std::fs::remove_file(&path);
    }
}

/// Zombie-detection hermetic tests for `wait_for_one_shot_exit`.
/// Linux-only: depends on `/proc/<pid>/stat`.
///
/// No `unsafe` code: child processes are created via
/// `std::process::Command`.  Rust's `Child` does not call `waitpid` on
/// drop, so an exited child stays in 'Z' state until the test calls
/// `child.wait()` for cleanup.
#[cfg(test)]
#[cfg(target_os = "linux")]
mod wait_for_one_shot_exit_tests {
    use super::*;
    use std::process::{Child, Command};

    /// Read the `starttime` field (column 22) for `pid` from
    /// `/proc/<pid>/stat`.  Panics if the file is missing or
    /// unparseable — this is a test-only helper.
    fn read_start_time_ticks(pid: u32) -> u64 {
        let path = format!("/proc/{pid}/stat");
        let content = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        supervisor::state::parse_proc_stat_starttime(&content)
            .unwrap_or_else(|e| panic!("parse {path}: {e}"))
    }

    /// Spawn `sleep 0` — the child exits in < 1 ms, leaving a zombie
    /// behind because Rust's `Child::drop` does not call `waitpid`.
    fn spawn_zombie_child() -> Child {
        Command::new("sleep")
            .arg("0")
            .spawn()
            .expect("spawn 'sleep 0'")
    }

    /// Spawn `sleep 30` — alive for the duration of the test.
    fn spawn_sleeping_child() -> Child {
        Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn 'sleep 30'")
    }

    // v1.2 asserts the zombie shortcut path: `wait_for_one_shot_exit`
    // must return `Ok(())` immediately (≤100 ms) when the target is in
    // state 'Z', without waiting for the full polling timeout.
    #[test]
    fn wait_for_one_shot_exit_returns_ok_on_zombie_child() {
        let mut child = spawn_zombie_child();
        let pid = child.id();

        // Give 'sleep 0' a moment to exit and become a zombie.
        std::thread::sleep(Duration::from_millis(50));

        // The zombie's /proc/<pid>/stat is still present with 'Z' state
        // and the original starttime; read it now.
        let start_ticks = read_start_time_ticks(pid);

        let t0 = Instant::now();
        let result = wait_for_one_shot_exit(pid as i32, start_ticks, Duration::from_millis(500));
        let elapsed = t0.elapsed();

        // Reap the zombie before asserting so it isn't left around on
        // a test failure.
        child.wait().expect("waitpid zombie child");

        assert_eq!(result, Ok(()), "expected Ok(()) for zombie child");
        assert!(
            elapsed <= Duration::from_millis(100),
            "zombie shortcut must fire in ≤100 ms; took {elapsed:?}"
        );
    }

    // v1.2 asserts the timeout path — `wait_for_one_shot_exit` must
    // return `Err("oneshot-timeout:<pid>")` when the target stays alive
    // through the full polling window.
    #[test]
    fn wait_for_one_shot_exit_times_out_on_alive_process() {
        let mut child = spawn_sleeping_child();
        let pid = child.id();

        // Give the child a moment to be scheduled.
        std::thread::sleep(Duration::from_millis(10));

        let start_ticks = read_start_time_ticks(pid);

        let t0 = Instant::now();
        let result = wait_for_one_shot_exit(pid as i32, start_ticks, Duration::from_millis(100));
        let elapsed = t0.elapsed();

        // Kill and reap the child before asserting.
        child.kill().expect("kill sleeping child");
        child.wait().expect("waitpid sleeping child");

        assert_eq!(
            result,
            Err(format!("oneshot-timeout:{pid}")),
            "expected timeout error for alive process"
        );
        // The timeout is 100 ms; the polling loop sleeps 100 ms per
        // iteration, so elapsed must be ≥ 90 ms.
        assert!(
            elapsed >= Duration::from_millis(90),
            "expected ≥90 ms for timeout path; took {elapsed:?}"
        );
    }
}

fn stop_unregistered_spawned_runner(
    state: &ServerState,
    vm: &str,
    role_id: &str,
    response: &nixling_ipc::broker_wire::SpawnRunnerResponse,
    received_fds: &[RawFd],
) {
    let pidfd = duplicate_received_fd(
        received_fds,
        response.pidfd_index,
        "duplicate failed-registration SpawnRunner pidfd",
    )
    .map_err(|error| error.message());
    if let Err(error) = &pidfd {
        tracing::warn!(
            vm = %vm,
            role = %role_id,
            pid = response.pid,
            error = %error,
            "spawn registration failed; could not duplicate pidfd for cleanup"
        );
    }

    signal_unregistered_spawned_runner(
        state,
        vm,
        role_id,
        response,
        pidfd.as_ref().ok(),
        RunnerSignal::Term,
    );
    if wait_unregistered_spawned_runner_reaped(state, vm, role_id, Duration::from_secs(2)) {
        deregister_runner_pidfd_via_broker(
            state,
            BrokerCallerRole::AdminUid { uid: 0 },
            vm,
            role_id,
        );
        return;
    }

    tracing::warn!(
        vm = %vm,
        role = %role_id,
        pid = response.pid,
        "spawn registration failed; SIGTERM cleanup did not reap runner, escalating"
    );
    signal_unregistered_spawned_runner(
        state,
        vm,
        role_id,
        response,
        pidfd.as_ref().ok(),
        RunnerSignal::Kill,
    );
    if wait_unregistered_spawned_runner_reaped(state, vm, role_id, Duration::from_secs(2)) {
        deregister_runner_pidfd_via_broker(
            state,
            BrokerCallerRole::AdminUid { uid: 0 },
            vm,
            role_id,
        );
    } else {
        tracing::warn!(
            vm = %vm,
            role = %role_id,
            pid = response.pid,
            "spawn registration failed; runner was not observed reaped after SIGKILL, leaving broker pidfd registered"
        );
    }
}

fn signal_unregistered_spawned_runner(
    state: &ServerState,
    vm: &str,
    role_id: &str,
    response: &nixling_ipc::broker_wire::SpawnRunnerResponse,
    pidfd: Option<&OwnedFd>,
    signal: RunnerSignal,
) {
    let signal_number = match signal {
        RunnerSignal::Term => libc::SIGTERM,
        RunnerSignal::Kill => libc::SIGKILL,
        RunnerSignal::Quit => libc::SIGQUIT,
    };
    let pidfd_signal = rustix::process::Signal::from_raw(signal_number);
    if let (Some(pidfd), Some(pidfd_signal)) = (pidfd, pidfd_signal) {
        match rustix::process::pidfd_send_signal(pidfd.as_fd(), pidfd_signal) {
            Ok(()) => {
                tracing::warn!(
                    vm = %vm,
                    role = %role_id,
                    pid = response.pid,
                    signal = runner_signal_label(signal),
                    "spawn registration failed; signaled unregistered runner by pidfd"
                );
                return;
            }
            Err(error) => tracing::warn!(
                vm = %vm,
                role = %role_id,
                pid = response.pid,
                signal = runner_signal_label(signal),
                error = %error,
                "spawn registration failed; direct pidfd signal failed, falling back to broker"
            ),
        }
    }

    let request = BrokerRequest::SignalRunner(SignalRunnerRequest {
        vm_id: VmId::new(vm),
        role_id: RoleId::new(role_id),
        signal,
        pid: Some(response.pid),
        expected_start_time_ticks: Some(response.start_time_ticks),
        tracing_span_id: None,
    });
    match dispatch_broker_request_as(state, request, BrokerCallerRole::AdminUid { uid: 0 }) {
        Ok(BrokerResponse::SignalRunner(resp))
            if resp.vm_id.as_str() == vm && resp.role_id.as_str() == role_id && resp.signaled =>
        {
            tracing::warn!(
                vm = %vm,
                role = %role_id,
                pid = response.pid,
                signal = runner_signal_label(signal),
                "spawn registration failed; broker signaled unregistered runner"
            );
        }
        Ok(other) => tracing::warn!(
            vm = %vm,
            role = %role_id,
            pid = response.pid,
            signal = runner_signal_label(signal),
            response = ?other,
            "spawn registration failed; broker cleanup signal returned unexpected response"
        ),
        Err(error) => tracing::warn!(
            vm = %vm,
            role = %role_id,
            pid = response.pid,
            signal = runner_signal_label(signal),
            error = ?error,
            "spawn registration failed; broker cleanup signal failed"
        ),
    }
}

fn wait_unregistered_spawned_runner_reaped(
    state: &ServerState,
    vm: &str,
    role_id: &str,
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        refresh_broker_reap_log(state, "unregistered-spawn-cleanup");
        if state.broker_reap_log.take_for(vm, role_id).is_some() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn write_runner_snapshot(
    state: &ServerState,
    vm: &str,
    role_id: &str,
    role: RunnerRole,
    pid: i32,
    start_time_ticks: u64,
) -> Result<(), String> {
    let store = supervisor::state::FilesystemSnapshotStore::new(&state.daemon_state_dir);
    supervisor::state::SnapshotStore::upsert(
        &store,
        &supervisor::state::RunnerSnapshotRecord {
            vm: vm.to_owned(),
            role_id: role_id.to_owned(),
            role,
            pid,
            start_time_ticks,
            snapshotted_at: chrono_like_rfc3339(),
        },
    )
    .map_err(|error| format!("runner-snapshot:{error}"))
}

fn remove_runner_snapshot(state: &ServerState, vm: &str, role_id: &str) {
    let store = supervisor::state::FilesystemSnapshotStore::new(&state.daemon_state_dir);
    if let Err(error) = supervisor::state::SnapshotStore::remove(&store, vm, role_id) {
        tracing::warn!(vm = %vm, role = %role_id, error = ?error, "failed to remove runner snapshot during cleanup");
    }
}

fn vm_supports_store_sync(resolver: &BundleResolver, vm: &str) -> bool {
    match resolver.manifest.vms.get(vm) {
        Some(entry) => entry.runtime.capabilities.store_sync,
        None => true,
    }
}

#[derive(Debug, Clone, Copy)]
enum RuntimeCapabilityGate {
    ConfigSync,
    Exec,
    GuestControl,
    Keys,
    Ssh,
    StoreSync,
    UsbHotplug,
}

impl RuntimeCapabilityGate {
    fn slug(self) -> &'static str {
        match self {
            Self::ConfigSync => "config-sync",
            Self::Exec => "exec",
            Self::GuestControl => "guest-control",
            Self::Keys => "keys",
            Self::Ssh => concat!("s", "sh"),
            Self::StoreSync => "store-sync",
            Self::UsbHotplug => "usb-hotplug",
        }
    }

    fn supported(self, entry: &ManifestVmEntry) -> bool {
        match self {
            Self::ConfigSync => entry.runtime.capabilities.config_sync,
            Self::Exec => entry.runtime.capabilities.exec,
            Self::GuestControl => entry.runtime.capabilities.guest_control,
            Self::Keys => entry.runtime.capabilities.keys,
            Self::Ssh => entry.runtime.capabilities.ssh,
            Self::StoreSync => entry.runtime.capabilities.store_sync,
            Self::UsbHotplug => entry.runtime.capabilities.usb_hotplug,
        }
    }
}

fn ensure_vm_runtime_capability(
    state: &ServerState,
    vm: &str,
    capability: RuntimeCapabilityGate,
    verb: &str,
) -> Result<(), TypedError> {
    let manifest: ManifestV04 = load_json(&state.config.artifacts.public_manifest_path)?;
    ensure_manifest_entry_runtime_capability(manifest.vms.get(vm), vm, capability, verb)
}

fn ensure_manifest_entry_runtime_capability(
    entry: Option<&ManifestVmEntry>,
    vm: &str,
    capability: RuntimeCapabilityGate,
    verb: &str,
) -> Result<(), TypedError> {
    let Some(entry) = entry else {
        return Ok(());
    };
    if capability.supported(entry) {
        return Ok(());
    }
    Err(TypedError::RuntimeCapabilityUnsupported {
        vm: vm.to_owned(),
        runtime_kind: serde_kebab_string(&entry.runtime.kind),
        capability: capability.slug().to_owned(),
        verb: verb.to_owned(),
    })
}

fn existing_vm_start_response_if_ready(
    state: &ServerState,
    vm: &str,
    runner_role_id: &str,
) -> Option<Value> {
    if !state.pidfd_table.contains(vm, runner_role_id) {
        return None;
    }

    Some(applied_response(
        "vm start",
        format!("vm.{vm}: already running; {runner_role_id} pidfd is live"),
    ))
}

fn cleanup_vm_start_registration(state: &ServerState, vm: &str, role_id: &str) {
    let _mguard = state.pidfd_table.mutation_guard();
    let _ = state.pidfd_table.deregister(vm, role_id);
    if let Err(error) = state.pidfd_table.snapshot() {
        tracing::warn!(vm = %vm, role = %role_id, error = ?error, "failed to persist pidfd table cleanup");
    }
    remove_runner_snapshot(state, vm, role_id);
}

fn rollback_failed_vm_start(
    state: &ServerState,
    vm: &str,
    tracked_roles: &[String],
) -> Result<(), Value> {
    let tracked: BTreeSet<&str> = tracked_roles.iter().map(String::as_str).collect();
    let mut entries = ordered_vm_stop_entries(state, vm)
        .into_iter()
        .filter(|entry| tracked.contains(entry.role.as_str()))
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return Ok(());
    }
    entries.sort_by(|left, right| {
        vm_stop_role_priority(infer_runner_role_for_vm_stop(&left.role))
            .cmp(&vm_stop_role_priority(infer_runner_role_for_vm_stop(
                &right.role,
            )))
            .then_with(|| left.role.cmp(&right.role))
    });
    for entry in entries {
        tracing::warn!(
            vm = %vm,
            role = %entry.role,
            "vm start failed; rolling back runner spawned during this start attempt",
        );
        stop_vm_pidfd_role(
            state,
            BrokerCallerRole::AdminUid { uid: 0 },
            "vm start",
            vm,
            &entry.role,
            Duration::from_secs(10),
            Duration::from_secs(5),
        )?;
    }
    let _mguard = state.pidfd_table.mutation_guard();
    if let Err(error) = state.pidfd_table.snapshot() {
        return Err(daemon_failure_response(
            "vm start",
            format!(
                "vm start {vm}: rollback stopped spawned runners but pidfd_table persistence failed ({error})"
            ),
        ));
    }
    Ok(())
}

fn qemu_media_primary_role_id(tracked_roles: &[String]) -> Option<&str> {
    tracked_roles
        .iter()
        .find(|role| role.as_str() == RunnerRole::QemuMedia.as_str())
        .map(String::as_str)
}

fn stale_qemu_media_dependency_roles_from_entries(
    tracked_roles: &[String],
    entries: &[PidfdRegistration],
) -> Vec<String> {
    let Some(primary_role) = qemu_media_primary_role_id(tracked_roles) else {
        return Vec::new();
    };
    if entries.iter().any(|entry| entry.role == primary_role) {
        return Vec::new();
    }

    let tracked: BTreeSet<&str> = tracked_roles.iter().map(String::as_str).collect();
    let mut stale_roles = entries
        .iter()
        .filter(|entry| {
            tracked.contains(entry.role.as_str()) && entry.role.as_str() != primary_role
        })
        .map(|entry| entry.role.clone())
        .collect::<Vec<_>>();
    stale_roles.sort_by(|left, right| {
        vm_stop_role_priority(infer_runner_role_for_vm_stop(left))
            .cmp(&vm_stop_role_priority(infer_runner_role_for_vm_stop(right)))
            .then_with(|| left.cmp(right))
    });
    stale_roles
}

fn cleanup_stale_qemu_media_dependencies_before_start(
    state: &ServerState,
    vm: &str,
    tracked_roles: &[String],
) -> Result<usize, Value> {
    let stale_roles = stale_qemu_media_dependency_roles_from_entries(
        tracked_roles,
        &ordered_vm_stop_entries(state, vm),
    );
    for role in &stale_roles {
        tracing::warn!(
            vm = %vm,
            role = %role,
            "qemu-media primary runner is absent; stopping leftover dependency before restart",
        );
        stop_vm_pidfd_role(
            state,
            BrokerCallerRole::AdminUid { uid: 0 },
            "vm start",
            vm,
            role,
            Duration::from_secs(10),
            Duration::from_secs(5),
        )?;
    }
    if !stale_roles.is_empty() {
        let _mguard = state.pidfd_table.mutation_guard();
        if let Err(error) = state.pidfd_table.snapshot() {
            return Err(daemon_failure_response(
                "vm start",
                format!(
                    "vm start {vm}: stale qemu-media dependency cleanup failed to persist pidfd_table ({error})"
                ),
            ));
        }
    }
    Ok(stale_roles.len())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VmStopRoleReport {
    role_id: String,
    required_sigkill: bool,
}

fn load_vm_stop_role_index(state: &ServerState, vm: &str) -> HashMap<String, RunnerRole> {
    let store = supervisor::state::FilesystemSnapshotStore::new(&state.daemon_state_dir);
    match supervisor::state::SnapshotStore::list(&store) {
        Ok(records) => records
            .into_iter()
            .filter(|record| record.vm == vm)
            .map(|record| (record.role_id, record.role))
            .collect(),
        Err(error) => {
            tracing::warn!(vm = %vm, error = ?error, "failed to load runner snapshots for vm stop ordering");
            HashMap::new()
        }
    }
}

fn infer_runner_role_for_vm_stop(role_id: &str) -> Option<RunnerRole> {
    if role_id == VM_RUNNER_ROLE_ID {
        Some(RunnerRole::CloudHypervisor)
    } else if role_id == RunnerRole::QemuMedia.as_str() || role_id.contains("qemu-media") {
        Some(RunnerRole::QemuMedia)
    } else if role_id == RunnerRole::SwtpmFlush.as_str() {
        Some(RunnerRole::SwtpmFlush)
    } else if role_id == RunnerRole::Swtpm.as_str() || role_id.starts_with("swtpm") {
        Some(RunnerRole::Swtpm)
    } else if role_id == RunnerRole::Virtiofsd.as_str() || role_id.contains("virtiofsd") {
        Some(RunnerRole::Virtiofsd)
    } else if role_id == RunnerRole::Gpu.as_str() || role_id.contains("gpu") {
        Some(RunnerRole::Gpu)
    } else if role_id == RunnerRole::Audio.as_str() || role_id.contains("audio") {
        Some(RunnerRole::Audio)
    } else if role_id == RunnerRole::Video.as_str() || role_id.contains("video") {
        Some(RunnerRole::Video)
    } else if role_id == RunnerRole::WaylandProxy.as_str()
        || role_id.contains("wayland-proxy")
        || role_id.contains("wlproxy")
    {
        Some(RunnerRole::WaylandProxy)
    } else if role_id == RunnerRole::Usbip.as_str() || role_id.contains("usbip") {
        Some(RunnerRole::Usbip)
    } else if role_id == RunnerRole::VsockRelay.as_str() || role_id.contains("vsock") {
        Some(RunnerRole::VsockRelay)
    } else if role_id == RunnerRole::OtelHostBridge.as_str()
        || role_id.contains("otel-host-bridge")
        || role_id.contains("otel_host_bridge")
    {
        Some(RunnerRole::OtelHostBridge)
    } else {
        None
    }
}

fn vm_stop_role_priority(role: Option<RunnerRole>) -> u8 {
    match role {
        Some(RunnerRole::CloudHypervisor) => 0,
        Some(RunnerRole::QemuMedia) => 0,
        Some(RunnerRole::Gpu) => 1,
        Some(RunnerRole::Audio) => 2,
        Some(RunnerRole::Video) => 3,
        // WaylandProxy is the upstream for the GPU runner; stop it after
        // GPU so the GPU runner can close its connection cleanly first.
        Some(RunnerRole::WaylandProxy) => 3,
        Some(RunnerRole::Usbip) => 4,
        Some(RunnerRole::VsockRelay) => 5,
        // OtelHostBridge is observability infrastructure; stop it before
        // swtpm/virtiofsd so trailing OTel spans flush
        // before the per-VM TPM + virtiofs are torn down.
        Some(RunnerRole::OtelHostBridge) => 5,
        Some(RunnerRole::Swtpm) => 6,
        Some(RunnerRole::Virtiofsd) => 7,
        Some(RunnerRole::SwtpmFlush) => 8,
        None => 9,
    }
}

fn ordered_vm_stop_entries(state: &ServerState, vm: &str) -> Vec<PidfdRegistration> {
    let role_index = load_vm_stop_role_index(state, vm);
    let mut entries = state.pidfd_table.list_for_vm(vm);
    entries.sort_by(|left, right| {
        let left_role = role_index
            .get(&left.role)
            .copied()
            .or_else(|| infer_runner_role_for_vm_stop(&left.role));
        let right_role = role_index
            .get(&right.role)
            .copied()
            .or_else(|| infer_runner_role_for_vm_stop(&right.role));
        vm_stop_role_priority(left_role)
            .cmp(&vm_stop_role_priority(right_role))
            .then_with(|| left.role.cmp(&right.role))
    });
    entries
}

fn runner_signal_label(signal: RunnerSignal) -> &'static str {
    match signal {
        RunnerSignal::Term => "SIGTERM",
        RunnerSignal::Kill => "SIGKILL",
        RunnerSignal::Quit => "SIGQUIT",
    }
}

fn broker_fallback_failure(
    vm: &str,
    role_id: &str,
    signal: RunnerSignal,
    detail: impl std::fmt::Display,
) -> Value {
    daemon_failure_response(
        "vm stop",
        format!(
            "vm stop {vm}: pidfd_table {} failed for {role_id}; broker fallback failed: {detail}",
            runner_signal_label(signal)
        ),
    )
}

fn signal_via_broker(
    state: &ServerState,
    caller_role: BrokerCallerRole,
    vm: &str,
    role_id: &str,
    signal: RunnerSignal,
) -> Result<(), Value> {
    let registration = state
        .pidfd_table
        .list_for_vm(vm)
        .into_iter()
        .find(|entry| entry.role == role_id);
    let request = BrokerRequest::SignalRunner(SignalRunnerRequest {
        vm_id: VmId::new(vm),
        role_id: RoleId::new(role_id),
        signal,
        pid: registration.as_ref().map(|entry| entry.pid),
        expected_start_time_ticks: registration.as_ref().map(|entry| entry.start_time_ticks),
        tracing_span_id: None,
    });
    match dispatch_broker_request_as(state, request, caller_role) {
        Ok(BrokerResponse::SignalRunner(resp))
            if resp.vm_id.as_str() == vm && resp.role_id.as_str() == role_id && resp.signaled =>
        {
            Ok(())
        }
        Ok(BrokerResponse::SignalRunner(resp)) => Err(broker_fallback_failure(
            vm,
            role_id,
            signal,
            format!(
                "SignalRunner returned vm={} role={} signaled={}",
                resp.vm_id.as_str(),
                resp.role_id.as_str(),
                resp.signaled
            ),
        )),
        Ok(BrokerResponse::Error(error)) => Err(broker_fallback_failure(
            vm,
            role_id,
            signal,
            format!(
                "SignalRunner rejected by broker: {} ({})",
                error.message, error.kind
            ),
        )),
        Ok(other) => Err(broker_fallback_failure(
            vm,
            role_id,
            signal,
            format!("SignalRunner returned unexpected response: {other:?}"),
        )),
        Err(err) => Err(broker_fallback_failure(
            vm,
            role_id,
            signal,
            format!("{err:?}"),
        )),
    }
}

fn deregister_runner_pidfd_via_broker(
    state: &ServerState,
    caller_role: BrokerCallerRole,
    vm: &str,
    role_id: &str,
) {
    let request = BrokerRequest::DeregisterRunnerPidfd(DeregisterRunnerPidfdRequest {
        vm_id: VmId::new(vm),
        role_id: RoleId::new(role_id),
        tracing_span_id: None,
    });
    match dispatch_broker_request_as(state, request, caller_role) {
        Ok(BrokerResponse::DeregisterRunnerPidfd(resp))
            if resp.vm_id.as_str() == vm && resp.role_id.as_str() == role_id =>
        {
            if !resp.removed {
                tracing::warn!(
                    vm = %vm,
                    role = %role_id,
                    removed = false,
                    "broker runner pidfd deregister reported no entry"
                );
            }
        }
        Ok(other) => tracing::warn!(
            vm = %vm,
            role = %role_id,
            response = ?other,
            "broker runner pidfd deregister returned unexpected response"
        ),
        Err(error) => tracing::warn!(
            vm = %vm,
            role = %role_id,
            error = ?error,
            "broker runner pidfd deregister failed"
        ),
    }
}

fn wait_terminated_with_broker_poll(
    state: &ServerState,
    vm: &str,
    role_id: &str,
    deadline: Instant,
) -> Result<WaitTermination, PidfdTableError> {
    let started = Instant::now();
    let mut poll_count: u32 = 0;
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Ok(WaitTermination::TimedOut);
        }
        let remaining = deadline.saturating_duration_since(now);
        match state.pidfd_table.wait_terminated(vm, role_id, remaining) {
            Ok(WaitTermination::Terminated) => return Ok(WaitTermination::Terminated),
            Ok(WaitTermination::TerminatedByBroker { exit_status }) => {
                return Ok(WaitTermination::TerminatedByBroker { exit_status });
            }
            Ok(WaitTermination::TimedOut) => return Ok(WaitTermination::TimedOut),
            Err(PidfdTableError::WaitFailed {
                errno: Some(libc::ECHILD),
                ..
            }) => {
                if !state.pidfd_table.still_alive_same_start_time(vm, role_id) {
                    return Ok(WaitTermination::Terminated);
                }
                poll_count = poll_count.saturating_add(1);
                let budget = remaining.min(Duration::from_millis(200));
                if let Ok(BrokerResponse::PollChildReaped(resp)) =
                    dispatch_broker_request_with_timeout(
                        state,
                        BrokerRequest::PollChildReaped,
                        budget,
                    )
                {
                    for notification in resp.notifications {
                        state.broker_reap_log.insert(notification);
                    }
                    if let Some(notification) = state.broker_reap_log.take_for(vm, role_id) {
                        let elapsed = started.elapsed();
                        tracing::info!(
                            outcome = "echild-broker-recovered",
                            vm = %vm,
                            role = %role_id,
                            poll_count,
                            elapsed_ms = elapsed.as_millis() as u64,
                            "stop reap recovered via broker reap log"
                        );
                        return Ok(WaitTermination::TerminatedByBroker {
                            exit_status: notification.exit_status,
                        });
                    }
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(error),
        }
    }
}

fn stop_vm_pidfd_role(
    state: &ServerState,
    caller_role: BrokerCallerRole,
    verb: &str,
    vm: &str,
    role_id: &str,
    term_timeout: Duration,
    kill_timeout: Duration,
) -> Result<VmStopRoleReport, Value> {
    tracing::info!(vm = %vm, role = %role_id, signal = "SIGTERM", "sending pidfd stop signal");
    match state.pidfd_table.signal(vm, role_id, libc::SIGTERM) {
        Ok(()) => {}
        Err(PidfdTableError::SignalFailed {
            errno: Some(libc::ESRCH),
            ..
        }) => {
            tracing::info!(
                vm = %vm,
                role = %role_id,
                signal = "SIGTERM",
                outcome = "already-exited",
                "pidfd signal target was already gone"
            );
        }
        Err(PidfdTableError::SignalFailed {
            errno: Some(libc::EPERM),
            ..
        }) => {
            signal_via_broker(state, caller_role.clone(), vm, role_id, RunnerSignal::Term)?;
            metrics::record_broker_request(
                &state.metrics_registry,
                "SignalRunner",
                "broker-fallback",
            );
            tracing::info!(
                outcome = "broker-fallback",
                broker_signaled = true,
                vm = %vm,
                role = %role_id,
                signal = "SIGTERM",
                "pidfd signal EPERM recovered through broker"
            );
        }
        Err(PidfdTableError::NotFound { .. }) => {
            return Err(invalid_request_response(
                verb,
                format!("vm '{}' has no registered {} pidfd", vm, role_id),
            ));
        }
        Err(error) => {
            tracing::warn!(vm = %vm, role = %role_id, error = ?error, "SIGTERM failed");
            return Err(daemon_failure_response(
                verb,
                format!("vm stop {vm}: pidfd_table SIGTERM failed for {role_id}"),
            ));
        }
    }

    refresh_broker_reap_log(state, "before-sigterm-wait");
    match wait_terminated_with_broker_poll(state, vm, role_id, Instant::now() + term_timeout) {
        Ok(WaitTermination::Terminated) | Ok(WaitTermination::TerminatedByBroker { .. }) => {
            tracing::info!(
                vm = %vm,
                role = %role_id,
                signal = "SIGTERM",
                timeout_ms = term_timeout.as_millis(),
                outcome = "terminated",
                "role terminated after SIGTERM"
            );
            let _ = state.pidfd_table.deregister(vm, role_id);
            deregister_runner_pidfd_via_broker(state, caller_role.clone(), vm, role_id);
            remove_runner_snapshot(state, vm, role_id);
            return Ok(VmStopRoleReport {
                role_id: role_id.to_owned(),
                required_sigkill: false,
            });
        }
        Ok(WaitTermination::TimedOut) => {
            tracing::warn!(
                vm = %vm,
                role = %role_id,
                signal = "SIGTERM",
                timeout_ms = term_timeout.as_millis(),
                "SIGTERM wait timed out; escalating to SIGKILL"
            );
        }
        Err(PidfdTableError::NotFound { .. }) => {
            return Err(invalid_request_response(
                verb,
                format!("vm '{}' has no registered {} pidfd", vm, role_id),
            ));
        }
        Err(error) => {
            tracing::warn!(vm = %vm, role = %role_id, error = ?error, "wait after SIGTERM failed");
            return Err(daemon_failure_response(
                verb,
                format!("vm stop {vm}: wait after SIGTERM failed for {role_id}"),
            ));
        }
    }

    tracing::info!(vm = %vm, role = %role_id, signal = "SIGKILL", "sending pidfd kill signal");
    match state.pidfd_table.signal(vm, role_id, libc::SIGKILL) {
        Ok(()) => {}
        Err(PidfdTableError::SignalFailed {
            errno: Some(libc::ESRCH),
            ..
        }) => {
            tracing::info!(
                vm = %vm,
                role = %role_id,
                signal = "SIGKILL",
                outcome = "already-exited",
                "pidfd kill target was already gone"
            );
        }
        Err(PidfdTableError::SignalFailed {
            errno: Some(libc::EPERM),
            ..
        }) => {
            signal_via_broker(state, caller_role.clone(), vm, role_id, RunnerSignal::Kill)?;
            metrics::record_broker_request(
                &state.metrics_registry,
                "SignalRunner",
                "broker-fallback",
            );
            tracing::info!(
                outcome = "broker-fallback",
                broker_signaled = true,
                vm = %vm,
                role = %role_id,
                signal = "SIGKILL",
                "pidfd signal EPERM recovered through broker"
            );
        }
        Err(PidfdTableError::NotFound { .. }) => {
            return Err(invalid_request_response(
                verb,
                format!("vm '{}' has no registered {} pidfd", vm, role_id),
            ));
        }
        Err(error) => {
            tracing::warn!(vm = %vm, role = %role_id, error = ?error, "SIGKILL failed");
            return Err(daemon_failure_response(
                verb,
                format!("vm stop {vm}: pidfd_table SIGKILL failed for {role_id}"),
            ));
        }
    }

    refresh_broker_reap_log(state, "before-sigkill-wait");
    match wait_terminated_with_broker_poll(state, vm, role_id, Instant::now() + kill_timeout) {
        Ok(WaitTermination::Terminated) | Ok(WaitTermination::TerminatedByBroker { .. }) => {
            tracing::info!(
                vm = %vm,
                role = %role_id,
                signal = "SIGKILL",
                timeout_ms = kill_timeout.as_millis(),
                outcome = "terminated",
                "role terminated after SIGKILL"
            );
            let _ = state.pidfd_table.deregister(vm, role_id);
            deregister_runner_pidfd_via_broker(state, caller_role, vm, role_id);
            remove_runner_snapshot(state, vm, role_id);
            Ok(VmStopRoleReport {
                role_id: role_id.to_owned(),
                required_sigkill: true,
            })
        }
        Ok(WaitTermination::TimedOut) => Err(daemon_failure_response(
            verb,
            format!("vm stop {vm}: timed out waiting for {role_id} after SIGKILL"),
        )),
        Err(PidfdTableError::NotFound { .. }) => Err(invalid_request_response(
            verb,
            format!("vm '{}' has no registered {} pidfd", vm, role_id),
        )),
        Err(error) => {
            tracing::warn!(vm = %vm, role = %role_id, error = ?error, "wait after SIGKILL failed");
            Err(daemon_failure_response(
                verb,
                format!("vm stop {vm}: wait after SIGKILL failed for {role_id}"),
            ))
        }
    }
}

fn vm_start_success_summary(report: &supervisor::dag::DagRunReport) -> String {
    let ready_nodes = report
        .history
        .iter()
        .filter_map(|entry| match &entry.outcome {
            supervisor::dag::NodeOutcome::Ready => Some(entry.node_id.0.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    format!(
        "vm start {}: supervisor DAG ready ({} nodes: {}); registered in pidfd_table",
        report.vm,
        ready_nodes.len(),
        ready_nodes.join(" -> ")
    )
}

fn vm_start_failure_response(report: &supervisor::dag::DagRunReport) -> Value {
    let summary = report
        .history
        .iter()
        .find_map(|entry| match &entry.outcome {
            supervisor::dag::NodeOutcome::Failed { .. } => {
                Some(format!("SpawnRunner failed at {}", entry.node_id.0))
            }
            _ => None,
        })
        .unwrap_or_else(|| "SpawnRunner failed".to_owned());
    broker_failure_response(
        "vm start",
        summary,
        "Supervisor DAG aborted before every readiness deadline passed. Admin: inspect `journalctl -u nixlingd` for the per-node supervisor audit.".to_owned(),
        None,
    )
}

fn log_vm_start_report(report: &supervisor::dag::DagRunReport) {
    for entry in &report.history {
        match &entry.outcome {
            supervisor::dag::NodeOutcome::Ready => tracing::info!(
                vm = %report.vm,
                node = %entry.node_id.0,
                outcome = "ready",
                "vm start DAG node completed"
            ),
            supervisor::dag::NodeOutcome::Failed { reason } => tracing::warn!(
                vm = %report.vm,
                node = %entry.node_id.0,
                outcome = "failed",
                reason = %reason,
                "vm start DAG node failed"
            ),
            supervisor::dag::NodeOutcome::Skipped { predecessor } => tracing::warn!(
                vm = %report.vm,
                node = %entry.node_id.0,
                outcome = "skipped",
                predecessor = %predecessor.0,
                "vm start DAG node skipped after predecessor failure"
            ),
        }
    }
}

/// Log the planned host-prep DAG so `journalctl -u nixlingd.service`
/// and the autostart-history
/// records carry the canonical step set per VM. Runs on every VM
/// start regardless of whether `NIXLING_HOST_PREP_DAG_EXECUTE` is
/// set.
fn log_host_prep_dag(vm: &str, steps: &[nixling_host::host_prep_dag::HostPrepStep]) {
    tracing::info!(
        vm = %vm,
        step_count = steps.len(),
        "host-prep DAG resolved"
    );
    for step in steps {
        tracing::info!(
            vm = %vm,
            step_id = %step.id,
            kind = step.kind.as_str(),
            broker_op = step.kind.broker_op_name(),
            depends_on = ?step.depends_on.iter().map(|d| d.as_str()).collect::<Vec<_>>(),
            "host-prep DAG step"
        );
    }
}

/// Extract a role id from the DAG step's bundle_ref, falling back to a
/// step-default. The runner intent id
/// shape `runner:vm:<vm>:role:<role>` lets us derive the role
/// mechanically; if the step did not carry a bundle_op_id we use
/// the default (`ch` for tap/vhost-net).
fn host_prep_role_id_from_bundle_ref(
    bundle_ref: &nixling_host::host_prep_dag::BundleStepRef,
    default_role: &str,
) -> RoleId {
    let role = bundle_ref
        .bundle_op_id
        .as_ref()
        .and_then(|id| id.as_str().rsplit_once(":role:").map(|(_, r)| r.to_owned()))
        .unwrap_or_else(|| default_role.to_owned());
    RoleId::new(role)
}

/// Dispatch a broker request for one host-prep DAG step where the broker
/// may return a typed response
/// (e.g. `CreatePersistentTap`, `SetBridgePortFlags`) rather than
/// the canonical `Ack`. Treats any non-`Error` response as success
/// and surfaces `Error` responses through the same launcher-side
/// redaction path used by `dispatch_broker_ack_request`. Any fd
/// the broker attaches via SCM_RIGHTS is silently discarded by the
/// kernel because `dispatch_broker_request` does not allocate an
/// ancillary buffer.
fn dispatch_broker_host_prep_step(
    state: &ServerState,
    verb: &str,
    op_name: &str,
    request: BrokerRequest,
) -> Result<(), Value> {
    match dispatch_broker_request(state, request) {
        Ok(BrokerResponse::Error(error)) => {
            tracing::warn!(
                broker_kind = %error.kind,
                broker_operation = %error.operation,
                broker_target_wave = error.target_wave.as_deref().unwrap_or("none"),
                broker_message = %error.message,
                broker_action = %error.action,
                "broker live op failed"
            );
            let (summary, remediation) = redact_broker_error_for_launcher(
                op_name,
                error.target_wave.as_deref(),
                &error.kind,
            );
            Err(broker_failure_response(
                verb,
                summary,
                remediation,
                error.target_wave,
            ))
        }
        Ok(_) => Ok(()),
        Err(error) => {
            tracing::warn!(op_name = op_name, error = ?error, "broker dispatch failed");
            let (summary, remediation) = redact_broker_dispatch_failure_for_launcher(op_name);
            Err(broker_failure_response(verb, summary, remediation, None))
        }
    }
}

/// Execute the host-prep DAG by dispatching the corresponding broker op
/// for each step in topo order. On step failure surfaces the broker
/// envelope; the operator sees the step id, the broker op kind, and the
/// broker error string (the typed `HostPrepStepFailed` shape lives in
/// `nixling_host::host_prep_dag`). Gated by
/// `NIXLING_HOST_PREP_DAG_EXECUTE` until the broker handlers land.
fn execute_host_prep_dag(
    state: &ServerState,
    vm: &str,
    steps: &[nixling_host::host_prep_dag::HostPrepStep],
) -> Result<(), Value> {
    use nixling_host::host_prep_dag::HostPrepStepKind;
    const VERB: &str = "vm start";
    // Resolve the per-VM state directory once (used by daemon-native
    // step handlers that need filesystem context, e.g. the
    // ssh-host-key preflight). Missing manifest entries are tolerated
    // here: the broker-dispatching path simply proceeds, and
    // daemon-native handlers gracefully no-op.
    let per_vm_state_dir: Option<PathBuf> = load_bundle_resolver(state)
        .ok()
        .and_then(|r| r.manifest.vms.get(vm).map(|m| PathBuf::from(&m.state_dir)));
    for step in steps {
        let op_name = step.kind.broker_op_name();
        let request = match step.kind {
            HostPrepStepKind::ApplyNftablesRules => {
                let nft_ref = step
                    .bundle_ref
                    .bundle_op_id
                    .clone()
                    .unwrap_or_else(|| BundleOpId::new(intent_id_nft_host()));
                let scope_id = step
                    .bundle_ref
                    .scope_id
                    .clone()
                    .unwrap_or_else(|| ScopeId::new("host"));
                BrokerRequest::ApplyNftables(BrokerApplyNftablesRequest {
                    bundle_nft_intent_ref: nft_ref,
                    scope_id,
                    desired_hash: None,
                    destroy: false,
                    tracing_span_id: None,
                })
            }
            HostPrepStepKind::SeedDnsmasqLease => {
                BrokerRequest::SeedDnsmasqLease(nixling_ipc::broker_wire::SeedDnsmasqLeaseRequest {
                    vm_id: step.bundle_ref.vm_id.clone(),
                    scope_id: step
                        .bundle_ref
                        .scope_id
                        .clone()
                        .unwrap_or_else(|| ScopeId::new("host")),
                    tracing_span_id: None,
                })
            }
            HostPrepStepKind::BindMountFromHardlinkFarm => {
                BrokerRequest::BindMountFromHardlinkFarm(
                    nixling_ipc::broker_wire::BindMountFromHardlinkFarmRequest {
                        vm_id: step.bundle_ref.vm_id.clone(),
                        bundle_store_view_intent_ref: step.bundle_ref.bundle_op_id.clone(),
                        tracing_span_id: None,
                    },
                )
            }
            HostPrepStepKind::OwnershipMatrixCheck => BrokerRequest::OwnershipMatrixCheck(
                nixling_ipc::broker_wire::OwnershipMatrixCheckRequest {
                    vm_id: step.bundle_ref.vm_id.clone(),
                    tracing_span_id: None,
                },
            ),
            HostPrepStepKind::SshHostKeyPreflight => {
                // Run the daemon-native posture check instead of dispatching
                // the broker stub. The broker variant remains in the
                // wire enum as a typed placeholder; the live handler
                // lives daemon-side because the check is a pure
                // filesystem stat against a state subtree the daemon
                // already has `CAP_DAC_READ_SEARCH` for.
                let keys_dir = match per_vm_state_dir.as_ref() {
                    Some(d) => d.join("sshd-host-keys"),
                    None => {
                        tracing::warn!(
                            vm = %vm,
                            step_id = %step.id,
                            "ssh-host-key-preflight: no manifest entry resolvable; skipping",
                        );
                        continue;
                    }
                };
                if !keys_dir.exists() {
                    tracing::warn!(
                        vm = %vm,
                        step_id = %step.id,
                        outcome = "skipped-keys-dir-absent",
                        "ssh-host-key-preflight: keys directory absent; skipping (will be materialized on first run)",
                    );
                    continue;
                }
                if let Err(drift) = ssh_host_key_preflight::check_sshd_host_keys(vm, &keys_dir) {
                    let path = drift.path().to_path_buf();
                    let drift_reason = drift.reason();
                    tracing::warn!(
                        vm = %vm,
                        step_id = %step.id,
                        outcome = "ssh-host-key-drift",
                        drift_kind = ?drift_reason,
                        "host-prep DAG step failed: ssh-host-key drift (path in typed envelope + audit log)",
                    );
                    return Err(TypedError::SshdHostKeyDrift {
                        vm: vm.to_owned(),
                        path,
                        drift: drift_reason,
                    }
                    .to_envelope_value());
                }
                continue;
            }
            // Live broker dispatch for the step kinds that previously
            // skipped-with-log. Each arm composes an existing broker op.
            HostPrepStepKind::BringUpTapInterface => {
                // Compose CreatePersistentTap. The DAG anchors tap ownership via
                // `runner:vm:<vm>:role:ch`; the host-prep DAG is
                // about the persistent-side setup (ifname pinned,
                // bridge port flags eventually applied), so we use
                // CreatePersistentTap rather than CreateTapFd.
                // CreateTapFd is the per-launch op that ships an
                // SCM_RIGHTS fd back; the runner re-opens it at
                // spawn time. Skipping in unit-test contexts is
                // controlled by `NIXLING_HOST_PREP_DAG_EXECUTE`
                // upstream; here we always issue the request.
                let role_id = host_prep_role_id_from_bundle_ref(&step.bundle_ref, "ch");
                let req = BrokerRequest::CreatePersistentTap(
                    nixling_ipc::broker_wire::CreatePersistentTapRequest {
                        role_id,
                        vm_id: step.bundle_ref.vm_id.clone(),
                        tracing_span_id: None,
                    },
                );
                if let Err(response) = dispatch_broker_host_prep_step(state, VERB, op_name, req) {
                    tracing::warn!(
                        vm = %vm,
                        step_id = %step.id,
                        op_kind = op_name,
                        "host-prep DAG step failed"
                    );
                    return Err(response);
                }
                continue;
            }
            HostPrepStepKind::PreOpenVhostNetFd => {
                // Dispatch OpenVhostNet for role `ch`. The broker returns
                // an SCM_RIGHTS fd
                // alongside an `Ack`; we don't need the fd here
                // (the runner re-requests it at spawn), so the
                // ack-only dispatcher discards it via MSG_CTRUNC.
                let role_id = host_prep_role_id_from_bundle_ref(&step.bundle_ref, "ch");
                let req =
                    BrokerRequest::OpenVhostNet(nixling_ipc::broker_wire::OpenVhostNetRequest {
                        role_id,
                        tracing_span_id: None,
                    });
                if let Err(response) = dispatch_broker_host_prep_step(state, VERB, op_name, req) {
                    tracing::warn!(
                        vm = %vm,
                        step_id = %step.id,
                        op_kind = op_name,
                        "host-prep DAG step failed"
                    );
                    return Err(response);
                }
                continue;
            }
            HostPrepStepKind::ApplyNmUnmanaged => {
                // Compose ApplyNmUnmanaged against the single host-wide
                // intent row
                // (`nm-unmanaged:host`). The scope_id falls back to
                // the bundle_ref's env scope when the DAG carries
                // one; otherwise "host".
                let scope_id = step
                    .bundle_ref
                    .scope_id
                    .clone()
                    .unwrap_or_else(|| ScopeId::new("host"));
                let req = BrokerRequest::ApplyNmUnmanaged(BrokerApplyNmUnmanagedRequest {
                    bundle_nm_intent_ref: BundleOpId::new(intent_id_nm_unmanaged_host()),
                    scope_id,
                    destroy: false,
                    tracing_span_id: None,
                });
                if let Err(response) = dispatch_broker_ack_request(state, VERB, op_name, req) {
                    tracing::warn!(
                        vm = %vm,
                        step_id = %step.id,
                        op_kind = op_name,
                        "host-prep DAG step failed"
                    );
                    return Err(response);
                }
                continue;
            }
            HostPrepStepKind::ApplySysctl => {
                // Iterate the resolver's sysctl intent ids for this VM's env
                // and dispatch ApplySysctl per key. The bundle's per-iface entries (bridges + TAPs)
                // are keyed by `sysctl:env:<env>:if:<if>:<key>`; we
                // filter by the env-scoped prefix so a single
                // workload VM start doesn't apply sysctls for the
                // entire host. If the bundle has no env scope on
                // this step, we skip with a log (a host-prep DAG
                // never emits ApplySysctl for env-less VMs today).
                let env_scope = match step.bundle_ref.scope_id.clone() {
                    Some(scope) => scope,
                    None => {
                        tracing::warn!(
                            vm = %vm,
                            step_id = %step.id,
                            "ApplySysctl step has no env scope; skipping",
                        );
                        continue;
                    }
                };
                let env_prefix = format!(
                    "{}:if:",
                    env_scope.as_str().replacen("env:", "sysctl:env:", 1)
                );
                let resolver_for_sysctls = match load_bundle_resolver(state) {
                    Ok(r) => r,
                    Err(_) => {
                        tracing::warn!(
                            vm = %vm,
                            step_id = %step.id,
                            "ApplySysctl: bundle resolver unavailable; skipping",
                        );
                        continue;
                    }
                };
                let intent_ids: Vec<String> = resolver_for_sysctls
                    .sysctl_intent_ids()
                    .filter(|id| id.starts_with(env_prefix.as_str()))
                    .map(ToOwned::to_owned)
                    .collect();
                for intent_id in intent_ids {
                    let req = BrokerRequest::ApplySysctl(BrokerApplySysctlRequest {
                        bundle_sysctl_intent_ref: BundleOpId::new(intent_id.clone()),
                        scope_id: env_scope.clone(),
                        destroy: false,
                        tracing_span_id: None,
                    });
                    if let Err(response) = dispatch_broker_ack_request(state, VERB, op_name, req) {
                        tracing::warn!(
                            vm = %vm,
                            step_id = %step.id,
                            op_kind = op_name,
                            intent_id = %intent_id,
                            "host-prep DAG step failed"
                        );
                        return Err(response);
                    }
                }
                continue;
            }
            HostPrepStepKind::SetBridgePortFlags => {
                // Dispatch SetBridgePortFlags for the workload LAN port. The broker
                // returns a typed
                // BridgePortFlagsResponse, not an Ack; the host-prep
                // dispatcher accepts any non-Error response.
                let role_id = load_bundle_resolver(state)
                    .ok()
                    .and_then(|resolver| {
                        resolver.find_manifest_vm(vm).map(|manifest_vm| {
                            if manifest_vm.is_net_vm {
                                "net-vm-lan"
                            } else {
                                "workload-lan"
                            }
                        })
                    })
                    .unwrap_or("workload-lan");
                let role_id = RoleId::new(role_id);
                let req = BrokerRequest::SetBridgePortFlags(
                    nixling_ipc::broker_wire::SetBridgePortFlagsRequest {
                        vm_id: step.bundle_ref.vm_id.clone(),
                        role_id,
                        tracing_span_id: None,
                    },
                );
                if let Err(response) = dispatch_broker_host_prep_step(state, VERB, op_name, req) {
                    tracing::warn!(
                        vm = %vm,
                        step_id = %step.id,
                        op_kind = op_name,
                        "host-prep DAG step failed"
                    );
                    return Err(response);
                }
                continue;
            }
            // HostNetRoutePreflight is host-scope and is executed inline
            // by the daemon at startup (and via
            // `nixling host reconcile --network`).
            // It is not dispatched per-VM through this DAG; the arm
            // exists for exhaustiveness only.
            HostPrepStepKind::HostNetRoutePreflight => {
                tracing::info!(
                    vm = %vm,
                    step_id = %step.id,
                    kind = step.kind.as_str(),
                    "host-prep DAG step skipped (host-scope; executed by daemon startup + reconcile path)"
                );
                continue;
            }
        };
        if let Err(response) = dispatch_broker_ack_request(state, VERB, op_name, request) {
            tracing::warn!(
                vm = %vm,
                step_id = %step.id,
                op_kind = op_name,
                "host-prep DAG step failed"
            );
            return Err(response);
        }
    }
    Ok(())
}

fn dispatch_broker_vm_start(
    state: &ServerState,
    request: public_wire::VmLifecycleRequest,
) -> Result<Value, TypedError> {
    const VERB: &str = "vm start";

    if let Some(response) = mutating_verb_preflight(VERB, &request.flags, Some(request.vm.as_str()))
    {
        return Ok(response);
    }

    let resolver = load_bundle_resolver(state)?;

    // For net VMs (`sys-<env>-net`), refuse start if the on-disk
    // dnsmasq.conf hash diverges from
    // the bundle's nft/route/hosts intent hash for the same env.
    // This catches the case where the bundle was updated but the
    // dnsmasq render step (host singleton or systemd unit) did not
    // rerun. Workload VMs short-circuit with no I/O. Default
    // dnsmasq parent dir is `/var/lib/nixling/dnsmasq`; the
    // `NIXLING_DNSMASQ_DIR` env var overrides it for hermetic
    // tests. Runs BEFORE the host-prep DAG so the failure surfaces
    // early and no host mutations are attempted on a stale net VM.
    {
        let dnsmasq_dir = std::env::var_os("NIXLING_DNSMASQ_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(net_vm_bundle_gate::DEFAULT_DNSMASQ_DIR));
        match net_vm_bundle_gate::check_net_vm_bundle_gate(&resolver, &request.vm, &dnsmasq_dir) {
            net_vm_bundle_gate::BundleGateOutcome::NotANetVm
            | net_vm_bundle_gate::BundleGateOutcome::Ok => {}
            net_vm_bundle_gate::BundleGateOutcome::Drift(drift) => {
                let path = drift.path();
                let reason = drift.reason();
                // ConfigMissing is a SOFT-DEFER, not a hard fail. The
                // dnsmasq config render is owned by a v1.1.1 daemon
                // host-prep DAG op
                // (`RenderDnsmasqEnvConf{env}`) that has not landed
                // yet; until it does, a fresh-install net-VM start
                // can legitimately hit ConfigMissing on the first
                // run. Soft-deferring lets the VM come up; the
                // operator sees a stderr warning explaining the
                // gap. HashMismatch / ConfigReadFailed / EnvMissing
                // remain hard fails because they indicate a real
                // contract violation (bundle vs disk drift, or a
                // malformed env declaration).
                if matches!(
                    drift,
                    net_vm_bundle_gate::BundleGateDrift::ConfigMissing { .. }
                ) {
                    tracing::warn!(
                        vm = %request.vm,
                        env = drift.env(),
                        path = %path.display(),
                        "net VM start: dnsmasq.conf missing (soft-defer per v1.1-final; v1.1.1 RenderDnsmasqEnvConf host-prep op will render before first start)",
                    );
                    // Fall through to normal start path.
                } else {
                    tracing::warn!(
                        vm = %request.vm,
                        env = drift.env(),
                        path = %path.display(),
                        "net VM start refused: bundle/dnsmasq drift",
                    );
                    let (env, expected, actual) = match &drift {
                        net_vm_bundle_gate::BundleGateDrift::HashMismatch {
                            env,
                            expected,
                            actual,
                            ..
                        } => (env.clone(), expected.clone(), actual.clone()),
                        net_vm_bundle_gate::BundleGateDrift::ConfigMissing { env, .. }
                        | net_vm_bundle_gate::BundleGateDrift::ConfigReadFailed { env, .. } => {
                            (env.clone(), String::new(), String::new())
                        }
                        net_vm_bundle_gate::BundleGateDrift::EnvMissing { .. } => {
                            (String::new(), String::new(), String::new())
                        }
                    };
                    return Ok(TypedError::BundleDnsmasqDrift {
                        vm: request.vm.clone(),
                        env,
                        path,
                        expected,
                        actual,
                        reason,
                    }
                    .to_envelope_value());
                }
            }
        }
    }

    // Build the host-prep DAG for this VM and (optionally) execute it
    // before driving the per-VM process DAG. The DAG is logged
    // unconditionally so operators and gates can observe the planned step
    // set; actual broker dispatch is gated on
    // `NIXLING_HOST_PREP_DAG_EXECUTE=1`. All step kinds dispatch a real
    // broker op (or a daemon-native check) — `OwnershipMatrixCheck` and
    // `SshHostKeyPreflight` still cover the two stubs that intentionally
    // remain typed-Unimplemented at the broker layer pending sibling
    // handlers.
    let host_prep_steps =
        nixling_host::host_prep_dag::build_host_prep_dag(request.vm.as_str(), &resolver);
    log_host_prep_dag(&request.vm, &host_prep_steps);
    if std::env::var("NIXLING_HOST_PREP_DAG_EXECUTE")
        .map(|v| v == "1")
        .unwrap_or(false)
        && let Err(response) = execute_host_prep_dag(state, &request.vm, &host_prep_steps)
    {
        return Ok(response);
    }

    let runner = VmStartRunner {
        state,
        resolver: &resolver,
    };

    let dag = resolver
        .processes
        .vms
        .iter()
        .find(|dag| dag.vm == request.vm)
        .ok_or_else(|| TypedError::InternalIo {
            context: format!("load process DAG for {}", request.vm),
            detail: "VM not present in processes.json".to_owned(),
        })?;

    // StoreSync owns the guest-served live marker
    // (`store-view/live/.nixling-marker-<vm>`) and postures it as
    // `nixlingd:users 0644`. Run it before ownership preflight so stale
    // markers from older broker/activation paths are repaired before the
    // fail-closed matrix check. The process DAG still contains the
    // StoreVirtiofsPreflight readiness node; that second StoreSync call is
    // idempotent and keeps the DAG contract explicit.
    if vm_supports_store_sync(&resolver, &request.vm)
        && let Err(error) = runner.sync_store_view(&request.vm)
    {
        tracing::warn!(
            vm = %request.vm,
            error = %error,
            "vm start: pre-ownership StoreSync failed"
        );
        return Ok(daemon_failure_response(
            VERB,
            format!(
                "vm start {}: store-view sync failed before ownership preflight",
                request.vm
            ),
        ));
    }

    // Refuse VM start if any per-VM state subdirectory has drifted from
    // the typed ownership matrix
    // declared in nixos-modules/options-ownership-matrix.nix. Missing
    // subdirectories surface as warn-only (state is materialized
    // lazily); owner/group/mode drift on existing paths fails closed.
    if let Some(manifest_entry) = resolver.manifest.vms.get(&request.vm)
        && vm_requires_nixos_state_preflights(manifest_entry)
    {
        let state_dir = std::path::PathBuf::from(&manifest_entry.state_dir);
        if let ownership_preflight::OwnershipPreflightOutcome::Drift(drift) =
            ownership_preflight::preflight(&request.vm, &state_dir)
        {
            let path = drift[0].path().to_path_buf();
            let drift_reason = ownership_preflight::render_drift_message(&request.vm, &drift);
            tracing::warn!(
                vm = %request.vm,
                path = %path.display(),
                drift_count = drift.len(),
                "vm start refused: ownership-matrix drift",
            );
            return Ok(TypedError::OwnershipMatrixDrift {
                vm: request.vm.clone(),
                path,
                drift_reason,
            }
            .to_envelope_value());
        }

        // Refuse VM start if the per-VM sshd host keys directory or any
        // `ssh_host_*_key`
        // leaf has drifted from the canonical posture (regular file,
        // root:root, 0o0400; no symlinks). The directory's own
        // ownership/mode are enforced by the ownership-matrix
        // preflight above; this preflight is fail-closed once the
        // directory exists.
        let keys_dir = state_dir.join("sshd-host-keys");
        if keys_dir.exists() {
            if let Err(drift) = ssh_host_key_preflight::check_sshd_host_keys(&request.vm, &keys_dir)
            {
                let path = drift.path().to_path_buf();
                let drift_reason = drift.reason();
                tracing::warn!(
                    vm = %request.vm,
                    path = %path.display(),
                    "vm start refused: sshd-host-key drift",
                );
                return Ok(TypedError::SshdHostKeyDrift {
                    vm: request.vm.clone(),
                    path,
                    drift: drift_reason,
                }
                .to_envelope_value());
            }
        } else {
            tracing::warn!(
                vm = %request.vm,
                path = %keys_dir.display(),
                "ssh-host-key-preflight: keys directory absent; skipping (will be materialized on first run)",
            );
        }
    }

    let tracked_roles = dag
        .nodes
        .iter()
        .filter_map(|node| match vm_start_node_mode(&node.role) {
            VmStartNodeMode::LongLived(_) => Some(tracked_role_id(node)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let _vm_start_lock = acquire_vm_start_lock(state, &request.vm)?;
    // v1.1.1fu14 B3 + B7: prune entries whose backing process
    // has died (or whose PID has been reused) BEFORE checking
    // for existing registrations. This handles the
    // operator-observed pattern where a daemon crash leaves a
    // pidfd-table.json with entries pointing at PIDs that were
    // since killed; without pruning, the daemon refuses every
    // subsequent vm start with "already has a registered
    // supervisor pidfd" even though no process exists.
    match state.pidfd_table.prune_dead_entries() {
        Ok(n) if n > 0 => {
            tracing::info!(
                vm = %request.vm,
                dropped = n,
                "vm start: pruned stale pidfd-table entries before duplicate check",
            );
        }
        Ok(_) => {}
        Err(err) => {
            tracing::warn!(
                vm = %request.vm,
                error = ?err,
                "vm start: pidfd-table prune failed; proceeding with stale entries",
            );
        }
    }
    match cleanup_stale_qemu_media_dependencies_before_start(state, &request.vm, &tracked_roles) {
        Ok(n) if n > 0 => {
            tracing::info!(
                vm = %request.vm,
                stopped = n,
                "vm start: stopped stale qemu-media dependency runners before duplicate check",
            );
        }
        Ok(_) => {}
        Err(response) => return Ok(response),
    }
    if let Some(existing_role_id) = tracked_roles
        .iter()
        .find(|role_id| state.pidfd_table.contains(&request.vm, role_id))
    {
        let runner_role_id = tracked_roles
            .iter()
            .find(|role_id| {
                *role_id == VM_RUNNER_ROLE_ID || role_id.as_str() == RunnerRole::QemuMedia.as_str()
            })
            .map(String::as_str)
            .unwrap_or(VM_RUNNER_ROLE_ID);
        if let Some(response) =
            existing_vm_start_response_if_ready(state, &request.vm, runner_role_id)
        {
            return Ok(response);
        }
        return Ok(invalid_request_response(
            VERB,
            format!(
                "vm '{}' already has a registered supervisor pidfd ({})",
                request.vm, existing_role_id
            ),
        ));
    }

    let api_timeout = Duration::from_secs(
        std::env::var("NIXLING_API_TIMEOUT_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(supervisor::dag::DEFAULT_API_TIMEOUT_SECONDS),
    );
    let readiness_timeout = Duration::from_secs(
        std::env::var("NIXLING_READINESS_TIMEOUT_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(api_timeout.as_secs().max(300)),
    );
    let split_mode = if request.no_wait_api {
        supervisor::dag::SplitReadinessMode::NoWaitApi
    } else {
        supervisor::dag::SplitReadinessMode::Strict
    };
    let budget = supervisor::dag::NodeBudget {
        readiness: readiness_timeout,
        ..supervisor::dag::NodeBudget::default()
    };
    let dag_start = Instant::now();
    let report = match block_on_future(
        supervisor::dag::DagExecutor::with_budget(runner, budget).run_split(
            dag,
            split_mode,
            api_timeout,
        ),
    ) {
        Ok(report) => report,
        Err(error) => {
            tracing::warn!(vm = %request.vm, error = ?error, "vm start DAG validation failed");
            return Ok(daemon_failure_response(
                VERB,
                format!(
                    "vm start {}: daemon could not validate the process DAG",
                    request.vm
                ),
            ));
        }
    };
    log_vm_start_report(&report);
    // Persist the api-ready state for `nixling vm status`.
    if let Some(ref api_ready_state) = report.api_ready {
        let api_ready_value = serde_json::to_value(api_ready_state).unwrap_or_default();
        if let Err(err) = daemon_audit::write_vm_api_ready_state(
            &state.daemon_state_dir,
            &request.vm,
            api_ready_value,
        ) {
            tracing::warn!(
                vm = %request.vm,
                error = %err,
                "vm start: failed to persist api-ready state (non-fatal)",
            );
        }
    }
    if matches!(split_mode, supervisor::dag::SplitReadinessMode::Strict)
        && matches!(
            report.api_ready,
            Some(supervisor::dag::ApiReadyState::Timeout)
        )
    {
        tracing::warn!(
            vm = %request.vm,
            "vm start: api-ready timeout (api-ready phase did not converge within {} seconds)",
            api_timeout.as_secs()
        );
        // Emit audit-log entry on api-ready timeout.
        if let Err(err) =
            state
                .daemon_audit
                .write_event(&daemon_audit::DaemonEvent::ApiReadyTimeout {
                    vm: request.vm.clone(),
                    runner: VM_RUNNER_ROLE_ID.to_owned(),
                    elapsed_secs: api_timeout.as_secs(),
                    mode: "strict".to_owned(),
                })
        {
            tracing::warn!(
                vm = %request.vm,
                error = %err,
                "vm start: failed to write ApiReadyTimeout audit event (non-fatal)",
            );
        }
        if let Err(response) = rollback_failed_vm_start(state, &request.vm, &tracked_roles) {
            return Ok(response);
        }
        return Ok(api_ready_timeout_response(
            VERB,
            format!("vm.{}: process-alive: ok; api-ready: timeout", request.vm),
        ));
    }
    if report.overall_ok {
        if !request.no_wait_api {
            // When the VM that just came up is the observability VM AND
            // observability is
            // enabled in the trusted bundle, block on the
            // OtelHostBridge readiness gate before declaring success.
            // On timeout we fall back to degraded mode (the VM stays
            // up; the response carries a degraded-mode annotation).
            // Strict-mode operators can flip the env var to convert
            // the timeout into a typed `otel-host-bridge-readiness-timeout`
            // refusal envelope (exit code 65). See
            // `docs/reference/otel-host-bridge-readiness.md`.
            let obs_meta = &resolver.manifest.observability;
            if obs_meta.enabled && obs_meta.vm_name == request.vm {
                let cfg = otel_host_bridge_readiness::ReadinessWaitConfig::from_env();
                let source = otel_host_bridge_readiness::PidfdAndSocketProbeSource {
                    pidfd_table: &state.pidfd_table,
                    vm: request.vm.as_str(),
                    runner_role_id: nixling_ipc::broker_wire::RunnerRole::OtelHostBridge.as_str(),
                    vsock_host_socket: std::path::PathBuf::from(
                        obs_meta.obs_vsock_host_socket.as_str(),
                    ),
                    exit_marker: None,
                };
                let outcome = otel_host_bridge_readiness::await_otel_host_bridge_readiness(
                    request.vm.as_str(),
                    &source,
                    &cfg,
                    std::thread::sleep,
                    std::time::Instant::now(),
                );
                if let otel_host_bridge_readiness::ReadinessWaitOutcome::DegradedTimeout {
                    vm,
                    elapsed_ms,
                    reason,
                } = &outcome
                {
                    if cfg.strict {
                        return Ok(TypedError::OtelHostBridgeReadinessTimeout {
                            vm: vm.clone(),
                            elapsed_ms: *elapsed_ms,
                        }
                        .to_envelope_value());
                    }
                    tracing::warn!(
                        vm = %vm,
                        elapsed_ms,
                        reason = %reason,
                        "vm start succeeded in degraded-mode: otel-host-bridge readiness gate did not close",
                    );
                }
            }
            // Post-readiness trigger. Once the per-VM DAG reports
            // overall_ok the guest is up, so it is safe to pin the host
            // pubkey into `/var/lib/nixling/known_hosts.nixling` via the
            // broker for the retained SSH-compat path. Failures here are
            // warn-only — matching the legacy
            // `nixling-known-hosts-refresh@<vm>.service` behaviour, which
            // left the old pin in place rather than failing the VM start.
            let outcome = known_hosts_refresh::refresh_known_hosts(
                &request.vm,
                &resolver.manifest,
                &DaemonRotateKnownHostBroker { state },
            );
            match &outcome {
                known_hosts_refresh::RefreshOutcome::Skipped { vm, reason } => tracing::info!(
                    vm = %vm,
                    reason = reason.as_str(),
                    "known-hosts refresh skipped",
                ),
                known_hosts_refresh::RefreshOutcome::Rotated { vm, response } => tracing::info!(
                    vm = %vm,
                    static_ip = %response.static_ip,
                    known_hosts_path = %response.known_hosts_path,
                    rewrote = response.removed,
                    "known-hosts refresh applied",
                ),
                known_hosts_refresh::RefreshOutcome::Failed { vm, detail } => tracing::warn!(
                    vm = %vm,
                    detail = %detail,
                    "known-hosts refresh failed (non-fatal, retained prior pin)",
                ),
            }
        }
        let summary = if request.no_wait_api {
            format!("vm.{}: process-alive: ok; api-ready: pending", request.vm)
        } else {
            vm_start_success_summary(&report)
        };
        let mut response = applied_response(VERB, summary);
        if request.no_wait_api {
            response.as_object_mut().unwrap().insert(
                "apiReady".to_owned(),
                serde_json::Value::String("pending".to_owned()),
            );
        }
        return Ok(response);
    }
    // Detect the runner-exited / runner-reused fast-fail BEFORE rollback
    // tears the runner down, so we can peek the buffered broker exit
    // status and emit a bounded audit event + an actionable, swtpm-aware
    // failure envelope (broker-error exit contract, not exit 1).
    let runner_exit = detect_runner_exit_failure(&report, dag);
    let runner_exit_response = runner_exit.map(|(role_id, reason_kind)| {
        let exit_status = state
            .broker_reap_log
            .peek_for(&request.vm, &role_id)
            .map(|notif| notif.exit_status);
        let elapsed_ms = u64::try_from(dag_start.elapsed().as_millis()).unwrap_or(u64::MAX);
        emit_vm_start_runner_exited_audit(
            state,
            &request.vm,
            &role_id,
            reason_kind,
            exit_status.as_ref(),
            elapsed_ms,
        );
        vm_start_runner_exited_response(&request.vm, &role_id, reason_kind, exit_status.as_ref())
    });
    if let Err(response) = rollback_failed_vm_start(state, &request.vm, &tracked_roles) {
        return Ok(response);
    }
    if let Some(response) = runner_exit_response {
        return Ok(response);
    }
    Ok(vm_start_failure_response(&report))
}

/// Scan the DAG run report for the first node that fast-failed because
/// its spawned runner exited (or its PID was reused) before readiness.
/// Returns the runner's tracked `role_id` and the closed reason kind.
fn detect_runner_exit_failure(
    report: &supervisor::dag::DagRunReport,
    dag: &nixling_core::processes::VmProcessDag,
) -> Option<(String, daemon_audit::VmStartRunnerExitReason)> {
    for entry in &report.history {
        let supervisor::dag::NodeOutcome::Failed { reason } = &entry.outcome else {
            continue;
        };
        let (reason_kind, marker) = if reason.contains("runner-exited:") {
            (
                daemon_audit::VmStartRunnerExitReason::RunnerExited,
                "runner-exited:",
            )
        } else if reason.contains("runner-reused:") {
            (
                daemon_audit::VmStartRunnerExitReason::RunnerReused,
                "runner-reused:",
            )
        } else {
            continue;
        };
        let node_id = reason.rsplit(marker).next().unwrap_or("").trim();
        let role_id = dag
            .nodes
            .iter()
            .find(|node| node.id.0 == node_id)
            .map(tracked_role_id)
            .unwrap_or_else(|| node_id.to_owned());
        return Some((role_id, reason_kind));
    }
    None
}

/// Bounded, closed-vocabulary cause phrase for a runner exit. Carries no
/// path, pid, or free-form node-reason text.
fn bounded_runner_exit_cause(
    reason_kind: daemon_audit::VmStartRunnerExitReason,
    status: Option<&nixling_ipc::broker_wire::ChildExitStatus>,
) -> String {
    use nixling_ipc::broker_wire::ChildExitKind;
    if matches!(
        reason_kind,
        daemon_audit::VmStartRunnerExitReason::RunnerReused
    ) {
        return "PID reused by another process".to_owned();
    }
    match status {
        Some(status) => match status.kind {
            ChildExitKind::Exited => match status.code {
                Some(code) => format!("exit code {code}"),
                None => "exited".to_owned(),
            },
            ChildExitKind::Signaled => match status.signal {
                Some(signal) => format!("terminated by signal {signal}"),
                None => "terminated by signal".to_owned(),
            },
            ChildExitKind::Killed => match status.signal {
                Some(signal) => format!("killed by signal {signal}"),
                None => "killed".to_owned(),
            },
        },
        None => "exited before readiness".to_owned(),
    }
}

/// Build the actionable, swtpm-aware failure envelope for a runner-exited
/// fast-fail. Maps to the broker-error outcome (the broker-error exit
/// contract), NOT exit 1.
fn vm_start_runner_exited_response(
    vm: &str,
    role_id: &str,
    reason_kind: daemon_audit::VmStartRunnerExitReason,
    status: Option<&nixling_ipc::broker_wire::ChildExitStatus>,
) -> Value {
    let cause = bounded_runner_exit_cause(reason_kind, status);
    let verb_word = match reason_kind {
        daemon_audit::VmStartRunnerExitReason::RunnerExited => "exited",
        daemon_audit::VmStartRunnerExitReason::RunnerReused => "was replaced (PID reused)",
    };
    let summary =
        format!("vm start {vm}: runner '{role_id}' {verb_word} before readiness ({cause})");
    let remediation = format!(
        "The '{role_id}' runner {verb_word} before its readiness signal fired ({cause}). \
         If this is the swtpm (per-VM TPM) runner, the TPM state must not be wiped: \
         clearing /var/lib/nixling/vms/{vm}/swtpm looks like device tampering to your \
         identity provider and forces re-enrollment. Admin: inspect \
         `journalctl -u nixlingd` and `journalctl -u nixling-priv-broker` for the \
         swtpm exit detail before retrying `nixling vm up {vm}`."
    );
    broker_failure_response("vm start", summary, remediation, None)
}

/// Emit the bounded `VmStartRunnerExited` daemon audit event. Best-effort:
/// an audit write failure is logged but never aborts the vm-start path.
fn emit_vm_start_runner_exited_audit(
    state: &ServerState,
    vm: &str,
    role_id: &str,
    reason_kind: daemon_audit::VmStartRunnerExitReason,
    status: Option<&nixling_ipc::broker_wire::ChildExitStatus>,
    elapsed_ms: u64,
) {
    use nixling_ipc::broker_wire::ChildExitKind;
    let exit_kind = status.map(|status| match status.kind {
        ChildExitKind::Exited => daemon_audit::RunnerExitKind::Exited,
        ChildExitKind::Signaled => daemon_audit::RunnerExitKind::Signaled,
        ChildExitKind::Killed => daemon_audit::RunnerExitKind::Killed,
    });
    let event = daemon_audit::DaemonEvent::VmStartRunnerExited {
        vm: vm.to_owned(),
        role_id: role_id.to_owned(),
        reason_kind,
        exit_kind,
        exit_code: status.and_then(|status| status.code),
        exit_signal: status.and_then(|status| status.signal),
        elapsed_ms,
    };
    if let Err(error) = state.daemon_audit.write_event(&event) {
        tracing::warn!(
            vm = %vm,
            role_id = %role_id,
            error = %error,
            "vm start: failed to write VmStartRunnerExited audit event (non-fatal)",
        );
    }
}

/// Production implementation of
/// [`known_hosts_refresh::RotateKnownHostBroker`] used by the
/// post-readiness hook in `dispatch_broker_vm_start`. Tests use a
/// fake recorder (see the module's `#[cfg(test)]` block).
struct DaemonRotateKnownHostBroker<'a> {
    state: &'a ServerState,
}

impl known_hosts_refresh::RotateKnownHostBroker for DaemonRotateKnownHostBroker<'_> {
    fn rotate(
        &self,
        request: nixling_ipc::broker_wire::RunRotateKnownHostRequest,
    ) -> Result<nixling_ipc::broker_wire::RunRotateKnownHostResponse, TypedError> {
        match dispatch_broker_request(self.state, BrokerRequest::RunRotateKnownHost(request))? {
            BrokerResponse::RunRotateKnownHost(response) => Ok(response),
            BrokerResponse::Error(error) => Err(TypedError::InternalBrokerUnavailable {
                path: broker_socket_path(self.state),
                detail: format!(
                    "RunRotateKnownHost broker error: kind={} message={}",
                    error.kind, error.message
                ),
            }),
            other => Err(TypedError::InternalBrokerUnavailable {
                path: broker_socket_path(self.state),
                detail: format!(
                    "RunRotateKnownHost: unexpected broker response kind {}",
                    broker_response_kind(&other)
                ),
            }),
        }
    }
}

#[cfg(test)]
fn dispatch_broker_vm_stop(
    state: &ServerState,
    request: public_wire::VmLifecycleRequest,
) -> Result<Value, TypedError> {
    dispatch_broker_vm_stop_as(state, request, BrokerCallerRole::LauncherUid { uid: 0 })
}

fn dispatch_broker_vm_stop_as(
    state: &ServerState,
    request: public_wire::VmLifecycleRequest,
    caller_role: BrokerCallerRole,
) -> Result<Value, TypedError> {
    dispatch_broker_vm_stop_with_timeout_as(
        state,
        request,
        caller_role,
        VM_STOP_TIMEOUT,
        VM_STOP_TIMEOUT,
    )
}

#[cfg(test)]
fn dispatch_broker_vm_stop_with_timeout(
    state: &ServerState,
    request: public_wire::VmLifecycleRequest,
    term_timeout: Duration,
    kill_timeout: Duration,
) -> Result<Value, TypedError> {
    dispatch_broker_vm_stop_with_timeout_as(
        state,
        request,
        BrokerCallerRole::LauncherUid { uid: 0 },
        term_timeout,
        kill_timeout,
    )
}

fn dispatch_broker_vm_stop_with_timeout_as(
    state: &ServerState,
    request: public_wire::VmLifecycleRequest,
    caller_role: BrokerCallerRole,
    term_timeout: Duration,
    kill_timeout: Duration,
) -> Result<Value, TypedError> {
    const VERB: &str = "vm stop";

    if let Some(response) = mutating_verb_preflight(VERB, &request.flags, Some(request.vm.as_str()))
    {
        return Ok(response);
    }

    let stop_entries = ordered_vm_stop_entries(state, &request.vm);
    if stop_entries.is_empty() {
        return Ok(invalid_request_response(
            VERB,
            format!("vm '{}' has no registered pidfd_table entries", request.vm),
        ));
    }

    let mut drained_roles = Vec::with_capacity(stop_entries.len());
    let mut sigkill_roles = Vec::new();
    for entry in &stop_entries {
        let report = match stop_vm_pidfd_role(
            state,
            caller_role.clone(),
            VERB,
            &request.vm,
            &entry.role,
            term_timeout,
            kill_timeout,
        ) {
            Ok(report) => report,
            Err(response) => return Ok(response),
        };
        drained_roles.push(report.role_id.clone());
        if report.required_sigkill {
            sigkill_roles.push(report.role_id);
        }
    }

    let _mguard = state.pidfd_table.mutation_guard();
    if let Err(error) = state.pidfd_table.snapshot() {
        tracing::warn!(vm = %request.vm, error = ?error, "pidfd_table snapshot failed after draining sidecars");
        return Ok(daemon_failure_response(
            VERB,
            format!(
                "vm stop {}: drained roles but pidfd_table persistence failed ({})",
                request.vm,
                drained_roles.join(", ")
            ),
        ));
    }

    let entry_word = if drained_roles.len() == 1 {
        "entry"
    } else {
        "entries"
    };
    let mut summary = format!(
        "vm stop {}: drained {} pidfd_table {} in reverse DAG order",
        request.vm,
        drained_roles.len(),
        entry_word
    );
    if !sigkill_roles.is_empty() {
        summary.push_str(&format!(
            " after SIGTERM timeout on {}",
            sigkill_roles.join(", ")
        ));
    }
    summary.push_str(&format!(" ({})", drained_roles.join(", ")));
    Ok(applied_response(VERB, summary))
}

#[cfg(test)]
fn dispatch_broker_vm_restart(
    state: &ServerState,
    request: public_wire::VmLifecycleRequest,
) -> Result<Value, TypedError> {
    dispatch_broker_vm_restart_as(state, request, BrokerCallerRole::LauncherUid { uid: 0 })
}

fn dispatch_broker_vm_restart_as(
    state: &ServerState,
    request: public_wire::VmLifecycleRequest,
    caller_role: BrokerCallerRole,
) -> Result<Value, TypedError> {
    const VERB: &str = "vm restart";

    if let Some(response) = mutating_verb_preflight(VERB, &request.flags, Some(request.vm.as_str()))
    {
        return Ok(response);
    }

    let stop_response = dispatch_broker_vm_stop_as(state, request.clone(), caller_role)?;
    if response_outcome(&stop_response) != Some("applied") {
        return Ok(retarget_mutating_response(&stop_response, VERB));
    }
    let start_response = dispatch_broker_vm_start(state, request.clone())?;
    if response_outcome(&start_response) != Some("applied") {
        return Ok(retarget_mutating_response(&start_response, VERB));
    }
    Ok(applied_response(
        VERB,
        format!(
            "vm restart {}: {}; {}",
            request.vm,
            response_summary(&stop_response).unwrap_or("stop applied"),
            response_summary(&start_response).unwrap_or("start applied"),
        ),
    ))
}

fn dispatch_broker_host_prepare(
    state: &ServerState,
    request: public_wire::HostPrepareRequest,
) -> Result<Value, TypedError> {
    const VERB: &str = "host prepare";

    if let Some(response) = mutating_verb_preflight(VERB, &request.flags, None) {
        return Ok(response);
    }

    let host = load_host_artifact(state)?;
    if let Err(response) = dispatch_broker_ack_request(
        state,
        VERB,
        "ApplyNftables",
        BrokerRequest::ApplyNftables(BrokerApplyNftablesRequest {
            bundle_nft_intent_ref: BundleOpId::new(intent_id_nft_host()),
            scope_id: ScopeId::new("host"),
            desired_hash: None,
            destroy: false,
            tracing_span_id: None,
        }),
    ) {
        return Ok(response);
    }

    let mut route_ops = 0usize;
    let mut sysctl_ops = 0usize;
    for env in &host.environments {
        let scope_id = ScopeId::new(format!("env:{}", env.env));
        for (idx, _) in env.net_vm_forward_blocklist.iter().enumerate() {
            route_ops += 1;
            if let Err(response) = dispatch_broker_ack_request(
                state,
                VERB,
                "ApplyRoute",
                BrokerRequest::ApplyRoute(BrokerApplyRouteRequest {
                    bundle_route_intent_ref: BundleOpId::new(intent_id_route_env(&env.env, idx)),
                    scope_id: scope_id.clone(),
                    destroy: false,
                    tracing_span_id: None,
                }),
            ) {
                return Ok(response);
            }
        }
        for entry in &env.ipv6_sysctls {
            for key in ipv6_sysctl_short_keys(entry) {
                sysctl_ops += 1;
                if let Err(response) = dispatch_broker_ack_request(
                    state,
                    VERB,
                    "ApplySysctl",
                    BrokerRequest::ApplySysctl(BrokerApplySysctlRequest {
                        bundle_sysctl_intent_ref: BundleOpId::new(intent_id_sysctl(
                            &env.env,
                            entry.if_name.as_str(),
                            key,
                        )),
                        scope_id: scope_id.clone(),
                        destroy: false,
                        tracing_span_id: None,
                    }),
                ) {
                    return Ok(response);
                }
            }
        }
    }
    if let Err(response) = dispatch_broker_ack_request(
        state,
        VERB,
        "UpdateHostsFile",
        BrokerRequest::UpdateHostsFile(BrokerUpdateHostsFileRequest {
            bundle_hosts_intent_ref: BundleOpId::new(intent_id_hosts_host()),
            destroy: false,
            tracing_span_id: None,
        }),
    ) {
        return Ok(response);
    }
    if let Err(response) = dispatch_broker_ack_request(
        state,
        VERB,
        "ApplyNmUnmanaged",
        BrokerRequest::ApplyNmUnmanaged(BrokerApplyNmUnmanagedRequest {
            bundle_nm_intent_ref: BundleOpId::new(intent_id_nm_unmanaged_host()),
            scope_id: ScopeId::new("host"),
            destroy: false,
            tracing_span_id: None,
        }),
    ) {
        return Ok(response);
    }

    Ok(applied_response(
        VERB,
        format!(
            "host prepare: applied 1 nft + {route_ops} route + {sysctl_ops} sysctl + 1 hosts + 1 nm-unmanaged ops"
        ),
    ))
}

fn dispatch_broker_host_destroy(
    state: &ServerState,
    request: public_wire::HostDestroyRequest,
) -> Result<Value, TypedError> {
    const VERB: &str = "host destroy";

    if let Some(response) = mutating_verb_preflight(VERB, &request.flags, None) {
        return Ok(response);
    }

    let host = load_host_artifact(state)?;
    let mut route_ops = 0usize;
    let mut sysctl_ops = 0usize;

    if let Err(response) = dispatch_broker_ack_request(
        state,
        VERB,
        "ApplyNmUnmanaged",
        BrokerRequest::ApplyNmUnmanaged(BrokerApplyNmUnmanagedRequest {
            bundle_nm_intent_ref: BundleOpId::new(intent_id_nm_unmanaged_host()),
            scope_id: ScopeId::new("host"),
            destroy: true,
            tracing_span_id: None,
        }),
    ) {
        return Ok(response);
    }
    for env in &host.environments {
        let scope_id = ScopeId::new(format!("env:{}", env.env));
        for (idx, _) in env.net_vm_forward_blocklist.iter().enumerate() {
            route_ops += 1;
            if let Err(response) = dispatch_broker_ack_request(
                state,
                VERB,
                "ApplyRoute",
                BrokerRequest::ApplyRoute(BrokerApplyRouteRequest {
                    bundle_route_intent_ref: BundleOpId::new(intent_id_route_env(&env.env, idx)),
                    scope_id: scope_id.clone(),
                    destroy: true,
                    tracing_span_id: None,
                }),
            ) {
                return Ok(response);
            }
        }
    }
    for env in &host.environments {
        let scope_id = ScopeId::new(format!("env:{}", env.env));
        for entry in &env.ipv6_sysctls {
            for key in ipv6_sysctl_short_keys(entry) {
                sysctl_ops += 1;
                if let Err(response) = dispatch_broker_ack_request(
                    state,
                    VERB,
                    "ApplySysctl",
                    BrokerRequest::ApplySysctl(BrokerApplySysctlRequest {
                        bundle_sysctl_intent_ref: BundleOpId::new(intent_id_sysctl(
                            &env.env,
                            entry.if_name.as_str(),
                            key,
                        )),
                        scope_id: scope_id.clone(),
                        destroy: true,
                        tracing_span_id: None,
                    }),
                ) {
                    return Ok(response);
                }
            }
        }
    }
    if let Err(response) = dispatch_broker_ack_request(
        state,
        VERB,
        "UpdateHostsFile",
        BrokerRequest::UpdateHostsFile(BrokerUpdateHostsFileRequest {
            bundle_hosts_intent_ref: BundleOpId::new(intent_id_hosts_host()),
            destroy: true,
            tracing_span_id: None,
        }),
    ) {
        return Ok(response);
    }
    if let Err(response) = dispatch_broker_ack_request(
        state,
        VERB,
        "ApplyNftables",
        BrokerRequest::ApplyNftables(BrokerApplyNftablesRequest {
            bundle_nft_intent_ref: BundleOpId::new(intent_id_nft_host()),
            scope_id: ScopeId::new("host"),
            desired_hash: None,
            destroy: true,
            tracing_span_id: None,
        }),
    ) {
        return Ok(response);
    }

    Ok(applied_response(
        VERB,
        format!(
            "host destroy: applied 1 nm-unmanaged-remove + {route_ops} route-del + {sysctl_ops} sysctl-revert + 1 hosts-remove + 1 nft-flush ops"
        ),
    ))
}

/// SOLE mutating recovery verb after the daemon enters operator-only
/// mode. Re-applies
/// the network slice of `host prepare` (host-scope nftables +
/// per-env routes + per-env ipv6 sysctls) — explicitly NOT the
/// `/etc/hosts` mutation or NetworkManager unmanaged file: those
/// are scoped to full `host prepare`. On success the persistent
/// preflight history is reset so the next daemon startup begins
/// with a clean consecutive-failure counter.
fn dispatch_broker_host_reconcile(
    state: &ServerState,
    request: public_wire::HostReconcileRequest,
) -> Result<Value, TypedError> {
    const VERB: &str = "host reconcile";

    if let Some(response) = mutating_verb_preflight(VERB, &request.flags, None) {
        return Ok(response);
    }

    if !request.network {
        return Err(TypedError::WireUnknownField {
            detail: "hostReconcile: at least one scope flag must be set; today only --network is supported".to_owned(),
        });
    }

    let host = load_host_artifact(state)?;
    if let Err(response) = dispatch_broker_ack_request(
        state,
        VERB,
        "ApplyNftables",
        BrokerRequest::ApplyNftables(BrokerApplyNftablesRequest {
            bundle_nft_intent_ref: BundleOpId::new(intent_id_nft_host()),
            scope_id: ScopeId::new("host"),
            desired_hash: None,
            destroy: false,
            tracing_span_id: None,
        }),
    ) {
        return Ok(response);
    }

    let mut route_ops = 0usize;
    let mut sysctl_ops = 0usize;
    for env in &host.environments {
        let scope_id = ScopeId::new(format!("env:{}", env.env));
        for (idx, _) in env.net_vm_forward_blocklist.iter().enumerate() {
            route_ops += 1;
            if let Err(response) = dispatch_broker_ack_request(
                state,
                VERB,
                "ApplyRoute",
                BrokerRequest::ApplyRoute(BrokerApplyRouteRequest {
                    bundle_route_intent_ref: BundleOpId::new(intent_id_route_env(&env.env, idx)),
                    scope_id: scope_id.clone(),
                    destroy: false,
                    tracing_span_id: None,
                }),
            ) {
                return Ok(response);
            }
        }
        for entry in &env.ipv6_sysctls {
            for key in ipv6_sysctl_short_keys(entry) {
                sysctl_ops += 1;
                if let Err(response) = dispatch_broker_ack_request(
                    state,
                    VERB,
                    "ApplySysctl",
                    BrokerRequest::ApplySysctl(BrokerApplySysctlRequest {
                        bundle_sysctl_intent_ref: BundleOpId::new(intent_id_sysctl(
                            &env.env,
                            entry.if_name.as_str(),
                            key,
                        )),
                        scope_id: scope_id.clone(),
                        destroy: false,
                        tracing_span_id: None,
                    }),
                ) {
                    return Ok(response);
                }
            }
        }
    }

    let history = net_route_preflight::PreflightHistory::new(&state.daemon_state_dir);
    if let Err(err) = history.reset_after_reconcile() {
        tracing::warn!(
            path = %history.path().display(),
            error = %err,
            "host reconcile: failed to reset net-route preflight history (apply succeeded; counter will clear on next successful startup pass)",
        );
    }

    Ok(applied_response(
        VERB,
        format!(
            "host reconcile --network: applied 1 nft + {route_ops} route + {sysctl_ops} sysctl ops; net-route preflight counter reset"
        ),
    ))
}

fn dispatch_broker_run_host_install(
    state: &ServerState,
    request: public_wire::HostInstallRequest,
) -> Result<Value, TypedError> {
    use nixling_ipc::public_wire::{MutatingVerbOutcome, MutatingVerbResponse};

    const VERB: &str = "host install";
    const OP_NAME: &str = "RunHostInstall";

    if let Some(response) = mutating_verb_preflight(VERB, &request.flags, None) {
        return Ok(response);
    }

    match dispatch_broker_request(
        state,
        BrokerRequest::RunHostInstall(BrokerRunHostInstallRequest {
            bundle_installer_intent_ref: BundleOpId::new(intent_id_installer_host()),
            enable: request.enable,
            start: request.start,
            no_start: request.no_start,
            tracing_span_id: None,
        }),
    ) {
        Ok(BrokerResponse::RunHostInstall(response)) => {
            Ok(wire::mutating_verb_response(MutatingVerbResponse {
                verb: VERB.to_owned(),
                outcome: MutatingVerbOutcome::Applied,
                target_wave: None,
                summary: Some(format!(
                    "nixling host install --apply executed via the native daemon → broker path \
                     (installed={}, enabled={}, started={}, artifactsWritten={})",
                    response.installed,
                    response.enabled,
                    response.started,
                    response.artifacts_written.len(),
                )),
                remediation: None,
                api_ready: None,
            }))
        }
        Ok(BrokerResponse::Error(error)) => {
            tracing::warn!(
                broker_kind = %error.kind,
                broker_operation = %error.operation,
                broker_target_wave = error.target_wave.as_deref().unwrap_or("none"),
                broker_message = %error.message,
                broker_action = %error.action,
                "broker live op failed"
            );
            let (summary, remediation) = redact_broker_error_for_launcher(
                OP_NAME,
                error.target_wave.as_deref(),
                &error.kind,
            );
            Ok(broker_failure_response(
                VERB,
                summary,
                remediation,
                error.target_wave,
            ))
        }
        Ok(other) => {
            tracing::warn!(
                op_name = OP_NAME,
                broker_response_kind = %broker_response_kind(&other),
                "broker returned unexpected response kind"
            );
            let (summary, remediation) =
                redact_broker_error_for_launcher(OP_NAME, None, "Broker.Protocol");
            Ok(broker_failure_response(VERB, summary, remediation, None))
        }
        Err(error) => {
            tracing::warn!(op_name = OP_NAME, error = ?error, "broker dispatch failed");
            let (summary, remediation) = redact_broker_dispatch_failure_for_launcher(OP_NAME);
            Ok(broker_failure_response(VERB, summary, remediation, None))
        }
    }
}

fn dispatch_broker_run_migrate(
    state: &ServerState,
    request: public_wire::MigrateRequest,
) -> Result<Value, TypedError> {
    use nixling_ipc::public_wire::{MutatingVerbOutcome, MutatingVerbResponse};

    const VERB: &str = "migrate";
    const OP_NAME: &str = "RunMigrate";

    if let Some(response) = mutating_verb_preflight(VERB, &request.flags, None) {
        return Ok(response);
    }

    match dispatch_broker_request(
        state,
        BrokerRequest::RunMigrate(BrokerRunMigrateRequest {
            bundle_migrate_intent_ref: BundleOpId::new(intent_id_migrate_host()),
            tracing_span_id: None,
        }),
    ) {
        Ok(BrokerResponse::RunMigrate(response)) => {
            Ok(wire::mutating_verb_response(MutatingVerbResponse {
                verb: VERB.to_owned(),
                outcome: MutatingVerbOutcome::Applied,
                target_wave: None,
                summary: Some(format!(
                    "nixling migrate --apply executed via the native daemon → broker path \
                     (migratedVmCount={}, notes={})",
                    response.migrated_vm_count,
                    response.notes.len(),
                )),
                remediation: None,
                api_ready: None,
            }))
        }
        Ok(BrokerResponse::Error(error)) => {
            tracing::warn!(
                broker_kind = %error.kind,
                broker_operation = %error.operation,
                broker_target_wave = error.target_wave.as_deref().unwrap_or("none"),
                broker_message = %error.message,
                broker_action = %error.action,
                "broker live op failed"
            );
            let (summary, remediation) = redact_broker_error_for_launcher(
                OP_NAME,
                error.target_wave.as_deref(),
                &error.kind,
            );
            Ok(broker_failure_response(
                VERB,
                summary,
                remediation,
                error.target_wave,
            ))
        }
        Ok(other) => {
            tracing::warn!(
                op_name = OP_NAME,
                broker_response_kind = %broker_response_kind(&other),
                "broker returned unexpected response kind"
            );
            let (summary, remediation) =
                redact_broker_error_for_launcher(OP_NAME, None, "Broker.Protocol");
            Ok(broker_failure_response(VERB, summary, remediation, None))
        }
        Err(error) => {
            tracing::warn!(op_name = OP_NAME, error = ?error, "broker dispatch failed");
            let (summary, remediation) = redact_broker_dispatch_failure_for_launcher(OP_NAME);
            Ok(broker_failure_response(VERB, summary, remediation, None))
        }
    }
}

fn activation_mode_label(mode: BrokerActivationMode) -> &'static str {
    match mode {
        BrokerActivationMode::Switch => "switch",
        BrokerActivationMode::Boot => "boot",
        BrokerActivationMode::Test => "test",
        BrokerActivationMode::Rollback => "rollback",
    }
}

fn dispatch_broker_activation(
    state: &ServerState,
    request: public_wire::ActivationRequest,
    verb: &'static str,
    mode: BrokerActivationMode,
) -> Result<Value, TypedError> {
    const OP_NAME: &str = "RunActivation";

    if let Some(response) = mutating_verb_preflight(verb, &request.flags, Some(request.vm.as_str()))
    {
        return Ok(response);
    }

    match dispatch_broker_request(
        state,
        BrokerRequest::RunActivation(BrokerRunActivationRequest {
            bundle_activation_intent_ref: BundleOpId::new(intent_id_activation(&request.vm)),
            mode,
            vm: request.vm.clone(),
            tracing_span_id: None,
        }),
    ) {
        Ok(BrokerResponse::RunActivation(response)) => {
            let generation_suffix = response
                .generation_number
                .map(|generation| format!(", generationNumber={generation}"))
                .unwrap_or_default();
            Ok(applied_response(
                verb,
                format!(
                    "nixling {verb} --apply executed via the native daemon → broker path \
                     (vm={}, mode={}, summary={}{})",
                    response.vm,
                    activation_mode_label(response.mode),
                    response.summary,
                    generation_suffix,
                ),
            ))
        }
        Ok(BrokerResponse::Error(error)) => {
            tracing::warn!(
                broker_kind = %error.kind,
                broker_operation = %error.operation,
                broker_target_wave = error.target_wave.as_deref().unwrap_or("none"),
                broker_message = %error.message,
                broker_action = %error.action,
                "broker live op failed"
            );
            let (summary, remediation) = redact_broker_error_for_launcher(
                OP_NAME,
                error.target_wave.as_deref(),
                &error.kind,
            );
            Ok(broker_failure_response(
                verb,
                summary,
                remediation,
                error.target_wave,
            ))
        }
        Ok(other) => {
            tracing::warn!(
                op_name = OP_NAME,
                broker_response_kind = %broker_response_kind(&other),
                "broker returned unexpected response kind"
            );
            let (summary, remediation) =
                redact_broker_error_for_launcher(OP_NAME, None, "Broker.Protocol");
            Ok(broker_failure_response(verb, summary, remediation, None))
        }
        Err(error) => {
            tracing::warn!(op_name = OP_NAME, error = ?error, "broker dispatch failed");
            let (summary, remediation) = redact_broker_dispatch_failure_for_launcher(OP_NAME);
            Ok(broker_failure_response(verb, summary, remediation, None))
        }
    }
}

fn dispatch_broker_switch(
    state: &ServerState,
    request: public_wire::ActivationRequest,
) -> Result<Value, TypedError> {
    ensure_vm_runtime_capability(
        state,
        &request.vm,
        RuntimeCapabilityGate::ConfigSync,
        "switch",
    )?;
    dispatch_broker_activation(state, request, "switch", BrokerActivationMode::Switch)
}

fn dispatch_broker_boot(
    state: &ServerState,
    request: public_wire::ActivationRequest,
) -> Result<Value, TypedError> {
    ensure_vm_runtime_capability(
        state,
        &request.vm,
        RuntimeCapabilityGate::ConfigSync,
        "boot",
    )?;
    dispatch_broker_activation(state, request, "boot", BrokerActivationMode::Boot)
}

fn dispatch_broker_test(
    state: &ServerState,
    request: public_wire::ActivationRequest,
) -> Result<Value, TypedError> {
    ensure_vm_runtime_capability(
        state,
        &request.vm,
        RuntimeCapabilityGate::ConfigSync,
        "test",
    )?;
    dispatch_broker_activation(state, request, "test", BrokerActivationMode::Test)
}

fn dispatch_broker_rollback(
    state: &ServerState,
    request: public_wire::ActivationRequest,
) -> Result<Value, TypedError> {
    ensure_vm_runtime_capability(
        state,
        &request.vm,
        RuntimeCapabilityGate::ConfigSync,
        "rollback",
    )?;
    dispatch_broker_activation(state, request, "rollback", BrokerActivationMode::Rollback)
}

fn dispatch_broker_gc(
    state: &ServerState,
    request: public_wire::GcRequest,
) -> Result<Value, TypedError> {
    const VERB: &str = "gc";
    const OP_NAME: &str = "RunGc";

    if let Some(response) = mutating_verb_preflight(VERB, &request.flags, None) {
        return Ok(response);
    }

    match dispatch_broker_request(
        state,
        BrokerRequest::RunGc(BrokerRunGcRequest {
            bundle_gc_intent_ref: BundleOpId::new(intent_id_gc_host()),
            keep_generations: request.keep_generations,
            tracing_span_id: None,
        }),
    ) {
        Ok(BrokerResponse::RunGc(response)) => Ok(applied_response(
            VERB,
            format!(
                "nixling gc --apply executed via the native daemon → broker path \
                 (retainedStorePaths={}, keepGenerations={:?}, summary={})",
                response.retained_store_path_count, response.keep_generations, response.summary,
            ),
        )),
        Ok(BrokerResponse::Error(error)) => {
            tracing::warn!(
                broker_kind = %error.kind,
                broker_operation = %error.operation,
                broker_target_wave = error.target_wave.as_deref().unwrap_or("none"),
                broker_message = %error.message,
                broker_action = %error.action,
                "broker live op failed"
            );
            let (summary, remediation) = redact_broker_error_for_launcher(
                OP_NAME,
                error.target_wave.as_deref(),
                &error.kind,
            );
            Ok(broker_failure_response(
                VERB,
                summary,
                remediation,
                error.target_wave,
            ))
        }
        Ok(other) => {
            tracing::warn!(
                op_name = OP_NAME,
                broker_response_kind = %broker_response_kind(&other),
                "broker returned unexpected response kind"
            );
            let (summary, remediation) =
                redact_broker_error_for_launcher(OP_NAME, None, "Broker.Protocol");
            Ok(broker_failure_response(VERB, summary, remediation, None))
        }
        Err(error) => {
            tracing::warn!(op_name = OP_NAME, error = ?error, "broker dispatch failed");
            let (summary, remediation) = redact_broker_dispatch_failure_for_launcher(OP_NAME);
            Ok(broker_failure_response(VERB, summary, remediation, None))
        }
    }
}

fn dispatch_broker_store_verify(
    state: &ServerState,
    request: public_wire::StoreVerifyRequest,
) -> Result<Value, TypedError> {
    ensure_vm_runtime_capability(
        state,
        &request.vm,
        RuntimeCapabilityGate::StoreSync,
        "store verify",
    )?;
    let response = match dispatch_broker_request(
        state,
        BrokerRequest::StoreVerify(BrokerStoreVerifyRequest {
            vm_id: VmId::new(&request.vm),
            repair: request.repair,
            tracing_span_id: None,
        }),
    ) {
        Ok(BrokerResponse::StoreVerify(response)) => response,
        Ok(BrokerResponse::Error(error)) => nixling_ipc::broker_wire::StoreVerifyResponse {
            vm: request.vm,
            status: nixling_ipc::broker_wire::StoreVerifyStatus::Failed,
            checked: 0,
            drifted: 0,
            repaired: 0,
            unknown_reason: None,
            audit_ref: None,
            remediation: Some(format!(
                "inspect audit_ref and broker logs, then retry ({}: {})",
                error.kind, error.message
            )),
        },
        Ok(other) => nixling_ipc::broker_wire::StoreVerifyResponse {
            vm: request.vm,
            status: nixling_ipc::broker_wire::StoreVerifyStatus::Failed,
            checked: 0,
            drifted: 0,
            repaired: 0,
            unknown_reason: None,
            audit_ref: None,
            remediation: Some(format!(
                "inspect audit_ref and broker logs, then retry (unexpected broker response {})",
                broker_response_kind(&other)
            )),
        },
        Err(error) => nixling_ipc::broker_wire::StoreVerifyResponse {
            vm: request.vm,
            status: nixling_ipc::broker_wire::StoreVerifyStatus::Failed,
            checked: 0,
            drifted: 0,
            repaired: 0,
            unknown_reason: None,
            audit_ref: None,
            remediation: Some(format!(
                "inspect audit_ref and broker logs, then retry ({})",
                error.message()
            )),
        },
    };
    Ok(wire::store_verify_response(response))
}

fn dispatch_broker_keys_rotate(
    state: &ServerState,
    request: public_wire::KeysRotateRequest,
) -> Result<Value, TypedError> {
    const VERB: &str = "keys rotate";
    const OP_NAME: &str = "RunKeysRotate";

    ensure_vm_runtime_capability(state, &request.vm, RuntimeCapabilityGate::Keys, VERB)?;
    if let Some(response) = mutating_verb_preflight(VERB, &request.flags, Some(request.vm.as_str()))
    {
        return Ok(response);
    }

    match dispatch_broker_request(
        state,
        BrokerRequest::RunKeysRotate(BrokerRunKeysRotateRequest {
            bundle_keys_intent_ref: BundleOpId::new(intent_id_keys_rotate(&request.vm)),
            vm: request.vm.clone(),
            tracing_span_id: None,
        }),
    ) {
        Ok(BrokerResponse::RunKeysRotate(response)) => Ok(applied_response(
            VERB,
            format!(
                "nixling keys rotate --apply executed via the native daemon → broker path \
                 (vm={}, fingerprint={}, keyPath={})",
                response.vm, response.public_key_fingerprint, response.key_path,
            ),
        )),
        Ok(BrokerResponse::Error(error)) => {
            tracing::warn!(
                broker_kind = %error.kind,
                broker_operation = %error.operation,
                broker_target_wave = error.target_wave.as_deref().unwrap_or("none"),
                broker_message = %error.message,
                broker_action = %error.action,
                "broker live op failed"
            );
            let (summary, remediation) = redact_broker_error_for_launcher(
                OP_NAME,
                error.target_wave.as_deref(),
                &error.kind,
            );
            Ok(broker_failure_response(
                VERB,
                summary,
                remediation,
                error.target_wave,
            ))
        }
        Ok(other) => {
            tracing::warn!(
                op_name = OP_NAME,
                broker_response_kind = %broker_response_kind(&other),
                "broker returned unexpected response kind"
            );
            let (summary, remediation) =
                redact_broker_error_for_launcher(OP_NAME, None, "Broker.Protocol");
            Ok(broker_failure_response(VERB, summary, remediation, None))
        }
        Err(error) => {
            tracing::warn!(op_name = OP_NAME, error = ?error, "broker dispatch failed");
            let (summary, remediation) = redact_broker_dispatch_failure_for_launcher(OP_NAME);
            Ok(broker_failure_response(VERB, summary, remediation, None))
        }
    }
}

fn dispatch_broker_trust(
    state: &ServerState,
    request: public_wire::TrustRequest,
) -> Result<Value, TypedError> {
    const VERB: &str = "trust";
    const OP_NAME: &str = "RunHostKeyTrust";

    ensure_vm_runtime_capability(state, &request.vm, RuntimeCapabilityGate::Ssh, VERB)?;
    if let Some(response) = mutating_verb_preflight(VERB, &request.flags, Some(request.vm.as_str()))
    {
        return Ok(response);
    }

    match dispatch_broker_request(
        state,
        BrokerRequest::RunHostKeyTrust(BrokerRunHostKeyTrustRequest {
            bundle_trust_intent_ref: BundleOpId::new(intent_id_trust(&request.vm)),
            vm: request.vm.clone(),
            tracing_span_id: None,
        }),
    ) {
        Ok(BrokerResponse::RunHostKeyTrust(response)) => Ok(applied_response(
            VERB,
            format!(
                "nixling trust --apply executed via the native daemon → broker path \
                 (vm={}, staticIp={}, knownHostsPath={}, updated={})",
                response.vm, response.static_ip, response.known_hosts_path, response.updated,
            ),
        )),
        Ok(BrokerResponse::Error(error)) => {
            tracing::warn!(
                broker_kind = %error.kind,
                broker_operation = %error.operation,
                broker_target_wave = error.target_wave.as_deref().unwrap_or("none"),
                broker_message = %error.message,
                broker_action = %error.action,
                "broker live op failed"
            );
            let (summary, remediation) = redact_broker_error_for_launcher(
                OP_NAME,
                error.target_wave.as_deref(),
                &error.kind,
            );
            Ok(broker_failure_response(
                VERB,
                summary,
                remediation,
                error.target_wave,
            ))
        }
        Ok(other) => {
            tracing::warn!(
                op_name = OP_NAME,
                broker_response_kind = %broker_response_kind(&other),
                "broker returned unexpected response kind"
            );
            let (summary, remediation) =
                redact_broker_error_for_launcher(OP_NAME, None, "Broker.Protocol");
            Ok(broker_failure_response(VERB, summary, remediation, None))
        }
        Err(error) => {
            tracing::warn!(op_name = OP_NAME, error = ?error, "broker dispatch failed");
            let (summary, remediation) = redact_broker_dispatch_failure_for_launcher(OP_NAME);
            Ok(broker_failure_response(VERB, summary, remediation, None))
        }
    }
}

fn dispatch_broker_rotate_known_host(
    state: &ServerState,
    request: public_wire::RotateKnownHostRequest,
) -> Result<Value, TypedError> {
    const VERB: &str = "rotate-known-host";
    const OP_NAME: &str = "RunRotateKnownHost";

    ensure_vm_runtime_capability(state, &request.vm, RuntimeCapabilityGate::Ssh, VERB)?;
    if let Some(response) = mutating_verb_preflight(VERB, &request.flags, Some(request.vm.as_str()))
    {
        return Ok(response);
    }

    match dispatch_broker_request(
        state,
        BrokerRequest::RunRotateKnownHost(BrokerRunRotateKnownHostRequest {
            bundle_rotate_known_host_intent_ref: BundleOpId::new(intent_id_rotate_known_host(
                &request.vm,
            )),
            vm: request.vm.clone(),
            tracing_span_id: None,
        }),
    ) {
        Ok(BrokerResponse::RunRotateKnownHost(response)) => Ok(applied_response(
            VERB,
            format!(
                "nixling rotate-known-host --apply executed via the native daemon → broker path \
                 (vm={}, staticIp={}, knownHostsPath={}, removed={})",
                response.vm, response.static_ip, response.known_hosts_path, response.removed,
            ),
        )),
        Ok(BrokerResponse::Error(error)) => {
            tracing::warn!(
                broker_kind = %error.kind,
                broker_operation = %error.operation,
                broker_target_wave = error.target_wave.as_deref().unwrap_or("none"),
                broker_message = %error.message,
                broker_action = %error.action,
                "broker live op failed"
            );
            let (summary, remediation) = redact_broker_error_for_launcher(
                OP_NAME,
                error.target_wave.as_deref(),
                &error.kind,
            );
            Ok(broker_failure_response(
                VERB,
                summary,
                remediation,
                error.target_wave,
            ))
        }
        Ok(other) => {
            tracing::warn!(
                op_name = OP_NAME,
                broker_response_kind = %broker_response_kind(&other),
                "broker returned unexpected response kind"
            );
            let (summary, remediation) =
                redact_broker_error_for_launcher(OP_NAME, None, "Broker.Protocol");
            Ok(broker_failure_response(VERB, summary, remediation, None))
        }
        Err(error) => {
            tracing::warn!(op_name = OP_NAME, error = ?error, "broker dispatch failed");
            let (summary, remediation) = redact_broker_dispatch_failure_for_launcher(OP_NAME);
            Ok(broker_failure_response(VERB, summary, remediation, None))
        }
    }
}

fn dispatch_list(
    state: &ServerState,
    request: public_wire::ListRequest,
) -> Result<Value, TypedError> {
    let manifest = load_manifest(&state.config.artifacts.public_manifest_path)?;
    let host = load_json::<HostJson>(&state.config.artifacts.host_path).ok();
    let processes = load_json::<ProcessesJson>(&state.config.artifacts.processes_path)?;
    if host
        .as_ref()
        .and_then(|host| host.qemu_media.as_ref())
        .is_some()
    {
        let resolver = load_bundle_resolver(state)?;
        refresh_qemu_media_registry_index_if_needed(state, &resolver)?;
    }
    let vms = manifest
        .iter()
        .filter(|(name, _)| !name.starts_with('_'))
        .filter(|(name, _)| request.vm.as_ref().map(|vm| vm == *name).unwrap_or(true))
        .filter(|(_, value)| {
            request
                .env
                .as_ref()
                .map(|env| value.get("env").and_then(Value::as_str) == Some(env.as_str()))
                .unwrap_or(true)
        })
        .map(|(name, value)| {
            let process_vm = processes.vms.iter().find(|entry| entry.vm == *name);
            let lifecycle = public_vm_lifecycle(state, name, value, process_vm);
            let runtime_kind = public_runtime_kind(value);
            let services = public_service_states(state, name, value, process_vm);
            let service_capabilities = public_service_capabilities(&services);
            json!({
                "name": name,
                "vm": name,
                "env": value.get("env").cloned().unwrap_or(Value::Null),
                "staticIp": value.get("staticIp").cloned().unwrap_or(Value::Null),
                "isNetVm": value.get("isNetVm").cloned().unwrap_or(Value::Bool(false)),
                "sshUser": value.get("sshUser").cloned().unwrap_or(Value::Null),
                "graphics": value.get("graphics").cloned().unwrap_or(Value::Bool(false)),
                "tpm": value.get("tpm").cloned().unwrap_or(Value::Bool(false)),
                "usbip": value.get("usbipYubikey").cloned().unwrap_or(Value::Bool(false)),
                "lifecycle": lifecycle,
                "runtime": public_runtime_summary(&lifecycle, value),
                "autostart": public_autostart_posture(value),
                "runtimeCapabilities": public_runtime_capabilities(value),
                "serviceCapabilities": service_capabilities,
                "unsupportedCapabilities": public_unsupported_capabilities(value),
                "qemuMedia": public_qemu_media_status(
                    state,
                    name,
                    runtime_kind.as_deref(),
                    host.as_ref(),
                    process_vm,
                    &services,
                ),
                "services": services,
            })
        })
        .collect();
    Ok(wire::list_response(vms))
}

fn dispatch_status(
    state: &ServerState,
    request: public_wire::StatusRequest,
) -> Result<Value, TypedError> {
    let manifest = load_manifest(&state.config.artifacts.public_manifest_path)?;
    let host = load_json::<HostJson>(&state.config.artifacts.host_path).ok();
    let processes = load_json::<ProcessesJson>(&state.config.artifacts.processes_path)?;
    if host
        .as_ref()
        .and_then(|host| host.qemu_media.as_ref())
        .is_some()
    {
        let resolver = load_bundle_resolver(state)?;
        refresh_qemu_media_registry_index_if_needed(state, &resolver)?;
    }
    let requested_vm = request.vm.clone();

    let statuses = manifest
        .iter()
        .filter(|(name, _)| !name.starts_with('_'))
        .filter(|(name, _)| requested_vm.as_ref().map(|vm| vm == *name).unwrap_or(true))
        .map(|(name, manifest_entry)| {
            let process_vm = processes.vms.iter().find(|entry| entry.vm == *name);
            let lifecycle = public_vm_lifecycle(state, name, manifest_entry, process_vm);
            let runtime_kind = public_runtime_kind(manifest_entry);
            let services = public_service_states(state, name, manifest_entry, process_vm);
            let service_capabilities = public_service_capabilities(&services);
            json!({
                "vm": name,
                "name": name,
                "env": manifest_entry.get("env").cloned().unwrap_or(Value::Null),
                "staticIp": manifest_entry.get("staticIp").cloned().unwrap_or(Value::Null),
                "sshUser": manifest_entry.get("sshUser").cloned().unwrap_or(Value::Null),
                "graphics": manifest_entry.get("graphics").cloned().unwrap_or(Value::Bool(false)),
                "tpm": manifest_entry.get("tpm").cloned().unwrap_or(Value::Bool(false)),
                "usbip": manifest_entry.get("usbipYubikey").cloned().unwrap_or(Value::Bool(false)),
                "isNetVm": manifest_entry.get("isNetVm").cloned().unwrap_or(Value::Bool(false)),
                "lifecycle": lifecycle,
                "runtime": public_runtime_summary(&lifecycle, manifest_entry),
                "autostart": public_autostart_posture(manifest_entry),
                "runtimeCapabilities": public_runtime_capabilities(manifest_entry),
                "serviceCapabilities": service_capabilities,
                "unsupportedCapabilities": public_unsupported_capabilities(manifest_entry),
                "qemuMedia": public_qemu_media_status(
                    state,
                    name,
                    runtime_kind.as_deref(),
                    host.as_ref(),
                    process_vm,
                    &services,
                ),
                "services": services,
                "bridgeChecks": [],
            })
        })
        .collect::<Vec<_>>();

    Ok(wire::status_response(json!({ "entries": statuses })))
}

fn public_vm_lifecycle(
    state: &ServerState,
    vm: &str,
    manifest_entry: &Value,
    process_vm: Option<&nixling_core::processes::VmProcessDag>,
) -> Value {
    let live_roles = state
        .pidfd_table
        .list_for_vm(vm)
        .into_iter()
        .filter(|registration| {
            state
                .pidfd_table
                .still_alive_same_start_time(vm, &registration.role)
        })
        .map(|registration| registration.role)
        .collect::<Vec<_>>();

    let runner_role = public_vm_runner_role_id(process_vm, manifest_entry);
    let lifecycle_state = if live_roles.iter().any(|role| role == &runner_role) {
        "Running"
    } else if live_roles.is_empty() {
        "Stopped"
    } else {
        "Starting"
    };

    let running = lifecycle_state == "Running";

    json!({
        "pendingRestart": running && public_pending_restart(manifest_entry),
        "state": lifecycle_state,
    })
}

fn public_pending_restart(manifest_entry: &Value) -> bool {
    let Some(state_dir) = manifest_entry.get("stateDir").and_then(Value::as_str) else {
        return false;
    };
    let state_dir = Path::new(state_dir);
    let current = fs::read_link(state_dir.join("current")).ok();
    let booted = fs::read_link(state_dir.join("booted")).ok();
    matches!((current, booted), (Some(current), Some(booted)) if current != booted)
}

fn public_runtime_summary(lifecycle: &Value, manifest_entry: &Value) -> Value {
    let detail = lifecycle
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("Unknown")
        .to_ascii_lowercase();
    let mut runtime = serde_json::Map::new();
    runtime.insert("detail".to_owned(), Value::String(detail));
    if let Some(kind) = public_runtime_kind(manifest_entry) {
        runtime.insert("kind".to_owned(), Value::String(kind));
    }
    if let Some(operation_capabilities) = manifest_entry.pointer("/runtime/operationCapabilities") {
        runtime.insert(
            "operationCapabilities".to_owned(),
            operation_capabilities.clone(),
        );
    }
    if let Some(services) = manifest_entry.pointer("/runtime/services") {
        runtime.insert("services".to_owned(), services.clone());
    }
    Value::Object(runtime)
}

fn public_runtime_kind(manifest_entry: &Value) -> Option<String> {
    manifest_entry
        .pointer("/runtime/kind")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn public_is_qemu_media(manifest_entry: &Value) -> bool {
    public_runtime_kind(manifest_entry).as_deref() == Some("qemu-media")
}

fn public_autostart_posture(manifest_entry: &Value) -> Option<Value> {
    public_is_qemu_media(manifest_entry).then(|| {
        json!({
            "mode": "manual-only",
            "reason": "qemu-media VMs are intentionally skipped by daemon autostart; start them explicitly with `nixling vm start <vm> --apply`"
        })
    })
}

fn public_runtime_capabilities(manifest_entry: &Value) -> Vec<String> {
    let Some(capabilities) = manifest_entry
        .pointer("/runtime/capabilities")
        .and_then(Value::as_object)
    else {
        return if public_is_qemu_media(manifest_entry) {
            vec![
                "display".to_owned(),
                "lifecycle".to_owned(),
                "usb-hotplug".to_owned(),
            ]
        } else {
            Vec::new()
        };
    };
    let mut supported = capabilities
        .iter()
        .filter(|(_name, value)| value.as_bool() == Some(true))
        .map(|(name, _value)| capability_name_for_public_output(name))
        .collect::<Vec<_>>();
    supported.sort();
    supported.dedup();
    supported
}

fn public_unsupported_capabilities(manifest_entry: &Value) -> Vec<String> {
    let Some(capabilities) = manifest_entry
        .pointer("/runtime/capabilities")
        .and_then(Value::as_object)
    else {
        return if public_is_qemu_media(manifest_entry) {
            vec![
                "config-sync".to_owned(),
                "exec".to_owned(),
                "guest-control".to_owned(),
                "in-guest-observability".to_owned(),
                "keys".to_owned(),
                "s".to_owned() + "sh",
                "store-sync".to_owned(),
            ]
        } else {
            Vec::new()
        };
    };
    let mut unsupported = capabilities
        .iter()
        .filter(|(_name, value)| value.as_bool() == Some(false))
        .map(|(name, _value)| capability_name_for_public_output(name))
        .collect::<Vec<_>>();
    unsupported.sort();
    unsupported.dedup();
    unsupported
}

fn capability_name_for_public_output(name: &str) -> String {
    match name {
        "configSync" => "config-sync",
        "guestControl" => "guest-control",
        "inGuestObservability" => "in-guest-observability",
        "storeSync" => "store-sync",
        "usbHotplug" => "usb-hotplug",
        other => other,
    }
    .to_owned()
}

fn public_service_capabilities(services: &Value) -> Vec<String> {
    let Some(services) = services.as_object() else {
        return Vec::new();
    };
    let mut capabilities = services
        .iter()
        .filter_map(|(name, state)| {
            if state.is_null() || state.as_str() == Some("unsupported") {
                None
            } else {
                Some(service_capability_name_for_public_output(name))
            }
        })
        .collect::<Vec<_>>();
    capabilities.sort();
    capabilities.dedup();
    capabilities
}

fn service_capability_name_for_public_output(name: &str) -> String {
    match name {
        "qemuMedia" => "qemu-media",
        "snd" => "audio",
        other => other,
    }
    .to_owned()
}

fn public_vm_runner_role_id(
    process_vm: Option<&nixling_core::processes::VmProcessDag>,
    manifest_entry: &Value,
) -> String {
    if process_vm
        .map(|entry| {
            entry
                .nodes
                .iter()
                .any(|node| node.role == ProcessRole::QemuMediaRunner)
        })
        .unwrap_or(false)
        || public_is_qemu_media(manifest_entry)
    {
        RunnerRole::QemuMedia.as_str().to_owned()
    } else {
        VM_RUNNER_ROLE_ID.to_owned()
    }
}

fn public_qemu_media_status(
    state: &ServerState,
    vm: &str,
    runtime_kind: Option<&str>,
    host: Option<&HostJson>,
    process_vm: Option<&nixling_core::processes::VmProcessDag>,
    services: &Value,
) -> Option<Value> {
    if runtime_kind != Some("qemu-media") {
        return None;
    }
    let runner = process_vm.and_then(|entry| {
        entry
            .nodes
            .iter()
            .find(|node| node.role == ProcessRole::QemuMediaRunner)
    });
    let qmp_socket = runner.and_then(qemu_media_qmp_socket);
    let state_text = services
        .get("qemuMedia")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| public_pidfd_role_state(state, vm, RunnerRole::QemuMedia.as_str()));
    let qemu_media_host = host.and_then(|host| host.qemu_media.as_ref());
    let media = qemu_media_host
        .map(|contract| {
            contract
                .sources
                .iter()
                .filter(|source| source.vm == vm)
                .map(|source| qemu_media_source_status(contract.registry_dir.as_str(), source))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let qmp_readiness = qmp_socket.as_deref().map(|path| {
        if qemu_media_unix_socket_listening(path) {
            "ready".to_owned()
        } else if state_text == "running" {
            "pending".to_owned()
        } else {
            "not-started".to_owned()
        }
    });
    let pre_cont_progress = match qmp_readiness.as_deref() {
        Some("ready") if state_text == "running" => "paused-before-cont",
        Some("pending") if state_text == "running" => "waiting-for-qmp",
        _ => "not-started",
    };
    Some(json!({
        "firmwareMode": "none",
        "runner": {
            "state": state_text,
            "role": RunnerRole::QemuMedia.as_str(),
            "preContProgress": pre_cont_progress,
            "qmpReadiness": qmp_readiness,
        },
        "media": media,
    }))
}

fn qemu_media_qmp_socket(node: &ProcessNode) -> Option<String> {
    node.readiness.iter().find_map(|predicate| match predicate {
        ReadinessPredicate::UnixSocketListening(path)
        | ReadinessPredicate::UnixSocketExists(path) => Some(path.clone()),
        _ => None,
    })
}

fn qemu_media_unix_socket_listening(path: &str) -> bool {
    const SO_ACCEPTCON: &str = "00010000";
    let Ok(contents) = fs::read_to_string("/proc/net/unix") else {
        return false;
    };
    contents.lines().skip(1).any(|line| {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        fields.get(3).copied() == Some(SO_ACCEPTCON) && fields.last().copied() == Some(path)
    })
}

fn qemu_media_source_status(registry_dir: &str, source: &QemuMediaSourceIntent) -> Value {
    let (state, remediation) = qemu_media_registry_state(registry_dir, source);
    let status = json!({
        "mediaRef": source.media_ref,
        "slot": source.slot,
        "sourceKind": serde_kebab_string(&source.source_kind),
        "format": serde_kebab_string(&source.format),
        "readOnly": source.read_only,
        "registry": {
            "state": state,
            "remediation": remediation,
        },
    });
    status
}

fn serde_kebab_string<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned())
}

fn qemu_media_registry_state(
    _registry_dir: &str,
    source: &QemuMediaSourceIntent,
) -> (String, Option<String>) {
    if serde_kebab_string(&source.source_kind) != "physical-usb" {
        return ("direct-config".to_owned(), None);
    }
    let records = qemu_media_probe_registry_records();
    let Some(record) = records
        .iter()
        .find(|record| record.vm == source.vm && record.media_ref == source.media_ref)
    else {
        return (
            "missing".to_owned(),
            Some(format!(
                "declare the boot-drive physical USB source for vm `{}` in config, then run `nixling usb probe` to verify the runtime selector for `{}` before starting or attaching this media",
                source.vm, source.media_ref
            )),
        );
    };
    let expected_kind = serde_kebab_string(&source.source_kind);
    let expected_format = serde_kebab_string(&source.format);
    if record.vm == source.vm
        && record.media_ref == source.media_ref
        && record.source_kind == expected_kind
        && record.format == expected_format
        && record.read_only == source.read_only
    {
        ("present".to_owned(), None)
    } else {
        (
            "stale".to_owned(),
            Some(
                "registry entry does not match the current declaration; update qemu-media config if needed, then run `nixling usb probe`"
                    .to_owned(),
            ),
        )
    }
}

fn public_service_states(
    state: &ServerState,
    vm: &str,
    manifest_entry: &Value,
    process_vm: Option<&nixling_core::processes::VmProcessDag>,
) -> Value {
    let has_role = |role: ProcessRole| {
        process_vm
            .map(|entry| entry.nodes.iter().any(|node| node.role == role))
            .unwrap_or(false)
    };
    let gpu_role_id = if has_role(ProcessRole::GpuRenderNode) {
        Some("gpu-render-node")
    } else if has_role(ProcessRole::Gpu)
        || manifest_entry
            .get("graphics")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        Some("gpu")
    } else {
        None
    };

    json!({
        "nixling": "active",
        "microvm": if public_is_qemu_media(manifest_entry) {
            "unsupported".to_owned()
        } else {
            public_pidfd_role_state(state, vm, VM_RUNNER_ROLE_ID)
        },
        "qemuMedia": public_is_qemu_media(manifest_entry)
            .then(|| public_pidfd_role_state(state, vm, RunnerRole::QemuMedia.as_str())),
        "virtiofsd": if public_is_qemu_media(manifest_entry) {
            "unsupported".to_owned()
        } else {
            public_pidfd_role_prefix_state(state, vm, "virtiofsd")
        },
        "gpu": gpu_role_id.map(|role| public_pidfd_role_state(state, vm, role)),
        "video": has_role(ProcessRole::Video).then(|| public_pidfd_role_state(state, vm, "video")),
        "snd": (has_role(ProcessRole::Audio)
            || manifest_entry.get("audio").and_then(Value::as_bool).unwrap_or(false))
            .then(|| public_pidfd_role_state(state, vm, "audio")),
        "swtpm": (has_role(ProcessRole::Swtpm)
            || manifest_entry.get("tpm").and_then(Value::as_bool).unwrap_or(false))
            .then(|| public_pidfd_role_state(state, vm, "swtpm")),
    })
}

fn public_pidfd_role_state(state: &ServerState, vm: &str, role: &str) -> String {
    public_pidfd_role_state_matching(state, vm, |candidate| candidate == role)
}

fn public_pidfd_role_prefix_state(state: &ServerState, vm: &str, prefix: &str) -> String {
    public_pidfd_role_state_matching(state, vm, |candidate| candidate.starts_with(prefix))
}

fn public_pidfd_role_state_matching<F>(state: &ServerState, vm: &str, role_matches: F) -> String
where
    F: Fn(&str) -> bool,
{
    let running = state
        .pidfd_table
        .list_for_vm(vm)
        .into_iter()
        .any(|registration| {
            role_matches(&registration.role)
                && state
                    .pidfd_table
                    .still_alive_same_start_time(vm, &registration.role)
        });
    if running { "running" } else { "stopped" }.to_owned()
}

#[cfg(test)]
mod public_status_tests {
    use super::supervisor::pidfd_table::{BrokerReapLog, PidfdEntry, PidfdTable};
    use super::supervisor::state::parse_proc_stat_starttime;
    use super::*;
    use std::os::fd::OwnedFd;

    fn test_state() -> (ServerState, tempfile::TempDir) {
        test_state_with_config(DaemonConfig::default())
    }

    fn test_state_with_config(config: DaemonConfig) -> (ServerState, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp daemon state");
        let broker_reap_log = BrokerReapLog::new();
        let state = ServerState {
            config,
            daemon_uid: 0,
            daemon_audit: Arc::new(daemon_audit::DaemonAuditLog::no_op()),
            daemon_state_dir: dir.path().to_path_buf(),
            pidfd_table: Arc::new(
                PidfdTable::new(dir.path().join("pidfd-table.json"))
                    .with_broker_reap_log(Arc::clone(&broker_reap_log)),
            ),
            broker_reap_log,
            metrics_registry: Arc::new(metrics::Registry::new()),
            exec_sessions: Arc::new(exec_session::SessionTable::new(
                exec_session::ExecSessionCaps::default(),
            )),
            gateway_display: crate::new_gateway_display_runtime_for_tests(),
            conn_semaphore: crate::concurrency::ConnSemaphore::new(8),
            op_locks: crate::concurrency::OpLockManager::new(),
        };
        (state, dir)
    }

    fn write_public_status_artifacts(root: &Path) -> ArtifactPaths {
        write_public_status_artifacts_with_state_dir(root, None)
    }

    fn admin_peer() -> PeerIdentity {
        PeerIdentity {
            role: PeerRole::Admin,
            uid: 1000,
        }
    }

    fn launcher_peer() -> PeerIdentity {
        PeerIdentity {
            role: PeerRole::Launcher,
            uid: 1001,
        }
    }

    fn gateway_config_for_waypipe(
        path: &Path,
        allow_host_relay_credentials: bool,
    ) -> GatewayFileConfig {
        GatewayFileConfig {
            gateway: "gateway".to_owned(),
            realm: "demo".to_owned(),
            state_dir: None,
            credential_path: Some(PathBuf::from("/run/nixling/test-gateway-credential.json")),
            seal_key_path: Some(PathBuf::from("/run/nixling/test-gateway-seal.key")),
            allow_host_relay_credentials,
            relay: GatewayRelayFileConfig {
                namespace: Some("relay.example.invalid".to_owned()),
                entity: Some("gateway".to_owned()),
            },
            aca: GatewayAcaFileConfig::default(),
            display: GatewayDisplayFileConfig {
                vsock_port: None,
                waypipe_compression: None,
                waypipe_socket: Some(path.display().to_string()),
            },
        }
    }

    fn bind_test_waypipe_socket(root: &Path, name: &str, mode: u32) -> PathBuf {
        let socket_path = root.join(name);
        let _listener =
            std::os::unix::net::UnixListener::bind(&socket_path).expect("bind test socket");
        fs::set_permissions(&socket_path, fs::Permissions::from_mode(mode))
            .expect("set socket mode");
        socket_path
    }

    fn gateway_unavailable_detail(error: &TypedError) -> &str {
        match error {
            TypedError::GatewayDisplayUnavailable { detail } => detail,
            other => panic!("expected GatewayDisplayUnavailable, got {other:?}"),
        }
    }

    #[test]
    fn gateway_host_relay_guard_accepts_default_guest_owned_store() {
        let dir = tempfile::tempdir().expect("gateway guard dir");
        let socket_path = bind_test_waypipe_socket(dir.path(), "waypipe.sock", 0o600);
        let config = gateway_config_for_waypipe(&socket_path, false);
        validate_gateway_host_relay_transition_guard(&config)
            .expect("default guest-owned credential store is accepted");
    }

    #[test]
    fn gateway_host_relay_guard_rejects_retired_escape_hatch() {
        let dir = tempfile::tempdir().expect("gateway guard dir");
        let socket_path = bind_test_waypipe_socket(dir.path(), "waypipe.sock", 0o600);
        let config = gateway_config_for_waypipe(&socket_path, true);
        let preflight = gateway_display_preflight_from_config(&config)
            .expect("display preflight stores config without touching the user-session socket");
        assert_eq!(preflight.waypipe_socket_path.as_ref(), Some(&socket_path));
        assert!(preflight.allow_host_relay_credentials);
        let err = validate_gateway_host_relay_transition_guard(&config)
            .expect_err("retired escape hatch is rejected");
        assert!(gateway_unavailable_detail(&err).contains("retired"));
        assert!(gateway_unavailable_detail(&err).contains("enroll inside gateway then retry"));
    }

    #[test]
    fn waypipe_receiver_socket_validation_rejects_symlink() {
        let dir = tempfile::tempdir().expect("waypipe symlink dir");
        let socket_path = bind_test_waypipe_socket(dir.path(), "waypipe.sock", 0o600);
        let link_path = dir.path().join("waypipe-link.sock");
        std::os::unix::fs::symlink(&socket_path, &link_path).expect("symlink test socket");
        let err = validate_waypipe_receiver_socket(&link_path)
            .expect_err("waypipe socket validation must not follow symlinks");
        assert!(gateway_unavailable_detail(&err).contains("symlink"));
    }

    #[test]
    fn waypipe_receiver_socket_validation_requires_0600() {
        let dir = tempfile::tempdir().expect("waypipe mode dir");
        let socket_path = bind_test_waypipe_socket(dir.path(), "waypipe.sock", 0o660);
        let err = validate_waypipe_receiver_socket(&socket_path)
            .expect_err("waypipe socket must be private to the owner");
        assert!(gateway_unavailable_detail(&err).contains("mode 0600"));
    }

    #[test]
    fn gateway_display_open_refuses_retired_host_relay_before_orchestrator() {
        let (mut state, _dir) = test_state();
        let dir = tempfile::tempdir().expect("waypipe dispatch dir");
        let socket_path = bind_test_waypipe_socket(dir.path(), "waypipe.sock", 0o600);
        state.gateway_display = Arc::new(GatewayDisplayRuntime {
            orchestrator: GatewayOrchestrator::new(
                daemon_gateway_deps(),
                1,
                LedgerLimits::default(),
            ),
            sessions: Mutex::new(HashMap::new()),
            lifecycle: Box::new(DaemonGatewayLifecycle),
            preflight: Some(GatewayDisplayPreflight {
                allow_host_relay_credentials: true,
                waypipe_socket_path: Some(socket_path),
            }),
        });

        let err = dispatch_request(
            &state,
            &admin_peer(),
            wire::Request::GatewayDisplay(public_wire::GatewayDisplayOp::Open(
                public_wire::GatewayDisplayOpenArgs {
                    target: "nl://demo.gw.work.nixling".to_owned(),
                    operation_id: "gw-exec-invalid-waypipe".to_owned(),
                    principal: "uid-1000".to_owned(),
                    app_argv: vec!["foot".to_owned()],
                    request_hash: 42,
                },
            )),
        )
        .expect_err("retired host relay credential path blocks gateway open");
        assert!(gateway_unavailable_detail(&err).contains("retired"));
        assert!(gateway_unavailable_detail(&err).contains("enroll inside gateway then retry"));
        assert!(state.gateway_display.sessions.lock().unwrap().is_empty());
    }

    #[test]
    fn gateway_display_open_dispatches_to_orchestrator() {
        let (state, _dir) = test_state();
        let value = dispatch_request(
            &state,
            &admin_peer(),
            wire::Request::GatewayDisplay(public_wire::GatewayDisplayOp::Open(
                public_wire::GatewayDisplayOpenArgs {
                    target: "nl://demo.gw.work.nixling".to_owned(),
                    operation_id: "gw-exec-1".to_owned(),
                    principal: "uid-1000".to_owned(),
                    app_argv: vec!["foot".to_owned()],
                    request_hash: 42,
                },
            )),
        )
        .expect("gateway display open dispatches");
        assert_eq!(
            value.get("type").and_then(Value::as_str),
            Some("gatewayDisplayResponse")
        );
        assert_eq!(value.get("op").and_then(Value::as_str), Some("open"));
        assert_eq!(
            value
                .get("result")
                .and_then(|r| r.get("state"))
                .and_then(Value::as_str),
            Some("running")
        );
        let session_id = value
            .get("result")
            .and_then(|r| r.get("sessionId"))
            .and_then(Value::as_str)
            .expect("open response has session id")
            .to_owned();

        let list = dispatch_request(
            &state,
            &admin_peer(),
            wire::Request::GatewayDisplay(public_wire::GatewayDisplayOp::ListDetailed(
                public_wire::GatewayDisplayListArgs { target: None },
            )),
        )
        .expect("gateway display list dispatches");
        let sessions = list
            .get("result")
            .and_then(|r| r.get("sessions"))
            .and_then(Value::as_array)
            .expect("list response sessions");
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].get("sessionId").and_then(Value::as_str),
            Some(session_id.as_str())
        );
        assert_eq!(
            sessions[0].get("operationId").and_then(Value::as_str),
            Some("gw-exec-1")
        );
        assert_eq!(
            sessions[0].get("principal").and_then(Value::as_str),
            Some("uid-1000")
        );

        let close = dispatch_request(
            &state,
            &admin_peer(),
            wire::Request::GatewayDisplay(public_wire::GatewayDisplayOp::Close(
                public_wire::GatewayDisplayCloseArgs {
                    session_id: session_id.clone(),
                },
            )),
        )
        .expect("gateway display close dispatches");
        assert_eq!(
            close
                .get("result")
                .and_then(|r| r.get("closed"))
                .and_then(Value::as_bool),
            Some(true)
        );

        let empty = dispatch_request(
            &state,
            &admin_peer(),
            wire::Request::GatewayDisplay(public_wire::GatewayDisplayOp::ListDetailed(
                public_wire::GatewayDisplayListArgs { target: None },
            )),
        )
        .expect("gateway display list after close dispatches");
        assert_eq!(
            empty
                .get("result")
                .and_then(|r| r.get("sessions"))
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
    }

    #[test]
    fn gateway_display_open_uses_peer_uid_as_trusted_principal() {
        let (state, _dir) = test_state();
        dispatch_request(
            &state,
            &launcher_peer(),
            wire::Request::GatewayDisplay(public_wire::GatewayDisplayOp::Open(
                public_wire::GatewayDisplayOpenArgs {
                    target: "nl://demo.gw.work.nixling".to_owned(),
                    operation_id: "gw-exec-launcher".to_owned(),
                    principal: "uid-9999".to_owned(),
                    app_argv: vec!["foot".to_owned()],
                    request_hash: 43,
                },
            )),
        )
        .expect("launcher can open owned gateway display session");

        let list = dispatch_request(
            &state,
            &launcher_peer(),
            wire::Request::GatewayDisplay(public_wire::GatewayDisplayOp::ListDetailed(
                public_wire::GatewayDisplayListArgs { target: None },
            )),
        )
        .expect("launcher list dispatches");
        let sessions = list
            .get("result")
            .and_then(|r| r.get("sessions"))
            .and_then(Value::as_array)
            .expect("list response sessions");
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].get("principal").and_then(Value::as_str),
            Some("uid-1001")
        );
    }

    #[test]
    fn gateway_display_open_rejects_invalid_operation_id() {
        let (state, _dir) = test_state();
        let err = dispatch_request(
            &state,
            &launcher_peer(),
            wire::Request::GatewayDisplay(public_wire::GatewayDisplayOp::Open(
                public_wire::GatewayDisplayOpenArgs {
                    target: "nl://demo.gw.work.nixling".to_owned(),
                    operation_id: "not valid".to_owned(),
                    principal: "uid-1001".to_owned(),
                    app_argv: vec!["foot".to_owned()],
                    request_hash: 46,
                },
            )),
        )
        .expect_err("invalid operation id is rejected before ledger insert");
        assert!(matches!(err, TypedError::WireInvalidFrame { .. }));
        assert!(state.gateway_display.sessions.lock().unwrap().is_empty());
    }

    #[test]
    fn gateway_display_list_and_close_are_owner_scoped_for_launchers() {
        let (state, _dir) = test_state();
        let other_peer = PeerIdentity {
            role: PeerRole::Launcher,
            uid: 1002,
        };

        let first = dispatch_request(
            &state,
            &launcher_peer(),
            wire::Request::GatewayDisplay(public_wire::GatewayDisplayOp::Open(
                public_wire::GatewayDisplayOpenArgs {
                    target: "nl://demo.gw.work.nixling".to_owned(),
                    operation_id: "gw-exec-owner".to_owned(),
                    principal: "ignored".to_owned(),
                    app_argv: vec!["foot".to_owned()],
                    request_hash: 44,
                },
            )),
        )
        .expect("first launcher open dispatches");
        let first_session_id = first
            .get("result")
            .and_then(|r| r.get("sessionId"))
            .and_then(Value::as_str)
            .expect("first open response has session id")
            .to_owned();

        dispatch_request(
            &state,
            &other_peer,
            wire::Request::GatewayDisplay(public_wire::GatewayDisplayOp::Open(
                public_wire::GatewayDisplayOpenArgs {
                    target: "nl://other.gw.work.nixling".to_owned(),
                    operation_id: "gw-exec-other".to_owned(),
                    principal: "ignored".to_owned(),
                    app_argv: vec!["foot".to_owned()],
                    request_hash: 45,
                },
            )),
        )
        .expect("second launcher open dispatches");

        let list = dispatch_request(
            &state,
            &launcher_peer(),
            wire::Request::GatewayDisplay(public_wire::GatewayDisplayOp::ListDetailed(
                public_wire::GatewayDisplayListArgs { target: None },
            )),
        )
        .expect("owner list dispatches");
        let sessions = list
            .get("result")
            .and_then(|r| r.get("sessions"))
            .and_then(Value::as_array)
            .expect("list response sessions");
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].get("principal").and_then(Value::as_str),
            Some("uid-1001")
        );

        let denied = dispatch_request(
            &state,
            &other_peer,
            wire::Request::GatewayDisplay(public_wire::GatewayDisplayOp::Close(
                public_wire::GatewayDisplayCloseArgs {
                    session_id: first_session_id.clone(),
                },
            )),
        )
        .expect_err("other launcher cannot close session");
        assert!(matches!(denied, TypedError::AuthzNotAdmin { .. }));

        let closed = dispatch_request(
            &state,
            &launcher_peer(),
            wire::Request::GatewayDisplay(public_wire::GatewayDisplayOp::Close(
                public_wire::GatewayDisplayCloseArgs {
                    session_id: first_session_id,
                },
            )),
        )
        .expect("owner close dispatches");
        assert_eq!(
            closed
                .get("result")
                .and_then(|r| r.get("closed"))
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn gateway_display_launcher_close_absent_session_is_idempotent() {
        let (state, _dir) = test_state();
        let close = dispatch_request(
            &state,
            &launcher_peer(),
            wire::Request::GatewayDisplay(public_wire::GatewayDisplayOp::Close(
                public_wire::GatewayDisplayCloseArgs {
                    session_id: "missing-session".to_owned(),
                },
            )),
        )
        .expect("absent close dispatches");
        assert_eq!(
            close
                .get("result")
                .and_then(|r| r.get("closed"))
                .and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn gateway_display_launcher_can_close_owned_terminal_tracked_session() {
        let (state, _dir) = test_state();
        let first = dispatch_request(
            &state,
            &launcher_peer(),
            wire::Request::GatewayDisplay(public_wire::GatewayDisplayOp::Open(
                public_wire::GatewayDisplayOpenArgs {
                    target: "nl://demo.gw.work.nixling".to_owned(),
                    operation_id: "gw-exec-terminal".to_owned(),
                    principal: "ignored".to_owned(),
                    app_argv: vec!["foot".to_owned()],
                    request_hash: 47,
                },
            )),
        )
        .expect("launcher open dispatches");
        let session_id = first
            .get("result")
            .and_then(|r| r.get("sessionId"))
            .and_then(Value::as_str)
            .expect("open response has session id")
            .to_owned();
        let open = state
            .gateway_display
            .sessions
            .lock()
            .unwrap()
            .get(&session_id)
            .expect("session still tracked")
            .open
            .clone();
        block_on_future(state.gateway_display.orchestrator.close(&open))
            .expect("orchestrator can mark session terminal");

        let closed = dispatch_request(
            &state,
            &launcher_peer(),
            wire::Request::GatewayDisplay(public_wire::GatewayDisplayOp::Close(
                public_wire::GatewayDisplayCloseArgs { session_id },
            )),
        )
        .expect("owner close dispatches for terminal tracked session");
        assert_eq!(
            closed
                .get("result")
                .and_then(|r| r.get("closed"))
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn gateway_display_start_and_stop_dispatch_lifecycle() {
        let (state, _dir) = test_state();
        let start = dispatch_request(
            &state,
            &admin_peer(),
            wire::Request::GatewayDisplay(public_wire::GatewayDisplayOp::Start(
                public_wire::GatewayDisplayStartArgs {
                    target: "nl://demo.gw.work.nixling".to_owned(),
                    operation_id: "gw-start-1".to_owned(),
                    principal: "uid-1000".to_owned(),
                    request_hash: 41,
                },
            )),
        )
        .expect("gateway display start dispatches");
        assert_eq!(
            start.get("type").and_then(Value::as_str),
            Some("gatewayDisplayResponse")
        );
        assert_eq!(start.get("op").and_then(Value::as_str), Some("start"));
        assert_eq!(
            start
                .get("result")
                .and_then(|r| r.get("state"))
                .and_then(Value::as_str),
            Some("ready")
        );

        let stop = dispatch_request(
            &state,
            &admin_peer(),
            wire::Request::GatewayDisplay(public_wire::GatewayDisplayOp::Stop(
                public_wire::GatewayDisplayStopArgs {
                    target: "nl://demo.gw.work.nixling".to_owned(),
                    operation_id: "gw-stop-1".to_owned(),
                    principal: "uid-1000".to_owned(),
                    request_hash: 42,
                },
            )),
        )
        .expect("gateway display stop dispatches");
        assert_eq!(stop.get("op").and_then(Value::as_str), Some("stop"));
        assert_eq!(
            stop.get("result")
                .and_then(|r| r.get("state"))
                .and_then(Value::as_str),
            Some("stopped")
        );
    }

    #[test]
    fn gateway_display_unconfigured_runtime_fails_closed() {
        let (mut state, _dir) = test_state();
        state.gateway_display = crate::new_gateway_display_runtime();
        let err = dispatch_request(
            &state,
            &admin_peer(),
            wire::Request::GatewayDisplay(public_wire::GatewayDisplayOp::Start(
                public_wire::GatewayDisplayStartArgs {
                    target: "nl://demo.gw.work.nixling".to_owned(),
                    operation_id: "gw-start-unconfigured".to_owned(),
                    principal: "uid-1000".to_owned(),
                    request_hash: 41,
                },
            )),
        )
        .expect_err("unconfigured gateway runtime must fail closed");
        assert!(matches!(err, TypedError::GatewayDisplayUnavailable { .. }));
    }

    #[test]
    fn gateway_display_replay_preserves_real_session_handles() {
        let (state, _dir) = test_state();
        let request = || {
            wire::Request::GatewayDisplay(public_wire::GatewayDisplayOp::Open(
                public_wire::GatewayDisplayOpenArgs {
                    target: "nl://demo.gw.work.nixling".to_owned(),
                    operation_id: "gw-exec-replay".to_owned(),
                    principal: "uid-1000".to_owned(),
                    app_argv: vec!["foot".to_owned()],
                    request_hash: 42,
                },
            ))
        };
        let first = dispatch_request(&state, &admin_peer(), request()).expect("first open");
        let session_id = first
            .get("result")
            .and_then(|r| r.get("sessionId"))
            .and_then(Value::as_str)
            .unwrap()
            .to_owned();
        let second = dispatch_request(&state, &admin_peer(), request()).expect("replay open");
        assert_eq!(
            second
                .get("result")
                .and_then(|r| r.get("sessionId"))
                .and_then(Value::as_str),
            Some(session_id.as_str())
        );
        let sessions = state.gateway_display.sessions.lock().unwrap();
        let session = sessions.get(&session_id).expect("session retained");
        assert!(!session.open.agent.0.is_empty());
        assert!(!session.open.listener.0.is_empty());
    }

    #[test]
    fn gateway_display_list_gc_expires_stale_sessions() {
        let (state, _dir) = test_state();
        let value = dispatch_request(
            &state,
            &admin_peer(),
            wire::Request::GatewayDisplay(public_wire::GatewayDisplayOp::Open(
                public_wire::GatewayDisplayOpenArgs {
                    target: "nl://demo.gw.work.nixling".to_owned(),
                    operation_id: "gw-exec-gc".to_owned(),
                    principal: "uid-1000".to_owned(),
                    app_argv: vec!["foot".to_owned()],
                    request_hash: 43,
                },
            )),
        )
        .expect("open session");
        let session_id = value
            .get("result")
            .and_then(|r| r.get("sessionId"))
            .and_then(Value::as_str)
            .unwrap()
            .to_owned();
        {
            let mut sessions = state.gateway_display.sessions.lock().unwrap();
            sessions
                .get_mut(&session_id)
                .expect("session exists")
                .opened_at = Instant::now() - GATEWAY_DISPLAY_SESSION_TTL - Duration::from_secs(1);
        }
        let list = dispatch_request(
            &state,
            &admin_peer(),
            wire::Request::GatewayDisplay(public_wire::GatewayDisplayOp::List(
                public_wire::GatewayDisplayListArgs { target: None },
            )),
        )
        .expect("list after gc");
        assert_eq!(
            list.get("result")
                .and_then(|r| r.get("sessions"))
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
        assert!(state.gateway_display.sessions.lock().unwrap().is_empty());
    }

    #[test]
    fn gateway_display_lifecycle_requires_admin_but_listing_is_launcher_scoped() {
        let (state, _dir) = test_state();
        let err = dispatch_request(
            &state,
            &launcher_peer(),
            wire::Request::GatewayDisplay(public_wire::GatewayDisplayOp::Start(
                public_wire::GatewayDisplayStartArgs {
                    target: "nl://demo.gw.work.nixling".to_owned(),
                    operation_id: "gw-start-launcher".to_owned(),
                    principal: "uid-1001".to_owned(),
                    request_hash: 1,
                },
            )),
        )
        .expect_err("launcher must not start gateway display lifecycle");
        assert!(matches!(err, TypedError::AuthzNotAdmin { .. }));

        let list = dispatch_request(
            &state,
            &launcher_peer(),
            wire::Request::GatewayDisplay(public_wire::GatewayDisplayOp::List(
                public_wire::GatewayDisplayListArgs { target: None },
            )),
        )
        .expect("launcher can list its own gateway display sessions");
        assert_eq!(
            list.get("result")
                .and_then(|r| r.get("sessions"))
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
    }

    fn write_public_status_artifacts_with_state_dir(
        root: &Path,
        vm_a_state_dir: Option<&Path>,
    ) -> ArtifactPaths {
        let public_manifest_path = root.join("vms.json");
        let bundle_path = root.join("bundle.json");
        let processes_path = root.join("processes.json");
        let host_path = root.join("host.json");
        let privileges_path = root.join("privileges.json");
        let closures_dir = root.join("closures");
        fs::create_dir_all(&closures_dir).expect("closures dir");
        let vm_a_state_dir = vm_a_state_dir
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| root.join("vm-a-state").display().to_string());

        fs::write(
            &public_manifest_path,
            serde_json::to_vec_pretty(&json!({
                "_manifest": { "manifestVersion": 6 },
                "vm-a": {
                    "name": "vm-a",
                    "env": "work",
                    "staticIp": "10.20.0.10",
                    "sshUser": "alice",
                    "isNetVm": false,
                    "stateDir": vm_a_state_dir,
                    "graphics": true,
                    "tpm": false,
                    "usbipYubikey": true,
                    "audio": false,
                    "runtime": {
                        "kind": "nixos",
                        "provider": {
                            "driver": "cloud-hypervisor",
                            "id": "local-cloud-hypervisor",
                            "type": "local"
                        },
                        "capabilities": {
                            "lifecycle": true,
                            "display": false,
                            "usbHotplug": true,
                            "guestControl": true,
                            "exec": true,
                            "configSync": true,
                            "ssh": true,
                            "storeSync": true,
                            "keys": true,
                            "inGuestObservability": true
                        }
                    }
                },
                "vm-b": {
                    "name": "vm-b",
                    "env": "personal",
                    "staticIp": "10.30.0.10",
                    "sshUser": "bob",
                    "isNetVm": false,
                    "graphics": false,
                    "tpm": false,
                    "usbipYubikey": false,
                    "audio": false,
                    "runtime": {
                        "kind": "nixos",
                        "provider": {
                            "driver": "cloud-hypervisor",
                            "id": "local-cloud-hypervisor",
                            "type": "local"
                        },
                        "capabilities": {
                            "lifecycle": true,
                            "display": false,
                            "usbHotplug": false,
                            "guestControl": true,
                            "exec": true,
                            "configSync": true,
                            "ssh": true,
                            "storeSync": true,
                            "keys": true,
                            "inGuestObservability": true
                        }
                    }
                }
            }))
            .expect("manifest json"),
        )
        .expect("write manifest");

        fs::write(
            &processes_path,
            serde_json::to_vec_pretty(&json!({
                "schemaVersion": "v2",
                "vms": []
            }))
            .expect("processes json"),
        )
        .expect("write processes");

        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/deny-unknown/host-valid.json"),
            &host_path,
        )
        .expect("copy host fixture");
        fs::write(
            &privileges_path,
            serde_json::to_vec_pretty(&json!({
                "schemaVersion": "v2",
                "operations": []
            }))
            .expect("privileges json"),
        )
        .expect("write privileges");

        fs::write(
            &bundle_path,
            serde_json::to_vec_pretty(&json!({
                "bundleVersion": 4,
                "schemaVersion": "v2",
                "publicManifestPath": public_manifest_path.display().to_string(),
                "hostPath": host_path.display().to_string(),
                "processesPath": processes_path.display().to_string(),
                "privilegesPath": privileges_path.display().to_string(),
                "closures": [],
                "minijailProfiles": [],
                "managedKeys": {},
                "generation": {
                    "generator": "public-status-test",
                    "sourceRevision": null,
                    "generatedAt": null
                }
            }))
            .expect("bundle json"),
        )
        .expect("write bundle");

        for path in [
            &bundle_path,
            &host_path,
            &privileges_path,
            &processes_path,
            &public_manifest_path,
        ] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o640))
                .expect("chmod public status test artifact");
        }

        ArtifactPaths {
            public_manifest_path,
            bundle_path,
            host_path,
            processes_path,
            closures_dir,
        }
    }

    fn make_generation_links(root: &Path, current: &str, booted: &str) -> PathBuf {
        let state_dir = root.join("vm-a-state");
        fs::create_dir_all(&state_dir).expect("state dir");
        std::os::unix::fs::symlink(current, state_dir.join("current")).expect("current link");
        std::os::unix::fs::symlink(booted, state_dir.join("booted")).expect("booted link");
        state_dir
    }

    fn current_process_entry() -> PidfdEntry {
        let pid = std::process::id() as i32;
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).expect("read current stat");
        let start_time_ticks = parse_proc_stat_starttime(&stat).expect("parse current start time");
        let pidfd: OwnedFd = File::open("/dev/null").expect("open dummy fd").into();
        PidfdEntry {
            pidfd,
            pid,
            start_time_ticks,
        }
    }

    fn lifecycle_state(value: &Value) -> &str {
        value
            .get("state")
            .and_then(Value::as_str)
            .expect("lifecycle state")
    }

    fn manifest_entry() -> Value {
        json!({ "stateDir": "/nonexistent/nixling-public-status-test" })
    }

    fn qemu_media_manifest_entry() -> Value {
        json!({
            "stateDir": "/nonexistent/nixling-public-status-test",
            "runtime": {
                "kind": "qemu-media",
                "capabilities": {
                    "configSync": false,
                    "exec": false,
                    "guestControl": false,
                    "keys": false,
                    "ssh": false,
                    "storeSync": false,
                    "usbHotplug": true
                }
            },
            "graphics": false,
            "audio": false,
            "tpm": false
        })
    }

    fn qemu_media_process_dag() -> nixling_core::processes::VmProcessDag {
        use nixling_core::processes::{NodeId, VmProcessDag, VmProcessInvariants};
        VmProcessDag {
            vm: "installer".to_owned(),
            nodes: vec![ProcessNode {
                id: NodeId("qemu-media".to_owned()),
                role: ProcessRole::QemuMediaRunner,
                unit: None,
                binary_path: Some("/nix/store/qemu/bin/qemu-system-x86_64".to_owned()),
                argv: Vec::new(),
                env: Vec::new(),
                plan_ops: Vec::new(),
                profile: nixling_core::test_support::RoleProfileBuilder::new()
                    .with_profile_id("vm-installer-qemu-media")
                    .build(),
                readiness: vec![ReadinessPredicate::UnixSocketListening(
                    "/run/nixling/vms/installer/qmp.sock".to_owned(),
                )],
            }],
            edges: Vec::new(),
            invariants: VmProcessInvariants {
                swtpm_pre_start_flush: true,
                per_vm_audit_pipeline: true,
                usbip_gating: true,
                tpm_ownership_migration_without_running_vm_mutation: true,
            },
        }
    }

    fn qemu_media_source() -> QemuMediaSourceIntent {
        QemuMediaSourceIntent {
            vm: "installer".to_owned(),
            media_ref: "installer-usb".to_owned(),
            slot: "boot".to_owned(),
            source_kind: nixling_core::host::QemuMediaSourceKind::PhysicalUsb,
            format: nixling_core::host::QemuMediaFormat::Raw,
            read_only: true,
            registry_scope: nixling_core::host::QemuMediaRegistryScope::RootOnlyRuntimeState,
            image_path: None,
            usb_selector: None,
        }
    }

    fn qemu_media_image_source() -> QemuMediaSourceIntent {
        QemuMediaSourceIntent {
            vm: "installer".to_owned(),
            media_ref: "image-boot".to_owned(),
            slot: "boot".to_owned(),
            source_kind: nixling_core::host::QemuMediaSourceKind::ImageFile,
            format: nixling_core::host::QemuMediaFormat::Raw,
            read_only: false,
            registry_scope: nixling_core::host::QemuMediaRegistryScope::DirectConfigPath,
            image_path: Some("/var/lib/nixling/images/installer.img".to_owned()),
            usb_selector: None,
        }
    }

    fn typed_manifest_vm(runtime: nixling_core::runtime::RuntimeMetadata) -> ManifestVmEntry {
        ManifestVmEntry {
            api_socket: None,
            audio: false,
            audio_service: None,
            audio_state_file: None,
            bridge: None,
            env: Some("dev".to_owned()),
            mtu: None,
            mss_clamp: None,
            lan: None,
            gpu_socket: None,
            graphics: false,
            is_net_vm: false,
            name: "installer".to_owned(),
            net_vm: None,
            observability: nixling_core::manifest_v04::VmObservability {
                agent_socket: None,
                enabled: false,
                vsock_cid: None,
                vsock_host_socket: None,
            },
            runtime,
            shell: None,
            ssh_user: None,
            state_dir: "/var/lib/nixling/vms/installer".to_owned(),
            static_ip: None,
            tap: "nl-installer".to_owned(),
            tpm: false,
            tpm_socket: None,
            usbip_yubikey: false,
            usbipd_host_ip: None,
        }
    }

    #[test]
    fn qemu_media_skips_nixos_state_preflights() {
        let qemu = typed_manifest_vm(nixling_core::runtime::RuntimeMetadata::local_qemu_media());
        let nixos = typed_manifest_vm(nixling_core::runtime::RuntimeMetadata::local_nixos());

        assert!(!vm_requires_nixos_state_preflights(&qemu));
        assert!(vm_requires_nixos_state_preflights(&nixos));
    }

    #[test]
    fn public_lifecycle_reports_stopped_with_no_live_roles() {
        let (state, _dir) = test_state();
        let manifest_entry = manifest_entry();
        let lifecycle = public_vm_lifecycle(&state, "vm-a", &manifest_entry, None);
        assert_eq!(lifecycle_state(&lifecycle), "Stopped");
        assert_eq!(
            public_runtime_summary(&lifecycle, &manifest_entry)
                .get("detail")
                .and_then(Value::as_str),
            Some("stopped")
        );
    }

    #[test]
    fn public_lifecycle_reports_running_when_ch_runner_is_live() {
        let (state, _dir) = test_state();
        state
            .pidfd_table
            .register(
                "vm-a".to_owned(),
                VM_RUNNER_ROLE_ID.to_owned(),
                current_process_entry(),
            )
            .expect("register ch runner");
        let manifest_entry = manifest_entry();
        let lifecycle = public_vm_lifecycle(&state, "vm-a", &manifest_entry, None);
        assert_eq!(lifecycle_state(&lifecycle), "Running");
        assert_eq!(
            public_runtime_summary(&lifecycle, &manifest_entry)
                .get("detail")
                .and_then(Value::as_str),
            Some("running")
        );
    }

    #[test]
    fn public_lifecycle_reports_running_for_qemu_media_runner() {
        let (state, _dir) = test_state();
        state
            .pidfd_table
            .register(
                "installer".to_owned(),
                RunnerRole::QemuMedia.as_str().to_owned(),
                current_process_entry(),
            )
            .expect("register qemu media runner");
        let manifest_entry = qemu_media_manifest_entry();
        let dag = qemu_media_process_dag();
        let lifecycle = public_vm_lifecycle(&state, "installer", &manifest_entry, Some(&dag));
        let services = public_service_states(&state, "installer", &manifest_entry, Some(&dag));
        let runtime = public_runtime_summary(&lifecycle, &manifest_entry);

        assert_eq!(lifecycle_state(&lifecycle), "Running");
        assert_eq!(
            runtime.get("kind").and_then(Value::as_str),
            Some("qemu-media")
        );
        assert_eq!(
            services.get("microvm").and_then(Value::as_str),
            Some("unsupported")
        );
        assert_eq!(
            services.get("qemuMedia").and_then(Value::as_str),
            Some("running")
        );
        assert_eq!(
            services.get("virtiofsd").and_then(Value::as_str),
            Some("unsupported")
        );
        assert_eq!(
            public_service_capabilities(&services),
            vec!["nixling".to_owned(), "qemu-media".to_owned()]
        );
    }

    #[test]
    fn qemu_media_status_reports_manual_runtime_and_missing_registry() {
        let root = tempfile::tempdir().expect("registry root");
        let (state, _dir) = test_state();
        let manifest_entry = qemu_media_manifest_entry();
        let dag = qemu_media_process_dag();
        let services = public_service_states(&state, "installer", &manifest_entry, Some(&dag));
        let qemu = public_qemu_media_status(
            &state,
            "installer",
            Some("qemu-media"),
            None,
            Some(&dag),
            &services,
        )
        .expect("qemu media status");

        assert_eq!(
            qemu.pointer("/firmwareMode").and_then(Value::as_str),
            Some("none")
        );
        assert_eq!(
            qemu.pointer("/runner/role").and_then(Value::as_str),
            Some("qemu-media")
        );
        assert_eq!(
            qemu.pointer("/runner/qmpSocket").and_then(Value::as_str),
            None
        );
        let source_status =
            qemu_media_source_status(&root.path().display().to_string(), &qemu_media_source());
        assert_eq!(
            source_status.pointer("/slot").and_then(Value::as_str),
            Some("boot")
        );
        assert_eq!(
            source_status
                .pointer("/registry/state")
                .and_then(Value::as_str),
            Some("missing")
        );
        assert!(
            source_status
                .pointer("/registry/remediation")
                .and_then(Value::as_str)
                .is_some_and(|text| text.contains("installer-usb"))
        );
        assert!(
            source_status
                .pointer("/registry/remediation")
                .and_then(Value::as_str)
                .is_some_and(
                    |text| text.contains("nixling usb probe") && !text.contains("usb enroll")
                )
        );
    }

    #[test]
    fn qemu_media_status_reports_direct_image_without_enrollment_remediation() {
        let source_status = qemu_media_source_status(
            "/var/lib/nixling/media-registry",
            &qemu_media_image_source(),
        );

        assert_eq!(
            source_status.pointer("/sourceKind").and_then(Value::as_str),
            Some("image-file")
        );
        assert_eq!(
            source_status.pointer("/imagePath").and_then(Value::as_str),
            None
        );
        assert_eq!(
            source_status
                .pointer("/registry/state")
                .and_then(Value::as_str),
            Some("direct-config")
        );
        assert!(
            source_status
                .pointer("/registry/remediation")
                .is_some_and(Value::is_null)
        );
    }

    #[test]
    fn runtime_capability_gate_rejects_unsupported_qemu_media_operations() {
        let qemu = typed_manifest_vm(nixling_core::runtime::RuntimeMetadata::local_qemu_media());
        let nixos = typed_manifest_vm(nixling_core::runtime::RuntimeMetadata::local_nixos());
        let mut qemu_without_hotplug = qemu.clone();
        qemu_without_hotplug.runtime.capabilities.usb_hotplug = false;

        ensure_manifest_entry_runtime_capability(
            Some(&nixos),
            "installer",
            RuntimeCapabilityGate::Exec,
            "exec",
        )
        .expect("nixos exec supported");
        let error = ensure_manifest_entry_runtime_capability(
            Some(&qemu),
            "installer",
            RuntimeCapabilityGate::Exec,
            "exec",
        )
        .expect_err("qemu-media exec unsupported");

        assert_eq!(error.kind(), "runtime-capability-unsupported");
        assert_eq!(error.exit_code(), 70);
        let envelope = error.to_envelope();
        assert!(envelope.message.contains("qemu-media"));
        assert!(envelope.message.contains("exec"));
        assert!(!envelope.message.contains("/var/lib"));

        ensure_manifest_entry_runtime_capability(
            Some(&qemu),
            "installer",
            RuntimeCapabilityGate::UsbHotplug,
            "usb attach",
        )
        .expect("qemu-media usb hotplug supported");
        let hotplug_error = ensure_manifest_entry_runtime_capability(
            Some(&qemu_without_hotplug),
            "installer",
            RuntimeCapabilityGate::UsbHotplug,
            "usb attach",
        )
        .expect_err("unsupported usb hotplug is denied");
        assert_eq!(hotplug_error.kind(), "runtime-capability-unsupported");
        assert!(hotplug_error.to_envelope().message.contains("usb-hotplug"));
    }

    #[test]
    fn public_service_states_follow_pidfd_roles() {
        let (state, _dir) = test_state();
        state
            .pidfd_table
            .register(
                "vm-a".to_owned(),
                VM_RUNNER_ROLE_ID.to_owned(),
                current_process_entry(),
            )
            .expect("register ch runner");
        state
            .pidfd_table
            .register(
                "vm-a".to_owned(),
                "virtiofsd-ro-store".to_owned(),
                current_process_entry(),
            )
            .expect("register virtiofsd");
        let services = public_service_states(
            &state,
            "vm-a",
            &json!({ "graphics": false, "audio": false, "tpm": false }),
            None,
        );
        assert_eq!(
            services.get("nixling").and_then(Value::as_str),
            Some("active")
        );
        assert_eq!(
            services.get("microvm").and_then(Value::as_str),
            Some("running")
        );
        assert_eq!(
            services.get("virtiofsd").and_then(Value::as_str),
            Some("running")
        );
    }

    #[test]
    fn dispatch_list_emits_manifest_features_and_live_lifecycle() {
        let root = tempfile::tempdir().expect("artifact root");
        let artifacts = write_public_status_artifacts(root.path());
        let (state, _dir) = test_state_with_config(DaemonConfig {
            artifacts,
            ..DaemonConfig::default()
        });
        state
            .pidfd_table
            .register(
                "vm-a".to_owned(),
                VM_RUNNER_ROLE_ID.to_owned(),
                current_process_entry(),
            )
            .expect("register ch runner");

        let frame = dispatch_list(
            &state,
            public_wire::ListRequest {
                env: Some("work".to_owned()),
                vm: None,
            },
        )
        .expect("list dispatch");

        assert_eq!(
            frame.get("type").and_then(Value::as_str),
            Some("listResponse")
        );
        let vms = frame.get("vms").and_then(Value::as_array).expect("vms");
        assert_eq!(vms.len(), 1);
        let vm = &vms[0];
        assert_eq!(vm.get("vm").and_then(Value::as_str), Some("vm-a"));
        assert_eq!(vm.get("graphics").and_then(Value::as_bool), Some(true));
        assert_eq!(vm.get("usbip").and_then(Value::as_bool), Some(true));
        assert_eq!(
            vm.pointer("/lifecycle/state").and_then(Value::as_str),
            Some("Running")
        );
        assert_eq!(
            vm.pointer("/runtime/detail").and_then(Value::as_str),
            Some("running")
        );
        assert!(
            vm.pointer("/runtimeCapabilities")
                .and_then(Value::as_array)
                .is_some_and(|items| items.iter().any(|item| item == "usb-hotplug"))
        );
        assert!(
            vm.pointer("/serviceCapabilities")
                .and_then(Value::as_array)
                .is_some_and(|items| items.iter().any(|item| item == "microvm"))
        );
        assert_eq!(
            vm.pointer("/services/microvm").and_then(Value::as_str),
            Some("running")
        );
    }

    #[test]
    fn dispatch_status_emits_entries_frame_and_service_states() {
        let root = tempfile::tempdir().expect("artifact root");
        let artifacts = write_public_status_artifacts(root.path());
        let (state, _dir) = test_state_with_config(DaemonConfig {
            artifacts,
            ..DaemonConfig::default()
        });
        state
            .pidfd_table
            .register(
                "vm-a".to_owned(),
                "virtiofsd-ro-store".to_owned(),
                current_process_entry(),
            )
            .expect("register virtiofsd");

        let frame = dispatch_status(
            &state,
            public_wire::StatusRequest {
                check_bridges: false,
                vm: Some("vm-a".to_owned()),
            },
        )
        .expect("status dispatch");

        assert_eq!(
            frame.get("type").and_then(Value::as_str),
            Some("statusResponse")
        );
        let entries = frame
            .pointer("/status/entries")
            .and_then(Value::as_array)
            .expect("status entries");
        assert_eq!(entries.len(), 1);
        let vm = &entries[0];
        assert_eq!(vm.get("vm").and_then(Value::as_str), Some("vm-a"));
        assert_eq!(
            vm.get("staticIp").and_then(Value::as_str),
            Some("10.20.0.10")
        );
        assert_eq!(
            vm.pointer("/lifecycle/state").and_then(Value::as_str),
            Some("Starting")
        );
        assert_eq!(
            vm.pointer("/runtime/detail").and_then(Value::as_str),
            Some("starting")
        );
        assert!(
            vm.pointer("/runtimeCapabilities")
                .and_then(Value::as_array)
                .is_some_and(|items| items.iter().any(|item| item == "usb-hotplug"))
        );
        assert!(
            vm.pointer("/serviceCapabilities")
                .and_then(Value::as_array)
                .is_some_and(|items| items.iter().any(|item| item == "virtiofsd"))
        );
        assert_eq!(
            vm.pointer("/services/microvm").and_then(Value::as_str),
            Some("stopped")
        );
        assert_eq!(
            vm.pointer("/services/virtiofsd").and_then(Value::as_str),
            Some("running")
        );
        assert!(
            vm.get("readiness").is_none(),
            "public status should not expose raw readiness path strings"
        );
    }

    #[test]
    fn dispatch_status_pending_restart_requires_running_vm() {
        let root = tempfile::tempdir().expect("artifact root");
        let state_dir =
            make_generation_links(root.path(), "/nix/store/current", "/nix/store/booted");
        let artifacts = write_public_status_artifacts_with_state_dir(root.path(), Some(&state_dir));
        let (state, _dir) = test_state_with_config(DaemonConfig {
            artifacts,
            ..DaemonConfig::default()
        });

        let stopped = dispatch_status(
            &state,
            public_wire::StatusRequest {
                check_bridges: false,
                vm: Some("vm-a".to_owned()),
            },
        )
        .expect("stopped status");
        assert_eq!(
            stopped
                .pointer("/status/entries/0/lifecycle/state")
                .and_then(Value::as_str),
            Some("Stopped")
        );
        assert_eq!(
            stopped
                .pointer("/status/entries/0/lifecycle/pendingRestart")
                .and_then(Value::as_bool),
            Some(false),
            "stopped VMs do not have a pending restart even if current/booted differ"
        );

        state
            .pidfd_table
            .register(
                "vm-a".to_owned(),
                VM_RUNNER_ROLE_ID.to_owned(),
                current_process_entry(),
            )
            .expect("register running ch");
        let running = dispatch_status(
            &state,
            public_wire::StatusRequest {
                check_bridges: false,
                vm: Some("vm-a".to_owned()),
            },
        )
        .expect("running status");
        assert_eq!(
            running
                .pointer("/status/entries/0/lifecycle/state")
                .and_then(Value::as_str),
            Some("Running")
        );
        assert_eq!(
            running
                .pointer("/status/entries/0/lifecycle/pendingRestart")
                .and_then(Value::as_bool),
            Some(true),
            "running VMs report pending restart when current/booted differ"
        );
    }

    #[test]
    fn public_lifecycle_reports_starting_for_sidecars_without_ch_runner() {
        let (state, _dir) = test_state();
        state
            .pidfd_table
            .register(
                "vm-a".to_owned(),
                "virtiofsd-ro-store".to_owned(),
                current_process_entry(),
            )
            .expect("register sidecar");
        let manifest_entry = manifest_entry();
        let lifecycle = public_vm_lifecycle(&state, "vm-a", &manifest_entry, None);
        assert_eq!(lifecycle_state(&lifecycle), "Starting");
    }
}

fn dispatch_audit(
    state: &ServerState,
    peer: &PeerIdentity,
    request: public_wire::AuditRequest,
) -> Result<Value, TypedError> {
    if peer.role != PeerRole::Admin {
        return Err(TypedError::AuthzAuditRequiresAdmin);
    }
    let socket = connect_seqpacket(&state.config.broker_socket_path).map_err(|error| {
        TypedError::InternalBrokerUnavailable {
            path: state.config.broker_socket_path.clone(),
            detail: error.message(),
        }
    })?;
    let hello = json!({
        "type": "hello",
        "clientVersion": state.config.accepted_client_version_range,
        "supportedFeatures": []
    });
    let hello_bytes = serde_json::to_vec(&hello).map_err(|err| TypedError::InternalIo {
        context: "serialize broker hello".to_owned(),
        detail: err.to_string(),
    })?;
    write_frame(&socket, &hello_bytes)?;
    let _ = read_frame(&socket)?;

    let broker_request = json!({
        "type": "exportBrokerAudit",
        "since": request.since,
        "filter": request.filter.as_ref().map(|filter| {
            json!({
                "env": filter.env,
                "vm": filter.vm,
            })
        }),
    });
    let request_bytes =
        serde_json::to_vec(&broker_request).map_err(|err| TypedError::InternalIo {
            context: "serialize broker audit request".to_owned(),
            detail: err.to_string(),
        })?;
    write_frame(&socket, &request_bytes)?;
    let response = read_frame(&socket)?;
    let value: Value =
        serde_json::from_slice(&response).map_err(|err| TypedError::InternalBrokerUnavailable {
            path: state.config.broker_socket_path.clone(),
            detail: err.to_string(),
        })?;
    let lines = value
        .get("lines")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    serde_json::to_value(wire::audit_response(lines)).map_err(|err| TypedError::InternalIo {
        context: "serialize audit response".to_owned(),
        detail: err.to_string(),
    })
}

fn dispatch_host_check(
    state: &ServerState,
    request: wire::HostCheckRequestExt,
) -> Result<Value, TypedError> {
    let bundle = load_json::<Bundle>(&state.config.artifacts.bundle_path)?;
    let bundle_dir = state
        .config
        .artifacts
        .bundle_path
        .parent()
        .unwrap_or_else(|| Path::new("/"));
    let host_path = resolve_bundle_artifact_path(bundle_dir, &bundle.host_path);
    let host = load_json::<HostJson>(&host_path)?;
    let closures = bundle
        .closures
        .iter()
        .map(|closure| {
            load_json::<ClosureMetadata>(&resolve_bundle_artifact_path(bundle_dir, &closure.path))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let report =
        host_check::run(&host, closures.iter(), request.request.strict).map_err(|err| {
            TypedError::InternalIo {
                context: "host check".to_owned(),
                detail: err.opaque_reason,
            }
        })?;
    let summary = json!({
        "warnings": report.summary.warn,
        "failures": report.summary.fail,
        "strict": report.strict,
    });
    let checks = report
        .findings
        .into_iter()
        .map(|finding| {
            let mut check = json!({
                "name": finding.id,
                "status": finding.severity.as_str(),
                "message": finding.message,
                "remediation": finding.remediation,
            });
            if let Some(vm) = finding.vm {
                check
                    .as_object_mut()
                    .expect("host check response is a JSON object")
                    .insert("vm".to_owned(), json!(vm));
            }
            if let Some(detail) = finding.detail {
                check
                    .as_object_mut()
                    .expect("host check response is a JSON object")
                    .insert("detail".to_owned(), json!(detail));
            }
            if !finding.details.is_empty() {
                check
                    .as_object_mut()
                    .expect("host check response is a JSON object")
                    .insert("details".to_owned(), json!(finding.details));
            }
            check
        })
        .collect();
    Ok(wire::host_check_response(summary, checks))
}

fn dispatch_auth_status(state: &ServerState, peer: &PeerIdentity) -> Value {
    let (role, allowed_subcommands, denied_subcommands) = if peer.role == PeerRole::Admin {
        (
            AuthRole::Admin,
            vec![
                "list",
                "status",
                "audit",
                "host check",
                "auth status",
                "op inspect",
                "realm list",
                "realm inspect",
                "realm enter",
                "realm run",
            ],
            Vec::new(),
        )
    } else {
        (
            AuthRole::Launcher,
            vec![
                "list",
                "status",
                "host check",
                "auth status",
                "op inspect",
                "realm list",
                "realm inspect",
                "realm enter",
                "realm run",
            ],
            vec![DeniedCommandHint {
                command: "audit".to_owned(),
                reason: "audit requires admin role in nixling.site.adminUsers".to_owned(),
            }],
        )
    };
    serde_json::to_value(wire::auth_status_response(AuthStatusResponse {
        allowed_subcommands: allowed_subcommands.into_iter().map(str::to_owned).collect(),
        denied_subcommands,
        role,
        sockets: vec![
            SocketReachability {
                reachable: true,
                socket: state.config.public_socket_path.display().to_string(),
            },
            SocketReachability {
                reachable: state.config.broker_socket_path.exists(),
                socket: state.config.broker_socket_path.display().to_string(),
            },
        ],
    }))
    .expect("auth status response serializes")
}

fn load_manifest(path: &Path) -> Result<serde_json::Map<String, Value>, TypedError> {
    let value: Value = load_json(path)?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| TypedError::InternalIo {
            context: format!("decode manifest {}", path.display()),
            detail: "manifest must be a JSON object".to_owned(),
        })
}

fn resolve_bundle_artifact_path(base_dir: &Path, raw_path: &str) -> PathBuf {
    let raw = Path::new(raw_path);
    if raw.is_absolute() && raw.exists() {
        raw.to_path_buf()
    } else if raw.is_absolute() {
        raw.file_name()
            .map(|name| base_dir.join(name))
            .unwrap_or_else(|| raw.to_path_buf())
    } else {
        base_dir.join(raw)
    }
}

fn load_json<T>(path: &Path) -> Result<T, TypedError>
where
    T: for<'de> Deserialize<'de>,
{
    let bytes = fs::read(path).map_err(|err| TypedError::InternalIo {
        context: format!("read {}", path.display()),
        detail: err.to_string(),
    })?;
    serde_json::from_slice(&bytes).map_err(|err| TypedError::InternalIo {
        context: format!("decode {}", path.display()),
        detail: err.to_string(),
    })
}

fn connect_seqpacket(path: &Path) -> Result<OwnedFd, TypedError> {
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
fn connect_seqpacket_with_timeout(
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

fn round_trip(socket: &impl AsRawFd, frame_json: &str) -> Result<Vec<u8>, TypedError> {
    write_frame(socket, frame_json.as_bytes())?;
    read_frame(socket)
}

fn write_json_frame<T>(socket: &impl AsRawFd, value: &T) -> Result<(), TypedError>
where
    T: Serialize,
{
    let bytes = serde_json::to_vec(value).map_err(|err| TypedError::InternalIo {
        context: "serialize JSON frame".to_owned(),
        detail: err.to_string(),
    })?;
    write_frame(socket, &bytes)
}

/// Set (or clear, with `None`) the read timeout on a connection socket.
/// Best-effort: a failure to set the deadline is non-fatal (the read
/// simply blocks as before). Used to bound hello/request frame reads so
/// a silent or slow-loris peer cannot pin a handler slot, and to CLEAR
/// the deadline before handing the socket to a blocking exec owner.
fn set_frame_read_deadline(socket: &Socket, deadline: Option<Duration>) {
    let _ = socket.set_read_timeout(deadline);
}

/// Write a JSON frame with a bounded write deadline, used for the
/// accept-loop refusal frames (authz reject / typed-busy) so the accept
/// loop never blocks on a peer that will not read. The deadline is
/// best-effort and the socket is closed by the caller afterwards.
fn write_json_frame_deadlined<T>(
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
fn drain_rejected_peer_input(socket: &Socket) {
    let _ = socket.set_read_timeout(Some(REJECTION_DRAIN_DEADLINE));
    for _ in 0..4 {
        match read_frame(socket) {
            Ok(_) => continue,
            Err(_) => break,
        }
    }
}

fn write_frame(socket: &impl AsRawFd, body: &[u8]) -> Result<(), TypedError> {
    if body.len() > wire::MAX_FRAME_SIZE {
        return Err(TypedError::WireFrameTooLarge {
            declared: body.len(),
        });
    }
    let mut frame = Vec::with_capacity(body.len() + 4);
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(body);
    let written = send(socket.as_raw_fd(), &frame, MsgFlags::empty()).map_err(|err| {
        TypedError::InternalIo {
            context: "send seqpacket frame".to_owned(),
            detail: err.to_string(),
        }
    })?;
    if written != frame.len() {
        return Err(TypedError::InternalIo {
            context: "send seqpacket frame".to_owned(),
            detail: format!("short write: {written} of {}", frame.len()),
        });
    }
    Ok(())
}

fn read_frame(socket: &impl AsRawFd) -> Result<Vec<u8>, TypedError> {
    let mut buffer = vec![0u8; wire::MAX_FRAME_SIZE + 5];
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
    if declared > wire::MAX_FRAME_SIZE {
        return Err(TypedError::WireFrameTooLarge { declared });
    }
    if read - 4 != declared {
        return Err(TypedError::WireInvalidFrame {
            detail: format!("declared {declared} bytes but received {}", read - 4),
        });
    }
    Ok(buffer[4..read].to_vec())
}

fn mark_fd_cloexec(fd: RawFd, context: &str) -> Result<(), TypedError> {
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

fn duplicate_fd_cloexec(fd: RawFd, context: &str) -> Result<OwnedFd, TypedError> {
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

fn read_frame_with_fds(socket: &impl AsRawFd) -> Result<(Vec<u8>, Vec<RawFd>), TypedError> {
    let mut buffer = vec![0u8; wire::MAX_FRAME_SIZE + 5];
    let mut iov = [IoSliceMut::new(&mut buffer)];
    let mut control = cmsg_space!([RawFd; 8]);
    let message = recvmsg::<UnixAddr>(
        socket.as_raw_fd(),
        &mut iov,
        Some(&mut control),
        MsgFlags::empty(),
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
    if declared > wire::MAX_FRAME_SIZE {
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

fn close_received_fds(fds: &[RawFd]) {
    for fd in fds {
        let _ = unistd::close(*fd);
    }
}

/// Test-only peer-credential injection. The accept path
/// ([`authorize_peer`]) reads the connecting peer's identity from
/// `SO_PEERCRED`; the accept-loop tests need to drive `handle_connection`
/// over an in-process socketpair while pretending the peer is a specific
/// launcher/admin uid. Rather than mutate process-global env (which is
/// `unsafe` under edition 2024) this is injected through a `#[cfg(test)]`
/// `Mutex`. In non-test builds it is compiled out and always `None`, so the
/// production accept path has no test backdoor at all.
#[cfg(test)]
static TEST_PEER_OVERRIDE: std::sync::Mutex<Option<PeerOverride>> = std::sync::Mutex::new(None);

/// Serializes the accept-loop tests that inject a [`PeerOverride`] so two of
/// them cannot interleave on the process-global injection slot.
#[cfg(test)]
static TEST_PEER_OVERRIDE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
fn peer_override_injected() -> Option<PeerOverride> {
    TEST_PEER_OVERRIDE
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
}

#[cfg(not(test))]
fn peer_override_injected() -> Option<PeerOverride> {
    None
}

/// Read a peer-credential override from the `NIXLINGD_TEST_PEER_*` env vars.
/// Used by integration tests that spawn the real daemon binary and pass these
/// via `Command::env`; reading env is safe under edition 2024. Returns `None`
/// (the normal production case) when `NIXLINGD_TEST_PEER_UID` is unset.
fn peer_override_from_env() -> Result<Option<PeerOverride>, TypedError> {
    let uid = match std::env::var("NIXLINGD_TEST_PEER_UID") {
        Ok(value) => value
            .parse::<u32>()
            .map_err(|err| TypedError::InternalConfig {
                detail: format!("NIXLINGD_TEST_PEER_UID: {err}"),
            })?,
        Err(std::env::VarError::NotPresent) => return Ok(None),
        Err(err) => {
            return Err(TypedError::InternalConfig {
                detail: format!("NIXLINGD_TEST_PEER_UID: {err}"),
            });
        }
    };
    let gid = match std::env::var("NIXLINGD_TEST_PEER_GID") {
        Ok(value) => value
            .parse::<u32>()
            .map_err(|err| TypedError::InternalConfig {
                detail: format!("NIXLINGD_TEST_PEER_GID: {err}"),
            })?,
        Err(std::env::VarError::NotPresent) => uid,
        Err(err) => {
            return Err(TypedError::InternalConfig {
                detail: format!("NIXLINGD_TEST_PEER_GID: {err}"),
            });
        }
    };
    let username = std::env::var("NIXLINGD_TEST_PEER_USERNAME").ok();
    let groups = std::env::var("NIXLINGD_TEST_PEER_GROUPS")
        .ok()
        .map(|value| {
            value
                .split(',')
                .filter(|part| !part.is_empty())
                .map(|part| part.to_owned())
                .collect::<Vec<_>>()
        });
    Ok(Some(PeerOverride {
        uid,
        gid,
        username,
        groups,
    }))
}

fn io_wrap(context: &'static str) -> impl FnOnce(nix::errno::Errno) -> TypedError {
    move |err| TypedError::InternalIo {
        context: context.to_owned(),
        detail: err.to_string(),
    }
}

#[cfg(test)]
mod runtime_acl_tests {
    //! Regression tests for the public-socket ACL + lock-parent shape
    //! under the non-root daemon contract.
    //!
    //! Coverage of the production deployment topology
    //! (`User=nixlingd`, `SupplementaryGroups=nixling`,
    //! tmpfile `d /run/nixling 0750 nixlingd nixling -`,
    //! socket `mode 0660 group nixling`) is split across
    //! these focused unit tests because the real system identities
    //! (`nixlingd`, `nixling`) only exist on the deployed
    //! NixOS host. Here we simulate `expect_root_owned_parent=true`
    //! with the caller's own uid+gid so the chown succeeds under
    //! `cargo test`, and assert the produced shape (owner / group /
    //! mode) matches what the production deployment will produce.

    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::PathBuf;

    use nix::unistd::{self, Gid, Uid};

    use super::{RuntimeIdentity, bind_public_socket, validate_lock_parent};

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nixlingd-runtime-acl-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default(),
        ));
        fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn caller_identity(expect_root_owned_parent: bool) -> RuntimeIdentity {
        RuntimeIdentity {
            daemon_uid: unistd::getuid(),
            daemon_gid: unistd::getgid(),
            public_socket_gid: unistd::getgid(),
            expect_root_owned_parent,
        }
    }

    /// Pick a supplementary group different from the caller's primary gid
    /// so we can prove the
    /// `expect_root_owned_parent=true` chgrp actually mutated the
    /// socket's gid. The caller is a member of every group `getgroups`
    /// returns, so `chown(None, Some(supp_gid))` is permitted by POSIX.
    /// Returns `None` when the runtime has only the primary gid (e.g.
    /// inside minimal CI containers); the caller skips the assertion in
    /// that case with a visible log line so the gap is documented
    /// rather than silently passing.
    fn distinct_supplementary_gid() -> Option<Gid> {
        let primary = unistd::getgid();
        let groups = match unistd::getgroups() {
            Ok(groups) => groups,
            Err(err) => {
                eprintln!("runtime_acl_tests: getgroups failed: {err}; cannot pick supp gid");
                return None;
            }
        };
        groups.into_iter().find(|&gid| gid != primary)
    }

    #[test]
    fn bind_public_socket_chgrps_to_public_socket_gid_even_when_non_root() {
        // Under the production unit the daemon never runs as root,
        // so the previous `if geteuid().is_root()` gate around the
        // chown left the socket with group `nixlingd` instead of
        // `nixling`. With the gate removed and `chown(path,
        // None, Some(public_socket_gid))`, the socket must always
        // pick up the requested group when
        // `expect_root_owned_parent` is true.
        //
        // The assertion is only meaningful if the socket's natural
        // (umask-inherited) gid differs from
        // `public_socket_gid`; otherwise a regression that silently
        // re-introduces the `is_root()` gate could pass the test
        // because the socket would already carry the expected gid by
        // inheritance. Pick a supplementary group that differs from
        // the caller's primary gid and use it as the public socket
        // gid. POSIX permits a non-root file owner to chown to any
        // group they belong to (real, effective, or supplementary),
        // so the chown succeeds; if `bind_public_socket` ever skips
        // it under non-root, the socket keeps the primary gid and
        // the assertion fails.
        let Some(supp_gid) = distinct_supplementary_gid() else {
            eprintln!(
                "bind_public_socket_chgrps_to_public_socket_gid_even_when_non_root: \
                 caller has no supplementary gid distinct from primary; \
                 skipping the strict chgrp regression (see runtime_acl_tests docstring)"
            );
            return;
        };

        let dir = scratch_dir("bind-chgrp");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o750))
            .expect("chmod scratch dir 0750");
        let socket_path = dir.join("public.sock");

        let identity = RuntimeIdentity {
            daemon_uid: unistd::getuid(),
            daemon_gid: unistd::getgid(),
            public_socket_gid: supp_gid,
            expect_root_owned_parent: true,
        };
        let _socket = bind_public_socket(&socket_path, &identity).expect("bind public socket");

        let meta = fs::symlink_metadata(&socket_path).expect("stat socket");
        assert_ne!(
            unistd::getgid().as_raw(),
            supp_gid.as_raw(),
            "supp_gid {} must differ from primary gid {} for this test to be meaningful",
            supp_gid,
            unistd::getgid()
        );
        assert_eq!(
            meta.gid(),
            supp_gid.as_raw(),
            "public socket group must equal public_socket_gid={supp_gid:?} under \
             expect_root_owned_parent=true; got gid={} (matches primary={})",
            meta.gid(),
            unistd::getgid()
        );
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o660,
            "public socket mode must be 0660, got 0{:o}",
            mode
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bind_public_socket_skips_chown_in_test_mode() {
        // The test-only path (`expect_root_owned_parent=false`) must
        // skip the chown so plain `cargo test` runs that do not
        // belong to the production socket group still succeed. The
        // socket inherits the caller's primary gid via the default
        // umask path.
        let dir = scratch_dir("bind-test-skip");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755))
            .expect("chmod scratch dir 0755");
        let socket_path = dir.join("public.sock");

        let identity = caller_identity(false);
        let _socket = bind_public_socket(&socket_path, &identity).expect("bind public socket");

        let meta = fs::symlink_metadata(&socket_path).expect("stat socket");
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o660,
            "public socket mode must be 0660 in test mode too"
        );
        // We do NOT assert gid here: the test path intentionally
        // skips chown and inherits whatever the umask gave us.
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn validate_lock_parent_accepts_production_tmpfile_shape() {
        // Production tmpfile: `d /run/nixling 0750 nixlingd
        // nixling -`. With expect_root_owned_parent=true,
        // the validator now expects (daemon_uid, public_socket_gid,
        // 0o750) — i.e. the daemon's own uid + the public socket
        // group + mode 0750, not the old (0, 0, 0755) root-owned
        // shape.
        let dir = scratch_dir("validate-prod");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o750))
            .expect("chmod scratch dir 0750");
        // We are the owner; gid is our primary gid. To exercise the
        // production semantics, point public_socket_gid at our gid
        // so the validator sees a match.
        let identity = caller_identity(true);
        let lock_path = dir.join("daemon.lock");
        validate_lock_parent(&lock_path, &identity).expect(
            "validator must accept (caller_uid, caller_gid, 0750) under expect_root_owned_parent",
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn validate_lock_parent_rejects_wrong_mode_in_production() {
        // 0o700 (the old `/run/nixling/locks` mode) is not acceptable
        // for `/run/nixling` itself because launcher users could not
        // traverse it. The validator must reject the wrong mode.
        let dir = scratch_dir("validate-bad-mode");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
            .expect("chmod scratch dir 0700");
        let identity = caller_identity(true);
        let lock_path = dir.join("daemon.lock");
        let err = validate_lock_parent(&lock_path, &identity)
            .expect_err("validator must reject mode 0o700 for the public socket parent");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("0700") || msg.contains("mode"),
            "error message must mention the mismatched mode; got {msg}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn validate_lock_parent_test_mode_accepts_either_0755_or_0750() {
        // Test mode (`expect_root_owned_parent=false`) accepts both
        // 0o755 and 0o750 because ad-hoc cargo-test scratch dirs may
        // be created with either depending on the caller's umask.
        for mode in [0o755u32, 0o750u32] {
            let dir = scratch_dir(&format!("validate-test-mode-{mode:o}"));
            fs::set_permissions(&dir, fs::Permissions::from_mode(mode)).expect("chmod scratch dir");
            let identity = caller_identity(false);
            let lock_path = dir.join("daemon.lock");
            validate_lock_parent(&lock_path, &identity).unwrap_or_else(|err| {
                panic!("validator must accept mode 0{mode:o} in test mode: {err:?}")
            });
            fs::remove_dir_all(&dir).ok();
        }
    }

    // Silence "unused import" when the file's imports are otherwise
    // visible only to non-test code.
    #[allow(dead_code)]
    fn _ensure_types_in_scope(_: Uid, _: Gid) {}
}

#[cfg(test)]
mod detached_exec_routing_tests {
    use super::supervisor::pidfd_table::{BrokerReapLog, PidfdTable};
    use super::*;

    use nix::sys::socket::{AddressFamily, SockFlag, SockType, socketpair};
    use nixling_ipc::guest_proto as pb;
    use nixling_ipc::guest_wire::ExecState;
    use nixling_ipc::public_wire::{
        ExecDetachedKillOutcome, ExecDetachedKillResult, ExecDetachedListEntry,
        ExecDetachedListResult, ExecDetachedLogsResult, ExecDetachedStatusResult, ExecOp,
        ExecStartArgs,
    };
    use protobuf::EnumOrUnknown;
    use serde_json::Value;
    use std::sync::Arc;

    fn test_state(caps: exec_session::ExecSessionCaps) -> ServerState {
        let broker_reap_log = BrokerReapLog::new();
        let temp_root = tempfile::Builder::new()
            .prefix("nixlingd-detached-tests.")
            .tempdir()
            .expect("temp detached test root");
        let test_root = temp_root.path().to_path_buf();
        std::mem::forget(temp_root);
        std::fs::create_dir_all(&test_root).expect("create detached test root");
        let public_manifest_path = test_root.join("vms.json");
        std::fs::write(
            &public_manifest_path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "_manifest": { "manifestVersion": 6 },
                "_observability": {
                    "enabled": false,
                    "vmName": "sys-obs",
                    "obsVsockCid": 1000,
                    "obsVsockHostSocket": "/var/lib/nixling/vms/sys-obs/vsock.sock",
                    "signozUrl": "http://127.0.0.1:3301",
                    "signozOtlpGrpcPort": 4317,
                    "signozOtlpHttpPort": 4318
                },
                "work": {
                    "name": "work",
                    "env": "work",
                    "staticIp": "10.20.0.10",
                    "sshUser": "alice",
                    "isNetVm": false,
                    "stateDir": "/var/lib/nixling/vms/work",
                    "apiSocket": "/var/lib/nixling/vms/work/work.sock",
                    "audioService": null,
                    "audioStateFile": null,
                    "bridge": "br-work-lan",
                    "gpuSocket": null,
                    "netVm": "sys-work-net",
                    "tap": "work-l10",
                    "tpmSocket": null,
                    "usbipdHostIp": null,
                    "graphics": false,
                    "tpm": false,
                    "usbipYubikey": false,
                    "audio": false,
                    "observability": {
                        "enabled": false,
                        "vsockCid": 4096,
                        "vsockHostSocket": "/var/lib/nixling/vms/work/vsock.sock",
                        "agentSocket": "/run/nixling/otlp.sock"
                    },
                    "runtime": {
                        "kind": "nixos",
                        "provider": {
                            "driver": "cloud-hypervisor",
                            "id": "local-cloud-hypervisor",
                            "type": "local"
                        },
                        "capabilities": {
                            "lifecycle": true,
                            "display": false,
                            "usbHotplug": false,
                            "guestControl": true,
                            "exec": true,
                            "configSync": true,
                            "ssh": true,
                            "storeSync": true,
                            "keys": true,
                            "inGuestObservability": true
                        }
                    }
                }
            }))
            .expect("detached manifest json"),
        )
        .expect("write detached manifest");
        let config = DaemonConfig {
            artifacts: ArtifactPaths {
                public_manifest_path,
                ..ArtifactPaths::default()
            },
            ..DaemonConfig::default()
        };
        ServerState {
            config,
            daemon_uid: 0,
            daemon_audit: Arc::new(daemon_audit::DaemonAuditLog::no_op()),
            daemon_state_dir: test_root.join("state"),
            pidfd_table: Arc::new(
                PidfdTable::new(test_root.join("pidfd.json"))
                    .with_broker_reap_log(Arc::clone(&broker_reap_log)),
            ),
            broker_reap_log,
            metrics_registry: Arc::new(metrics::Registry::new()),
            exec_sessions: Arc::new(exec_session::SessionTable::new(caps)),
            gateway_display: crate::new_gateway_display_runtime(),
            conn_semaphore: crate::concurrency::ConnSemaphore::new(8),
            op_locks: crate::concurrency::OpLockManager::new(),
        }
    }

    fn admin_peer() -> PeerIdentity {
        PeerIdentity {
            role: PeerRole::Admin,
            uid: 4242,
        }
    }

    fn launcher_peer() -> PeerIdentity {
        PeerIdentity {
            role: PeerRole::Launcher,
            uid: 1000,
        }
    }

    fn seqpacket_pair() -> (Socket, Socket) {
        let (a, b) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::empty(),
        )
        .expect("seqpacket socketpair");
        (Socket::from(a), Socket::from(b))
    }

    fn recv_reply(socket: &Socket) -> Value {
        let bytes = read_frame(socket).expect("client reads reply");
        serde_json::from_slice(&bytes).expect("reply is JSON")
    }

    fn detached_start(argv0: &str) -> ExecOp {
        ExecOp::Start(ExecStartArgs {
            vm: "work".to_owned(),
            argv: vec![argv0.to_owned()],
            tty: false,
            detached: true,
            env: Some(vec![public_wire::ExecEnvVar {
                key: "SENTINEL_ENV_KEY".to_owned(),
                value: "SENTINEL_ENV_VALUE".to_owned(),
            }]),
            cwd: Some("SENTINEL_CWD".to_owned()),
            term_size: None,
        })
    }

    fn hello_frame() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "type": "hello",
            "clientVersion": ">=0.4.0, <0.5.0",
        }))
        .expect("encode hello frame")
    }

    fn exec_frame(op_id: u64, op: &ExecOp) -> Vec<u8> {
        let mut value = serde_json::to_value(op).expect("encode exec op");
        let object = value.as_object_mut().expect("exec op object");
        object.insert("type".to_owned(), serde_json::json!("exec"));
        object.insert("opId".to_owned(), serde_json::json!(op_id));
        serde_json::to_vec(&value).expect("serialize exec frame")
    }

    struct PeerOverrideEnv {
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl PeerOverrideEnv {
        fn launcher() -> Self {
            let lock = TEST_PEER_OVERRIDE_LOCK
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            *TEST_PEER_OVERRIDE.lock().unwrap_or_else(|p| p.into_inner()) = Some(PeerOverride {
                uid: 1000,
                gid: 1000,
                username: Some("launcher".to_owned()),
                groups: Some(Vec::new()),
            });
            Self { _lock: lock }
        }
    }

    impl Drop for PeerOverrideEnv {
        fn drop(&mut self) {
            *TEST_PEER_OVERRIDE.lock().unwrap_or_else(|p| p.into_inner()) = None;
        }
    }

    #[test]
    fn detached_create_is_one_shot_and_does_not_reserve_attached_session() {
        let caps = exec_session::ExecSessionCaps {
            global: 0,
            ..exec_session::ExecSessionCaps::default()
        };
        let state = test_state(caps);
        let hook = Arc::new(|request| {
            assert_eq!(
                request,
                exec_detached::DetachedTestRequest::Create {
                    vm: "work".to_owned(),
                    argv_len: 1,
                    env_len: 1,
                    has_cwd: true,
                }
            );
            Ok(exec_detached::DetachedTestResponse::Create(
                public_wire::ExecDetachedCreateResult {
                    exec_id: "exec-detached-1".to_owned(),
                    state: ExecState::Running,
                },
            ))
        });
        let _guard = exec_detached::set_test_hook(hook);
        let (daemon, client) = seqpacket_pair();
        let run_state = state.clone();
        let handle = std::thread::spawn(move || {
            run_exec_owner(
                daemon,
                run_state,
                admin_peer(),
                77,
                detached_start("SENTINEL_ARGV"),
                None,
            );
        });

        let reply = recv_reply(&client);
        handle.join().expect("detached owner returns");
        assert_eq!(reply["type"], "execResponse");
        assert_eq!(reply["opId"], 77);
        assert_eq!(reply["op"], "detachedCreate");
        assert_eq!(reply["result"]["execId"], "exec-detached-1");
        assert_eq!(reply["result"]["state"], "running");
        assert_eq!(
            state.exec_sessions.len(),
            0,
            "detached create must not reserve an attached session slot"
        );

        let records = state.daemon_audit.captured.lock().expect("audit capture");
        assert_eq!(records.len(), 1, "detached create writes one audit event");
        assert!(!records[0].contains("SENTINEL_ARGV"));
        assert!(!records[0].contains("SENTINEL_ENV"));
        assert!(!records[0].contains("SENTINEL_CWD"));
        let record: Value = serde_json::from_str(&records[0]).expect("parse audit");
        assert_eq!(
            record["event"]["kind"].as_str(),
            Some("guest_control_exec_detached_create")
        );
        assert_eq!(record["event"]["vm"].as_str(), Some("work"));
        assert_eq!(record["event"]["peer_uid"].as_u64(), Some(4242));
        assert_eq!(record["event"]["action"].as_str(), Some("create"));
        assert_eq!(record["event"]["result"].as_str(), Some("created"));
        assert_eq!(record["event"]["exec_id"].as_str(), Some("exec-detached-1"));
    }

    #[test]
    fn detached_create_denies_launcher_before_owner_backend_or_session_table() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let _env = PeerOverrideEnv::launcher();
        let mut state = test_state(exec_session::ExecSessionCaps::default());
        state.config.launcher_users = vec!["launcher".to_owned()];
        state.config.admin_users = vec![];
        let touched = Arc::new(AtomicUsize::new(0));
        let hook_touched = Arc::clone(&touched);
        let _guard = exec_detached::set_test_hook(Arc::new(move |request| {
            hook_touched.fetch_add(1, Ordering::SeqCst);
            panic!("launcher denial must not touch detached create backend: {request:?}");
        }));
        let (daemon, client) = seqpacket_pair();
        let run_state = state.clone();
        let handle = std::thread::spawn(move || handle_connection(daemon, &run_state, None));

        write_frame(&client, &hello_frame()).expect("client sends hello");
        let hello_ok = recv_reply(&client);
        assert_eq!(hello_ok["type"], "helloOk");
        write_frame(&client, &exec_frame(88, &detached_start("true")))
            .expect("client sends detached create");
        let reply = recv_reply(&client);
        drop(client);
        handle
            .join()
            .expect("daemon thread joins")
            .expect("connection exits after client EOF");

        assert_eq!(reply["type"], "error");
        assert_eq!(reply["error"]["kind"], "authz-not-admin");
        assert_eq!(reply["error"]["exitCode"], 75);
        assert_eq!(state.exec_sessions.len(), 0);
        assert_eq!(
            touched.load(Ordering::SeqCst),
            0,
            "admin gate must short-circuit before detached create backend"
        );
    }

    #[test]
    fn exec_detached_not_advertised_surfaces_clear_error() {
        let caps = vec![
            EnumOrUnknown::new(pb::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED),
            EnumOrUnknown::new(pb::GuestCapability::GUEST_CAPABILITY_EXEC_LOGS),
        ];
        let err = exec_detached::gate_detached_capabilities(&caps)
            .expect_err("missing EXEC_DETACHED must fail");
        let typed = map_exec_op_error(err);
        assert_eq!(typed.kind(), "guest-control-exec-detached-unavailable");
        assert_eq!(typed.exit_code(), 70);
        assert!(typed.message().contains("detached exec"));
    }

    fn management_success_hook() -> exec_detached::DetachedTestHook {
        Arc::new(|request| match request {
            exec_detached::DetachedTestRequest::List { vm } => {
                assert_eq!(vm, "work");
                Ok(exec_detached::DetachedTestResponse::List(
                    ExecDetachedListResult {
                        execs: vec![ExecDetachedListEntry {
                            exec_id: "exec-1".to_owned(),
                            state: ExecState::Running,
                            exit_code: None,
                            signal: None,
                            started_at: "1700000001".to_owned(),
                            start_offset: 1,
                            end_offset: 9,
                            stdout_start_offset: 1,
                            stdout_end_offset: 5,
                            stderr_start_offset: 2,
                            stderr_end_offset: 9,
                            dropped_bytes: 3,
                            stdout_dropped_bytes: 1,
                            stderr_dropped_bytes: 2,
                            truncated: true,
                            stdout_truncated: false,
                            stderr_truncated: true,
                        }],
                    },
                ))
            }
            exec_detached::DetachedTestRequest::Logs {
                vm,
                exec_id,
                stdout_offset,
                stderr_offset,
                max_len,
            } => {
                assert_eq!((vm.as_str(), exec_id.as_str()), ("work", "exec-1"));
                assert_eq!(stdout_offset, None);
                assert_eq!(stderr_offset, None);
                assert_eq!(max_len, None);
                Ok(exec_detached::DetachedTestResponse::Logs(
                    ExecDetachedLogsResult {
                        exec_id,
                        stdout_base64: "b3V0".to_owned(),
                        stderr_base64: "ZXJy".to_owned(),
                        start_offset: 1,
                        end_offset: 9,
                        dropped_bytes: 4,
                        truncated: true,
                        stdout_start_offset: 1,
                        stdout_end_offset: 5,
                        stdout_next_offset: 5,
                        stdout_eof: true,
                        stdout_dropped_bytes: 1,
                        stdout_truncated: false,
                        stderr_start_offset: 2,
                        stderr_end_offset: 9,
                        stderr_next_offset: 7,
                        stderr_eof: false,
                        stderr_dropped_bytes: 3,
                        stderr_truncated: true,
                    },
                ))
            }
            exec_detached::DetachedTestRequest::Status { vm, exec_id } => {
                assert_eq!((vm.as_str(), exec_id.as_str()), ("work", "exec-1"));
                Ok(exec_detached::DetachedTestResponse::Status(
                    ExecDetachedStatusResult {
                        exec_id,
                        state: ExecState::Exited,
                        reason: None,
                        exit_code: Some(0),
                        signal: None,
                        start_offset: 1,
                        end_offset: 9,
                        dropped_bytes: 4,
                        truncated: true,
                    },
                ))
            }
            exec_detached::DetachedTestRequest::Kill { vm, exec_id } => {
                assert_eq!((vm.as_str(), exec_id.as_str()), ("work", "exec-1"));
                Ok(exec_detached::DetachedTestResponse::Kill(
                    ExecDetachedKillResult {
                        exec_id,
                        result: ExecDetachedKillOutcome::Cancelling,
                        state: ExecState::Cancelled,
                    },
                ))
            }
            exec_detached::DetachedTestRequest::Create { .. } => {
                panic!("management hook should not receive create")
            }
        })
    }

    #[test]
    fn management_verbs_route_to_one_shot_detached_backend() {
        let state = test_state(exec_session::ExecSessionCaps::default());
        let _guard = exec_detached::set_test_hook(management_success_hook());
        let peer = admin_peer();

        let list = dispatch_request(
            &state,
            &peer,
            wire::Request::Exec(ExecOp::List(public_wire::ExecDetachedListArgs {
                vm: "work".to_owned(),
            })),
        )
        .expect("list dispatch succeeds");
        assert_eq!(list["type"], "execResponse");
        assert_eq!(list["op"], "list");
        assert_eq!(list["result"]["execs"][0]["execId"], "exec-1");
        assert_eq!(list["result"]["execs"][0]["startOffset"], 1);
        assert_eq!(list["result"]["execs"][0]["endOffset"], 9);
        assert_eq!(list["result"]["execs"][0]["stdoutStartOffset"], 1);
        assert_eq!(list["result"]["execs"][0]["stdoutEndOffset"], 5);
        assert_eq!(list["result"]["execs"][0]["stderrStartOffset"], 2);
        assert_eq!(list["result"]["execs"][0]["stderrEndOffset"], 9);
        assert_eq!(list["result"]["execs"][0]["droppedBytes"], 3);
        assert_eq!(list["result"]["execs"][0]["stdoutDroppedBytes"], 1);
        assert_eq!(list["result"]["execs"][0]["stderrDroppedBytes"], 2);
        assert_eq!(list["result"]["execs"][0]["truncated"], true);
        assert_eq!(list["result"]["execs"][0]["stdoutTruncated"], false);
        assert_eq!(list["result"]["execs"][0]["stderrTruncated"], true);
        assert!(!list.to_string().contains("argv"));

        let logs = dispatch_request(
            &state,
            &peer,
            wire::Request::Exec(ExecOp::Logs(public_wire::ExecDetachedLogsArgs {
                vm: "work".to_owned(),
                exec_id: "exec-1".to_owned(),
                stdout_offset: None,
                stderr_offset: None,
                max_len: None,
            })),
        )
        .expect("logs dispatch succeeds");
        assert_eq!(logs["op"], "logs");
        assert_eq!(logs["result"]["stdoutBase64"], "b3V0");
        assert_eq!(logs["result"]["stderrBase64"], "ZXJy");
        assert_eq!(logs["result"]["startOffset"], 1);
        assert_eq!(logs["result"]["endOffset"], 9);
        assert_eq!(logs["result"]["droppedBytes"], 4);
        assert_eq!(logs["result"]["truncated"], true);
        assert_eq!(logs["result"]["stdoutStartOffset"], 1);
        assert_eq!(logs["result"]["stdoutEndOffset"], 5);
        assert_eq!(logs["result"]["stdoutNextOffset"], 5);
        assert_eq!(logs["result"]["stdoutEof"], true);
        assert_eq!(logs["result"]["stdoutDroppedBytes"], 1);
        assert_eq!(logs["result"]["stdoutTruncated"], false);
        assert_eq!(logs["result"]["stderrStartOffset"], 2);
        assert_eq!(logs["result"]["stderrEndOffset"], 9);
        assert_eq!(logs["result"]["stderrNextOffset"], 7);
        assert_eq!(logs["result"]["stderrEof"], false);
        assert_eq!(logs["result"]["stderrDroppedBytes"], 3);
        assert_eq!(logs["result"]["stderrTruncated"], true);

        let status = dispatch_request(
            &state,
            &peer,
            wire::Request::Exec(ExecOp::Status(public_wire::ExecDetachedStatusArgs {
                vm: "work".to_owned(),
                exec_id: "exec-1".to_owned(),
            })),
        )
        .expect("status dispatch succeeds");
        assert_eq!(status["op"], "status");
        assert_eq!(status["result"]["state"], "exited");
        assert_eq!(status["result"]["exitCode"], 0);
        assert_eq!(status["result"]["startOffset"], 1);
        assert_eq!(status["result"]["endOffset"], 9);
        assert_eq!(status["result"]["droppedBytes"], 4);
        assert_eq!(status["result"]["truncated"], true);

        let kill = dispatch_request(
            &state,
            &peer,
            wire::Request::Exec(ExecOp::Kill(public_wire::ExecDetachedKillArgs {
                vm: "work".to_owned(),
                exec_id: "exec-1".to_owned(),
            })),
        )
        .expect("kill dispatch succeeds");
        assert_eq!(kill["op"], "kill");
        assert_eq!(kill["result"]["result"], "cancelling");
        assert_eq!(kill["result"]["state"], "cancelled");

        let records = state.daemon_audit.captured.lock().expect("audit capture");
        assert_eq!(records.len(), 1, "kill writes one audit event");
        let record: Value = serde_json::from_str(&records[0]).expect("parse kill audit");
        assert_eq!(
            record["event"]["kind"].as_str(),
            Some("guest_control_exec_detached_kill")
        );
        assert_eq!(record["event"]["vm"].as_str(), Some("work"));
        assert_eq!(record["event"]["peer_uid"].as_u64(), Some(4242));
        assert_eq!(record["event"]["action"].as_str(), Some("cancel"));
        assert_eq!(record["event"]["result"].as_str(), Some("cancelling"));
        assert_eq!(record["event"]["exec_id"].as_str(), Some("exec-1"));
    }

    #[test]
    fn detached_kill_duplicate_maps_already_terminal_and_audits_idempotent_result() {
        let state = test_state(exec_session::ExecSessionCaps::default());
        let _guard = exec_detached::set_test_hook(Arc::new(|request| match request {
            exec_detached::DetachedTestRequest::Kill { vm, exec_id } => {
                assert_eq!((vm.as_str(), exec_id.as_str()), ("work", "exec-1"));
                Ok(exec_detached::DetachedTestResponse::Kill(
                    ExecDetachedKillResult {
                        exec_id,
                        result: ExecDetachedKillOutcome::AlreadyTerminal,
                        state: ExecState::Exited,
                    },
                ))
            }
            other => panic!("unexpected detached backend request: {other:?}"),
        }));

        let kill = dispatch_request(
            &state,
            &admin_peer(),
            wire::Request::Exec(ExecOp::Kill(public_wire::ExecDetachedKillArgs {
                vm: "work".to_owned(),
                exec_id: "exec-1".to_owned(),
            })),
        )
        .expect("kill dispatch succeeds");
        assert_eq!(kill["op"], "kill");
        assert_eq!(kill["result"]["result"], "already-terminal");
        assert_eq!(kill["result"]["state"], "exited");

        let records = state.daemon_audit.captured.lock().expect("audit capture");
        assert_eq!(records.len(), 1, "kill writes one audit event");
        let record: Value = serde_json::from_str(&records[0]).expect("parse kill audit");
        assert_eq!(record["event"]["result"].as_str(), Some("already-terminal"));
        assert_eq!(record["event"]["exec_id"].as_str(), Some("exec-1"));
    }

    #[test]
    fn detached_kill_error_audit_redacts_unvalidated_caller_exec_id() {
        use crate::typed_error::GuestControlExecErrorKind as K;

        let state = test_state(exec_session::ExecSessionCaps::default());
        let _guard = exec_detached::set_test_hook(Arc::new(|request| match request {
            exec_detached::DetachedTestRequest::Kill { vm, exec_id } => {
                assert_eq!(vm, "work");
                assert_eq!(exec_id, "SENTINEL_UNVALIDATED_EXEC_ID\nwith-control");
                Err(TypedError::GuestControlExecFailed {
                    kind: K::ExecNotFound,
                })
            }
            other => panic!("unexpected detached backend request: {other:?}"),
        }));

        let err = dispatch_request(
            &state,
            &admin_peer(),
            wire::Request::Exec(ExecOp::Kill(public_wire::ExecDetachedKillArgs {
                vm: "work".to_owned(),
                exec_id: "SENTINEL_UNVALIDATED_EXEC_ID\nwith-control".to_owned(),
            })),
        )
        .expect_err("kill should surface backend error");
        assert_eq!(err.kind(), "guest-control-exec-not-found");

        let records = state.daemon_audit.captured.lock().expect("audit capture");
        assert_eq!(records.len(), 1, "kill errors still write one audit event");
        assert!(
            !records[0].contains("SENTINEL_UNVALIDATED_EXEC_ID"),
            "audit must not persist unvalidated caller exec_id: {}",
            records[0]
        );
        let record: Value = serde_json::from_str(&records[0]).expect("parse kill audit");
        assert_eq!(record["event"]["result"].as_str(), Some("error"));
        assert_eq!(
            record["event"]["exec_id"].as_str(),
            Some("<redacted-on-error>")
        );
    }

    fn management_ops() -> Vec<ExecOp> {
        vec![
            ExecOp::List(public_wire::ExecDetachedListArgs {
                vm: "work".to_owned(),
            }),
            ExecOp::Logs(public_wire::ExecDetachedLogsArgs {
                vm: "work".to_owned(),
                exec_id: "exec-1".to_owned(),
                stdout_offset: None,
                stderr_offset: None,
                max_len: None,
            }),
            ExecOp::Status(public_wire::ExecDetachedStatusArgs {
                vm: "work".to_owned(),
                exec_id: "exec-1".to_owned(),
            }),
            ExecOp::Kill(public_wire::ExecDetachedKillArgs {
                vm: "work".to_owned(),
                exec_id: "exec-1".to_owned(),
            }),
        ]
    }

    fn detached_create_and_management_ops() -> Vec<ExecOp> {
        let mut ops = vec![detached_start("true")];
        ops.extend(management_ops());
        ops
    }

    #[test]
    fn detached_exec_ops_deny_launcher_before_backend_or_session_table() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let state = test_state(exec_session::ExecSessionCaps::default());
        let touched = Arc::new(AtomicUsize::new(0));
        let hook_touched = Arc::clone(&touched);
        let _guard = exec_detached::set_test_hook(Arc::new(move |request| {
            hook_touched.fetch_add(1, Ordering::SeqCst);
            panic!("launcher denial must not touch detached backend: {request:?}");
        }));

        for op in detached_create_and_management_ops() {
            let err = dispatch_request(&state, &launcher_peer(), wire::Request::Exec(op))
                .expect_err("launcher must be denied before exec backend");
            match &err {
                TypedError::AuthzNotAdmin { verb } => assert_eq!(verb, "exec"),
                other => panic!("expected AuthzNotAdmin for exec, got {other:?}"),
            }
            assert_eq!(err.exit_code(), 75);
            assert_eq!(state.exec_sessions.len(), 0);
        }
        assert_eq!(
            touched.load(Ordering::SeqCst),
            0,
            "admin gate must short-circuit before detached backend"
        );
    }

    #[test]
    fn management_verbs_preserve_typed_guest_error_mapping() {
        use crate::typed_error::GuestControlExecErrorKind as K;
        for expected in [
            K::StaleSession,
            K::ExecNotFound,
            K::ExecExpired,
            K::Protocol,
        ] {
            for op in management_ops() {
                let state = test_state(exec_session::ExecSessionCaps::default());
                let _guard = exec_detached::set_test_hook(Arc::new(move |_| {
                    Err(TypedError::GuestControlExecFailed { kind: expected })
                }));
                let err = dispatch_request(&state, &admin_peer(), wire::Request::Exec(op))
                    .expect_err("management op should surface typed error");
                assert_eq!(err.kind(), expected.wire_kind());
            }
        }
    }

    #[test]
    fn invalid_program_maps_to_actionable_exec_error() {
        let mut guest_error = pb::GuestControlError::new();
        guest_error.kind =
            EnumOrUnknown::new(pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_INVALID_PROGRAM);
        let op_error = exec_session_real::map_guest_control_error(&guest_error);
        assert_eq!(
            op_error,
            exec_session::ExecOpError::Guest(exec_session::GuestOpError::InvalidProgram)
        );

        let typed = map_exec_op_error(op_error);
        assert_eq!(typed.kind(), "guest-control-invalid-program");
        assert_eq!(typed.exit_code(), 2);
        assert!(
            typed.message().contains(
                "command must be a program name or absolute path and must not start with '-'"
            ),
            "unexpected message: {}",
            typed.message()
        );
        assert!(
            !typed.remediation().contains("already exited"),
            "invalid-program remediation must not use stale exec wording"
        );

        let establish = map_exec_establish_error(exec_session::ExecEstablishError::Guest(
            exec_session::GuestOpError::InvalidProgram,
        ));
        assert_eq!(establish.kind(), "guest-control-invalid-program");
        assert_eq!(establish.message(), typed.message());
    }
}

/// The public.sock accept loop is serial: it accepts one connection, runs
/// `handle_connection`, then accepts the next. An exec session's owner
/// connection is long-lived, so `handle_connection` MUST hand the exec session
/// off to a spawned owner thread and return immediately — otherwise the single
/// accept loop would be pinned for the entire lifetime of one exec session and
/// no other client could be served. These hermetic tests drive
/// `handle_connection` over a real `SOCK_SEQPACKET` pair (no live VM) and prove
/// the exec branch spawns-and-returns while a second request is still served.
#[cfg(test)]
mod accept_loop_concurrency_tests {
    use super::supervisor::pidfd_table::{BrokerReapLog, PidfdTable};
    use super::*;

    use std::sync::mpsc;
    use std::time::Duration;

    use nixling_ipc::public_wire::{ExecOp, ExecStartArgs};
    use serde_json::json;
    use tempfile::TempDir;

    fn seqpacket_pair() -> (Socket, Socket) {
        use nix::sys::socket::{AddressFamily, SockFlag, SockType, socketpair};
        let (a, b) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::empty(),
        )
        .expect("seqpacket socketpair");
        (Socket::from(a), Socket::from(b))
    }

    /// A `ServerState` whose config admits a single admin/launcher principal so
    /// the test SO_PEERCRED override resolves to `PeerRole::Admin` (exec
    /// requires admin). Returns the live `TempDir` so it outlives the daemon.
    fn admin_exec_state() -> (ServerState, TempDir) {
        let dir = tempfile::tempdir().expect("daemon state dir");
        let broker_reap_log = BrokerReapLog::new();
        let config = DaemonConfig {
            launcher_users: vec!["execadmin".to_owned()],
            admin_users: vec!["execadmin".to_owned()],
            ..DaemonConfig::default()
        };
        let state = ServerState {
            config,
            daemon_uid: 0,
            daemon_audit: Arc::new(daemon_audit::DaemonAuditLog::no_op()),
            daemon_state_dir: dir.path().to_path_buf(),
            pidfd_table: Arc::new(
                PidfdTable::new(dir.path().join("pidfd-table.json"))
                    .with_broker_reap_log(Arc::clone(&broker_reap_log)),
            ),
            broker_reap_log,
            metrics_registry: Arc::new(metrics::Registry::new()),
            exec_sessions: Arc::new(exec_session::SessionTable::new(
                exec_session::ExecSessionCaps::default(),
            )),
            gateway_display: crate::new_gateway_display_runtime(),
            conn_semaphore: crate::concurrency::ConnSemaphore::new(8),
            op_locks: crate::concurrency::OpLockManager::new(),
        };
        (state, dir)
    }

    fn admin_peer_identity() -> PeerIdentity {
        PeerIdentity {
            role: PeerRole::Admin,
            uid: 4242,
        }
    }

    fn hello_frame() -> Vec<u8> {
        serde_json::to_vec(&json!({
            "type": "hello",
            "clientVersion": ">=0.4.0, <0.5.0",
        }))
        .expect("encode hello frame")
    }

    fn exec_start_frame(op_id: u64) -> Vec<u8> {
        let op = ExecOp::Start(ExecStartArgs {
            vm: "work".to_owned(),
            argv: vec!["true".to_owned()],
            tty: false,
            detached: false,
            env: None,
            cwd: None,
            term_size: None,
        });
        let mut value = serde_json::to_value(&op).expect("encode exec op");
        let object = value.as_object_mut().expect("exec op object");
        object.insert("type".to_owned(), json!("exec"));
        object.insert("opId".to_owned(), json!(op_id));
        serde_json::to_vec(&value).expect("serialize exec frame")
    }

    /// Scoped guard for the test SO_PEERCRED override so a panic still clears
    /// the process-global injection slot. Only [`authorize_peer`] (via
    /// `handle_connection`) reads it, and only these accept-loop tests inject
    /// it; the guard also serializes those tests against each other.
    struct PeerOverrideEnv {
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl PeerOverrideEnv {
        fn admin() -> Self {
            let lock = TEST_PEER_OVERRIDE_LOCK
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            *TEST_PEER_OVERRIDE.lock().unwrap_or_else(|p| p.into_inner()) = Some(PeerOverride {
                uid: 4242,
                gid: 4242,
                username: Some("execadmin".to_owned()),
                groups: Some(Vec::new()),
            });
            Self { _lock: lock }
        }

        /// A peer whose username is in neither `launcher_users` nor
        /// `admin_users`, so `authorize_peer` rejects it with
        /// `AuthzNotALauncher` before any frame is read.
        fn denied() -> Self {
            let lock = TEST_PEER_OVERRIDE_LOCK
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            *TEST_PEER_OVERRIDE.lock().unwrap_or_else(|p| p.into_inner()) = Some(PeerOverride {
                uid: 9999,
                gid: 9999,
                username: Some("nobody-unlisted".to_owned()),
                groups: Some(Vec::new()),
            });
            Self { _lock: lock }
        }
    }

    impl Drop for PeerOverrideEnv {
        fn drop(&mut self) {
            *TEST_PEER_OVERRIDE.lock().unwrap_or_else(|p| p.into_inner()) = None;
        }
    }

    #[test]
    fn exec_dispatch_returns_to_the_accept_loop_and_a_second_request_is_served() {
        use std::sync::{Condvar, Mutex};

        let _env = PeerOverrideEnv::admin();
        let (state, _state_dir) = admin_exec_state();
        let state = Arc::new(state);

        // Install an owner-body hook that genuinely HOLDS the exec owner session
        // open: it flags `running`, signals `entered`, then blocks until the
        // test releases it. Because the hook runs at the top of
        // `run_exec_owner`, a hypothetical inline `handle_connection` would block
        // HERE on the accept-loop thread and never return — the watchdog below
        // would fire. An off-loop spawn (the real behaviour) returns Ok promptly
        // while this body is still blocked.
        #[derive(Default)]
        struct HookState {
            entered: bool,
            running: bool,
            release: bool,
        }
        let shared = Arc::new((Mutex::new(HookState::default()), Condvar::new()));

        // Clear the process-global hook on scope exit (panic-safe) so no other
        // test ever observes it.
        struct HookGuard;
        impl Drop for HookGuard {
            fn drop(&mut self) {
                exec_owner_test_hook::clear();
            }
        }
        let _hook_guard = HookGuard;

        let hook_shared = Arc::clone(&shared);
        let hook: exec_owner_test_hook::Hook = Arc::new(move || {
            let (lock, cv) = &*hook_shared;
            {
                let mut s = lock.lock().expect("hook state lock");
                s.entered = true;
                s.running = true;
                cv.notify_all();
            }
            let mut s = lock.lock().expect("hook state lock");
            while !s.release {
                s = cv.wait(s).expect("hook release wait");
            }
            s.running = false;
            cv.notify_all();
        });
        exec_owner_test_hook::set(hook);

        // --- Connection A: opens a long-lived exec owner session. ---
        let (server_a, client_a) = seqpacket_pair();
        // SOCK_SEQPACKET preserves message boundaries, so both datagrams can be
        // buffered before the daemon reads them.
        write_frame(&client_a, &hello_frame()).expect("client A sends hello");
        write_frame(&client_a, &exec_start_frame(1)).expect("client A sends exec start");
        // The client deliberately keeps its end OPEN for the session's lifetime.

        let state_a = Arc::clone(&state);
        let (done_tx, done_rx) = mpsc::channel();
        let handle_a = std::thread::spawn(move || {
            let result = handle_connection(server_a, &state_a, None);
            let _ = done_tx.send(result.is_ok());
        });

        // The owner body must actually be entered (the exec branch dispatched a
        // real, blocking owner session — not a fast-failed stub).
        {
            let (lock, cv) = &*shared;
            let mut s = lock.lock().expect("hook state lock");
            let deadline = Duration::from_secs(10);
            let start = std::time::Instant::now();
            while !s.entered {
                let remaining = deadline
                    .checked_sub(start.elapsed())
                    .expect("exec owner body was not entered within the deadline");
                let (guard, timeout) = cv.wait_timeout(s, remaining).expect("hook entered wait");
                s = guard;
                assert!(!timeout.timed_out(), "exec owner body was not entered");
            }
        }

        // handle_connection must have returned Ok PROMPTLY even though the owner
        // body is still blocked in the hook. An inline implementation would be
        // stuck in the hook on this very thread's predecessor and never send.
        let returned = done_rx.recv_timeout(Duration::from_secs(10));
        assert!(
            matches!(returned, Ok(true)),
            "handle_connection must spawn the exec owner off the serial accept \
             loop and return Ok promptly, not run the session inline (got \
             {returned:?})"
        );
        handle_a.join().expect("accept-loop thread joins");

        // Prove the owner session is STILL HELD OPEN (the body has not torn
        // down) at the moment handle_connection has already returned — i.e. the
        // dispatch was genuinely off-loop, concurrent with the accept loop.
        {
            let (lock, _cv) = &*shared;
            let s = lock.lock().expect("hook state lock");
            assert!(
                s.running && !s.release,
                "the exec owner session must still be held open after \
                 handle_connection returned (off-loop dispatch)"
            );
        }

        // --- Connection B: a SECOND public.sock request is accepted and served
        //     while connection A's owner session is still held open. ---
        let (server_b, client_b) = seqpacket_pair();
        let client_b = std::thread::spawn(move || {
            write_frame(&client_b, &hello_frame()).expect("client B sends hello");
            let hello_ok = read_frame(&client_b).expect("client B reads helloOk");
            let hello_ok: serde_json::Value =
                serde_json::from_slice(&hello_ok).expect("helloOk is JSON");
            assert_eq!(hello_ok["type"], "helloOk", "second connection negotiates");
            write_frame(
                &client_b,
                &serde_json::to_vec(&json!({ "type": "authStatus" })).unwrap(),
            )
            .expect("client B sends authStatus");
            let response = read_frame(&client_b).expect("client B reads response");
            let response: serde_json::Value =
                serde_json::from_slice(&response).expect("response is JSON");
            // Dropping client_b on return signals EOF so handle_connection ends.
            response
        });

        let result_b = handle_connection(server_b, &state, None);
        assert!(
            result_b.is_ok(),
            "the second connection must be served while the exec owner is held \
             open: {result_b:?}"
        );
        let response = client_b.join().expect("client B thread joins");
        assert_eq!(
            response["type"], "authStatusResponse",
            "second request was actually served, not merely accepted"
        );

        // Release the held owner body and wait for it to finish, so the spawned
        // owner thread is wound down before the test returns.
        {
            let (lock, cv) = &*shared;
            let mut s = lock.lock().expect("hook state lock");
            s.release = true;
            cv.notify_all();
            while s.running {
                s = cv.wait(s).expect("hook finish wait");
            }
        }

        // Connection A's owner session is torn down with its client end.
        drop(client_a);
    }

    /// fix2b: SO_PEERCRED authorization runs in `handle_connection` BEFORE the
    /// hello frame is read. A denied peer that never sends a hello must still
    /// be rejected promptly (it cannot stall a handler waiting on a read), and
    /// the rejection is the typed `AuthzNotALauncher` envelope.
    #[test]
    fn unauthorized_peer_is_rejected_before_reading_hello() {
        let _env = PeerOverrideEnv::denied();
        let (state, _state_dir) = admin_exec_state();

        let (server, client) = seqpacket_pair();
        // The client deliberately sends NO hello frame. If authz ran after the
        // read, the handler would block here; authz-first returns promptly.
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            let result = handle_connection(server, &state, None);
            let _ = tx.send(result);
        });

        let outcome = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("handle_connection must reject the peer before any read");
        assert!(
            matches!(outcome, Err(TypedError::AuthzNotALauncher { .. })),
            "unauthorized peer must be rejected with AuthzNotALauncher, got {outcome:?}"
        );

        // The handler also wrote a helloRejected frame the client can observe.
        let frame = read_frame(&client).expect("client reads rejection frame");
        let value: serde_json::Value =
            serde_json::from_slice(&frame).expect("rejection frame is JSON");
        assert_eq!(value["type"], "helloRejected");
        handle.join().expect("handler thread joins");
        drop(client);
    }

    /// fix2b: the admission permit moved into a handler is released when the
    /// handler returns, on BOTH the success path (clean EOF) and the error
    /// path (malformed hello). After each handler returns the in-flight count
    /// drops back to zero, so a refused slot is never leaked.
    #[test]
    fn admission_permit_is_released_on_handler_success_and_error() {
        let _env = PeerOverrideEnv::admin();
        let (state, _state_dir) = admin_exec_state();
        assert_eq!(state.conn_semaphore.in_flight(), 0);

        // --- Success path: hello, helloOk read by client, then EOF. The
        //     handler runs on a worker thread; a client thread reads the
        //     helloOk frame then closes, so the request-loop read sees a clean
        //     EOF and the handler returns Ok. ---
        {
            let permit = state
                .conn_semaphore
                .try_acquire()
                .expect("cap admits first permit");
            assert_eq!(state.conn_semaphore.in_flight(), 1);
            let (server, client) = seqpacket_pair();
            std::thread::scope(|scope| {
                let client_thread = scope.spawn(move || {
                    write_frame(&client, &hello_frame()).expect("send hello");
                    let frame = read_frame(&client).expect("client reads helloOk");
                    let value: serde_json::Value =
                        serde_json::from_slice(&frame).expect("helloOk is JSON");
                    assert_eq!(value["type"], "helloOk");
                    // Drop the client end -> the handler's next read sees EOF.
                    drop(client);
                });
                let result = handle_connection_authorized(
                    server,
                    &state,
                    admin_peer_identity(),
                    Some(permit),
                );
                client_thread.join().expect("client thread joins");
                assert!(result.is_ok(), "clean EOF handler returns Ok: {result:?}");
            });
            assert_eq!(
                state.conn_semaphore.in_flight(),
                0,
                "permit released after success"
            );
        }

        // --- Error path: a malformed hello -> handler returns Err. ---
        {
            let permit = state
                .conn_semaphore
                .try_acquire()
                .expect("cap admits permit again");
            assert_eq!(state.conn_semaphore.in_flight(), 1);
            let (server, client) = seqpacket_pair();
            write_frame(&client, b"{not valid json").expect("send malformed hello");
            let result =
                handle_connection_authorized(server, &state, admin_peer_identity(), Some(permit));
            assert!(result.is_err(), "malformed hello handler returns Err");
            drop(client);
            assert_eq!(
                state.conn_semaphore.in_flight(),
                0,
                "permit released after error"
            );
        }
    }

    /// fix2b: when the in-flight cap is saturated the accept loop refuses the
    /// connection with the typed `DaemonBusy` envelope. The refusal is
    /// non-fatal to the daemon (it maps to the broker-error exit contract, not
    /// exit 1) and carries the stable `daemon-busy` kind.
    #[test]
    fn daemon_busy_refusal_frame_is_typed_and_nonfatal() {
        let busy = TypedError::DaemonBusy;
        let frame = wire::error_frame(&busy);
        let value = serde_json::to_value(&frame).expect("encode busy frame");
        assert_eq!(value["type"], "error");
        assert_eq!(
            value["error"]["kind"], "daemon-busy",
            "busy refusal carries the stable daemon-busy kind"
        );
        assert_eq!(
            busy.exit_code(),
            75,
            "DaemonBusy maps to the broker-error exit contract, not exit 1"
        );
    }

    /// fix2b: a saturated semaphore refuses further admissions with `None`
    /// (non-blocking), and a released permit re-opens a slot. This is the
    /// admission decision the accept loop makes before spawning a handler.
    #[test]
    fn semaphore_refuses_at_cap_then_readmits_after_release() {
        let sem = crate::concurrency::ConnSemaphore::new(2);
        let p1 = sem.try_acquire().expect("first admit");
        let p2 = sem.try_acquire().expect("second admit");
        assert_eq!(sem.in_flight(), 2);
        assert!(
            sem.try_acquire().is_none(),
            "cap-hit must refuse without blocking"
        );
        drop(p1);
        let p3 = sem.try_acquire().expect("slot reopened after release");
        assert_eq!(sem.in_flight(), 2);
        drop(p2);
        drop(p3);
        assert_eq!(sem.in_flight(), 0);
    }
}

#[cfg(test)]
mod broker_dispatch_tests {
    use std::fs::File;
    use std::io::{self, IoSlice, Read, Write};
    use std::net::TcpListener;
    use std::os::fd::{AsRawFd, RawFd};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use std::{fs, thread};

    use nix::sys::socket::{
        AddressFamily, Backlog, ControlMessage, MsgFlags, SockFlag, SockType, UnixAddr, accept4,
        bind, listen, recv, sendmsg, socket,
    };
    use nix::unistd::close;
    use nixling_core::processes::ProcessRole;
    use nixling_ipc::broker_wire::{
        BrokerRequest, BrokerRequestEnvelope, BrokerResponse, ChildExitKind, ChildExitStatus,
        ChildReapedNotification, DeregisterRunnerPidfdResponse, PollChildReapedResponse,
        RunnerRole, RunnerSignal, SignalRunnerResponse, SpawnRunnerResponse,
    };
    use nixling_ipc::public_wire::{
        ActivationRequest, GcRequest, HostDestroyRequest, HostInstallRequest, HostPrepareRequest,
        KeysRotateRequest, MigrateRequest, MutationFlags, RotateKnownHostRequest, TrustRequest,
        VmLifecycleRequest,
    };
    use nixling_ipc::types::{RoleId, VmId};
    use serde::Serialize;
    use serde_json::json;

    use super::supervisor::pidfd_table::{
        BrokerReapLog, PidfdEntry, PidfdRegistration, PidfdTable, WaitTermination,
        force_signal_eperm_for_tests,
    };
    use super::supervisor::state::{
        FilesystemSnapshotStore, PidfdOpener, ProcReader, RunnerSnapshotRecord, SnapshotStore,
        parse_proc_stat_starttime,
    };
    use super::{
        ArtifactPaths, DaemonConfig, PeerIdentity, PeerRole, ServerState, VM_RUNNER_ROLE_ID,
        VmStartNodeMode, adopt_orphaned_runners_on_startup_with, daemon_audit,
        dispatch_broker_boot, dispatch_broker_gc, dispatch_broker_host_destroy,
        dispatch_broker_host_prepare, dispatch_broker_keys_rotate, dispatch_broker_rollback,
        dispatch_broker_rotate_known_host, dispatch_broker_run_host_install,
        dispatch_broker_run_migrate, dispatch_broker_switch, dispatch_broker_test,
        dispatch_broker_trust, dispatch_broker_vm_restart, dispatch_broker_vm_start,
        dispatch_broker_vm_stop, dispatch_broker_vm_stop_with_timeout, dispatch_request,
        redact_broker_dispatch_failure_for_launcher, redact_broker_error_for_launcher,
        resolve_store_view_intent_for_vm, stale_qemu_media_dependency_roles_from_entries,
        vm_start_node_mode,
    };

    static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(0);

    struct ChildGuard {
        child: Child,
    }

    impl ChildGuard {
        fn new(child: Child) -> Self {
            Self { child }
        }

        fn child(&self) -> &Child {
            &self.child
        }

        fn wait(mut self) -> std::process::ExitStatus {
            self.child.wait().expect("wait child")
        }
    }

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if let Ok(None) = self.child.try_wait() {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
    }

    fn test_daemon_state_dir(test_name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("t");
        fs::create_dir_all(&dir).expect("create broker dispatch scratch dir");
        let state_dir = dir.join(format!(
            "{test_name}-{}-{}",
            std::process::id(),
            NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&state_dir).expect("create daemon state dir");
        state_dir
    }

    fn test_state_with_broker_socket(path: PathBuf) -> ServerState {
        let daemon_state_dir = test_daemon_state_dir("broker-socket");
        let broker_reap_log = BrokerReapLog::new();
        ServerState {
            config: DaemonConfig {
                broker_socket_path: path,
                artifacts: write_minimal_vm_start_bundle_artifacts(&daemon_state_dir),
                ..DaemonConfig::default()
            },
            daemon_uid: 0,
            daemon_audit: Arc::new(daemon_audit::DaemonAuditLog::no_op()),
            daemon_state_dir: daemon_state_dir.clone(),
            pidfd_table: Arc::new(
                PidfdTable::new(daemon_state_dir.join("pidfd-table.json"))
                    .with_broker_reap_log(Arc::clone(&broker_reap_log)),
            ),
            broker_reap_log,
            metrics_registry: Arc::new(crate::metrics::Registry::new()),
            exec_sessions: Arc::new(crate::exec_session::SessionTable::new(
                crate::exec_session::ExecSessionCaps::default(),
            )),
            gateway_display: crate::new_gateway_display_runtime(),
            conn_semaphore: crate::concurrency::ConnSemaphore::new(8),
            op_locks: crate::concurrency::OpLockManager::new(),
        }
    }

    fn test_state_with_broker_socket_and_host(path: PathBuf, host_path: PathBuf) -> ServerState {
        let daemon_state_dir = test_daemon_state_dir("broker-host");
        let broker_reap_log = BrokerReapLog::new();
        ServerState {
            config: DaemonConfig {
                broker_socket_path: path,
                artifacts: ArtifactPaths {
                    host_path,
                    ..ArtifactPaths::default()
                },
                ..DaemonConfig::default()
            },
            daemon_uid: 0,
            daemon_audit: Arc::new(daemon_audit::DaemonAuditLog::no_op()),
            daemon_state_dir: daemon_state_dir.clone(),
            pidfd_table: Arc::new(
                PidfdTable::new(daemon_state_dir.join("pidfd-table.json"))
                    .with_broker_reap_log(Arc::clone(&broker_reap_log)),
            ),
            broker_reap_log,
            metrics_registry: Arc::new(crate::metrics::Registry::new()),
            exec_sessions: Arc::new(crate::exec_session::SessionTable::new(
                crate::exec_session::ExecSessionCaps::default(),
            )),
            gateway_display: crate::new_gateway_display_runtime(),
            conn_semaphore: crate::concurrency::ConnSemaphore::new(8),
            op_locks: crate::concurrency::OpLockManager::new(),
        }
    }

    fn unreachable_broker_socket_path(test_name: &str) -> PathBuf {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .or_else(|| {
                let candidate = PathBuf::from("/run/user")
                    .join(nix::unistd::Uid::current().as_raw().to_string());
                candidate.exists().then_some(candidate)
            })
            .or_else(|| std::env::var_os("CARGO_TARGET_TMPDIR").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));
        let mut hasher = DefaultHasher::new();
        test_name.hash(&mut hasher);
        let digest = hasher.finish();
        base.join(format!("nl-{digest:016x}-{}.sock", std::process::id()))
    }

    fn host_fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/deny-unknown/host-valid.json")
    }

    fn write_json_file(path: &Path, value: &serde_json::Value) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create json parent");
        }
        fs::write(
            path,
            serde_json::to_vec_pretty(value).expect("serialize json fixture"),
        )
        .expect("write json fixture");
    }

    fn minimal_role_profile(profile_id: &str, subtree: &str) -> serde_json::Value {
        json!({
            "profileId": profile_id,
            "uid": 0,
            "gid": 0,
            "adr_carve_out": null,
            "caps": [],
            "namespaces": {
                "mount": false,
                "pid": false,
                "net": false,
                "ipc": false,
                "uts": false,
                "user": false
            },
            "seccompPolicyRef": null,
            "mountPolicy": {
                "readOnlyPaths": [],
                "writablePaths": [],
                "nixStoreReadOnly": true,
                "hideDeviceNodesByDefault": true
            },
            "cgroupPlacement": {
                "subtree": subtree,
                "controllers": [],
                "delegated": false
            }
        })
    }

    fn write_minimal_vm_start_bundle_artifacts(root: &Path) -> ArtifactPaths {
        let bundle_dir = root.join("bundle-fixture");
        fs::create_dir_all(&bundle_dir).expect("create bundle fixture dir");
        let manifest_path = bundle_dir.join("vms.json");
        let processes_path = bundle_dir.join("processes.json");
        let bundle_path = bundle_dir.join("bundle.json");
        let privileges_path = bundle_dir.join("privileges.json");
        let api_socket = root.join("vm-a.api.sock");

        write_json_file(
            &manifest_path,
            &json!({
                "_manifest": { "manifestVersion": 6 },
                "_observability": {
                    "enabled": false,
                    "signozUrl": "http://127.0.0.1:8080",
                    "signozOtlpGrpcPort": 4317,
                    "signozOtlpHttpPort": 4318,
                    "obsVsockCid": 7,
                    "obsVsockHostSocket": "/run/nixling/obs.sock",
                    "vmName": "obs"
                },
                "vm-a": {
                    "apiSocket": api_socket.display().to_string(),
                    "audio": false,
                    "audioService": "nixling-vm-a-snd.service",
                    "audioStateFile": "/var/lib/nixling/vms/vm-a/state/audio-state.json",
                    "bridge": null,
                    "env": "dev",
                    "gpuSocket": "/run/nixling-gpu/vm-a/gpu.sock",
                    "graphics": false,
                    "isNetVm": false,
                    "name": "vm-a",
                    "netVm": null,
                    "observability": {
                        "agentSocket": "/run/nixling/vms/vm-a/otel.sock",
                        "enabled": false,
                        "vsockCid": 0,
                        "vsockHostSocket": "/run/nixling/otel.sock"
                    },
                    "runtime": {
                        "kind": "nixos",
                        "provider": {
                            "id": "local-cloud-hypervisor",
                            "type": "local",
                            "driver": "cloud-hypervisor"
                        },
                        "capabilities": {
                            "lifecycle": true,
                            "display": true,
                            "usbHotplug": true,
                            "guestControl": true,
                            "exec": true,
                            "configSync": true,
                            "ssh": true,
                            "storeSync": true,
                            "keys": true,
                            "inGuestObservability": true
                        }
                    },
                    "sshUser": "alice",
                    "stateDir": "/var/lib/nixling/vms/vm-a",
                    "staticIp": "127.0.0.1",
                    "tap": "nl-vm-a",
                    "tpm": false,
                    "tpmSocket": "/run/swtpm/vm-a/swtpm.sock",
                    "usbipYubikey": false,
                    "usbipdHostIp": null
                }
            }),
        );

        write_json_file(
            &processes_path,
            &json!({
                "schemaVersion": "v2",
                "vms": [
                    {
                        "vm": "vm-a",
                        "nodes": [
                            {
                                "id": "cloud-hypervisor",
                                "role": "cloud-hypervisor-runner",
                                "unit": null,
                                "profile": minimal_role_profile("vm-vm-a-cloud-hypervisor", "nixling.slice/vm-a/cloud-hypervisor"),
                                "readiness": []
                            }
                        ],
                        "edges": [],
                        "invariants": {
                            "swtpmPreStartFlush": true,
                            "perVmAuditPipeline": true,
                            "usbipGating": true,
                            "tpmOwnershipMigrationWithoutRunningVmMutation": true
                        }
                    }
                ]
            }),
        );
        write_json_file(
            &privileges_path,
            &json!({ "schemaVersion": "v2", "operations": [] }),
        );
        write_json_file(
            &bundle_path,
            &json!({
                "bundleVersion": 4,
                "schemaVersion": "v2",
                "publicManifestPath": manifest_path.display().to_string(),
                "hostPath": host_fixture_path().display().to_string(),
                "processesPath": processes_path.display().to_string(),
                "privilegesPath": privileges_path.display().to_string(),
                "closures": [],
                "minijailProfiles": [],
                "managedKeys": {
                    "keysDir": "/var/lib/nixling/keys",
                    "knownHostsPath": "/var/lib/nixling/known_hosts.nixling",
                    "overrides": []
                },
                "generation": {
                    "generator": "tests",
                    "sourceRevision": null,
                    "generatedAt": null
                }
            }),
        );

        ArtifactPaths {
            bundle_path,
            public_manifest_path: manifest_path,
            host_path: host_fixture_path(),
            processes_path,
            ..ArtifactPaths::default()
        }
    }

    #[test]
    fn vm_start_store_sync_resolves_bare_vm_name() {
        let root = test_daemon_state_dir("store-sync-resolves-bare-vm");
        let artifacts = write_minimal_vm_start_bundle_artifacts(&root);
        let bundle_dir = artifacts
            .bundle_path
            .parent()
            .expect("bundle path parent")
            .to_path_buf();
        let host_path = bundle_dir.join("host.json");
        fs::copy(host_fixture_path(), &host_path).expect("copy host fixture");
        let closure_path = bundle_dir.join("closures/vm-a.json");
        let fake_store = root.join("nix-store-mock");
        fs::create_dir_all(&fake_store).expect("create fake store");
        let toplevel = fake_store.join("aaaaaaaaaaaaaaaa-vm-a-system");
        fs::create_dir_all(&toplevel).expect("create fake toplevel");
        let db_dump = root.join("vm-a.db.dump");
        fs::write(&db_dump, b"db").expect("write db dump");
        write_json_file(
            &closure_path,
            &json!({
                "schemaVersion": "v2",
                "vm": "vm-a",
                "toplevel": toplevel.display().to_string(),
                "closurePaths": [toplevel.display().to_string()],
                "dbDumpPath": db_dump.display().to_string(),
                "declaredRunner": "/run/current-system/sw/bin/cloud-hypervisor",
                "runnerParityPath": "/run/current-system/sw/bin/cloud-hypervisor",
                "runnerParityOk": true,
                "generation": {
                    "hostGeneration": 7,
                    "vmGeneration": "7",
                    "sourceRevision": "test",
                    "generatedAt": "2026-01-01T00:00:00Z"
                }
            }),
        );
        let mut bundle_json: serde_json::Value =
            serde_json::from_slice(&fs::read(&artifacts.bundle_path).expect("read bundle"))
                .expect("parse bundle");
        bundle_json["schemaVersion"] = json!("v1");
        bundle_json["hostPath"] = json!(host_path.display().to_string());
        bundle_json["closures"] = json!([
            { "vm": "vm-a", "path": "closures/vm-a.json" }
        ]);
        write_json_file(&artifacts.bundle_path, &bundle_json);
        for path in [
            &artifacts.bundle_path,
            &artifacts.processes_path,
            &bundle_dir.join("privileges.json"),
            &host_path,
            &closure_path,
        ] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o640))
                .expect("chmod test bundle artifact");
        }
        let resolver = nixling_core::bundle_resolver::BundleResolver::load_with_policy(
            &artifacts.bundle_path,
            &nixling_core::bundle_resolver::BundleVerifyPolicy::for_tests(),
        )
        .expect("load resolver with store-view closure");

        let intent = resolve_store_view_intent_for_vm(&resolver, "vm-a")
            .expect("bare VM name resolves store-view intent");
        assert_eq!(
            intent.intent_id,
            nixling_core::bundle_resolver::intent_id_store_view("vm-a")
        );
        assert!(
            resolver
                .find_store_view_intent(&nixling_core::bundle_resolver::intent_id_store_view(
                    "vm-a"
                ))
                .is_none(),
            "passing a pre-wrapped store-view id double-wraps and must not be used"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[allow(dead_code)]
    fn write_custom_vm_start_bundle_artifacts(
        root: &Path,
        api_socket: &Path,
        processes: serde_json::Value,
    ) -> ArtifactPaths {
        let bundle_dir = root.join("bundle-fixture-custom");
        fs::create_dir_all(&bundle_dir).expect("create custom bundle fixture dir");
        let manifest_path = bundle_dir.join("vms.json");
        let processes_path = bundle_dir.join("processes.json");
        let bundle_path = bundle_dir.join("bundle.json");
        let privileges_path = bundle_dir.join("privileges.json");

        write_json_file(
            &manifest_path,
            &json!({
                "_manifest": { "manifestVersion": 6 },
                "_observability": {
                    "enabled": false,
                    "signozUrl": "http://127.0.0.1:8080",
                    "signozOtlpGrpcPort": 4317,
                    "signozOtlpHttpPort": 4318,
                    "obsVsockCid": 7,
                    "obsVsockHostSocket": "/run/nixling/obs.sock",
                    "vmName": "obs"
                },
                "vm-a": {
                    "apiSocket": api_socket.display().to_string(),
                    "audio": false,
                    "audioService": "nixling-vm-a-snd.service",
                    "audioStateFile": "/var/lib/nixling/vms/vm-a/state/audio-state.json",
                    "bridge": null,
                    "env": "dev",
                    "gpuSocket": "/run/nixling-gpu/vm-a/gpu.sock",
                    "graphics": false,
                    "isNetVm": false,
                    "name": "vm-a",
                    "netVm": null,
                    "observability": {
                        "agentSocket": "/run/nixling/vms/vm-a/otel.sock",
                        "enabled": false,
                        "vsockCid": 0,
                        "vsockHostSocket": "/run/nixling/otel.sock"
                    },
                    "runtime": {
                        "kind": "nixos",
                        "provider": {
                            "id": "local-cloud-hypervisor",
                            "type": "local",
                            "driver": "cloud-hypervisor"
                        },
                        "capabilities": {
                            "lifecycle": true,
                            "display": true,
                            "usbHotplug": true,
                            "guestControl": true,
                            "exec": true,
                            "configSync": true,
                            "ssh": true,
                            "storeSync": true,
                            "keys": true,
                            "inGuestObservability": true
                        }
                    },
                    "sshUser": "alice",
                    "stateDir": "/var/lib/nixling/vms/vm-a",
                    "staticIp": "127.0.0.1",
                    "tap": "nl-vm-a",
                    "tpm": false,
                    "tpmSocket": "/run/swtpm/vm-a/swtpm.sock",
                    "usbipYubikey": false,
                    "usbipdHostIp": null
                }
            }),
        );
        write_json_file(&processes_path, &processes);
        write_json_file(
            &privileges_path,
            &json!({ "schemaVersion": "v2", "operations": [] }),
        );
        write_json_file(
            &bundle_path,
            &json!({
                "bundleVersion": 4,
                "schemaVersion": "v2",
                "publicManifestPath": manifest_path.display().to_string(),
                "hostPath": host_fixture_path().display().to_string(),
                "processesPath": processes_path.display().to_string(),
                "privilegesPath": privileges_path.display().to_string(),
                "closures": [],
                "minijailProfiles": [],
                "managedKeys": {
                    "keysDir": "/var/lib/nixling/keys",
                    "knownHostsPath": "/var/lib/nixling/known_hosts.nixling",
                    "overrides": []
                },
                "generation": {
                    "generator": "tests",
                    "sourceRevision": null,
                    "generatedAt": null
                }
            }),
        );

        ArtifactPaths {
            bundle_path,
            public_manifest_path: manifest_path,
            host_path: host_fixture_path(),
            processes_path,
            ..ArtifactPaths::default()
        }
    }

    fn read_test_frame(fd: RawFd) -> io::Result<Vec<u8>> {
        let mut buffer = vec![0_u8; super::wire::MAX_FRAME_SIZE + 4];
        let received = recv(fd, &mut buffer, MsgFlags::empty())
            .map_err(|err| io::Error::from_raw_os_error(err as i32))?;
        if received < 4 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "short frame from seqpacket socket",
            ));
        }
        let expected = u32::from_le_bytes(buffer[..4].try_into().expect("frame prefix")) as usize;
        if expected > super::wire::MAX_FRAME_SIZE || expected + 4 > received {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "malformed seqpacket frame",
            ));
        }
        Ok(buffer[4..4 + expected].to_vec())
    }

    fn write_test_json_frame_with_fds<T: Serialize>(
        fd: RawFd,
        message: &T,
        fds: &[RawFd],
    ) -> io::Result<()> {
        let payload = serde_json::to_vec(message)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        if payload.len() > super::wire::MAX_FRAME_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "frame exceeds 1 MiB limit",
            ));
        }
        let mut frame = Vec::with_capacity(payload.len() + 4);
        frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        frame.extend_from_slice(&payload);
        let iov = [IoSlice::new(&frame)];
        let sent = if fds.is_empty() {
            sendmsg::<()>(fd, &iov, &[], MsgFlags::empty(), None)
        } else {
            let cmsgs = [ControlMessage::ScmRights(fds)];
            sendmsg::<()>(fd, &iov, &cmsgs, MsgFlags::empty(), None)
        }
        .map_err(|err| io::Error::from_raw_os_error(err as i32))?;
        if sent != frame.len() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "short write on seqpacket socket",
            ));
        }
        Ok(())
    }

    fn write_test_json_frame<T: Serialize>(fd: RawFd, message: &T) -> io::Result<()> {
        write_test_json_frame_with_fds(fd, message, &[])
    }

    fn start_test_broker_server<F>(
        test_name: &str,
        requests: usize,
        mut handler: F,
    ) -> (PathBuf, thread::JoinHandle<()>)
    where
        F: FnMut(usize, BrokerRequestEnvelope, RawFd) + Send + 'static,
    {
        let socket_path = unreachable_broker_socket_path(test_name);
        if let Some(parent) = socket_path.parent() {
            fs::create_dir_all(parent).expect("create broker socket parent");
        }
        fs::remove_file(&socket_path).ok();
        let listener = socket(
            AddressFamily::Unix,
            SockType::SeqPacket,
            SockFlag::SOCK_CLOEXEC,
            None,
        )
        .expect("create broker listener");
        let address = UnixAddr::new(&socket_path).expect("broker socket address");
        bind(listener.as_raw_fd(), &address).expect("bind broker listener");
        listen(&listener, Backlog::new(16).expect("listener backlog")).expect("listen broker");
        let server_socket_path = socket_path.clone();
        let join = thread::spawn(move || {
            for index in 0..requests {
                let accepted_fd = accept4(listener.as_raw_fd(), SockFlag::SOCK_CLOEXEC)
                    .expect("accept broker peer");
                let frame = read_test_frame(accepted_fd).expect("read broker request frame");
                let envelope: BrokerRequestEnvelope =
                    serde_json::from_slice(&frame).expect("decode broker request frame");
                handler(index, envelope, accepted_fd);
                close(accepted_fd).expect("close broker peer");
            }
            fs::remove_file(&server_socket_path).ok();
        });
        (socket_path, join)
    }

    /// The production `BrokerSigner` must forward a `GuestControlSign`
    /// request to the broker byte-for-byte (every field), not just the
    /// subset a `RecordingSigner` would observe in-process. This drives
    /// the real `dispatch_broker_request_to_socket` framing path against
    /// a fake seqpacket broker that records the decoded request.
    #[test]
    fn broker_signer_forwards_guest_control_sign_request_verbatim() {
        use crate::guest_control_bridge::{BrokerSigner, GUEST_CONTROL_ATTEMPT_CAP};
        use crate::guest_control_health::{AttemptBudget, GuestControlSigner};
        use nixling_ipc::broker_wire::{
            GuestBootIdWire, GuestControlAuthPurpose, GuestControlDirection, GuestControlProofRole,
            GuestControlSignRequest, GuestControlSignResponse,
        };
        use nixling_ipc::guest_auth::{AUTH_NONCE_LEN, GUEST_CONTROL_AUTH_PORT};
        use nixling_ipc::guest_wire::GUEST_CONTROL_PROTOCOL_VERSION;
        use nixling_ipc::types::VmId;

        let request = GuestControlSignRequest {
            vm_id: VmId::new("corp-vm"),
            role: GuestControlProofRole::GuestProof,
            protocol_version: GUEST_CONTROL_PROTOCOL_VERSION,
            direction: GuestControlDirection::HostToGuest,
            purpose: GuestControlAuthPurpose::GuestControlAuthV1,
            guest_control_port: GUEST_CONTROL_AUTH_PORT,
            peer_cid: Some(7),
            host_nonce: vec![0x11; AUTH_NONCE_LEN],
            guest_nonce: vec![0x22; AUTH_NONCE_LEN],
            guest_boot_id: GuestBootIdWire::new("boot-xyz"),
            capabilities_hash: Some("caps-sha256".to_owned()),
            tracing_span_id: None,
        };
        let expected = request.clone();
        let recorded: Arc<Mutex<Vec<GuestControlSignRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let recorded_server = Arc::clone(&recorded);
        let response_tag = vec![0xCDu8; 32];
        let response_tag_server = response_tag.clone();
        let (socket_path, broker) = start_test_broker_server(
            "guest-control-sign-verbatim",
            1,
            move |_, env, fd| match env.request {
                BrokerRequest::GuestControlSign(req) => {
                    recorded_server.lock().unwrap().push(req);
                    write_test_json_frame(
                        fd,
                        &BrokerResponse::GuestControlSign(GuestControlSignResponse {
                            tag: response_tag_server.clone(),
                        }),
                    )
                    .expect("write sign response");
                }
                other => panic!("unexpected broker request {other:?}"),
            },
        );
        let signer = BrokerSigner::new(
            socket_path,
            AttemptBudget::from_now(Duration::from_secs(10), GUEST_CONTROL_ATTEMPT_CAP),
        );
        let response = signer.sign(request).expect("broker signer succeeds");
        broker.join().expect("broker join");

        assert_eq!(response.tag, response_tag);
        let recorded = recorded.lock().unwrap();
        assert_eq!(recorded.len(), 1, "exactly one request forwarded");
        assert_eq!(
            recorded[0], expected,
            "BrokerSigner must forward every GuestControlSign field verbatim",
        );
    }

    #[test]
    fn broker_remaining_before_op_fails_closed_after_deadline() {
        // D1: the whole-round-trip deadline check returns the remaining
        // budget while time is left, and fails CLOSED with a broker
        // timeout (NOT a fresh op) once the deadline is reached, so no
        // doomed connect/write/read is issued past the caller's deadline.
        let path = Path::new("/run/nixling/priv.sock");
        let future = Instant::now() + Duration::from_secs(5);
        let remaining = super::broker_remaining_before_op(future, path)
            .expect("remaining must be positive before the deadline");
        assert!(remaining > Duration::ZERO);
        assert!(remaining <= Duration::from_secs(5));

        let past = Instant::now()
            .checked_sub(Duration::from_millis(1))
            .expect("instant has 1ms of headroom");
        let err = super::broker_remaining_before_op(past, path)
            .expect_err("a passed deadline must fail closed");
        assert!(matches!(
            err,
            crate::typed_error::TypedError::InternalBrokerTimeout { .. }
        ));
    }

    #[test]
    fn broker_signer_slow_broker_is_deadline_bounded_and_maps_to_timeout() {
        // D1: a stalled/backlogged broker must NOT let one sign exceed its
        // per-attempt deadline by multiples. The whole connect+write+read
        // round trip is bounded by the single slice the signer draws from
        // the shared absolute attempt budget; a deadline exhaustion
        // surfaces as Timeout (slug guest-control-timeout) end to end, not
        // a generic Signer failure. The fake broker reads the request then
        // holds the connection OPEN without responding so the client's
        // read blocks until its own deadline.
        use crate::guest_control_bridge::{BrokerSigner, GUEST_CONTROL_ATTEMPT_CAP};
        use crate::guest_control_health::{
            AttemptBudget, GuestControlHealthError, GuestControlSigner,
        };
        use nixling_ipc::broker_wire::{
            GuestBootIdWire, GuestControlAuthPurpose, GuestControlDirection, GuestControlProofRole,
            GuestControlSignRequest,
        };
        use nixling_ipc::guest_auth::{AUTH_NONCE_LEN, GUEST_CONTROL_AUTH_PORT};
        use nixling_ipc::guest_wire::GUEST_CONTROL_PROTOCOL_VERSION;
        use nixling_ipc::types::VmId;

        let attempt = Duration::from_millis(300);
        let broker_stall = Duration::from_millis(1500);
        let (socket_path, broker) =
            start_test_broker_server("guest-control-sign-slow", 1, move |_, env, _fd| {
                match env.request {
                    BrokerRequest::GuestControlSign(_) => {
                        // Keep the accepted connection open without a
                        // reply so the client's read times out at its own
                        // deadline, then drop it.
                        thread::sleep(broker_stall);
                    }
                    other => panic!("unexpected broker request {other:?}"),
                }
            });
        let signer = BrokerSigner::new(
            socket_path,
            AttemptBudget::from_now(attempt, GUEST_CONTROL_ATTEMPT_CAP),
        );
        let request = GuestControlSignRequest {
            vm_id: VmId::new("corp-vm"),
            role: GuestControlProofRole::HostProof,
            protocol_version: GUEST_CONTROL_PROTOCOL_VERSION,
            direction: GuestControlDirection::HostToGuest,
            purpose: GuestControlAuthPurpose::GuestControlAuthV1,
            guest_control_port: GUEST_CONTROL_AUTH_PORT,
            peer_cid: Some(crate::guest_control_bridge::VMADDR_CID_HOST),
            host_nonce: vec![0x11; AUTH_NONCE_LEN],
            guest_nonce: vec![0x22; AUTH_NONCE_LEN],
            guest_boot_id: GuestBootIdWire::new("boot-1"),
            capabilities_hash: None,
            tracing_span_id: None,
        };
        let started = Instant::now();
        let result = signer.sign(request);
        let elapsed = started.elapsed();
        broker.join().expect("broker join");

        assert_eq!(
            result,
            Err(GuestControlHealthError::Timeout),
            "a stalled broker sign must surface as Timeout, not Signer"
        );
        // The sign returned near its OWN deadline slice, NOT after the
        // (5x larger) broker stall: the round trip is deadline-bounded.
        assert!(
            elapsed < attempt * 3,
            "sign must be deadline-bounded (no multiples of the slice); took {elapsed:?}"
        );
    }

    fn read_child_start_time(child: &Child) -> u64 {
        let path = format!("/proc/{}/stat", child.id());
        let content = fs::read_to_string(&path).expect("read child stat");
        parse_proc_stat_starttime(&content).expect("parse child start time")
    }

    fn open_child_pidfd(child: &Child) -> std::os::fd::OwnedFd {
        rustix::process::pidfd_open(
            rustix::process::Pid::from_child(child),
            rustix::process::PidfdFlags::empty(),
        )
        .expect("pidfd_open child")
    }

    fn spawn_term_ignoring_child() -> Child {
        let helper_dir = test_daemon_state_dir("term-ignore-helper");
        let source = helper_dir.join("term-ignore.c");
        let binary = helper_dir.join("term-ignore");
        fs::write(
            &source,
            b"#include <signal.h>\n#include <string.h>\n#include <unistd.h>\nint main(void) { struct sigaction sa; memset(&sa, 0, sizeof(sa)); sa.sa_handler = SIG_IGN; sigemptyset(&sa.sa_mask); if (sigaction(SIGTERM, &sa, 0) != 0) return 2; for (;;) pause(); }\n",
        )
        .expect("write term-ignore helper source");
        let status = Command::new("gcc")
            .arg(&source)
            .arg("-O2")
            .arg("-o")
            .arg(&binary)
            .status()
            .expect("compile term-ignore helper");
        assert!(status.success(), "gcc must build the term-ignore helper");
        Command::new(&binary)
            .spawn()
            .expect("spawn term-ignoring helper")
    }

    fn register_sleep_runner_for_role(
        state: &ServerState,
        vm: &str,
        role_id: &str,
        role: RunnerRole,
        ignore_term: bool,
    ) -> ChildGuard {
        let child = if ignore_term {
            let child = spawn_term_ignoring_child();
            std::thread::sleep(std::time::Duration::from_millis(100));
            child
        } else {
            Command::new("sleep")
                .arg("600")
                .spawn()
                .expect("spawn sleep child")
        };
        let child = ChildGuard::new(child);
        let pid = child.child().id() as i32;
        let start_time_ticks = read_child_start_time(child.child());
        state
            .pidfd_table
            .register(
                vm.to_owned(),
                role_id.to_owned(),
                PidfdEntry {
                    pidfd: open_child_pidfd(child.child()),
                    pid,
                    start_time_ticks,
                },
            )
            .expect("register child pidfd");
        super::write_runner_snapshot(state, vm, role_id, role, pid, start_time_ticks)
            .expect("write runner snapshot");
        child
    }

    fn register_sleep_runner(state: &ServerState, vm: &str, ignore_term: bool) -> ChildGuard {
        register_sleep_runner_for_role(
            state,
            vm,
            VM_RUNNER_ROLE_ID,
            RunnerRole::CloudHypervisor,
            ignore_term,
        )
    }

    fn assert_redacted_broker_error(
        response: &serde_json::Value,
        verb: &str,
        expected_summary: &str,
        expected_remediation: &str,
    ) {
        assert_eq!(
            response.get("type").and_then(serde_json::Value::as_str),
            Some("mutatingVerbResponse")
        );
        assert_eq!(
            response.get("verb").and_then(serde_json::Value::as_str),
            Some(verb)
        );
        assert_eq!(
            response.get("outcome").and_then(serde_json::Value::as_str),
            Some("broker-error")
        );
        assert!(
            response
                .get("summary")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|summary| summary.starts_with(expected_summary))
        );
        assert_eq!(
            response
                .get("remediation")
                .and_then(serde_json::Value::as_str),
            Some(expected_remediation)
        );
        assert!(
            response
                .get("remediation")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|message| !message.contains("broker.sock"))
        );
    }

    #[test]
    fn redact_broker_error_for_launcher_covers_every_kind() {
        let kinds = [
            "Broker.BundleResolverUnavailable",
            "Broker.BundleIntentMissing",
            "Broker.StoreViewFilesystemMismatch",
            "Broker.StoreViewMarkerMissing",
            "Broker.LiveHandlerFailed",
            "Broker.CoexistenceRefused",
            "Broker.NftScriptParseFailed",
            "Broker.CarveoutOrderingViolation",
            "Broker.NftablesDriftDetected",
            "Broker.ValidateBundleFailed",
            "Broker.Protocol",
            "Broker.Unimplemented",
            "unknown-operation",
            "authz-audit-requires-admin",
        ];
        for kind in &kinds {
            let (summary, remediation) =
                redact_broker_error_for_launcher("Op", Some("op-15"), kind);
            assert!(!summary.is_empty(), "kind={kind} summary empty");
            assert!(!remediation.is_empty(), "kind={kind} remediation empty");
            for forbidden in &["/etc/", "/var/lib/", "systemctl", "execve", "Caused by"] {
                assert!(
                    !remediation.contains(forbidden),
                    "kind={kind} leaks privileged token {forbidden}: {remediation}"
                );
            }
        }
    }

    fn assert_unreachable_broker_response(response: serde_json::Value, verb: &str, op_name: &str) {
        let (_expected_summary, expected_remediation) =
            redact_broker_dispatch_failure_for_launcher(op_name);
        let expected_summary = format!("{op_name} failed");
        assert_eq!(
            response.get("type").and_then(serde_json::Value::as_str),
            Some("mutatingVerbResponse")
        );
        assert_eq!(
            response.get("verb").and_then(serde_json::Value::as_str),
            Some(verb)
        );
        assert_eq!(
            response.get("outcome").and_then(serde_json::Value::as_str),
            Some("broker-error")
        );
        assert_eq!(
            response.get("summary").and_then(serde_json::Value::as_str),
            Some(expected_summary.as_str())
        );
        assert_eq!(
            response
                .get("remediation")
                .and_then(serde_json::Value::as_str),
            Some(expected_remediation.as_str())
        );
        assert!(
            response
                .get("remediation")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|message| !message.contains("broker.sock"))
        );
    }

    fn normalize_verb(verb: &str) -> String {
        verb.chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .flat_map(|ch| ch.to_lowercase())
            .collect()
    }

    fn expected_mutating_response_verb(verb: &str) -> &str {
        match verb {
            "usbipBind" => "usb attach",
            "usbipUnbind" => "usb detach",
            other => other,
        }
    }

    fn destructive_mutating_requests() -> Vec<(&'static str, super::wire::Request)> {
        vec![
            (
                "vmStart",
                super::wire::Request::VmStart(VmLifecycleRequest {
                    vm: "vm-a".to_owned(),
                    flags: MutationFlags {
                        dry_run: true,
                        ..MutationFlags::default()
                    },
                    no_wait_api: false,
                }),
            ),
            (
                "vmStop",
                super::wire::Request::VmStop(VmLifecycleRequest {
                    vm: "vm-a".to_owned(),
                    flags: MutationFlags {
                        dry_run: true,
                        ..MutationFlags::default()
                    },
                    no_wait_api: false,
                }),
            ),
            (
                "vmRestart",
                super::wire::Request::VmRestart(VmLifecycleRequest {
                    vm: "vm-a".to_owned(),
                    flags: MutationFlags {
                        dry_run: true,
                        ..MutationFlags::default()
                    },
                    no_wait_api: false,
                }),
            ),
            (
                "switch",
                super::wire::Request::Switch(ActivationRequest {
                    vm: "vm-a".to_owned(),
                    flags: MutationFlags {
                        dry_run: true,
                        ..MutationFlags::default()
                    },
                }),
            ),
            (
                "boot",
                super::wire::Request::Boot(ActivationRequest {
                    vm: "vm-a".to_owned(),
                    flags: MutationFlags {
                        dry_run: true,
                        ..MutationFlags::default()
                    },
                }),
            ),
            (
                "test",
                super::wire::Request::Test(ActivationRequest {
                    vm: "vm-a".to_owned(),
                    flags: MutationFlags {
                        dry_run: true,
                        ..MutationFlags::default()
                    },
                }),
            ),
            (
                "rollback",
                super::wire::Request::Rollback(ActivationRequest {
                    vm: "vm-a".to_owned(),
                    flags: MutationFlags {
                        dry_run: true,
                        ..MutationFlags::default()
                    },
                }),
            ),
            (
                "gc",
                super::wire::Request::Gc(GcRequest {
                    flags: MutationFlags {
                        dry_run: true,
                        ..MutationFlags::default()
                    },
                    keep_generations: Some(2),
                }),
            ),
            (
                "keysRotate",
                super::wire::Request::KeysRotate(KeysRotateRequest {
                    vm: "vm-a".to_owned(),
                    flags: MutationFlags {
                        dry_run: true,
                        ..MutationFlags::default()
                    },
                }),
            ),
            (
                "trust",
                super::wire::Request::Trust(TrustRequest {
                    vm: "vm-a".to_owned(),
                    flags: MutationFlags {
                        dry_run: true,
                        ..MutationFlags::default()
                    },
                }),
            ),
            (
                "rotateKnownHost",
                super::wire::Request::RotateKnownHost(RotateKnownHostRequest {
                    vm: "vm-a".to_owned(),
                    flags: MutationFlags {
                        dry_run: true,
                        ..MutationFlags::default()
                    },
                }),
            ),
            (
                "usbipBind",
                super::wire::Request::UsbipBind(nixling_ipc::public_wire::UsbipBindCliRequest {
                    vm: "vm-a".to_owned(),
                    bus_id: "1-1".to_owned(),
                    flags: MutationFlags {
                        dry_run: true,
                        ..MutationFlags::default()
                    },
                }),
            ),
            (
                "usbipUnbind",
                super::wire::Request::UsbipUnbind(
                    nixling_ipc::public_wire::UsbipUnbindCliRequest {
                        vm: "vm-a".to_owned(),
                        bus_id: "1-1".to_owned(),
                        flags: MutationFlags {
                            dry_run: true,
                            ..MutationFlags::default()
                        },
                    },
                ),
            ),
            (
                "migrate",
                super::wire::Request::Migrate(MigrateRequest {
                    flags: MutationFlags {
                        dry_run: true,
                        ..MutationFlags::default()
                    },
                }),
            ),
            (
                "hostPrepare",
                super::wire::Request::HostPrepare(HostPrepareRequest {
                    flags: MutationFlags {
                        dry_run: true,
                        ..MutationFlags::default()
                    },
                }),
            ),
            (
                "hostDestroy",
                super::wire::Request::HostDestroy(HostDestroyRequest {
                    flags: MutationFlags {
                        dry_run: true,
                        ..MutationFlags::default()
                    },
                }),
            ),
            (
                "hostInstall",
                super::wire::Request::HostInstall(HostInstallRequest {
                    flags: MutationFlags {
                        dry_run: true,
                        ..MutationFlags::default()
                    },
                    ..HostInstallRequest::default()
                }),
            ),
        ]
    }

    fn launcher_peer() -> PeerIdentity {
        PeerIdentity {
            role: PeerRole::Launcher,
            uid: 1000,
        }
    }

    fn admin_peer() -> PeerIdentity {
        PeerIdentity {
            role: PeerRole::Admin,
            uid: 0,
        }
    }

    #[test]
    fn non_admin_peer_rejected_for_all_destructive_verbs() {
        let state = test_state_with_broker_socket(unreachable_broker_socket_path("authz-admin"));
        let peer = launcher_peer();
        let requests = destructive_mutating_requests();
        assert_eq!(
            requests.len(),
            17,
            "update destructive_mutating_requests when the mutating request surface changes"
        );
        for (verb, request) in requests {
            let err =
                dispatch_request(&state, &peer, request).expect_err("launcher must be denied");
            match &err {
                super::typed_error::TypedError::AuthzNotAdmin { verb: actual_verb } => {
                    assert_eq!(normalize_verb(actual_verb), normalize_verb(verb));
                }
                other => panic!("expected AuthzNotAdmin for {verb}, got {other:?}"),
            }
            assert_eq!(err.exit_code(), 75);
        }
    }

    #[test]
    fn admin_peer_can_reach_all_destructive_verbs() {
        let state = test_state_with_broker_socket(unreachable_broker_socket_path("authz-admin-ok"));
        let peer = admin_peer();
        let requests = destructive_mutating_requests();
        assert_eq!(
            requests.len(),
            17,
            "update destructive_mutating_requests when the mutating request surface changes"
        );
        for (verb, request) in requests {
            let response = dispatch_request(&state, &peer, request)
                .unwrap_or_else(|err| panic!("admin request {verb} unexpectedly failed: {err:?}"));
            assert_eq!(
                response.get("type").and_then(serde_json::Value::as_str),
                Some("mutatingVerbResponse")
            );
            assert_eq!(
                response
                    .get("verb")
                    .and_then(serde_json::Value::as_str)
                    .map(normalize_verb),
                Some(normalize_verb(expected_mutating_response_verb(verb)))
            );
            assert_eq!(
                response.get("outcome").and_then(serde_json::Value::as_str),
                Some("dry-run-planned")
            );
        }
    }

    #[test]
    fn vm_start_sidecar_roles_use_long_lived_runner_modes() {
        let cases = [
            (ProcessRole::Gpu, RunnerRole::Gpu),
            (ProcessRole::Audio, RunnerRole::Audio),
            (ProcessRole::Video, RunnerRole::Video),
            (ProcessRole::QemuMediaRunner, RunnerRole::QemuMedia),
            (ProcessRole::VsockRelay, RunnerRole::VsockRelay),
            (ProcessRole::Usbip, RunnerRole::Usbip),
            (ProcessRole::WaylandProxy, RunnerRole::WaylandProxy),
        ];
        for (role, expected_runner_role) in cases {
            match vm_start_node_mode(&role) {
                VmStartNodeMode::LongLived(actual) => assert_eq!(actual, expected_runner_role),
                other => panic!("expected LongLived for {role:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn qemu_media_stale_dependency_cleanup_detects_proxy_without_primary() {
        let tracked_roles = vec!["wayland-proxy".to_owned(), "qemu-media".to_owned()];
        let entries = vec![PidfdRegistration {
            vm: "dark-live".to_owned(),
            role: "wayland-proxy".to_owned(),
            pid: 42,
            start_time_ticks: 1,
        }];

        assert_eq!(
            stale_qemu_media_dependency_roles_from_entries(&tracked_roles, &entries),
            vec!["wayland-proxy".to_owned()]
        );
    }

    #[test]
    fn qemu_media_stale_dependency_cleanup_keeps_running_primary() {
        let tracked_roles = vec!["wayland-proxy".to_owned(), "qemu-media".to_owned()];
        let entries = vec![
            PidfdRegistration {
                vm: "dark-live".to_owned(),
                role: "wayland-proxy".to_owned(),
                pid: 42,
                start_time_ticks: 1,
            },
            PidfdRegistration {
                vm: "dark-live".to_owned(),
                role: "qemu-media".to_owned(),
                pid: 43,
                start_time_ticks: 1,
            },
        ];

        assert!(
            stale_qemu_media_dependency_roles_from_entries(&tracked_roles, &entries).is_empty()
        );
    }

    #[test]
    fn qemu_media_stale_dependency_cleanup_ignores_non_qemu_media_dags() {
        let tracked_roles = vec!["wayland-proxy".to_owned(), VM_RUNNER_ROLE_ID.to_owned()];
        let entries = vec![PidfdRegistration {
            vm: "work".to_owned(),
            role: "wayland-proxy".to_owned(),
            pid: 42,
            start_time_ticks: 1,
        }];

        assert!(
            stale_qemu_media_dependency_roles_from_entries(&tracked_roles, &entries).is_empty()
        );
    }

    #[test]
    fn host_install_broker_unreachable_returns_broker_error() {
        let response = dispatch_broker_run_host_install(
            &test_state_with_broker_socket(unreachable_broker_socket_path(
                "host-install-unreachable",
            )),
            HostInstallRequest {
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
                enable: true,
                no_start: true,
                ..HostInstallRequest::default()
            },
        )
        .expect("host install response");
        assert_unreachable_broker_response(response, "host install", "RunHostInstall");
    }

    #[test]
    fn migrate_broker_unreachable_returns_broker_error() {
        let response = dispatch_broker_run_migrate(
            &test_state_with_broker_socket(unreachable_broker_socket_path("migrate-unreachable")),
            MigrateRequest {
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
            },
        )
        .expect("migrate response");
        assert_unreachable_broker_response(response, "migrate", "RunMigrate");
    }

    #[test]
    fn switch_broker_unreachable_returns_broker_error() {
        let response = dispatch_broker_switch(
            &test_state_with_broker_socket(unreachable_broker_socket_path("switch-unreachable")),
            ActivationRequest {
                vm: "vm-a".to_owned(),
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
            },
        )
        .expect("switch response");
        assert_unreachable_broker_response(response, "switch", "RunActivation");
    }

    #[test]
    fn boot_broker_unreachable_returns_broker_error() {
        let response = dispatch_broker_boot(
            &test_state_with_broker_socket(unreachable_broker_socket_path("boot-unreachable")),
            ActivationRequest {
                vm: "vm-a".to_owned(),
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
            },
        )
        .expect("boot response");
        assert_unreachable_broker_response(response, "boot", "RunActivation");
    }

    #[test]
    fn test_broker_unreachable_returns_broker_error() {
        let response = dispatch_broker_test(
            &test_state_with_broker_socket(unreachable_broker_socket_path("test-unreachable")),
            ActivationRequest {
                vm: "vm-a".to_owned(),
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
            },
        )
        .expect("test response");
        assert_unreachable_broker_response(response, "test", "RunActivation");
    }

    #[test]
    fn rollback_broker_unreachable_returns_broker_error() {
        let response = dispatch_broker_rollback(
            &test_state_with_broker_socket(unreachable_broker_socket_path("rollback-unreachable")),
            ActivationRequest {
                vm: "vm-a".to_owned(),
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
            },
        )
        .expect("rollback response");
        assert_unreachable_broker_response(response, "rollback", "RunActivation");
    }

    #[test]
    fn gc_broker_unreachable_returns_broker_error() {
        let response = dispatch_broker_gc(
            &test_state_with_broker_socket(unreachable_broker_socket_path("gc-unreachable")),
            GcRequest {
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
                keep_generations: Some(2),
            },
        )
        .expect("gc response");
        assert_unreachable_broker_response(response, "gc", "RunGc");
    }

    #[test]
    fn keys_rotate_broker_unreachable_returns_broker_error() {
        let response = dispatch_broker_keys_rotate(
            &test_state_with_broker_socket(unreachable_broker_socket_path(
                "keys-rotate-unreachable",
            )),
            KeysRotateRequest {
                vm: "vm-a".to_owned(),
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
            },
        )
        .expect("keys rotate response");
        assert_unreachable_broker_response(response, "keys rotate", "RunKeysRotate");
    }

    #[test]
    fn trust_broker_unreachable_returns_broker_error() {
        let response = dispatch_broker_trust(
            &test_state_with_broker_socket(unreachable_broker_socket_path("trust-unreachable")),
            TrustRequest {
                vm: "vm-a".to_owned(),
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
            },
        )
        .expect("trust response");
        assert_unreachable_broker_response(response, "trust", "RunHostKeyTrust");
    }

    #[test]
    fn rotate_known_host_broker_unreachable_returns_broker_error() {
        let response = dispatch_broker_rotate_known_host(
            &test_state_with_broker_socket(unreachable_broker_socket_path(
                "rotate-known-host-unreachable",
            )),
            RotateKnownHostRequest {
                vm: "vm-a".to_owned(),
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
            },
        )
        .expect("rotate-known-host response");
        assert_unreachable_broker_response(response, "rotate-known-host", "RunRotateKnownHost");
    }

    #[test]
    #[cfg_attr(
        not(test_root),
        ignore = "P2fu1 software-r2 (longstanding pre-existing): test fixture writes bundle artifacts as the developer's uid, but BundleResolver::load uses production policy (root:nixlingd:0640). Run as root, or under a sandbox that lets fchown to root, to exercise. Tracked for a follow-up that introduces NIXLING_TEST_BUNDLE_POLICY_RELAXED env var to opt-into a current-user policy in dev runs."
    )]
    fn vm_start_broker_unreachable_returns_broker_error() {
        let response = dispatch_broker_vm_start(
            &test_state_with_broker_socket(unreachable_broker_socket_path("vm-start-unreachable")),
            VmLifecycleRequest {
                vm: "vm-a".to_owned(),
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
                no_wait_api: false,
            },
        )
        .expect("vm start response");
        let expected_remediation = "Supervisor DAG aborted before every readiness deadline passed. Admin: inspect `journalctl -u nixlingd` for the per-node supervisor audit.";

        assert_redacted_broker_error(&response, "vm start", "SpawnRunner", expected_remediation);
    }

    #[test]
    #[cfg_attr(
        not(test_root),
        ignore = "P2fu1 software-r2 (longstanding pre-existing): same root/owner requirement as vm_start_broker_unreachable_returns_broker_error."
    )]
    fn vm_start_registers_pidfd_table_entry_from_broker_fd() {
        use nixling_ipc::broker_wire::{BrokerRequest, RunnerRole};
        use nixling_ipc::types::{RoleId, VmId};

        let socket_path = unreachable_broker_socket_path("vm-start-registers");
        if let Some(parent) = socket_path.parent() {
            fs::create_dir_all(parent).expect("create broker socket parent");
        }
        fs::remove_file(&socket_path).ok();

        let listener = socket(
            AddressFamily::Unix,
            SockType::SeqPacket,
            SockFlag::SOCK_CLOEXEC,
            None,
        )
        .expect("create broker listener");
        let address = UnixAddr::new(&socket_path).expect("broker socket address");
        bind(listener.as_raw_fd(), &address).expect("bind broker listener");
        listen(&listener, Backlog::new(1).expect("listener backlog"))
            .expect("listen on broker socket");

        let daemon_state_dir = test_daemon_state_dir("vm-start-registers");
        let artifacts = write_minimal_vm_start_bundle_artifacts(&daemon_state_dir);
        let broker_reap_log = BrokerReapLog::new();
        let state = ServerState {
            config: DaemonConfig {
                broker_socket_path: socket_path.clone(),
                artifacts,
                ..DaemonConfig::default()
            },
            daemon_uid: 0,
            daemon_audit: Arc::new(daemon_audit::DaemonAuditLog::no_op()),
            daemon_state_dir: daemon_state_dir.clone(),
            pidfd_table: Arc::new(
                PidfdTable::new(daemon_state_dir.join("pidfd-table.json"))
                    .with_broker_reap_log(Arc::clone(&broker_reap_log)),
            ),
            broker_reap_log,
            metrics_registry: Arc::new(crate::metrics::Registry::new()),
            exec_sessions: Arc::new(crate::exec_session::SessionTable::new(
                crate::exec_session::ExecSessionCaps::default(),
            )),
            gateway_display: crate::new_gateway_display_runtime(),
            conn_semaphore: crate::concurrency::ConnSemaphore::new(8),
            op_locks: crate::concurrency::OpLockManager::new(),
        };
        let server_socket_path = socket_path.clone();
        let broker = thread::spawn(move || {
            let accepted_fd =
                accept4(listener.as_raw_fd(), SockFlag::SOCK_CLOEXEC).expect("accept broker peer");
            let frame = read_test_frame(accepted_fd).expect("read broker request frame");
            let envelope: BrokerRequestEnvelope =
                serde_json::from_slice(&frame).expect("decode broker request frame");
            match envelope.request {
                BrokerRequest::SpawnRunner(request) => {
                    assert_eq!(request.vm_id.as_str(), "vm-a");
                    assert_eq!(request.role_id.as_str(), VM_RUNNER_ROLE_ID);
                    assert_eq!(request.role, RunnerRole::CloudHypervisor);
                }
                other => panic!("unexpected broker request: {other:?}"),
            }

            let child = ChildGuard::new(
                Command::new("sleep")
                    .arg("600")
                    .spawn()
                    .expect("spawn child for broker reply"),
            );
            let pidfd = open_child_pidfd(child.child());
            write_test_json_frame_with_fds(
                accepted_fd,
                &BrokerResponse::SpawnRunner(SpawnRunnerResponse {
                    vm_id: VmId::new("vm-a"),
                    role_id: RoleId::new(VM_RUNNER_ROLE_ID),
                    role: RunnerRole::CloudHypervisor,
                    pid: child.child().id() as i32,
                    start_time_ticks: read_child_start_time(child.child()),
                    pidfd_index: 0,
                }),
                &[pidfd.as_raw_fd()],
            )
            .expect("write spawn response with pidfd");
            close(accepted_fd).expect("close broker peer");
            fs::remove_file(&server_socket_path).ok();
            child
        });

        let response = dispatch_broker_vm_start(
            &state,
            VmLifecycleRequest {
                vm: "vm-a".to_owned(),
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
                no_wait_api: false,
            },
        )
        .expect("vm start response");

        assert_eq!(
            response.get("outcome").and_then(serde_json::Value::as_str),
            Some("applied")
        );
        assert!(
            response
                .get("summary")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .contains("registered in pidfd_table")
        );
        assert!(state.pidfd_table.contains("vm-a", VM_RUNNER_ROLE_ID));

        let child = broker.join().expect("join broker thread");
        state
            .pidfd_table
            .signal("vm-a", VM_RUNNER_ROLE_ID, libc::SIGKILL)
            .expect("cleanup signal");
        assert!(matches!(
            state
                .pidfd_table
                .wait_terminated("vm-a", VM_RUNNER_ROLE_ID, std::time::Duration::from_secs(5))
                .expect("cleanup wait"),
            WaitTermination::Terminated | WaitTermination::TerminatedByBroker { .. }
        ));
        state.pidfd_table.snapshot().expect("cleanup snapshot");
        let status = child.wait();
        assert!(!status.success());
    }

    #[test]
    #[ignore = "flaky on shared hosts; Unix socket reuse races"]
    fn vm_start_drives_supervisor_dag_in_topo_order() {
        use nixling_ipc::broker_wire::{BrokerRequest, RunnerRole};
        use nixling_ipc::types::{RoleId, VmId};

        let daemon_state_dir = test_daemon_state_dir("vm-start-dag");
        let socket_path = unreachable_broker_socket_path("vm-start-dag");
        if let Some(parent) = socket_path.parent() {
            fs::create_dir_all(parent).expect("create broker socket parent");
        }
        fs::remove_file(&socket_path).ok();

        let store_marker = daemon_state_dir.join("store-marker");
        fs::write(&store_marker, b"ok").expect("write store marker");
        let short_socket_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target");
        fs::create_dir_all(&short_socket_dir).expect("create short socket dir");
        let socket_id = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
        let share_ro_socket = short_socket_dir.join(format!("vm-start-ro-{socket_id}.sock"));
        let share_meta_socket = short_socket_dir.join(format!("vm-start-meta-{socket_id}.sock"));
        let api_socket = short_socket_dir.join(format!("vm-start-api-{socket_id}.sock"));
        fs::remove_file(&share_ro_socket).ok();
        fs::remove_file(&share_meta_socket).ok();
        fs::remove_file(&api_socket).ok();
        let ssh_listener = TcpListener::bind("127.0.0.1:0").expect("bind ssh readiness port");
        let ssh_port = ssh_listener.local_addr().expect("ssh listener addr").port();
        let artifacts = write_custom_vm_start_bundle_artifacts(
            &daemon_state_dir,
            &api_socket,
            json!({
                "schemaVersion": "v2",
                "vms": [
                    {
                        "vm": "vm-a",
                        "nodes": [
                            {
                                "id": "host-reconcile",
                                "role": "host-reconcile",
                                "unit": null,
                                "profile": minimal_role_profile("vm-vm-a-host-reconcile", "nixling.slice/vm-a/host-reconcile"),
                                "readiness": [
                                    { "kind": "component-specific", "value": "host ready" }
                                ]
                            },
                            {
                                "id": "store-virtiofs-preflight",
                                "role": "store-virtiofs-preflight",
                                "unit": null,
                                "profile": minimal_role_profile("vm-vm-a-store-virtiofs-preflight", "nixling.slice/vm-a/store-virtiofs-preflight"),
                                "readiness": [
                                    { "kind": "command", "value": ["test", "-e", store_marker.display().to_string()] }
                                ]
                            },
                            {
                                "id": "virtiofsd-ro-store",
                                "role": "virtiofsd",
                                "unit": null,
                                "profile": minimal_role_profile("vm-vm-a-virtiofsd-ro-store", "nixling.slice/vm-a/virtiofsd-ro-store"),
                                "readiness": [
                                    { "kind": "unix-socket-exists", "value": share_ro_socket.display().to_string() }
                                ]
                            },
                            {
                                "id": "virtiofsd-nl-meta",
                                "role": "virtiofsd",
                                "unit": null,
                                "profile": minimal_role_profile("vm-vm-a-virtiofsd-nl-meta", "nixling.slice/vm-a/virtiofsd-nl-meta"),
                                "readiness": [
                                    { "kind": "unix-socket-exists", "value": share_meta_socket.display().to_string() }
                                ]
                            },
                            {
                                "id": "cloud-hypervisor",
                                "role": "cloud-hypervisor-runner",
                                "unit": null,
                                "profile": minimal_role_profile("vm-vm-a-cloud-hypervisor", "nixling.slice/vm-a/cloud-hypervisor"),
                                "readiness": [
                                    { "kind": "api-socket-info", "value": api_socket.display().to_string() }
                                ]
                            },
                            {
                                "id": "guest-ssh-readiness",
                                "role": "guest-ssh-readiness",
                                "unit": null,
                                "profile": minimal_role_profile("vm-vm-a-guest-ssh-readiness", "nixling.slice/vm-a/guest-ssh-readiness"),
                                "readiness": [
                                    { "kind": "tcp-port", "value": { "host": "127.0.0.1", "port": ssh_port } }
                                ]
                            }
                        ],
                        "edges": [
                            { "from": "host-reconcile", "to": "store-virtiofs-preflight", "reason": "host before store" },
                            { "from": "store-virtiofs-preflight", "to": "virtiofsd-ro-store", "reason": "share one" },
                            { "from": "store-virtiofs-preflight", "to": "virtiofsd-nl-meta", "reason": "share two" },
                            { "from": "virtiofsd-ro-store", "to": "cloud-hypervisor", "reason": "share one ready" },
                            { "from": "virtiofsd-nl-meta", "to": "cloud-hypervisor", "reason": "share two ready" },
                            { "from": "cloud-hypervisor", "to": "guest-ssh-readiness", "reason": "ssh after ch" }
                        ],
                        "invariants": {
                            "swtpmPreStartFlush": true,
                            "perVmAuditPipeline": true,
                            "usbipGating": true,
                            "tpmOwnershipMigrationWithoutRunningVmMutation": true
                        }
                    }
                ]
            }),
        );
        let broker_reap_log = BrokerReapLog::new();
        let state = ServerState {
            config: DaemonConfig {
                broker_socket_path: socket_path.clone(),
                artifacts,
                ..DaemonConfig::default()
            },
            daemon_uid: 0,
            daemon_audit: Arc::new(daemon_audit::DaemonAuditLog::no_op()),
            daemon_state_dir: daemon_state_dir.clone(),
            pidfd_table: Arc::new(
                PidfdTable::new(daemon_state_dir.join("pidfd-table.json"))
                    .with_broker_reap_log(Arc::clone(&broker_reap_log)),
            ),
            broker_reap_log,
            metrics_registry: Arc::new(crate::metrics::Registry::new()),
            exec_sessions: Arc::new(crate::exec_session::SessionTable::new(
                crate::exec_session::ExecSessionCaps::default(),
            )),
            gateway_display: crate::new_gateway_display_runtime(),
            conn_semaphore: crate::concurrency::ConnSemaphore::new(8),
            op_locks: crate::concurrency::OpLockManager::new(),
        };

        let listener = socket(
            AddressFamily::Unix,
            SockType::SeqPacket,
            SockFlag::SOCK_CLOEXEC,
            None,
        )
        .expect("create broker listener");
        let address = UnixAddr::new(&socket_path).expect("broker socket address");
        bind(listener.as_raw_fd(), &address).expect("bind broker listener");
        listen(&listener, Backlog::new(3).expect("listener backlog"))
            .expect("listen on broker socket");

        let request_order = Arc::new(Mutex::new(Vec::<String>::new()));
        let request_order_thread = Arc::clone(&request_order);
        let server_socket_path = socket_path.clone();
        let share_ro_socket_for_thread = share_ro_socket.clone();
        let share_meta_socket_for_thread = share_meta_socket.clone();
        let api_socket_for_thread = api_socket.clone();
        let broker = thread::spawn(move || {
            let mut children = Vec::new();
            let mut share_listeners = Vec::new();
            for _ in 0..3 {
                let accepted_fd = accept4(listener.as_raw_fd(), SockFlag::SOCK_CLOEXEC)
                    .expect("accept broker peer");
                let frame = read_test_frame(accepted_fd).expect("read broker request frame");
                let envelope: BrokerRequestEnvelope =
                    serde_json::from_slice(&frame).expect("decode broker request frame");
                let request = match envelope.request {
                    BrokerRequest::SpawnRunner(request) => request,
                    other => panic!("unexpected broker request: {other:?}"),
                };
                request_order_thread
                    .lock()
                    .expect("lock request order")
                    .push(request.role_id.as_str().to_owned());
                assert_eq!(request.vm_id.as_str(), "vm-a");
                let expected_runner_role = if request.role_id.as_str() == VM_RUNNER_ROLE_ID {
                    RunnerRole::CloudHypervisor
                } else {
                    RunnerRole::Virtiofsd
                };
                assert_eq!(request.role, expected_runner_role);

                match request.role_id.as_str() {
                    "virtiofsd-ro-store" => {
                        fs::remove_file(&share_ro_socket_for_thread).ok();
                        share_listeners.push(
                            UnixListener::bind(&share_ro_socket_for_thread)
                                .expect("bind ro-store socket"),
                        );
                    }
                    "virtiofsd-nl-meta" => {
                        fs::remove_file(&share_meta_socket_for_thread).ok();
                        share_listeners.push(
                            UnixListener::bind(&share_meta_socket_for_thread)
                                .expect("bind nl-meta socket"),
                        );
                    }
                    VM_RUNNER_ROLE_ID => {}
                    other => panic!("unexpected runner role id: {other}"),
                }

                let child = ChildGuard::new(
                    Command::new("sleep")
                        .arg("600")
                        .spawn()
                        .expect("spawn child for broker reply"),
                );
                let pidfd = open_child_pidfd(child.child());
                write_test_json_frame_with_fds(
                    accepted_fd,
                    &BrokerResponse::SpawnRunner(SpawnRunnerResponse {
                        vm_id: VmId::new("vm-a"),
                        role_id: RoleId::new(request.role_id.as_str()),
                        role: expected_runner_role,
                        pid: child.child().id() as i32,
                        start_time_ticks: read_child_start_time(child.child()),
                        pidfd_index: 0,
                    }),
                    &[pidfd.as_raw_fd()],
                )
                .expect("write spawn response with pidfd");
                close(accepted_fd).expect("close broker peer");
                if request.role_id.as_str() == VM_RUNNER_ROLE_ID {
                    fs::remove_file(&api_socket_for_thread).ok();
                    let api_listener =
                        UnixListener::bind(&api_socket_for_thread).expect("bind api socket");
                    let (mut stream, _) = api_listener.accept().expect("accept api peer");
                    let mut buffer = [0_u8; 512];
                    let _ = stream.read(&mut buffer);
                    stream
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                        .expect("write api response");
                }
                children.push(child);
            }
            fs::remove_file(&server_socket_path).ok();
            children
        });

        let response = dispatch_broker_vm_start(
            &state,
            VmLifecycleRequest {
                vm: "vm-a".to_owned(),
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
                no_wait_api: false,
            },
        )
        .expect("vm start response");

        assert_eq!(
            response.get("outcome").and_then(serde_json::Value::as_str),
            Some("applied")
        );
        assert!(
            response
                .get("summary")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .contains("registered in pidfd_table")
        );
        let order = request_order.lock().expect("lock request order").clone();
        assert_eq!(order.len(), 3);
        assert_eq!(order.last().map(String::as_str), Some(VM_RUNNER_ROLE_ID));
        let mut share_roles = order[..2].to_vec();
        share_roles.sort();
        assert_eq!(
            share_roles,
            vec![
                "virtiofsd-nl-meta".to_owned(),
                "virtiofsd-ro-store".to_owned()
            ]
        );
        assert!(state.pidfd_table.contains("vm-a", "virtiofsd-ro-store"));
        assert!(state.pidfd_table.contains("vm-a", "virtiofsd-nl-meta"));
        assert!(state.pidfd_table.contains("vm-a", VM_RUNNER_ROLE_ID));

        let children = broker.join().expect("join broker thread");
        for role in ["virtiofsd-ro-store", "virtiofsd-nl-meta", VM_RUNNER_ROLE_ID] {
            state
                .pidfd_table
                .signal("vm-a", role, libc::SIGKILL)
                .expect("cleanup signal");
            assert!(matches!(
                state
                    .pidfd_table
                    .wait_terminated("vm-a", role, std::time::Duration::from_secs(5))
                    .expect("cleanup wait"),
                WaitTermination::Terminated | WaitTermination::TerminatedByBroker { .. }
            ));
            let _ = state.pidfd_table.deregister("vm-a", role);
        }
        state.pidfd_table.snapshot().expect("cleanup snapshot");
        for child in children {
            let status = child.wait();
            assert!(!status.success());
        }
        fs::remove_file(&share_ro_socket).ok();
        fs::remove_file(&share_meta_socket).ok();
        fs::remove_file(&api_socket).ok();
    }

    #[test]
    fn startup_adoption_reads_runner_snapshots() {
        struct FixedProcReader;

        impl ProcReader for FixedProcReader {
            fn proc_starttime(&self, pid: i32) -> Result<Option<u64>, String> {
                match pid {
                    4242 => Ok(Some(55)),
                    _ => Ok(None),
                }
            }
        }

        struct RecordingOpener {
            calls: Mutex<Vec<(String, String, i32, u64)>>,
        }

        impl RecordingOpener {
            fn new() -> Self {
                Self {
                    calls: Mutex::new(Vec::new()),
                }
            }
        }

        impl PidfdOpener for RecordingOpener {
            fn open_pidfd(
                &self,
                vm: &str,
                role_id: &str,
                pid: i32,
                expected_start_time_ticks: u64,
            ) -> Result<std::os::fd::OwnedFd, String> {
                self.calls.lock().expect("lock opener calls").push((
                    vm.to_owned(),
                    role_id.to_owned(),
                    pid,
                    expected_start_time_ticks,
                ));
                let file = File::open("/dev/null").expect("open /dev/null");
                Ok(file.into())
            }
        }

        let daemon_state_dir = test_daemon_state_dir("startup-adoption");
        let store = FilesystemSnapshotStore::new(&daemon_state_dir);
        SnapshotStore::upsert(
            &store,
            &RunnerSnapshotRecord {
                vm: "vm-a".to_owned(),
                role_id: "virtiofsd-ro-store".to_owned(),
                role: RunnerRole::Virtiofsd,
                pid: 4242,
                start_time_ticks: 55,
                snapshotted_at: "2026-05-30T00:00:00Z".to_owned(),
            },
        )
        .expect("write runner snapshot");

        let broker_reap_log = BrokerReapLog::new();
        let state = ServerState {
            config: DaemonConfig::default(),
            daemon_uid: 0,
            daemon_audit: Arc::new(daemon_audit::DaemonAuditLog::no_op()),
            daemon_state_dir: daemon_state_dir.clone(),
            pidfd_table: Arc::new(
                PidfdTable::new(daemon_state_dir.join("pidfd-table.json"))
                    .with_broker_reap_log(Arc::clone(&broker_reap_log)),
            ),
            broker_reap_log,
            metrics_registry: Arc::new(crate::metrics::Registry::new()),
            exec_sessions: Arc::new(crate::exec_session::SessionTable::new(
                crate::exec_session::ExecSessionCaps::default(),
            )),
            gateway_display: crate::new_gateway_display_runtime(),
            conn_semaphore: crate::concurrency::ConnSemaphore::new(8),
            op_locks: crate::concurrency::OpLockManager::new(),
        };
        let opener = RecordingOpener::new();
        adopt_orphaned_runners_on_startup_with(&state, &store, &FixedProcReader, &opener)
            .expect("adopt startup snapshots");

        assert_eq!(
            *opener.calls.lock().expect("lock opener calls"),
            vec![("vm-a".to_owned(), "virtiofsd-ro-store".to_owned(), 4242, 55)]
        );
        assert!(state.pidfd_table.contains("vm-a", "virtiofsd-ro-store"));
    }

    #[test]
    fn vm_stop_signals_registered_pidfd_and_waits() {
        let state = test_state_with_broker_socket(unreachable_broker_socket_path("vm-stop-local"));
        let child = register_sleep_runner(&state, "vm-a", false);

        let response = dispatch_broker_vm_stop(
            &state,
            VmLifecycleRequest {
                vm: "vm-a".to_owned(),
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
                no_wait_api: false,
            },
        )
        .expect("vm stop response");

        assert_eq!(
            response.get("outcome").and_then(serde_json::Value::as_str),
            Some("applied")
        );
        assert!(
            response
                .get("summary")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .contains("pidfd_table")
        );
        assert!(!state.pidfd_table.contains("vm-a", VM_RUNNER_ROLE_ID));
        let store = FilesystemSnapshotStore::new(&state.daemon_state_dir);
        assert!(
            SnapshotStore::list(&store)
                .expect("list runner snapshots")
                .is_empty()
        );
        let status = child.wait();
        assert!(!status.success());
    }

    #[test]
    fn vm_stop_drains_all_sidecar_pidfds() {
        let state =
            test_state_with_broker_socket(unreachable_broker_socket_path("vm-stop-sidecars"));
        let roles = [
            ("virtiofsd-ro-store", RunnerRole::Virtiofsd),
            ("virtiofsd-nl-meta", RunnerRole::Virtiofsd),
            ("swtpm", RunnerRole::Swtpm),
            ("vsock-relay", RunnerRole::VsockRelay),
            ("gpu", RunnerRole::Gpu),
            ("audio", RunnerRole::Audio),
            ("video", RunnerRole::Video),
            ("qemu-media", RunnerRole::QemuMedia),
            (VM_RUNNER_ROLE_ID, RunnerRole::CloudHypervisor),
        ];
        let children = roles
            .iter()
            .map(|(role_id, role)| {
                register_sleep_runner_for_role(&state, "vm-a", role_id, *role, false)
            })
            .collect::<Vec<_>>();

        let response = dispatch_broker_vm_stop(
            &state,
            VmLifecycleRequest {
                vm: "vm-a".to_owned(),
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
                no_wait_api: false,
            },
        )
        .expect("vm stop response");

        assert_eq!(
            response.get("outcome").and_then(serde_json::Value::as_str),
            Some("applied")
        );
        let summary = response
            .get("summary")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        assert!(summary.contains("drained 9 pidfd_table entries in reverse DAG order"));
        assert!(summary.contains(
            "ch-runner, qemu-media, gpu, audio, video, vsock-relay, swtpm, virtiofsd-nl-meta, virtiofsd-ro-store"
        ));
        assert!(state.pidfd_table.list_for_vm("vm-a").is_empty());
        let store = FilesystemSnapshotStore::new(&state.daemon_state_dir);
        assert!(
            SnapshotStore::list(&store)
                .expect("list runner snapshots")
                .is_empty()
        );
        for child in children {
            let status = child.wait();
            assert!(!status.success());
        }
    }

    #[test]
    #[ignore = "flaky on shared hosts; SIGKILL escalation timing varies"]
    fn vm_stop_escalates_to_sigkill_after_term_timeout() {
        let state =
            test_state_with_broker_socket(unreachable_broker_socket_path("vm-stop-sigkill"));
        let child = register_sleep_runner(&state, "vm-a", true);

        let response = dispatch_broker_vm_stop_with_timeout(
            &state,
            VmLifecycleRequest {
                vm: "vm-a".to_owned(),
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
                no_wait_api: false,
            },
            std::time::Duration::from_millis(100),
            std::time::Duration::from_secs(5),
        )
        .expect("vm stop response");

        assert_eq!(
            response.get("outcome").and_then(serde_json::Value::as_str),
            Some("applied")
        );
        assert!(
            response
                .get("summary")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .contains("SIGTERM timeout")
        );
        assert!(!state.pidfd_table.contains("vm-a", VM_RUNNER_ROLE_ID));
        let status = child.wait();
        assert!(!status.success());
    }

    fn assert_applied(response: &serde_json::Value) {
        assert_eq!(
            response.get("outcome").and_then(serde_json::Value::as_str),
            Some("applied")
        );
    }

    fn assert_daemon_failure_contains(response: &serde_json::Value, needle: &str) {
        assert_eq!(
            response.get("outcome").and_then(serde_json::Value::as_str),
            Some("broker-error")
        );
        assert!(
            response
                .get("summary")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .contains(needle)
        );
    }

    fn assert_broker_envelope_launcher_role(caller_role_display: &str) {
        assert_eq!(caller_role_display, concat!("nixling-", "launcher"));
    }

    #[test]
    fn stop_vm_pidfd_role_falls_back_to_broker_on_sigterm_eperm() {
        let vm = "vm-eperm-term";
        let role = VM_RUNNER_ROLE_ID;
        let child = Command::new("sleep")
            .arg("600")
            .spawn()
            .expect("spawn child");
        let child = ChildGuard::new(child);
        let pid = child.child().id() as i32;
        let (socket_path, broker) =
            start_test_broker_server("eperm-term", 3, move |index, env, fd| {
                let caller_role_display = env.caller_role.for_display();
                match (index, env.request) {
                    (0, BrokerRequest::SignalRunner(req)) => {
                        assert_broker_envelope_launcher_role(caller_role_display);
                        assert_eq!(req.vm_id.as_str(), vm);
                        assert_eq!(req.role_id.as_str(), role);
                        assert_eq!(req.signal, RunnerSignal::Term);
                        nix::sys::signal::kill(
                            nix::unistd::Pid::from_raw(pid),
                            nix::sys::signal::Signal::SIGTERM,
                        )
                        .expect("broker kill child");
                        write_test_json_frame(
                            fd,
                            &BrokerResponse::SignalRunner(SignalRunnerResponse {
                                signaled: true,
                                vm_id: VmId::new(vm),
                                role_id: RoleId::new(role),
                            }),
                        )
                        .expect("write signal response");
                    }
                    (1, BrokerRequest::PollChildReaped) => {
                        write_test_json_frame(
                            fd,
                            &BrokerResponse::PollChildReaped(PollChildReapedResponse {
                                notifications: vec![],
                            }),
                        )
                        .expect("write poll response");
                    }
                    (2, BrokerRequest::DeregisterRunnerPidfd(req)) => {
                        assert_broker_envelope_launcher_role(caller_role_display);
                        assert_eq!(req.vm_id.as_str(), vm);
                        assert_eq!(req.role_id.as_str(), role);
                        write_test_json_frame(
                            fd,
                            &BrokerResponse::DeregisterRunnerPidfd(DeregisterRunnerPidfdResponse {
                                vm_id: VmId::new(vm),
                                role_id: RoleId::new(role),
                                removed: true,
                            }),
                        )
                        .expect("write dereg response");
                    }
                    other => panic!("unexpected request {other:?}"),
                }
            });
        let state = test_state_with_broker_socket(socket_path);
        let start_time_ticks = read_child_start_time(child.child());
        state
            .pidfd_table
            .register(
                vm.to_owned(),
                role.to_owned(),
                PidfdEntry {
                    pidfd: open_child_pidfd(child.child()),
                    pid,
                    start_time_ticks,
                },
            )
            .expect("register child");
        super::write_runner_snapshot(
            &state,
            vm,
            role,
            RunnerRole::CloudHypervisor,
            pid,
            start_time_ticks,
        )
        .expect("write snapshot");
        force_signal_eperm_for_tests(vm, role, true);
        let response = dispatch_broker_vm_stop(
            &state,
            VmLifecycleRequest {
                vm: vm.to_owned(),
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
                no_wait_api: false,
            },
        )
        .expect("stop response");
        force_signal_eperm_for_tests(vm, role, false);
        assert_applied(&response);
        assert!(state.metrics_registry.render().contains(
            "nixling_daemon_broker_request_total{op=\"SignalRunner\",outcome=\"broker-fallback\"} 1"
        ));
        assert!(!state.pidfd_table.contains(vm, role));
        broker.join().expect("broker join");
        let _ = child.wait();
    }

    #[test]
    fn unregistered_spawn_cleanup_uses_broker_and_waits_for_reap_before_deregister() {
        let vm = "vm-unregistered-cleanup";
        let role = "video";
        let pid = 4242;
        let start_time_ticks = 99;
        let (socket_path, broker) =
            start_test_broker_server("unregistered-cleanup", 3, move |index, env, fd| {
                match (index, env.request) {
                    (0, BrokerRequest::SignalRunner(req)) => {
                        assert_eq!(req.vm_id.as_str(), vm);
                        assert_eq!(req.role_id.as_str(), role);
                        assert_eq!(req.signal, RunnerSignal::Term);
                        assert_eq!(req.pid, Some(pid));
                        assert_eq!(req.expected_start_time_ticks, Some(start_time_ticks));
                        write_test_json_frame(
                            fd,
                            &BrokerResponse::SignalRunner(SignalRunnerResponse {
                                signaled: true,
                                vm_id: VmId::new(vm),
                                role_id: RoleId::new(role),
                            }),
                        )
                        .expect("write signal response");
                    }
                    (1, BrokerRequest::PollChildReaped) => {
                        write_test_json_frame(
                            fd,
                            &BrokerResponse::PollChildReaped(PollChildReapedResponse {
                                notifications: vec![ChildReapedNotification {
                                    runner_id: format!("{vm}:{role}"),
                                    pid,
                                    exit_status: ChildExitStatus {
                                        kind: ChildExitKind::Exited,
                                        code: Some(0),
                                        signal: None,
                                    },
                                    reaped_at_ms: 123,
                                }],
                            }),
                        )
                        .expect("write poll response");
                    }
                    (2, BrokerRequest::DeregisterRunnerPidfd(req)) => {
                        assert_eq!(req.vm_id.as_str(), vm);
                        assert_eq!(req.role_id.as_str(), role);
                        write_test_json_frame(
                            fd,
                            &BrokerResponse::DeregisterRunnerPidfd(DeregisterRunnerPidfdResponse {
                                vm_id: VmId::new(vm),
                                role_id: RoleId::new(role),
                                removed: true,
                            }),
                        )
                        .expect("write dereg response");
                    }
                    other => panic!("unexpected request {other:?}"),
                }
            });
        let state = test_state_with_broker_socket(socket_path);
        super::stop_unregistered_spawned_runner(
            &state,
            vm,
            role,
            &SpawnRunnerResponse {
                vm_id: VmId::new(vm),
                role_id: RoleId::new(role),
                role: RunnerRole::Video,
                pid,
                start_time_ticks,
                pidfd_index: 0,
            },
            &[],
        );
        broker.join().expect("broker join");
    }

    #[test]
    fn stop_vm_pidfd_role_falls_back_to_broker_on_sigkill_eperm() {
        let vm = "vm-eperm-kill";
        let role = VM_RUNNER_ROLE_ID;
        let child = register_sleep_runner_for_role(
            &test_state_with_broker_socket(unreachable_broker_socket_path("dummy")),
            "dummy",
            role,
            RunnerRole::CloudHypervisor,
            true,
        );
        let pid = child.child().id() as i32;
        let (socket_path, broker) =
            start_test_broker_server("eperm-kill", 5, move |index, env, fd| {
                let caller_role_display = env.caller_role.for_display();
                match (index, env.request) {
                    (0, BrokerRequest::SignalRunner(req)) => {
                        assert_broker_envelope_launcher_role(caller_role_display);
                        assert_eq!(req.signal, RunnerSignal::Term);
                        write_test_json_frame(
                            fd,
                            &BrokerResponse::SignalRunner(SignalRunnerResponse {
                                signaled: true,
                                vm_id: req.vm_id,
                                role_id: req.role_id,
                            }),
                        )
                        .expect("write term signal response");
                    }
                    (1, BrokerRequest::PollChildReaped) | (3, BrokerRequest::PollChildReaped) => {
                        write_test_json_frame(
                            fd,
                            &BrokerResponse::PollChildReaped(PollChildReapedResponse {
                                notifications: vec![],
                            }),
                        )
                        .expect("write poll response");
                    }
                    (2, BrokerRequest::SignalRunner(req)) => {
                        assert_broker_envelope_launcher_role(caller_role_display);
                        assert_eq!(req.signal, RunnerSignal::Kill);
                        nix::sys::signal::kill(
                            nix::unistd::Pid::from_raw(pid),
                            nix::sys::signal::Signal::SIGKILL,
                        )
                        .expect("broker sigkill child");
                        write_test_json_frame(
                            fd,
                            &BrokerResponse::SignalRunner(SignalRunnerResponse {
                                signaled: true,
                                vm_id: req.vm_id,
                                role_id: req.role_id,
                            }),
                        )
                        .expect("write kill signal response");
                    }
                    (4, BrokerRequest::DeregisterRunnerPidfd(req)) => {
                        assert_broker_envelope_launcher_role(caller_role_display);
                        write_test_json_frame(
                            fd,
                            &BrokerResponse::DeregisterRunnerPidfd(DeregisterRunnerPidfdResponse {
                                vm_id: req.vm_id,
                                role_id: req.role_id,
                                removed: true,
                            }),
                        )
                        .expect("write dereg response");
                    }
                    other => panic!("unexpected request {other:?}"),
                }
            });
        let state = test_state_with_broker_socket(socket_path);
        let start_time_ticks = read_child_start_time(child.child());
        state
            .pidfd_table
            .register(
                vm.to_owned(),
                role.to_owned(),
                PidfdEntry {
                    pidfd: open_child_pidfd(child.child()),
                    pid,
                    start_time_ticks,
                },
            )
            .expect("register child");
        super::write_runner_snapshot(
            &state,
            vm,
            role,
            RunnerRole::CloudHypervisor,
            pid,
            start_time_ticks,
        )
        .expect("write snapshot");
        force_signal_eperm_for_tests(vm, role, true);
        let response = dispatch_broker_vm_stop_with_timeout(
            &state,
            VmLifecycleRequest {
                vm: vm.to_owned(),
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
                no_wait_api: false,
            },
            Duration::from_millis(100),
            Duration::from_secs(5),
        )
        .expect("stop response");
        force_signal_eperm_for_tests(vm, role, false);
        assert_applied(&response);
        assert!(
            response
                .get("summary")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .contains("SIGTERM timeout")
        );
        assert!(state.metrics_registry.render().contains(
            "nixling_daemon_broker_request_total{op=\"SignalRunner\",outcome=\"broker-fallback\"} 2"
        ));
        broker.join().expect("broker join");
        let _ = child.wait();
    }

    #[test]
    fn stop_vm_pidfd_role_multi_role_one_eperm() {
        let vm = "vm-eperm-multi";
        let eperm_role = "virtiofsd-ro-store";
        let normal_role = VM_RUNNER_ROLE_ID;
        let eperm_child = register_sleep_runner_for_role(
            &test_state_with_broker_socket(unreachable_broker_socket_path("dummy-multi")),
            "dummy-multi",
            eperm_role,
            RunnerRole::Virtiofsd,
            false,
        );
        let pid = eperm_child.child().id() as i32;
        let (socket_path, broker) =
            start_test_broker_server("eperm-multi", 5, move |index, env, fd| {
                match (index, env.request) {
                    (0, BrokerRequest::PollChildReaped) | (3, BrokerRequest::PollChildReaped) => {
                        write_test_json_frame(
                            fd,
                            &BrokerResponse::PollChildReaped(PollChildReapedResponse {
                                notifications: vec![],
                            }),
                        )
                        .expect("write poll response");
                    }
                    (1, BrokerRequest::DeregisterRunnerPidfd(req)) => {
                        assert_eq!(req.role_id.as_str(), normal_role);
                        write_test_json_frame(
                            fd,
                            &BrokerResponse::DeregisterRunnerPidfd(DeregisterRunnerPidfdResponse {
                                vm_id: req.vm_id,
                                role_id: req.role_id,
                                removed: true,
                            }),
                        )
                        .expect("write normal dereg response");
                    }
                    (2, BrokerRequest::SignalRunner(req)) => {
                        assert_eq!(req.role_id.as_str(), eperm_role);
                        nix::sys::signal::kill(
                            nix::unistd::Pid::from_raw(pid),
                            nix::sys::signal::Signal::SIGTERM,
                        )
                        .expect("broker signal eperm role");
                        write_test_json_frame(
                            fd,
                            &BrokerResponse::SignalRunner(SignalRunnerResponse {
                                signaled: true,
                                vm_id: req.vm_id,
                                role_id: req.role_id,
                            }),
                        )
                        .expect("write signal response");
                    }
                    (4, BrokerRequest::DeregisterRunnerPidfd(req)) => {
                        assert_eq!(req.role_id.as_str(), eperm_role);
                        write_test_json_frame(
                            fd,
                            &BrokerResponse::DeregisterRunnerPidfd(DeregisterRunnerPidfdResponse {
                                vm_id: req.vm_id,
                                role_id: req.role_id,
                                removed: true,
                            }),
                        )
                        .expect("write dereg response");
                    }
                    other => panic!("unexpected request {other:?}"),
                }
            });
        let state = test_state_with_broker_socket(socket_path);
        let normal_child = register_sleep_runner_for_role(
            &state,
            vm,
            normal_role,
            RunnerRole::CloudHypervisor,
            false,
        );
        let start_time_ticks = read_child_start_time(eperm_child.child());
        state
            .pidfd_table
            .register(
                vm.to_owned(),
                eperm_role.to_owned(),
                PidfdEntry {
                    pidfd: open_child_pidfd(eperm_child.child()),
                    pid,
                    start_time_ticks,
                },
            )
            .expect("register eperm child");
        super::write_runner_snapshot(
            &state,
            vm,
            eperm_role,
            RunnerRole::Virtiofsd,
            pid,
            start_time_ticks,
        )
        .expect("write snapshot");
        force_signal_eperm_for_tests(vm, eperm_role, true);
        let response = dispatch_broker_vm_stop(
            &state,
            VmLifecycleRequest {
                vm: vm.to_owned(),
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
                no_wait_api: false,
            },
        )
        .expect("stop response");
        force_signal_eperm_for_tests(vm, eperm_role, false);
        assert_applied(&response);
        assert!(state.pidfd_table.list_for_vm(vm).is_empty());
        broker.join().expect("broker join");
        let _ = eperm_child.wait();
        let _ = normal_child.wait();
    }

    #[test]
    fn stop_vm_pidfd_role_idempotent_after_deregistration() {
        let vm = "vm-eperm-idempotent";
        let role = VM_RUNNER_ROLE_ID;
        let child = register_sleep_runner_for_role(
            &test_state_with_broker_socket(unreachable_broker_socket_path("dummy-idem")),
            "dummy-idem",
            role,
            RunnerRole::CloudHypervisor,
            false,
        );
        let pid = child.child().id() as i32;
        let (socket_path, broker) =
            start_test_broker_server("eperm-idem", 3, move |index, env, fd| {
                match (index, env.request) {
                    (0, BrokerRequest::SignalRunner(req)) => {
                        nix::sys::signal::kill(
                            nix::unistd::Pid::from_raw(pid),
                            nix::sys::signal::Signal::SIGTERM,
                        )
                        .expect("kill child");
                        write_test_json_frame(
                            fd,
                            &BrokerResponse::SignalRunner(SignalRunnerResponse {
                                signaled: true,
                                vm_id: req.vm_id,
                                role_id: req.role_id,
                            }),
                        )
                        .expect("write signal");
                    }
                    (1, BrokerRequest::PollChildReaped) => {
                        write_test_json_frame(
                            fd,
                            &BrokerResponse::PollChildReaped(PollChildReapedResponse {
                                notifications: vec![],
                            }),
                        )
                        .expect("write poll");
                    }
                    (2, BrokerRequest::DeregisterRunnerPidfd(req)) => {
                        write_test_json_frame(
                            fd,
                            &BrokerResponse::DeregisterRunnerPidfd(DeregisterRunnerPidfdResponse {
                                vm_id: req.vm_id,
                                role_id: req.role_id,
                                removed: true,
                            }),
                        )
                        .expect("write dereg");
                    }
                    other => panic!("unexpected request {other:?}"),
                }
            });
        let state = test_state_with_broker_socket(socket_path);
        let start_time_ticks = read_child_start_time(child.child());
        state
            .pidfd_table
            .register(
                vm.to_owned(),
                role.to_owned(),
                PidfdEntry {
                    pidfd: open_child_pidfd(child.child()),
                    pid,
                    start_time_ticks,
                },
            )
            .expect("register");
        super::write_runner_snapshot(
            &state,
            vm,
            role,
            RunnerRole::CloudHypervisor,
            pid,
            start_time_ticks,
        )
        .expect("snapshot");
        force_signal_eperm_for_tests(vm, role, true);
        let first = dispatch_broker_vm_stop(
            &state,
            VmLifecycleRequest {
                vm: vm.to_owned(),
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
                no_wait_api: false,
            },
        )
        .expect("first stop");
        force_signal_eperm_for_tests(vm, role, false);
        assert_applied(&first);
        let second = dispatch_broker_vm_stop(
            &state,
            VmLifecycleRequest {
                vm: vm.to_owned(),
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
                no_wait_api: false,
            },
        )
        .expect("second stop");
        assert_eq!(
            second.get("outcome").and_then(serde_json::Value::as_str),
            Some("invalid-request")
        );
        assert!(
            second
                .get("remediation")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .contains("no registered pidfd_table entries")
        );
        broker.join().expect("broker join");
        let _ = child.wait();
    }

    #[test]
    fn stop_vm_pidfd_role_broker_unreachable_preserves_eperm_context() {
        let vm = "vm-eperm-unreachable";
        let role = VM_RUNNER_ROLE_ID;
        let state =
            test_state_with_broker_socket(unreachable_broker_socket_path("eperm-unreachable"));
        let child =
            register_sleep_runner_for_role(&state, vm, role, RunnerRole::CloudHypervisor, false);
        force_signal_eperm_for_tests(vm, role, true);
        let response = dispatch_broker_vm_stop(
            &state,
            VmLifecycleRequest {
                vm: vm.to_owned(),
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
                no_wait_api: false,
            },
        )
        .expect("stop response");
        force_signal_eperm_for_tests(vm, role, false);
        assert_daemon_failure_contains(&response, "pidfd_table SIGTERM failed");
        state.pidfd_table.signal(vm, role, libc::SIGKILL).ok();
        let _ = child.wait();
    }

    #[test]
    fn stop_vm_pidfd_role_broker_signaled_false_is_daemon_failure() {
        let vm = "vm-eperm-false";
        let role = VM_RUNNER_ROLE_ID;
        let (socket_path, broker) =
            start_test_broker_server("eperm-false", 1, move |_, env, fd| match env.request {
                BrokerRequest::SignalRunner(req) => write_test_json_frame(
                    fd,
                    &BrokerResponse::SignalRunner(SignalRunnerResponse {
                        signaled: false,
                        vm_id: req.vm_id,
                        role_id: req.role_id,
                    }),
                )
                .expect("write false response"),
                other => panic!("unexpected request {other:?}"),
            });
        let state = test_state_with_broker_socket(socket_path);
        let child =
            register_sleep_runner_for_role(&state, vm, role, RunnerRole::CloudHypervisor, false);
        force_signal_eperm_for_tests(vm, role, true);
        let response = dispatch_broker_vm_stop(
            &state,
            VmLifecycleRequest {
                vm: vm.to_owned(),
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
                no_wait_api: false,
            },
        )
        .expect("stop response");
        force_signal_eperm_for_tests(vm, role, false);
        assert_daemon_failure_contains(&response, "pidfd_table SIGTERM failed");
        assert!(state.pidfd_table.contains(vm, role));
        broker.join().expect("broker join");
        state.pidfd_table.signal(vm, role, libc::SIGKILL).ok();
        let _ = child.wait();
    }

    #[test]
    fn stop_vm_pidfd_role_broker_accepts_then_eof_preserves_eperm() {
        let vm = "vm-eperm-eof";
        let role = VM_RUNNER_ROLE_ID;
        let (socket_path, broker) = start_test_broker_server("eperm-eof", 1, move |_, env, _fd| {
            assert!(matches!(env.request, BrokerRequest::SignalRunner(_)));
        });
        let state = test_state_with_broker_socket(socket_path);
        let child =
            register_sleep_runner_for_role(&state, vm, role, RunnerRole::CloudHypervisor, false);
        force_signal_eperm_for_tests(vm, role, true);
        let response = dispatch_broker_vm_stop(
            &state,
            VmLifecycleRequest {
                vm: vm.to_owned(),
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
                no_wait_api: false,
            },
        )
        .expect("stop response");
        force_signal_eperm_for_tests(vm, role, false);
        assert_daemon_failure_contains(&response, "pidfd_table SIGTERM failed");
        broker.join().expect("broker join");
        state.pidfd_table.signal(vm, role, libc::SIGKILL).ok();
        let _ = child.wait();
    }

    #[test]
    fn stop_vm_pidfd_role_broker_accepts_then_short_frame_preserves_eperm() {
        let vm = "vm-eperm-short";
        let role = VM_RUNNER_ROLE_ID;
        let (socket_path, broker) =
            start_test_broker_server("eperm-short", 1, move |_, env, fd| {
                assert!(matches!(env.request, BrokerRequest::SignalRunner(_)));
                nix::sys::socket::send(fd, &[1, 0], MsgFlags::empty()).expect("send short frame");
            });
        let state = test_state_with_broker_socket(socket_path);
        let child =
            register_sleep_runner_for_role(&state, vm, role, RunnerRole::CloudHypervisor, false);
        force_signal_eperm_for_tests(vm, role, true);
        let response = dispatch_broker_vm_stop(
            &state,
            VmLifecycleRequest {
                vm: vm.to_owned(),
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
                no_wait_api: false,
            },
        )
        .expect("stop response");
        force_signal_eperm_for_tests(vm, role, false);
        assert_daemon_failure_contains(&response, "pidfd_table SIGTERM failed");
        broker.join().expect("broker join");
        state.pidfd_table.signal(vm, role, libc::SIGKILL).ok();
        let _ = child.wait();
    }

    #[test]
    fn stop_vm_pidfd_role_broker_wrong_response_variant_preserves_eperm() {
        let vm = "vm-eperm-wrong";
        let role = VM_RUNNER_ROLE_ID;
        let (socket_path, broker) =
            start_test_broker_server("eperm-wrong", 1, move |_, env, fd| {
                assert!(matches!(env.request, BrokerRequest::SignalRunner(_)));
                write_test_json_frame(
                    fd,
                    &BrokerResponse::DeregisterRunnerPidfd(DeregisterRunnerPidfdResponse {
                        vm_id: VmId::new(vm),
                        role_id: RoleId::new(role),
                        removed: true,
                    }),
                )
                .expect("write wrong response");
            });
        let state = test_state_with_broker_socket(socket_path);
        let child =
            register_sleep_runner_for_role(&state, vm, role, RunnerRole::CloudHypervisor, false);
        force_signal_eperm_for_tests(vm, role, true);
        let response = dispatch_broker_vm_stop(
            &state,
            VmLifecycleRequest {
                vm: vm.to_owned(),
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
                no_wait_api: false,
            },
        )
        .expect("stop response");
        force_signal_eperm_for_tests(vm, role, false);
        assert_daemon_failure_contains(&response, "pidfd_table SIGTERM failed");
        assert!(state.pidfd_table.contains(vm, role));
        let store = FilesystemSnapshotStore::new(&state.daemon_state_dir);
        assert!(
            !SnapshotStore::list(&store)
                .expect("list snapshots")
                .is_empty()
        );
        broker.join().expect("broker join");
        state.pidfd_table.signal(vm, role, libc::SIGKILL).ok();
        let _ = child.wait();
    }

    #[test]
    fn stop_vm_pidfd_role_broker_dereg_removed_false_is_idempotent_cleanup() {
        let vm = "vm-eperm-dereg-false";
        let role = VM_RUNNER_ROLE_ID;
        let child = Command::new("sleep")
            .arg("600")
            .spawn()
            .expect("spawn child");
        let child = ChildGuard::new(child);
        let pid = child.child().id() as i32;
        let (socket_path, broker) =
            start_test_broker_server("eperm-dereg-false", 3, move |index, env, fd| {
                match (index, env.request) {
                    (0, BrokerRequest::SignalRunner(req)) => {
                        nix::sys::signal::kill(
                            nix::unistd::Pid::from_raw(pid),
                            nix::sys::signal::Signal::SIGTERM,
                        )
                        .expect("kill child");
                        write_test_json_frame(
                            fd,
                            &BrokerResponse::SignalRunner(SignalRunnerResponse {
                                signaled: true,
                                vm_id: req.vm_id,
                                role_id: req.role_id,
                            }),
                        )
                        .expect("write signal");
                    }
                    (1, BrokerRequest::PollChildReaped) => {
                        write_test_json_frame(
                            fd,
                            &BrokerResponse::PollChildReaped(PollChildReapedResponse {
                                notifications: vec![],
                            }),
                        )
                        .expect("write poll");
                    }
                    (2, BrokerRequest::DeregisterRunnerPidfd(req)) => {
                        write_test_json_frame(
                            fd,
                            &BrokerResponse::DeregisterRunnerPidfd(DeregisterRunnerPidfdResponse {
                                vm_id: req.vm_id,
                                role_id: req.role_id,
                                removed: false,
                            }),
                        )
                        .expect("write dereg false");
                    }
                    other => panic!("unexpected request {other:?}"),
                }
            });
        let state = test_state_with_broker_socket(socket_path);
        let start_time_ticks = read_child_start_time(child.child());
        state
            .pidfd_table
            .register(
                vm.to_owned(),
                role.to_owned(),
                PidfdEntry {
                    pidfd: open_child_pidfd(child.child()),
                    pid,
                    start_time_ticks,
                },
            )
            .expect("register");
        super::write_runner_snapshot(
            &state,
            vm,
            role,
            RunnerRole::CloudHypervisor,
            pid,
            start_time_ticks,
        )
        .expect("snapshot");
        force_signal_eperm_for_tests(vm, role, true);
        let response = dispatch_broker_vm_stop(
            &state,
            VmLifecycleRequest {
                vm: vm.to_owned(),
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
                no_wait_api: false,
            },
        )
        .expect("stop response");
        force_signal_eperm_for_tests(vm, role, false);
        assert_applied(&response);
        assert!(!state.pidfd_table.contains(vm, role));
        assert!(
            SnapshotStore::list(&FilesystemSnapshotStore::new(&state.daemon_state_dir))
                .expect("snapshots")
                .is_empty()
        );
        broker.join().expect("broker join");
        let _ = child.wait();
    }

    fn register_echild_wait_entry(state: &ServerState, vm: &str, role: &str) -> ChildGuard {
        let child = Command::new("sleep")
            .arg("600")
            .spawn()
            .expect("spawn child");
        let child = ChildGuard::new(child);
        let pid = child.child().id() as i32;
        let start_time_ticks = read_child_start_time(child.child());
        let self_pid = rustix::process::Pid::from_raw(std::process::id() as i32).expect("self pid");
        let self_pidfd =
            rustix::process::pidfd_open(self_pid, rustix::process::PidfdFlags::empty())
                .expect("pidfd_open self");
        state
            .pidfd_table
            .register(
                vm.to_owned(),
                role.to_owned(),
                PidfdEntry {
                    pidfd: self_pidfd,
                    pid,
                    start_time_ticks,
                },
            )
            .expect("register echild entry");
        child
    }

    #[test]
    fn wait_terminated_with_broker_poll_echild_polls_reap_log() {
        let vm = "vm-echild-poll";
        let role = VM_RUNNER_ROLE_ID;
        let (socket_path, broker) =
            start_test_broker_server("echild-poll", 1, move |_, env, fd| {
                assert!(matches!(env.request, BrokerRequest::PollChildReaped));
                write_test_json_frame(
                    fd,
                    &BrokerResponse::PollChildReaped(PollChildReapedResponse {
                        notifications: vec![ChildReapedNotification {
                            pid: 424242,
                            runner_id: format!("{vm}:{role}"),
                            exit_status: ChildExitStatus {
                                kind: ChildExitKind::Exited,
                                code: Some(0),
                                signal: None,
                            },
                            reaped_at_ms: 1,
                        }],
                    }),
                )
                .expect("write poll response");
            });
        let state = test_state_with_broker_socket(socket_path);
        let child = register_echild_wait_entry(&state, vm, role);
        let result = super::wait_terminated_with_broker_poll(
            &state,
            vm,
            role,
            Instant::now() + Duration::from_secs(2),
        )
        .expect("wait wrapper");
        assert!(matches!(result, WaitTermination::TerminatedByBroker { .. }));
        broker.join().expect("broker join");
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(child.child().id() as i32),
            nix::sys::signal::Signal::SIGKILL,
        )
        .ok();
        let _ = child.wait();
    }

    #[test]
    fn wait_terminated_with_broker_poll_echild_respects_deadline() {
        let vm = "vm-echild-deadline";
        let role = VM_RUNNER_ROLE_ID;
        let state =
            test_state_with_broker_socket(unreachable_broker_socket_path("echild-deadline"));
        let child = register_echild_wait_entry(&state, vm, role);
        let result = super::wait_terminated_with_broker_poll(
            &state,
            vm,
            role,
            Instant::now() + Duration::from_millis(120),
        )
        .expect("wait wrapper");
        assert_eq!(result, WaitTermination::TimedOut);
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(child.child().id() as i32),
            nix::sys::signal::Signal::SIGKILL,
        )
        .ok();
        let _ = child.wait();
    }

    #[test]
    #[cfg_attr(
        not(test_root),
        ignore = "P2fu1 software-r2 (longstanding pre-existing): same root/owner requirement as vm_start_broker_unreachable_returns_broker_error."
    )]
    fn vm_restart_stops_then_surfaces_start_failure() {
        let state =
            test_state_with_broker_socket(unreachable_broker_socket_path("vm-restart-unreachable"));
        let child = register_sleep_runner(&state, "vm-a", false);

        let response = dispatch_broker_vm_restart(
            &state,
            VmLifecycleRequest {
                vm: "vm-a".to_owned(),
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
                no_wait_api: false,
            },
        )
        .expect("vm restart response");
        let expected_remediation = "Supervisor DAG aborted before every readiness deadline passed. Admin: inspect `journalctl -u nixlingd` for the per-node supervisor audit.";

        assert_redacted_broker_error(&response, "vm restart", "SpawnRunner", expected_remediation);
        assert!(!state.pidfd_table.contains("vm-a", VM_RUNNER_ROLE_ID));
        let status = child.wait();
        assert!(!status.success());
    }

    #[test]
    fn host_prepare_broker_unreachable_returns_broker_error() {
        let response = dispatch_broker_host_prepare(
            &test_state_with_broker_socket_and_host(
                unreachable_broker_socket_path("host-prepare-unreachable"),
                host_fixture_path(),
            ),
            HostPrepareRequest {
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
            },
        )
        .expect("host prepare response");
        let (_expected_summary, expected_remediation) =
            redact_broker_dispatch_failure_for_launcher("ApplyNftables");

        assert_redacted_broker_error(
            &response,
            "host prepare",
            "ApplyNftables failed",
            &expected_remediation,
        );
    }

    #[test]
    fn host_destroy_deletes_routes_before_restoring_sysctls_and_flushing_nft() {
        use nix::sys::socket::{
            AddressFamily, Backlog, MsgFlags, SockFlag, SockType, UnixAddr, accept4, bind, listen,
            recv, send, socket,
        };
        use nix::unistd::close;
        use nixling_ipc::broker_wire::{AckResponse, BrokerRequestEnvelope, BrokerResponse};
        use serde::Serialize;
        use std::fs;
        use std::io;
        use std::os::fd::{AsRawFd, RawFd};
        use std::thread;

        fn read_test_frame(fd: RawFd) -> io::Result<Vec<u8>> {
            let mut buffer = vec![0_u8; super::wire::MAX_FRAME_SIZE + 4];
            let received = recv(fd, &mut buffer, MsgFlags::empty())
                .map_err(|err| io::Error::from_raw_os_error(err as i32))?;
            if received < 4 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "short frame from seqpacket socket",
                ));
            }
            let expected =
                u32::from_le_bytes(buffer[..4].try_into().expect("frame prefix")) as usize;
            if expected > super::wire::MAX_FRAME_SIZE || expected + 4 > received {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "malformed seqpacket frame",
                ));
            }
            Ok(buffer[4..4 + expected].to_vec())
        }

        fn write_test_json_frame<T: Serialize>(fd: RawFd, message: &T) -> io::Result<()> {
            let payload = serde_json::to_vec(message)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            if payload.len() > super::wire::MAX_FRAME_SIZE {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "frame exceeds 1 MiB limit",
                ));
            }
            let mut frame = Vec::with_capacity(payload.len() + 4);
            frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            frame.extend_from_slice(&payload);
            let sent = send(fd, &frame, MsgFlags::empty())
                .map_err(|err| io::Error::from_raw_os_error(err as i32))?;
            if sent != frame.len() {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "short write on seqpacket socket",
                ));
            }
            Ok(())
        }

        let socket_path = unreachable_broker_socket_path("host-destroy-order");
        if let Some(parent) = socket_path.parent() {
            fs::create_dir_all(parent).expect("create broker socket parent");
        }
        fs::remove_file(&socket_path).ok();

        let listener = socket(
            AddressFamily::Unix,
            SockType::SeqPacket,
            SockFlag::SOCK_CLOEXEC,
            None,
        )
        .expect("create broker listener");
        let address = UnixAddr::new(&socket_path).expect("broker socket address");
        bind(listener.as_raw_fd(), &address).expect("bind broker listener");
        listen(&listener, Backlog::new(1).expect("listener backlog"))
            .expect("listen on broker socket");

        let server_socket_path = socket_path.clone();
        let broker = thread::spawn(move || {
            let mut operations = Vec::new();
            for _ in 0..9 {
                let accepted_fd = accept4(listener.as_raw_fd(), SockFlag::SOCK_CLOEXEC)
                    .expect("accept broker peer");
                let frame = read_test_frame(accepted_fd).expect("read broker request frame");
                let envelope: BrokerRequestEnvelope =
                    serde_json::from_slice(&frame).expect("decode broker request frame");
                let operation = envelope.request.op_name().to_owned();
                operations.push(operation.clone());
                write_test_json_frame(
                    accepted_fd,
                    &BrokerResponse::Ack(AckResponse {
                        accepted: true,
                        operation,
                    }),
                )
                .expect("write broker ack frame");
                close(accepted_fd).expect("close broker peer");
            }
            fs::remove_file(&server_socket_path).ok();
            operations
        });

        let response = dispatch_broker_host_destroy(
            &test_state_with_broker_socket_and_host(socket_path.clone(), host_fixture_path()),
            HostDestroyRequest {
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
            },
        )
        .expect("host destroy response");

        assert_eq!(
            broker.join().expect("join broker thread"),
            vec![
                "ApplyNmUnmanaged",
                "ApplyRoute",
                "ApplySysctl",
                "ApplySysctl",
                "ApplySysctl",
                "ApplySysctl",
                "ApplySysctl",
                "UpdateHostsFile",
                "ApplyNftables",
            ]
        );
        assert_eq!(
            response.get("type").and_then(serde_json::Value::as_str),
            Some("mutatingVerbResponse")
        );
        assert_eq!(
            response.get("verb").and_then(serde_json::Value::as_str),
            Some("host destroy")
        );
        assert_eq!(
            response.get("outcome").and_then(serde_json::Value::as_str),
            Some("applied")
        );
        assert_eq!(
            response.get("summary").and_then(serde_json::Value::as_str),
            Some(
                "host destroy: applied 1 nm-unmanaged-remove + 1 route-del + 5 sysctl-revert + 1 hosts-remove + 1 nft-flush ops"
            )
        );

        fs::remove_file(&socket_path).ok();
    }

    #[test]
    fn host_destroy_broker_unreachable_returns_broker_error() {
        let response = dispatch_broker_host_destroy(
            &test_state_with_broker_socket_and_host(
                unreachable_broker_socket_path("host-destroy-unreachable"),
                host_fixture_path(),
            ),
            HostDestroyRequest {
                flags: MutationFlags {
                    apply: true,
                    ..MutationFlags::default()
                },
            },
        )
        .expect("host destroy response");
        let (_expected_summary, expected_remediation) =
            redact_broker_dispatch_failure_for_launcher("ApplyNmUnmanaged");

        assert_redacted_broker_error(
            &response,
            "host destroy",
            "ApplyNmUnmanaged failed",
            &expected_remediation,
        );
    }

    /// Wiring test: verify that `ServerState.daemon_audit` is wired with a
    /// `DaemonAuditLog` that can capture `DaemonEvent::ApiReadyTimeout`
    /// events, and that the event serialises with the expected field shape.
    ///
    /// This complements the deeper unit tests in `daemon_audit::tests` by
    /// asserting that the `ServerState` construction and the `DaemonEvent` fields
    /// match what `dispatch_broker_vm_start` actually writes at the timeout site.
    #[test]
    fn api_ready_timeout_audit_event_captured_via_server_state() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let broker_reap_log = BrokerReapLog::new();
        let state_dir = dir.path().to_path_buf();
        let state = ServerState {
            config: DaemonConfig::default(),
            daemon_uid: 0,
            daemon_audit: Arc::new(daemon_audit::DaemonAuditLog::new(&state_dir)),
            daemon_state_dir: state_dir.clone(),
            pidfd_table: Arc::new(
                PidfdTable::new(state_dir.join("pidfd-table.json"))
                    .with_broker_reap_log(Arc::clone(&broker_reap_log)),
            ),
            broker_reap_log,
            metrics_registry: Arc::new(crate::metrics::Registry::new()),
            exec_sessions: Arc::new(crate::exec_session::SessionTable::new(
                crate::exec_session::ExecSessionCaps::default(),
            )),
            gateway_display: crate::new_gateway_display_runtime(),
            conn_semaphore: crate::concurrency::ConnSemaphore::new(8),
            op_locks: crate::concurrency::OpLockManager::new(),
        };

        // Emit the same event that the timeout handler in
        // dispatch_broker_vm_start writes.
        state
            .daemon_audit
            .write_event(&daemon_audit::DaemonEvent::ApiReadyTimeout {
                vm: "vm-a".to_owned(),
                runner: VM_RUNNER_ROLE_ID.to_owned(),
                elapsed_secs: 120,
                mode: "strict".to_owned(),
            })
            .expect("write ApiReadyTimeout audit event");

        let captured = state
            .daemon_audit
            .captured
            .lock()
            .expect("lock captured records");
        assert_eq!(
            captured.len(),
            1,
            "expected exactly one captured audit record"
        );
        let record: serde_json::Value =
            serde_json::from_str(&captured[0]).expect("parse captured record as JSON");
        let event = record.get("event").expect("event field must be present");
        assert_eq!(
            event.get("kind").and_then(|v| v.as_str()),
            Some("api_ready_timeout"),
            "event.kind must be 'api_ready_timeout'",
        );
        assert_eq!(event.get("vm").and_then(|v| v.as_str()), Some("vm-a"),);
        assert_eq!(
            event.get("runner").and_then(|v| v.as_str()),
            Some(VM_RUNNER_ROLE_ID),
        );
        assert_eq!(
            event.get("elapsed_secs").and_then(|v| v.as_u64()),
            Some(120),
        );
        assert_eq!(event.get("mode").and_then(|v| v.as_str()), Some("strict"),);
    }

    // ----- v1.2fu53 panel-test R1 must-fix regression test -----

    /// fu53 panel-test R1 #2: hermetic regression test for the D9
    /// daemon-side DiskInit dispatch decision.  The original D9 hole
    /// (closed by fu46) was missed precisely because no unit test
    /// exercised the `spawn_runner` path's `node.plan_ops` branch.
    ///
    /// This test pins the predicate `node_requires_disk_init_dispatch`
    /// to its correct behavior:
    ///   - empty plan_ops → do NOT dispatch DiskInit
    ///   - plan_ops contains DiskInit → DO dispatch DiskInit
    ///
    /// The integration of the predicate + actual broker dispatch is
    /// covered by `tests/integration/live/live-vm-smoke.sh` against a live deploy.
    /// This hermetic test catches the "predicate accidentally
    /// short-circuited to `false`" regression that would otherwise
    /// silently re-introduce the v1.2 D9 hole.
    #[test]
    fn node_requires_disk_init_dispatch_returns_false_for_empty_plan_ops() {
        use super::node_requires_disk_init_dispatch;
        use nixling_core::processes::{NodeId, ProcessNode, ProcessRole};

        let node = ProcessNode {
            id: NodeId("cloud-hypervisor".to_owned()),
            role: ProcessRole::CloudHypervisorRunner,
            unit: None,
            binary_path: None,
            argv: vec![],
            env: vec![],
            plan_ops: vec![],
            profile: nixling_core::test_support::RoleProfileBuilder::new()
                .with_profile_id("ch-runner")
                .with_uid(0)
                .with_gid(0)
                .build(),
            readiness: vec![],
        };
        assert!(
            !node_requires_disk_init_dispatch(&node),
            "no plan_ops → no DiskInit dispatch (would otherwise be wasted broker traffic)"
        );
    }

    #[test]
    fn node_requires_disk_init_dispatch_returns_true_for_disk_init_op() {
        use super::node_requires_disk_init_dispatch;
        use nixling_core::processes::{NodeId, ProcessNode, ProcessRole, SpawnRunnerPlanOp};
        use std::path::PathBuf;

        let node = ProcessNode {
            id: NodeId("cloud-hypervisor".to_owned()),
            role: ProcessRole::CloudHypervisorRunner,
            unit: None,
            binary_path: None,
            argv: vec![],
            env: vec![],
            plan_ops: vec![SpawnRunnerPlanOp::DiskInit {
                target_path: PathBuf::from("/var/lib/nixling/vms/test-vm/store-overlay.img"),
                size_bytes: 1_073_741_824,
                mode: 0o600,
                owner_uid: 12345,
                owner_gid: 12345,
                if_absent: true,
            }],
            profile: nixling_core::test_support::RoleProfileBuilder::new()
                .with_profile_id("ch-runner")
                .with_uid(0)
                .with_gid(0)
                .build(),
            readiness: vec![],
        };
        assert!(
            node_requires_disk_init_dispatch(&node),
            "plan_ops contains DiskInit → MUST dispatch BrokerRequest::DiskInit before SpawnRunner; otherwise CH boots without overlay file and fatals with NotFound (the original D9 hole — closed by fu46, regression-pinned by this test)"
        );
    }

    #[test]
    fn stateless_readiness_for_guest_control_health_fails_loud() {
        use super::readiness_predicate_ready;
        use nixling_core::processes::ReadinessPredicate;

        // The live readiness path intercepts `GuestControlHealth` nodes in
        // `VmStartRunner::spawn_and_wait_ready` and never reaches the stateless
        // helper. If the stateless arm is ever hit, the state-aware routing
        // regressed, so it MUST fail loud (not silently never-ready).
        let predicate = ReadinessPredicate::GuestControlHealth {
            vm: "work".to_owned(),
        };
        let result = readiness_predicate_ready(&predicate);
        assert_eq!(
            result,
            Err("guest-control-health-needs-state-aware-path".to_owned()),
            "stateless guest-control readiness MUST be a loud Err so a routing regression cannot masquerade as a benign never-ready"
        );
    }

    /// Scripted observe-only liveness fake driving the readiness loop
    /// through the same call site as production. Pops the next scripted
    /// verdict each call; repeats the last verdict once exhausted.
    struct ScriptedLivenessProbe {
        inner: std::sync::Mutex<(
            std::collections::VecDeque<super::supervisor::readiness_liveness::RunnerLiveness>,
            super::supervisor::readiness_liveness::RunnerLiveness,
        )>,
    }

    impl ScriptedLivenessProbe {
        fn always(verdict: super::supervisor::readiness_liveness::RunnerLiveness) -> Self {
            Self {
                inner: std::sync::Mutex::new((std::collections::VecDeque::new(), verdict)),
            }
        }
    }

    impl super::supervisor::readiness_liveness::LivenessProbe for ScriptedLivenessProbe {
        fn probe(&self) -> super::supervisor::readiness_liveness::RunnerLiveness {
            let mut guard = self.inner.lock().expect("scripted liveness mutex");
            if let Some(verdict) = guard.0.pop_front() {
                guard.1 = verdict.clone();
                verdict
            } else {
                guard.1.clone()
            }
        }
    }

    fn readiness_test_node() -> nixling_core::processes::ProcessNode {
        use nixling_core::processes::{NodeId, ProcessNode, ProcessRole};
        ProcessNode {
            id: NodeId("swtpm".to_owned()),
            role: ProcessRole::Swtpm,
            unit: None,
            binary_path: None,
            argv: vec![],
            env: vec![],
            plan_ops: vec![],
            profile: nixling_core::test_support::RoleProfileBuilder::new()
                .with_profile_id("swtpm")
                .with_uid(0)
                .with_gid(0)
                .build(),
            readiness: vec![],
        }
    }

    #[test]
    fn wait_for_readiness_alive_and_ready_is_ok() {
        use super::supervisor::readiness_liveness::RunnerLiveness;
        use nixling_core::processes::ReadinessPredicate;

        let node = readiness_test_node();
        // ComponentSpecific is trivially ready; Alive liveness lets it pass.
        let ready = vec![ReadinessPredicate::ComponentSpecific("x".to_owned())];
        let probe = ScriptedLivenessProbe::always(RunnerLiveness::Alive);
        let result = super::wait_for_readiness(&node, &ready, Duration::from_secs(5), Some(&probe));
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn wait_for_readiness_alive_but_unready_polls_to_timeout() {
        use super::supervisor::readiness_liveness::RunnerLiveness;
        use nixling_core::processes::ReadinessPredicate;

        let node = readiness_test_node();
        // A socket that never exists keeps the predicate false; Alive
        // liveness means we poll until the (tiny) deadline elapses.
        let unready = vec![ReadinessPredicate::UnixSocketExists(
            "/nonexistent/nixling-readiness-test.sock".to_owned(),
        )];
        let probe = ScriptedLivenessProbe::always(RunnerLiveness::Alive);
        let result =
            super::wait_for_readiness(&node, &unready, Duration::from_millis(50), Some(&probe));
        assert_eq!(result, Err("readiness-timeout:swtpm".to_owned()));
    }

    #[test]
    fn wait_for_readiness_exited_fast_fails_before_deadline() {
        use super::supervisor::readiness_liveness::RunnerLiveness;
        use nixling_core::processes::ReadinessPredicate;

        let node = readiness_test_node();
        let unready = vec![ReadinessPredicate::UnixSocketExists(
            "/nonexistent/nixling-readiness-test.sock".to_owned(),
        )];
        // Exited liveness short-circuits at the top of the loop. A long
        // deadline proves the fast-fail does NOT wait for the budget.
        let probe = ScriptedLivenessProbe::always(RunnerLiveness::Exited(None));
        let started = Instant::now();
        let result =
            super::wait_for_readiness(&node, &unready, Duration::from_secs(300), Some(&probe));
        assert_eq!(result, Err("runner-exited:swtpm".to_owned()));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "runner-exited must fast-fail, not block to the 300s readiness budget"
        );
    }

    #[test]
    fn wait_for_readiness_reused_fast_fails() {
        use super::supervisor::readiness_liveness::RunnerLiveness;
        use nixling_core::processes::ReadinessPredicate;

        let node = readiness_test_node();
        let unready = vec![ReadinessPredicate::UnixSocketExists(
            "/nonexistent/nixling-readiness-test.sock".to_owned(),
        )];
        let probe = ScriptedLivenessProbe::always(RunnerLiveness::Reused);
        let result =
            super::wait_for_readiness(&node, &unready, Duration::from_secs(300), Some(&probe));
        assert_eq!(result, Err("runner-reused:swtpm".to_owned()));
    }

    #[test]
    fn wait_for_readiness_stale_listening_socket_not_false_ready() {
        use super::supervisor::readiness_liveness::RunnerLiveness;
        use nixling_core::processes::ReadinessPredicate;

        let node = readiness_test_node();
        // The predicate reports ready (stale listening socket), but the
        // runner has exited — the liveness re-check must veto false-ready.
        let ready = vec![ReadinessPredicate::ComponentSpecific("x".to_owned())];
        let probe = ScriptedLivenessProbe::always(RunnerLiveness::Exited(None));
        let result =
            super::wait_for_readiness(&node, &ready, Duration::from_secs(300), Some(&probe));
        assert_eq!(
            result,
            Err("runner-exited:swtpm".to_owned()),
            "a stale listening socket must not yield false-ready when the runner has exited"
        );
    }

    #[test]
    fn wait_for_readiness_unknown_liveness_keeps_polling() {
        use super::supervisor::readiness_liveness::RunnerLiveness;
        use nixling_core::processes::ReadinessPredicate;

        let node = readiness_test_node();
        let unready = vec![ReadinessPredicate::UnixSocketExists(
            "/nonexistent/nixling-readiness-test.sock".to_owned(),
        )];
        // Unknown is non-terminal: the loop keeps polling to the deadline.
        let probe = ScriptedLivenessProbe::always(RunnerLiveness::Unknown);
        let result =
            super::wait_for_readiness(&node, &unready, Duration::from_millis(50), Some(&probe));
        assert_eq!(result, Err("readiness-timeout:swtpm".to_owned()));
    }

    #[test]
    fn vm_start_runner_exited_response_is_broker_error_with_swtpm_remediation() {
        use nixling_ipc::broker_wire::{ChildExitKind, ChildExitStatus};

        let status = ChildExitStatus {
            kind: ChildExitKind::Exited,
            code: Some(1),
            signal: None,
        };
        let value = super::vm_start_runner_exited_response(
            "work",
            "swtpm",
            super::daemon_audit::VmStartRunnerExitReason::RunnerExited,
            Some(&status),
        );
        assert_eq!(
            super::response_outcome(&value),
            Some("broker-error"),
            "runner-exited maps to the broker-error exit contract, not exit 1"
        );
        let remediation = super::response_remediation(&value).unwrap_or_default();
        assert!(remediation.contains("swtpm"), "remediation names swtpm");
        assert!(
            remediation.contains("must not be wiped"),
            "remediation warns the TPM state must not be wiped"
        );
        assert!(
            remediation.contains("exit code 1"),
            "remediation carries the bounded underlying cause"
        );
    }

    #[test]
    fn detect_runner_exit_failure_extracts_role_from_reason() {
        use nixling_core::processes::{
            NodeId, ProcessNode, ProcessRole, VmProcessDag, VmProcessInvariants,
        };

        let node = ProcessNode {
            id: NodeId("swtpm-node".to_owned()),
            role: ProcessRole::Swtpm,
            unit: None,
            binary_path: None,
            argv: vec![],
            env: vec![],
            plan_ops: vec![],
            profile: nixling_core::test_support::RoleProfileBuilder::new()
                .with_profile_id("swtpm")
                .with_uid(0)
                .with_gid(0)
                .build(),
            readiness: vec![],
        };
        let dag = VmProcessDag {
            vm: "work".to_owned(),
            nodes: vec![node],
            edges: vec![],
            invariants: VmProcessInvariants {
                swtpm_pre_start_flush: false,
                per_vm_audit_pipeline: true,
                usbip_gating: false,
                tpm_ownership_migration_without_running_vm_mutation: false,
            },
        };
        let report = super::supervisor::dag::DagRunReport {
            vm: "work".to_owned(),
            history: vec![super::supervisor::dag::NodeHistory {
                node_id: NodeId("swtpm-node".to_owned()),
                outcome: super::supervisor::dag::NodeOutcome::Failed {
                    reason: "runner-exited:swtpm-node".to_owned(),
                },
            }],
            overall_ok: false,
            api_ready: None,
        };
        let detected = super::detect_runner_exit_failure(&report, &dag);
        assert_eq!(
            detected,
            Some((
                "swtpm-node".to_owned(),
                super::daemon_audit::VmStartRunnerExitReason::RunnerExited
            )),
            "the swtpm node id maps to its tracked role id and runner-exited kind"
        );
    }

    #[test]
    fn guest_control_health_is_readiness_only_node_mode() {
        use super::{VmStartNodeMode, vm_start_node_mode};
        use nixling_core::processes::ProcessRole;

        // GuestControlHealth must remain a readiness-only node (no runner is
        // spawned for it); the state-aware probe is driven by the readiness
        // interception, not by a long-lived/one-shot spawn.
        assert!(matches!(
            vm_start_node_mode(&ProcessRole::GuestControlHealth),
            VmStartNodeMode::ReadinessOnly
        ));
    }

    #[test]
    fn guest_control_health_empty_readiness_fallthrough_is_intercepted() {
        use super::{VmStartNodeMode, vm_start_node_mode, wait_for_readiness};
        use nixling_core::processes::{NodeId, ProcessNode, ProcessRole};
        use std::time::Duration;

        let node = ProcessNode {
            id: NodeId("guest-control-health".to_owned()),
            role: ProcessRole::GuestControlHealth,
            unit: None,
            binary_path: None,
            argv: vec![],
            env: vec![],
            plan_ops: vec![],
            profile: nixling_core::test_support::RoleProfileBuilder::new()
                .with_profile_id("gc-health")
                .with_uid(0)
                .with_gid(0)
                .build(),
            readiness: vec![],
        };

        // HAZARD: the stateless readiness helper treats an EMPTY predicate
        // slice as TRIVIALLY ready — it returns Ok without running any
        // probe. `spawn_and_check_process_alive` (the process-alive fast
        // path, e.g. `--no-wait-api`) delegates the node to
        // `spawn_and_wait_ready(vm, node, &[], budget)` with exactly this
        // empty slice. If a GuestControlHealth node ever reached
        // `wait_for_readiness`, an absent/auth-failing guest-control
        // listener would be reported "ready" with NO authenticated probe.
        assert_eq!(
            wait_for_readiness(&node, &[], Duration::from_millis(0), None),
            Ok(()),
            "empty readiness is trivially ready; the authenticated probe \
             must be reached via the GuestControlHealth interception, never \
             via this helper"
        );

        // GUARD: GuestControlHealth is a ReadinessOnly node, so
        // `spawn_and_check_process_alive` does NOT take the LongLived
        // process-alive-only short-circuit (which registers a node as alive
        // after spawn with no probe at all). It falls through to
        // `spawn_and_wait_ready`, whose `node.role == GuestControlHealth`
        // special case runs `wait_for_guest_control_health` BEFORE the empty
        // readiness slice can reach the trivially-ready `wait_for_readiness`.
        // Were GuestControlHealth ever made LongLived, or the interception
        // removed, the authenticated probe would be bypassed on this path.
        assert!(matches!(
            vm_start_node_mode(&ProcessRole::GuestControlHealth),
            VmStartNodeMode::ReadinessOnly
        ));
    }

    #[test]
    fn cloud_hypervisor_vsock_socket_extracts_socket_field() {
        use super::cloud_hypervisor_vsock_socket;
        use std::path::PathBuf;

        let argv = vec![
            "cloud-hypervisor".to_owned(),
            "--vsock".to_owned(),
            "cid=42,socket=/var/lib/nixling/vms/work/vsock.sock".to_owned(),
            "--api-socket".to_owned(),
            "/var/lib/nixling/vms/work/api.sock".to_owned(),
        ];
        assert_eq!(
            cloud_hypervisor_vsock_socket(&argv),
            Some(PathBuf::from("/var/lib/nixling/vms/work/vsock.sock"))
        );

        let no_vsock = vec!["cloud-hypervisor".to_owned(), "--api-socket".to_owned()];
        assert_eq!(cloud_hypervisor_vsock_socket(&no_vsock), None);

        // Old-generation / pre-guest-control CH argv: a `--vsock` device with no
        // `socket=` subfield resolves to None, which `resolve_guest_control_probe_params`
        // turns into a fail-closed `no-vsock-socket` error (no endpoint to probe,
        // so exec never proxies and never falls back).
        let vsock_without_socket = vec![
            "cloud-hypervisor".to_owned(),
            "--vsock".to_owned(),
            "cid=42".to_owned(),
        ];
        assert_eq!(cloud_hypervisor_vsock_socket(&vsock_without_socket), None);
    }

    #[test]
    fn read_guest_config_verb_is_admin_only() {
        use super::verb_requires_admin;
        // The verb crosses into the guest over the authenticated transport, so
        // a launcher / non-admin peer MUST be denied BEFORE any probe / sign /
        // read runs (the gate is enforced in `dispatch_request`).
        assert!(verb_requires_admin("readGuestConfig"));
    }

    #[test]
    fn shell_verb_is_admin_only() {
        use super::verb_requires_admin;
        // Shell operations cross into the guest and can attach/detach/kill a
        // workload-user terminal, so the daemon must deny launchers before any
        // session lookup, guest-control probe, or owner reservation.
        assert!(verb_requires_admin("shell"));
    }

    #[test]
    fn read_guest_config_dispatch_denies_launcher_before_any_side_effect() {
        // The broker socket is unreachable. If the admin gate did NOT
        // short-circuit, dispatch_read_guest_config would load the bundle
        // resolver, resolve probe params, BrokerSign, and read guest bytes —
        // producing a transport / broker error, never AuthzNotAdmin.
        // Receiving AuthzNotAdmin proves the launcher was denied at the gate
        // BEFORE any bundle load / probe / sign / guest-byte read.
        let state = test_state_with_broker_socket(unreachable_broker_socket_path(
            "read-guest-config-authz",
        ));
        let request = super::wire::Request::ReadGuestConfig(
            nixling_ipc::public_wire::ReadGuestConfigRequest {
                vm: "vm-a".to_owned(),
            },
        );
        let err = dispatch_request(&state, &launcher_peer(), request)
            .expect_err("launcher must be denied readGuestConfig");
        match &err {
            super::typed_error::TypedError::AuthzNotAdmin { verb } => {
                assert_eq!(verb, "readGuestConfig");
            }
            other => panic!("expected AuthzNotAdmin for readGuestConfig, got {other:?}"),
        }
        assert_eq!(err.exit_code(), 75);
    }

    #[test]
    fn read_guest_config_dispatch_admin_clears_gate_and_reaches_handler() {
        // The admin peer clears the authz gate, so dispatch reaches the
        // handler and fails LATER (the bundle vm has no guest-control node /
        // the broker is unreachable) with a guest-control read or transport
        // error — never an authz error. This proves the gate is the only
        // thing denying the launcher above, not some unrelated failure.
        let state = test_state_with_broker_socket(unreachable_broker_socket_path(
            "read-guest-config-admin",
        ));
        let request = super::wire::Request::ReadGuestConfig(
            nixling_ipc::public_wire::ReadGuestConfigRequest {
                vm: "vm-a".to_owned(),
            },
        );
        let err = dispatch_request(&state, &admin_peer(), request)
            .expect_err("the read must fail after the gate is cleared");
        assert!(
            !matches!(err, super::typed_error::TypedError::AuthzNotAdmin { .. }),
            "admin must clear the authz gate, got {err:?}"
        );
    }

    #[test]
    fn guest_file_read_error_maps_to_closed_daemon_kinds() {
        use super::map_guest_file_read_error;
        use crate::guest_control_health::{GuestControlHealthError as H, GuestFileReadError as E};
        use crate::typed_error::GuestControlReadErrorKind as K;

        let cases = [
            (E::Probe(H::TransportIo), K::Transport),
            (E::Probe(H::Signer), K::Transport),
            (E::Probe(H::Ttrpc), K::Transport),
            (E::Probe(H::Timeout), K::Timeout),
            (E::Probe(H::AuthFailed), K::AuthFailed),
            (E::Probe(H::StaleSession), K::AuthFailed),
            (E::Probe(H::Protocol), K::Protocol),
            (E::CapabilityUnavailable, K::CapabilityUnavailable),
            (E::FileNotFound, K::FileNotFound),
            (E::FileTooLarge, K::FileTooLarge),
            (E::PathUnsafe, K::PathUnsafe),
            (E::ReadDenied, K::ReadDenied),
            (E::Protocol, K::Protocol),
        ];
        for (input, expected) in cases {
            match map_guest_file_read_error(input) {
                crate::typed_error::TypedError::GuestControlReadFailed { kind } => {
                    assert_eq!(kind, expected);
                }
                other => panic!("expected GuestControlReadFailed, got {other:?}"),
            }
        }
    }
}

#[cfg(test)]
mod guest_control_readiness_tracing_tests {
    use super::emit_guest_control_readiness_event;
    use crate::guest_control_bridge::ReadinessObservation;
    use std::sync::{Arc, Mutex};
    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::{Layer, Registry};

    /// Captured events: one inner vec of `(field_name, value)` pairs per
    /// recorded tracing event.
    type CapturedEvents = Arc<Mutex<Vec<Vec<(String, String)>>>>;

    #[derive(Default)]
    struct FieldCollector {
        fields: Vec<(String, String)>,
    }
    impl Visit for FieldCollector {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.fields
                .push((field.name().to_string(), format!("{value:?}")));
        }
        fn record_str(&mut self, field: &Field, value: &str) {
            self.fields
                .push((field.name().to_string(), value.to_string()));
        }
        fn record_u64(&mut self, field: &Field, value: u64) {
            self.fields
                .push((field.name().to_string(), value.to_string()));
        }
        fn record_i64(&mut self, field: &Field, value: i64) {
            self.fields
                .push((field.name().to_string(), value.to_string()));
        }
    }

    struct CapturingLayer {
        events: CapturedEvents,
    }
    impl<S: tracing::Subscriber> Layer<S> for CapturingLayer {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            let mut collector = FieldCollector::default();
            event.record(&mut collector);
            self.events.lock().unwrap().push(collector.fields);
        }
    }

    fn observation(error_kind: &'static str, outcome: &'static str) -> ReadinessObservation {
        ReadinessObservation {
            subsystem: "guest-control-health",
            outcome,
            health_state: "degraded",
            health_reason: "quota-exceeded",
            error_kind,
            attempt_count: 3,
            duration_ms: 1234,
        }
    }

    #[test]
    fn readiness_tracing_events_carry_only_leak_safe_fields() {
        // APPROVED field-name allowlist for the readiness observation
        // events. Hardcoded (not derived from the call site) so that
        // adding a new field to the `tracing::info!`/`warn!` macro — e.g.
        // a raw path, nonce, or guest-supplied string — fails this test.
        const APPROVED_FIELDS: &[&str] = &[
            "message",
            "kind",
            "subsystem",
            "outcome",
            "health_state",
            "health_reason",
            "error_kind",
            "attempt_count",
            "duration_ms",
        ];
        // Field names that MUST NEVER appear on a readiness event.
        const FORBIDDEN_FIELDS: &[&str] = &[
            "vm",
            "env",
            "node",
            "role_id",
            "path",
            "socket",
            "state_dir",
            "store_path",
            "nonce",
            "token",
            "auth_tag",
            "guest_boot_id",
            "capabilities_hash",
            "peer_cid",
            "guest_bytes",
            "content",
            "error",
            "error_message",
        ];

        let events = Arc::new(Mutex::new(Vec::new()));
        let layer = CapturingLayer {
            events: events.clone(),
        };
        let subscriber = Registry::default().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            // Exercise BOTH the ready and not-ready arms so both call
            // sites are covered.
            emit_guest_control_readiness_event(&observation("none", "ready"), true);
            emit_guest_control_readiness_event(&observation("auth-failed", "not-ready"), false);
        });

        let captured = events.lock().unwrap();
        assert_eq!(captured.len(), 2, "expected exactly two readiness events");
        for fields in captured.iter() {
            assert!(!fields.is_empty(), "readiness event recorded no fields");
            for (name, value) in fields {
                assert!(
                    APPROVED_FIELDS.contains(&name.as_str()),
                    "unapproved readiness tracing field: {name}={value}"
                );
                assert!(
                    !FORBIDDEN_FIELDS.contains(&name.as_str()),
                    "forbidden readiness tracing field: {name}"
                );
                assert!(
                    !value.contains('/'),
                    "path-like value leaked: {name}={value}"
                );
                assert!(
                    !value.contains("SENTINEL"),
                    "guest content leaked: {name}={value}"
                );
            }
        }

        // The ready arm must NOT carry error_kind; the not-ready arm MUST.
        let ready_fields: Vec<&str> = captured[0].iter().map(|(n, _)| n.as_str()).collect();
        let not_ready_fields: Vec<&str> = captured[1].iter().map(|(n, _)| n.as_str()).collect();
        assert!(
            !ready_fields.contains(&"error_kind"),
            "ready event must not carry error_kind"
        );
        assert!(
            not_ready_fields.contains(&"error_kind"),
            "not-ready event must carry error_kind"
        );
    }
}

#[cfg(test)]
mod exec_established_tracing_tests {
    //! The single kind=critical exec session-establishment event must carry
    //! ONLY redaction-safe identifiers. This guards against a future edit that
    //! adds argv/env/cwd/output bytes (or any guest-supplied string) to the
    //! span, which would leak operator command lines into the daemon log.

    use super::emit_exec_established_event;
    use std::sync::{Arc, Mutex};
    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::{Layer, Registry};

    type CapturedEvents = Arc<Mutex<Vec<Vec<(String, String)>>>>;

    #[derive(Default)]
    struct FieldCollector {
        fields: Vec<(String, String)>,
    }
    impl Visit for FieldCollector {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.fields
                .push((field.name().to_string(), format!("{value:?}")));
        }
        fn record_str(&mut self, field: &Field, value: &str) {
            self.fields
                .push((field.name().to_string(), value.to_string()));
        }
        fn record_u64(&mut self, field: &Field, value: u64) {
            self.fields
                .push((field.name().to_string(), value.to_string()));
        }
        fn record_i64(&mut self, field: &Field, value: i64) {
            self.fields
                .push((field.name().to_string(), value.to_string()));
        }
        fn record_bool(&mut self, field: &Field, value: bool) {
            self.fields
                .push((field.name().to_string(), value.to_string()));
        }
    }

    struct CapturingLayer {
        events: CapturedEvents,
    }
    impl<S: tracing::Subscriber> Layer<S> for CapturingLayer {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            let mut collector = FieldCollector::default();
            event.record(&mut collector);
            self.events.lock().unwrap().push(collector.fields);
        }
    }

    #[test]
    fn exec_established_event_carries_only_leak_safe_fields() {
        // APPROVED field-name allowlist for the establishment event. The opaque
        // session handle is deliberately NOT approved — per AGENTS, session
        // handles must never reach a span, log, audit, or metric.
        const APPROVED_FIELDS: &[&str] = &["message", "kind", "subsystem", "vm", "peer_uid", "tty"];
        // Field names that MUST NEVER appear (would leak the command line, the
        // session handle, or guest-supplied content).
        const FORBIDDEN_FIELDS: &[&str] = &[
            "argv",
            "command",
            "cmd",
            "env",
            "cwd",
            "stdin",
            "stdout",
            "stderr",
            "output",
            "nonce",
            "token",
            "auth_tag",
            "exec_id",
            "guest_boot_id",
            "session_handle",
            "session",
            "handle",
        ];
        // A sentinel that would only appear if argv/env/cwd leaked into a field.
        const SENTINEL: &str = "NIXLING_ARGV_LEAK_CANARY";

        let events = Arc::new(Mutex::new(Vec::new()));
        let layer = CapturingLayer {
            events: events.clone(),
        };
        let subscriber = Registry::default().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            emit_exec_established_event("work", 1000, true);
        });

        let captured = events.lock().unwrap();
        assert_eq!(
            captured.len(),
            1,
            "expected exactly one establishment event"
        );
        for fields in captured.iter() {
            assert!(!fields.is_empty(), "establishment event recorded no fields");
            for (name, value) in fields {
                assert!(
                    APPROVED_FIELDS.contains(&name.as_str()),
                    "unapproved establishment tracing field: {name}={value}"
                );
                assert!(
                    !FORBIDDEN_FIELDS.contains(&name.as_str()),
                    "forbidden establishment tracing field: {name}"
                );
                assert!(
                    !value.contains(SENTINEL),
                    "argv/env/cwd sentinel leaked: {name}={value}"
                );
            }
        }
    }
}
