use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File, OpenOptions};
use std::future::Future;
use std::io::{IoSliceMut, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nix::cmsg_space;
use nix::fcntl::{fcntl, FcntlArg, FdFlag};
use nix::sys::socket::{
    connect, getsockopt, recv, recvmsg, send, socket, sockopt::PeerCredentials, AddressFamily,
    ControlMessageOwned, MsgFlags, SockFlag, SockType, UnixAddr,
};
use nix::unistd::{self, Gid, Group, Uid, User};
use nixling_core::bundle::Bundle;
use nixling_core::bundle_resolver::{
    intent_id_activation, intent_id_gc_host, intent_id_hosts_host, intent_id_installer_host,
    intent_id_keys_rotate, intent_id_migrate_host, intent_id_nft_host, intent_id_nm_unmanaged_host,
    intent_id_rotate_known_host, intent_id_route_env, intent_id_runner, intent_id_sysctl,
    intent_id_trust, BundleResolver,
};
use nixling_core::closures::ClosureMetadata;
use nixling_core::error::BundleError;
use nixling_core::host::{HostJson, Ipv6SysctlEntry};
use nixling_core::host_check;
use nixling_core::manifest_v04::{ManifestV04, VmEntry as ManifestVmEntry};
use nixling_core::processes::{ProcessNode, ProcessRole, ProcessesJson, ReadinessPredicate};
use nixling_host::ssh_keygen;
use nixling_ipc::{
    broker_wire::{
        ActivationMode as BrokerActivationMode, ApplyNftablesRequest as BrokerApplyNftablesRequest,
        ApplyNmUnmanagedRequest as BrokerApplyNmUnmanagedRequest,
        ApplyRouteRequest as BrokerApplyRouteRequest,
        ApplySysctlRequest as BrokerApplySysctlRequest, BrokerRequest, BrokerRequestEnvelope,
        BrokerResponse, OpenPidfdRequest as BrokerOpenPidfdRequest,
        RunActivationRequest as BrokerRunActivationRequest, RunGcRequest as BrokerRunGcRequest,
        RunHostInstallRequest as BrokerRunHostInstallRequest,
        RunHostKeyTrustRequest as BrokerRunHostKeyTrustRequest,
        RunKeysRotateRequest as BrokerRunKeysRotateRequest,
        RunMigrateRequest as BrokerRunMigrateRequest,
        RunRotateKnownHostRequest as BrokerRunRotateKnownHostRequest, RunnerRole,
        SpawnRunnerRequest as BrokerSpawnRunnerRequest,
        UpdateHostsFileRequest as BrokerUpdateHostsFileRequest,
        UsbipBindRequest as BrokerUsbipBindRequest,
        UsbipProxyReconcileRequest as BrokerUsbipProxyReconcileRequest,
        UsbipUnbindRequest as BrokerUsbipUnbindRequest,
    },
    public_wire::{self, AuthRole, AuthStatusResponse, DeniedCommandHint, SocketReachability},
    types::{BundleOpId, RoleId, ScopeId, VmId},
    KnownFeatureFlag, BROKER_SOCKET_PATH,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use socket2::{Domain, SockAddr, Socket, Type};
use supervisor::pidfd_table::{
    PidfdEntry, PidfdRegistration, PidfdTable, PidfdTableError, WaitTermination,
};
use uzers::{get_user_by_uid, get_user_groups};

pub mod supervisor;
pub mod typed_error;
pub mod wire;
// W4-H8: `[pending restart]` machinery. Pure module + filesystem
// reader trait so the CLI can compute the daemon-level
// pending-restart signal post-restart without requiring /run live.
pub mod daemon_version;
// P2 ph2-p2-ownership-matrix: per-VM state-directory ownership
// preflight invoked from `dispatch_broker_vm_start`. The pure
// enforcer lives in `nixling_host::ownership_matrix`.
pub mod ownership_preflight;
// P2 ph2-known-hosts-refresh: daemon-side replacement for the
// retired `nixling-known-hosts-refresh@<vm>.service` oneshot.
// Invoked from `dispatch_broker_vm_start` after the per-VM DAG
// reports `overall_ok` (i.e. the VM's readiness signal has
// fired). See `known_hosts_refresh` for the pure intent builder
// + side-effect wrapper.
pub mod known_hosts_refresh;
// P2 ph2-p2-ssh-host-key-preflight: per-VM sshd host key
// posture preflight invoked from `dispatch_broker_vm_start` and
// from the host-prep DAG executor. The pure check lives in
// `crate::ssh_host_key_preflight`.
pub mod ssh_host_key_preflight;
// P2 ph2-p2-net-vm-bundle-gate: refuses to start a `sys-<env>-net`
// VM when the on-disk dnsmasq.conf for that env diverges from the
// bundle's nft/route/hosts intent hash. Catches the case where the
// bundle was updated but the dnsmasq render step did not rerun.
// See `docs/reference/net-vm-bundle-gate.md`.
pub mod net_vm_bundle_gate;
// P3 ph3-p3-kernel-module-check: daemon startup self-check
// that verifies the kernel-module matrix the running config
// requires is loaded; refuses to start on missing required
// modules and marks VMs as degraded on optional misses. See
// `docs/reference/kernel-module-check.md`.
pub mod kernel_module_check;
// P3 ph3-p3-otelbridge-readiness: typed readiness gate that
// blocks `dispatch_broker_vm_start` from declaring the
// observability VM successful until the broker-spawned
// OtelHostBridge runner has registered its pidfd AND opened its
// obs vsock host socket. On timeout the daemon falls back to
// degraded mode (VM is up; observability annotated as broken).
// See `docs/reference/otel-host-bridge-readiness.md`.
pub mod otel_host_bridge_readiness;
// P3 ph3-p3-net-route-degraded-mode: daemon startup self-check
// that replaces the legacy `nixling-net-route-preflight.service`
// host singleton (scheduled for removal in P6). Probes each env's
// LAN bridge, persists a small history, and engages an
// operator-only mode after N consecutive failures. Recovery is
// via the new `nixling host reconcile --network --apply` verb.
// See `docs/explanation/host-prepare.md` and plan.md row
// `ph3-p3-net-route-degraded-mode`.
pub mod net_route_preflight;
// P2 ph2-p2-daemon-autostart: contract for bringing autostart VMs
// up on daemon startup (net VMs first, concurrency cap,
// degraded-mode tolerant, idempotent). See
// docs/reference/daemon-autostart.md.
pub mod autostart;
// P3 ph3-usbipd-perenv: daemon-side per-env usbipd autostart. Folds
// the 9 transitional `nixling-sys-<env>-usbipd-{backend,proxy}.{service,socket}`
// units into broker `SpawnRunner` with `RunnerRole::Usbip`, keyed
// per-env on `vm_id = sys-<env>-usbipd` with role_ids `backend` /
// `proxy`. See plan.md row `ph3-usbipd-perenv`.
pub mod usbipd_perenv_autostart;
// P3 ph3-p3-prometheus-otlp-shape: Prometheus scrape endpoint shape.
// Owns the canonical metric inventory (see
// `docs/reference/daemon-metrics.md`) and a minimal HTTP/1.1
// `GET /metrics` handler. The registry is process-local; serving is
// wired through the daemon's public socket accept loop.
pub mod metrics;
// P3 ph3-p3-ch-exporter-retire: per-VM Cloud Hypervisor stats
// scraper folded into the daemon's `/metrics` endpoint. Replaces
// the host-side `nixling-ch-exporter.service` singleton (still
// installed during P3 transition, removed in P6). See
// `docs/reference/daemon-metrics.md` for the metric inventory.
pub mod ch_stats;
// P3 ph3-p3-audit-check-retire: in-daemon replacement for the
// `nixling-audit-check.{service,timer}` host singleton + timer that
// previously sanity-checked broker audit log shape on a daily cadence.
// Exposes `GET /health/audit-check` on the daemon's HTTP surface and
// a pure check function suitable for invocation from the supervisor
// event loop. See `docs/reference/daemon-audit-check.md`.
pub mod audit_check;
// P3 ph3-p3-usbip-state-machine: typed, per-busid USBIP state
// machine that pins the canonical bring-up order
// `modprobe → lock → withhold → firewall → backend → bind → proxy`
// (AGENTS.md "Critical subsystems"). Each step is a typed broker
// op or daemon-side action; failures are fail-fast and surface as
// `TypedError::UsbipStepFailed { busid, step, reason }`
// (exit code 67). See `docs/reference/usbip-state-machine.md`.
pub mod usbip_state_machine;

use typed_error::TypedError;

pub const DEFAULT_CONFIG_PATH: &str = "/etc/nixling/daemon-config.json";
pub const DEFAULT_SERVER_VERSION: &str = "0.4.0";
pub const DEFAULT_ACCEPTED_VERSION_RANGE: &str = ">=0.4.0, <0.5.0";
pub const DEFAULT_DAEMON_STATE_DIR: &str = "/var/lib/nixling/daemon-state";
const VM_RUNNER_ROLE_ID: &str = "ch-runner";
const VM_STOP_TIMEOUT: Duration = Duration::from_secs(30);

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
    /// P2 ph2-p2-daemon-autostart: concurrency cap for the
    /// autostart pass that runs on daemon startup. Default `3`.
    /// Mirrors `nixling.daemon.autostart.parallelism`.
    #[serde(default = "default_autostart_parallelism")]
    pub autostart_parallelism: usize,
}

fn default_autostart_parallelism() -> usize {
    autostart::DEFAULT_PARALLELISM
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
            public_socket_group: "nixling-launchers".to_owned(),
            launcher_users: Vec::new(),
            admin_users: Vec::new(),
            server_version: default_server_version(),
            accepted_client_version_range: default_accepted_version_range(),
            artifacts: ArtifactPaths::default(),
            autostart_parallelism: autostart::DEFAULT_PARALLELISM,
        }
    }
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
}

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

/// P3 ph3-p3-host-doctor-extended: path of the persisted
/// kernel-module-check report. `nixling host doctor --read-only`
/// reads this file to surface the kernel-module matrix posture
/// without re-running the bundle resolver in the CLI process.
pub fn kernel_module_report_path(daemon_state_dir: &Path) -> PathBuf {
    daemon_state_dir.join("kernel-module-report.json")
}

/// P3 ph3-p3-host-doctor-extended: path of the persisted
/// autostart-pass report (summary + per-VM outcomes). `nixling host
/// doctor --read-only` reads this file to report degraded-VM count.
pub fn autostart_report_path(daemon_state_dir: &Path) -> PathBuf {
    daemon_state_dir.join("autostart-report.json")
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
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(err) = std::fs::write(&path, &json) {
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
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(err) = std::fs::write(&path, &json) {
        tracing::warn!(
            error = %err,
            path = %path.display(),
            "autostart: persist report failed",
        );
    }
}

pub fn banner() -> String {
    "nixlingd 0.0.0-bootstrap (W0a stub)".to_owned()
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

pub async fn serve(options: ServeOptions) -> Result<(), TypedError> {
    let mut config = load_config(&options.config_path)?;
    apply_overrides(&mut config, &options);
    let runtime_identity =
        resolve_runtime_identity(&config, options.allow_unprivileged_runtime_dir)?;
    validate_lock_parent(&config.state_lock_path, &runtime_identity)?;
    ensure_locks_dir(&config.locks_dir, &runtime_identity)?;
    let _lock_file = acquire_state_lock(&config.state_lock_path, &runtime_identity)?;
    let listener = bind_public_socket(&config.public_socket_path, &runtime_identity)?;

    if options.drop_privileges {
        drop_privileges_if_root(&runtime_identity)?;
    }

    // W4-H8 + W4 GPT-5.5 panel: write /run/nixling/version on
    // daemon startup so the CLI's [pending restart] machinery has
    // an authoritative version + binary-path snapshot. Failures are
    // logged but non-fatal — operators can still drive the daemon
    // without the pending-restart signal.
    write_daemon_version_file();
    maybe_write_state_restore_report(&options)?;

    let daemon_state_dir = effective_daemon_state_dir(&options);
    let pidfd_table_path = pidfd_table_state_path(&daemon_state_dir);
    let pidfd_table = Arc::new(PidfdTable::restore_from_disk(&pidfd_table_path).map_err(
        |err| TypedError::InternalIo {
            context: format!("restore pidfd table {}", pidfd_table_path.display()),
            detail: err.to_string(),
        },
    )?);

    let state = ServerState {
        daemon_uid: runtime_identity.daemon_uid.as_raw(),
        config,
        daemon_state_dir,
        pidfd_table,
    };
    adopt_orphaned_runners_on_startup(&state);

    // P3 ph3-p3-kernel-module-check: startup self-check on the
    // kernel-module matrix the bundle requires. Fatal misses
    // refuse daemon start; optional misses are logged and the
    // affected VMs are skipped (Degraded) by the autostart pass.
    // If the bundle resolver itself is unavailable we skip the
    // gate — the autostart pass already logs and short-circuits
    // in that case, and the daemon must remain reachable for
    // diagnostic verbs (status / doctor / audit).
    let module_degraded_vms: BTreeSet<String> = match load_bundle_resolver(&state) {
        Ok(resolver) => {
            let report = kernel_module_check::run_kernel_module_check(&resolver);
            if report.is_fatal() {
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
    // P3 ph3-p3-net-route-degraded-mode: daemon-side net-route
    // preflight (replaces `nixling-net-route-preflight.service`).
    // For each env in the host artifact, probe its LAN bridge.
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
                        if let Some(env) = &vm.env {
                            if failed_envs.contains(env) {
                                net_pre_degraded_vms.insert(name.clone());
                            }
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
        let result = handle_connection(stream, &state);
        if let Err(error) = result {
            eprintln!("{}", error.message());
        }
        if options.once {
            break;
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
        if let Ok(value) = serde_json::from_slice::<Value>(&response) {
            if let Some(code) = value
                .get("error")
                .and_then(|error| error.get("exitCode"))
                .and_then(Value::as_u64)
            {
                exit_code = code as u8;
            }
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
// P2 ph2-p2-daemon-autostart: nixlingd autostart contract glue.
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
        };
        match dispatch_broker_vm_start(&self.state, request) {
            Ok(value) => {
                // dispatch_broker_vm_start returns a JSON envelope
                // even on logical failure (so the public verb can
                // surface it). For autostart we treat any
                // non-applied envelope as a failure so the
                // degraded-mode bookkeeping kicks in.
                if value
                    .get("disposition")
                    .and_then(|v| v.as_str())
                    .map(|s| s == "applied")
                    .unwrap_or(false)
                {
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

    // P3 ph3-usbipd-perenv: after per-VM autostart settles, fold the
    // 9 transitional per-env usbipd systemd units (`nixling-sys-
    // <env>-usbipd-{backend,proxy}.{service,socket}`) into broker
    // SpawnRunner with `RunnerRole::Usbip`. The spawn path is
    // idempotent (`is_running` short-circuit) and degrades fail-open
    // to `SkippedPendingBundle` when the bundle does not yet carry
    // the `sys-<env>-usbipd` runner intents — the singleton units
    // remain load-bearing through P3 → P5 while bundle wiring lands
    // in P6. See plan.md row `ph3-usbipd-perenv`.
    run_usbipd_perenv_autostart(state, &resolver).await;
}

/// Drive the per-env usbipd spawn plan derived from the manifest.
/// Best-effort: any failure to dispatch a single env's spawn is
/// logged and the loop continues; the transitional NixOS units
/// remain in place to keep operators served while the daemon path
/// bakes in production.
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
/// Translates `BundleIntentMissing` into `SkippedPendingBundle` per
/// the trait contract so the transitional window (P3 → P5) does not
/// fail-closed before `processes-json.nix` grows the new DAGs.
struct BrokerPerEnvUsbipdSpawner {
    state: Arc<ServerState>,
}

impl usbipd_perenv_autostart::PerEnvUsbipdSpawner for BrokerPerEnvUsbipdSpawner {
    fn is_running(&self, vm_id: &str, role_id: &str) -> bool {
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
            Ok((BrokerResponse::SpawnRunner(_), received_fds)) => {
                // We do not register a pidfd here; the per-env
                // usbipd lifecycle is owned by the broker spawn
                // (the daemon currently does not host a pidfd table
                // slot for `sys-<env>-usbipd`). Close any received
                // fds to avoid leaks.
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
    // W3fu5 H3 (security-1): the production tmpfile rule installs
    // /run/nixling as `nixlingd:nixling-launchers 0750` so launcher
    // users (members of `nixling-launchers`) can traverse the directory
    // to reach `/run/nixling/public.sock` (mode 0660, group
    // nixling-launchers). The previous validation expected the
    // root-owned 0755 shape that pre-dates W3fu2 H5; under the
    // non-root daemon it would have refused to start. The expected
    // shape now matches the systemd tmpfile contract: owner =
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
    // W3fu5 H3 (security-1): always chgrp the socket to
    // `public_socket_gid` (i.e. `nixling-launchers` in production).
    // The previous `geteuid().is_root()` gate meant the non-root
    // systemd unit (User=nixlingd, SupplementaryGroups=nixling-launchers
    // per W3fu2 H5) left the socket with group `nixlingd`, which made
    // launcher users unable to connect even though they have a seat in
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

/// W4-H8 + W4 GPT-5.5 panel notable #3: write the daemon's
/// canonicalized binary path + version + start-time to
/// `/run/nixling/version` on startup so the CLI's
/// `daemon_version::compute_restart_status` can compute the
/// `[pending restart]` signal post-restart. Failures are logged
/// to stderr and non-fatal — the absence of the version file
/// surfaces in the CLI as `DaemonRestartStatus::DaemonNotRunning`,
/// which is a reasonable degraded shape.
fn write_daemon_version_file() {
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
    let path = std::path::Path::new("/run/nixling/version");
    if let Some(parent) = path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            eprintln!("nixlingd: could not create /run/nixling for version file: {err}");
            return;
        }
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

fn handle_connection(stream: Socket, state: &ServerState) -> Result<(), TypedError> {
    let hello_bytes = read_frame(&stream)?;
    let peer = match authorize_peer(&stream, state) {
        Ok(peer) => peer,
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

fn authorize_peer(stream: &Socket, state: &ServerState) -> Result<PeerIdentity, TypedError> {
    let peer_override = match peer_override_from_env()? {
        Some(peer) => peer,
        None => {
            let peer = getsockopt(stream, PeerCredentials).map_err(io_wrap("read SO_PEERCRED"))?;
            PeerOverride {
                uid: peer.uid() as u32,
                gid: peer.gid() as u32,
                username: None,
                groups: None,
            }
        }
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

    Ok(PeerIdentity { role })
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
            | "migrate"
            | "hostPrepare"
            | "hostDestroy"
            | "hostInstall"
            | "hostReconcile"
    )
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
    match request {
        wire::Request::List(request) => dispatch_list(state, request),
        wire::Request::Status(request) => dispatch_status(state, request),
        wire::Request::Audit(request) => dispatch_audit(state, peer, request),
        wire::Request::HostCheck(request) => dispatch_host_check(state, request),
        wire::Request::AuthStatus => Ok(dispatch_auth_status(state, peer)),
        wire::Request::KeysList => dispatch_keys_list(state),
        wire::Request::KeysShow(request) => dispatch_keys_show(state, request),
        // W14d: mutating-verb apply dispatch is now fully direct.
        // All 13 W14 backlog verbs route from these request arms
        // straight to their `dispatch_broker_<verb>` helpers, and the
        // HostInstall/Migrate paths stay on their dedicated broker
        // helpers from W14b/W15.
        //
        // The old shared `dispatch_mutating_verb` split no longer
        // applies in nixlingd; only `mutating_verb_preflight` remains
        // to emit the typed InvalidRequest / dry-run-planned envelope
        // before apply dispatch runs.
        wire::Request::VmStart(req) => dispatch_broker_vm_start(state, req),
        wire::Request::VmStop(req) => dispatch_broker_vm_stop(state, req),
        wire::Request::VmRestart(req) => dispatch_broker_vm_restart(state, req),
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
        wire::Request::Migrate(req) => dispatch_broker_run_migrate(state, req),
        wire::Request::HostPrepare(req) => dispatch_broker_host_prepare(state, req),
        wire::Request::HostDestroy(req) => dispatch_broker_host_destroy(state, req),
        wire::Request::HostInstall(req) => dispatch_broker_run_host_install(state, req),
        wire::Request::HostReconcile(req) => dispatch_broker_host_reconcile(state, req),
    }
}

fn dispatch_keys_list(state: &ServerState) -> Result<Value, TypedError> {
    let bundle: Bundle = load_json(&state.config.artifacts.bundle_path)?;
    let manifest: ManifestV04 = load_json(&state.config.artifacts.public_manifest_path)?;
    let ssh_keygen_binary = PathBuf::from("/run/current-system/sw/bin/ssh-keygen");
    let entries = manifest
        .vms
        .iter()
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
    Ok(applied_response(
        VERB,
        format!(
            "nixling usb attach --apply: bound busid '{}' for vm '{}' via the native daemon → broker path",
            request.bus_id, request.vm
        ),
    ))
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
    Ok(applied_response(
        VERB,
        format!(
            "nixling usb detach --apply: unbound busid '{}' for vm '{}' via the native daemon → broker path",
            request.bus_id, request.vm
        ),
    ))
}

fn dispatch_broker_usbip_probe(state: &ServerState) -> Result<Value, TypedError> {
    const VERB: &str = "usb probe";
    if let Err(response) = dispatch_broker_ack_request(
        state,
        VERB,
        "UsbipProxyReconcile",
        BrokerRequest::UsbipProxyReconcile(BrokerUsbipProxyReconcileRequest {
            scope_id: ScopeId::new("host"),
        }),
    ) {
        return Ok(response);
    }
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
    let entries = resolver
        .usbip_bind_intent_ids()
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
            }
        })
        .collect();
    Ok(wire::usbip_probe_response(
        public_wire::UsbipProbeResponse { entries },
    ))
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
        }));
    }

    if flags.dry_run {
        let summary = match target_vm {
            Some(vm) => format!("nixling {verb} --dry-run: daemon-side plan for vm '{vm}' (W14b)"),
            None => format!("nixling {verb} --dry-run: daemon-side plan (W14b)"),
        };
        return Some(wire::mutating_verb_response(MutatingVerbResponse {
            verb: verb.to_owned(),
            outcome: MutatingVerbOutcome::DryRunPlanned,
            target_wave: None,
            summary: Some(summary),
            remediation: None,
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

fn dispatch_broker_request(
    state: &ServerState,
    request: BrokerRequest,
) -> Result<BrokerResponse, TypedError> {
    let socket_path = broker_socket_path(state);
    let socket = connect_seqpacket(&socket_path)?;
    write_json_frame(
        &socket,
        &BrokerRequestEnvelope {
            request,
            caller_role: Default::default(),
            test_peer_uid: None,
        },
    )?;
    let response = read_frame(&socket)?;
    serde_json::from_slice(&response).map_err(|err| TypedError::InternalBrokerUnavailable {
        path: socket_path,
        detail: err.to_string(),
    })
}

fn dispatch_broker_request_with_fds_timeout(
    state: &ServerState,
    request: BrokerRequest,
    timeout: Duration,
) -> Result<(BrokerResponse, Vec<RawFd>), TypedError> {
    let socket_path = broker_socket_path(state);
    let socket = Socket::from(connect_seqpacket(&socket_path)?);
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
            "{op_name} references a bundle intent that the broker did not find. Admin: ask `nixling audit --strict` for the intent id."
        ),
        "Broker.StoreViewFilesystemMismatch" => format!(
            "{op_name} refused: the per-VM store view is not on the same filesystem as /nix/store. Admin: check the VM state dir layout and retry."
        ),
        "Broker.StoreViewMarkerMissing" => format!(
            "{op_name} refused: the prepared store-view generation is missing its marker. Admin: rebuild the store view and retry."
        ),
        "Broker.LiveHandlerFailed" => format!(
            "{op_name} failed at the broker live handler. Admin: inspect `nixling audit --strict` for the underlying syscall/exit code."
        ),
        "Broker.CoexistenceRefused" => "{op_name} refused: another firewall manager owns the table per FirewallCoexistencePolicy. Admin: check nixling.site.firewallCoexistencePolicy."
            .replace("{op_name}", op_name),
        "Broker.NftScriptParseFailed" => "{op_name} failed: bundle nft script could not be parsed. Admin: inspect `nixling audit --strict` for the parse error."
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
            "{op_name} failed; admin should inspect `nixling audit --strict` for details"
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
    BundleResolver::load(&state.config.artifacts.bundle_path).map_err(|err| match err {
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
        ProcessRole::Gpu => VmStartNodeMode::LongLived(RunnerRole::Gpu),
        ProcessRole::Audio => VmStartNodeMode::LongLived(RunnerRole::Audio),
        ProcessRole::Video => VmStartNodeMode::LongLived(RunnerRole::Video),
        ProcessRole::VsockRelay => VmStartNodeMode::LongLived(RunnerRole::VsockRelay),
        ProcessRole::Usbip => VmStartNodeMode::LongLived(RunnerRole::Usbip),
        ProcessRole::HostReconcile
        | ProcessRole::StoreVirtiofsPreflight
        | ProcessRole::GuestSshReadiness => VmStartNodeMode::ReadinessOnly,
    }
}

fn tracked_role_id(node: &ProcessNode) -> String {
    match node.role {
        ProcessRole::CloudHypervisorRunner => VM_RUNNER_ROLE_ID.to_owned(),
        _ => node.id.0.clone(),
    }
}

struct VmStartRunner<'a> {
    state: &'a ServerState,
    resolver: &'a BundleResolver,
}

impl VmStartRunner<'_> {
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
        match dispatch_broker_request_with_fds_timeout(
            self.state,
            BrokerRequest::SpawnRunner(BrokerSpawnRunnerRequest {
                vm_id: VmId::new(vm),
                role_id: RoleId::new(role_id),
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
        match vm_start_node_mode(&node.role) {
            VmStartNodeMode::ReadinessOnly => wait_for_readiness(node, readiness, budget.readiness),
            VmStartNodeMode::OneShot(runner_role) => {
                let response = self.spawn_runner(vm, node, runner_role, budget.spawn)?;
                wait_for_one_shot_exit(response.pid, response.start_time_ticks, budget.readiness)
            }
            VmStartNodeMode::LongLived(runner_role) => {
                let response = self.spawn_runner(vm, node, runner_role, budget.spawn)?;
                if let Err(error) = wait_for_readiness(node, readiness, budget.readiness) {
                    cleanup_vm_start_registration(self.state, vm, &tracked_role_id(node));
                    return Err(error);
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
}

fn wait_for_readiness(
    node: &ProcessNode,
    readiness: &[ReadinessPredicate],
    timeout: Duration,
) -> Result<(), String> {
    if readiness.is_empty() {
        return Ok(());
    }
    let deadline = Instant::now() + timeout;
    loop {
        let mut all_ready = true;
        for predicate in readiness {
            if !readiness_predicate_ready(predicate)? {
                all_ready = false;
                break;
            }
        }
        if all_ready {
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
        ReadinessPredicate::TcpPort { host, port } => Ok(tcp_port_ready(host, *port)),
        ReadinessPredicate::Command(command) => command_ready(command),
        ReadinessPredicate::ComponentSpecific(_) => Ok(true),
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

fn tcp_port_ready(host: &str, port: u16) -> bool {
    let Ok(addrs) = format!("{host}:{port}").to_socket_addrs() else {
        return false;
    };
    addrs
        .into_iter()
        .any(|addr| TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok())
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

fn wait_for_one_shot_exit(
    pid: i32,
    start_time_ticks: u64,
    timeout: Duration,
) -> Result<(), String> {
    let proc_reader = supervisor::state::SystemProcReader;
    let deadline = Instant::now() + timeout;
    loop {
        match supervisor::state::ProcReader::proc_starttime(&proc_reader, pid) {
            Ok(Some(observed)) if observed == start_time_ticks => {
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

fn cleanup_vm_start_registration(state: &ServerState, vm: &str, role_id: &str) {
    let _ = state.pidfd_table.deregister(vm, role_id);
    if let Err(error) = state.pidfd_table.snapshot() {
        tracing::warn!(vm = %vm, role = %role_id, error = ?error, "failed to persist pidfd table cleanup");
    }
    remove_runner_snapshot(state, vm, role_id);
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
        Some(RunnerRole::Gpu) => 1,
        Some(RunnerRole::Audio) => 2,
        Some(RunnerRole::Video) => 3,
        Some(RunnerRole::Usbip) => 4,
        Some(RunnerRole::VsockRelay) => 5,
        // P1: OtelHostBridge is observability infrastructure; stop
        // it before swtpm/virtiofsd so trailing OTel spans flush
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

fn stop_vm_pidfd_role(
    state: &ServerState,
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

    match state.pidfd_table.wait_terminated(vm, role_id, term_timeout) {
        Ok(WaitTermination::Terminated) => {
            tracing::info!(
                vm = %vm,
                role = %role_id,
                signal = "SIGTERM",
                timeout_ms = term_timeout.as_millis(),
                outcome = "terminated",
                "role terminated after SIGTERM"
            );
            let _ = state.pidfd_table.deregister(vm, role_id);
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

    match state.pidfd_table.wait_terminated(vm, role_id, kill_timeout) {
        Ok(WaitTermination::Terminated) => {
            tracing::info!(
                vm = %vm,
                role = %role_id,
                signal = "SIGKILL",
                timeout_ms = kill_timeout.as_millis(),
                outcome = "terminated",
                "role terminated after SIGKILL"
            );
            let _ = state.pidfd_table.deregister(vm, role_id);
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

/// P2 ph2-dag-host-prep: log the planned host-prep DAG so
/// `journalctl -u nixlingd.service` and the autostart-history
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

/// P3 host-prep-broker-arms: extract a role id from the DAG step's
/// bundle_ref, falling back to a step-default. The runner intent id
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

/// P3 host-prep-broker-arms: dispatch a broker request for one
/// host-prep DAG step where the broker may return a typed response
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

/// P2 ph2-dag-host-prep: execute the host-prep DAG by dispatching
/// the corresponding broker op for each step in topo order. On
/// step failure surfaces the broker envelope; the operator sees
/// the step id, the broker op kind, and the broker error string
/// (the typed `HostPrepStepFailed` shape lives in
/// `nixling_host::host_prep_dag`). Gated by
/// `NIXLING_HOST_PREP_DAG_EXECUTE` until the P2/P3 broker handlers
/// land.
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
                // P2 ph2-p2-ssh-host-key-preflight: run the
                // daemon-native posture check instead of dispatching
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
            // P3 host-prep-broker-arms: live broker dispatch for the
            // 5 step kinds that previously skipped-with-log. Each
            // arm composes an existing W3 broker op.
            HostPrepStepKind::BringUpTapInterface => {
                // P3 host-prep-broker-arms: compose CreatePersistentTap.
                // The DAG anchors tap ownership via
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
                // P3 host-prep-broker-arms: dispatch OpenVhostNet for
                // role `ch`. The broker returns an SCM_RIGHTS fd
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
                // P3 host-prep-broker-arms: compose ApplyNmUnmanaged
                // against the single host-wide intent row
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
                // P3 host-prep-broker-arms: iterate the resolver's
                // sysctl intent ids for this VM's env and dispatch
                // ApplySysctl per key. The bundle's per-tap entries
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
                // P3 host-prep-broker-arms: dispatch SetBridgePortFlags
                // for role `ch`. The broker returns a typed
                // BridgePortFlagsResponse, not an Ack; the host-prep
                // dispatcher accepts any non-Error response.
                let role_id = host_prep_role_id_from_bundle_ref(&step.bundle_ref, "ch");
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
            // P3 ph3-p3-net-route-degraded-mode: HostNetRoutePreflight
            // is host-scope and is executed inline by the daemon at
            // startup (and via `nixling host reconcile --network`).
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

    // P2 ph2-p2-net-vm-bundle-gate: for net VMs (`sys-<env>-net`),
    // refuse start if the on-disk dnsmasq.conf hash diverges from
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

    // P2 ph2-dag-host-prep: build the host-prep DAG for this VM
    // and (optionally) execute it before driving the per-VM
    // process DAG. The DAG is logged unconditionally so operators
    // and gates can observe the planned step set; actual broker
    // dispatch is gated on `NIXLING_HOST_PREP_DAG_EXECUTE=1`. As of
    // P3 host-prep-broker-arms, all 10 step kinds dispatch a real
    // broker op (or a daemon-native check) — `OwnershipMatrixCheck`
    // and `SshHostKeyPreflight` still cover the two P2 stubs that
    // intentionally remain typed-Unimplemented at the broker layer
    // pending sibling P3 wave-B handlers.
    let host_prep_steps =
        nixling_host::host_prep_dag::build_host_prep_dag(request.vm.as_str(), &resolver);
    log_host_prep_dag(&request.vm, &host_prep_steps);
    if std::env::var("NIXLING_HOST_PREP_DAG_EXECUTE")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        if let Err(response) = execute_host_prep_dag(state, &request.vm, &host_prep_steps) {
            return Ok(response);
        }
    }

    let dag = resolver
        .processes
        .vms
        .iter()
        .find(|dag| dag.vm == request.vm)
        .ok_or_else(|| TypedError::InternalIo {
            context: format!("load process DAG for {}", request.vm),
            detail: "VM not present in processes.json".to_owned(),
        })?;

    // P2 ph2-p2-ownership-matrix: refuse VM start if any per-VM
    // state subdirectory has drifted from the typed ownership matrix
    // declared in nixos-modules/options-ownership-matrix.nix. Missing
    // subdirectories surface as warn-only (state is materialized
    // lazily); owner/group/mode drift on existing paths fails closed.
    if let Some(manifest_entry) = resolver.manifest.vms.get(&request.vm) {
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

        // P2 ph2-p2-ssh-host-key-preflight: refuse VM start if the
        // per-VM sshd host keys directory or any `ssh_host_*_key`
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
    if tracked_roles
        .iter()
        .any(|role_id| state.pidfd_table.contains(&request.vm, role_id))
    {
        return Ok(invalid_request_response(
            VERB,
            format!(
                "vm '{}' already has a registered supervisor pidfd ({})",
                request.vm,
                tracked_roles
                    .into_iter()
                    .find(|role_id| state.pidfd_table.contains(&request.vm, role_id))
                    .unwrap_or_else(|| VM_RUNNER_ROLE_ID.to_owned())
            ),
        ));
    }

    let runner = VmStartRunner {
        state,
        resolver: &resolver,
    };
    let report = match block_on_future(supervisor::dag::DagExecutor::new(runner).run(dag)) {
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
    if report.overall_ok {
        // P3 ph3-p3-otelbridge-readiness: when the VM that just
        // came up is the observability VM AND observability is
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
        // P2 ph2-known-hosts-refresh: post-readiness trigger.
        // The per-VM DAG's `GuestSshReadiness` node is the
        // canonical sd_notify-from-guest signal; once the DAG
        // reports overall_ok we know sshd inside the VM has
        // accepted at least one probe, so it is safe to pin the
        // host pubkey into `/var/lib/nixling/known_hosts.nixling`
        // via the broker. Failures here are warn-only — matching
        // the legacy `nixling-known-hosts-refresh@<vm>.service`
        // behaviour, which left the old pin in place rather than
        // failing the VM start.
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
        return Ok(applied_response(VERB, vm_start_success_summary(&report)));
    }
    Ok(vm_start_failure_response(&report))
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

fn dispatch_broker_vm_stop(
    state: &ServerState,
    request: public_wire::VmLifecycleRequest,
) -> Result<Value, TypedError> {
    dispatch_broker_vm_stop_with_timeout(state, request, VM_STOP_TIMEOUT, VM_STOP_TIMEOUT)
}

fn dispatch_broker_vm_stop_with_timeout(
    state: &ServerState,
    request: public_wire::VmLifecycleRequest,
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

fn dispatch_broker_vm_restart(
    state: &ServerState,
    request: public_wire::VmLifecycleRequest,
) -> Result<Value, TypedError> {
    const VERB: &str = "vm restart";

    if let Some(response) = mutating_verb_preflight(VERB, &request.flags, Some(request.vm.as_str()))
    {
        return Ok(response);
    }

    let stop_response = dispatch_broker_vm_stop(state, request.clone())?;
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

/// P3 `ph3-p3-net-route-degraded-mode`: SOLE mutating recovery
/// verb after the daemon enters operator-only mode. Re-applies
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
    dispatch_broker_activation(state, request, "switch", BrokerActivationMode::Switch)
}

fn dispatch_broker_boot(
    state: &ServerState,
    request: public_wire::ActivationRequest,
) -> Result<Value, TypedError> {
    dispatch_broker_activation(state, request, "boot", BrokerActivationMode::Boot)
}

fn dispatch_broker_test(
    state: &ServerState,
    request: public_wire::ActivationRequest,
) -> Result<Value, TypedError> {
    dispatch_broker_activation(state, request, "test", BrokerActivationMode::Test)
}

fn dispatch_broker_rollback(
    state: &ServerState,
    request: public_wire::ActivationRequest,
) -> Result<Value, TypedError> {
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

fn dispatch_broker_keys_rotate(
    state: &ServerState,
    request: public_wire::KeysRotateRequest,
) -> Result<Value, TypedError> {
    const VERB: &str = "keys rotate";
    const OP_NAME: &str = "RunKeysRotate";

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
    let vms = manifest
        .into_iter()
        .filter(|(name, _)| !name.starts_with('_'))
        .filter(|(name, _)| request.vm.as_ref().map(|vm| vm == name).unwrap_or(true))
        .filter(|(_, value)| {
            request
                .env
                .as_ref()
                .map(|env| value.get("env").and_then(Value::as_str) == Some(env.as_str()))
                .unwrap_or(true)
        })
        .map(|(name, value)| {
            json!({
                "name": name,
                "env": value.get("env").cloned().unwrap_or(Value::Null),
                "staticIp": value.get("staticIp").cloned().unwrap_or(Value::Null),
                "isNetVm": value.get("isNetVm").cloned().unwrap_or(Value::Bool(false)),
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
    let bundle = load_json::<Bundle>(&state.config.artifacts.bundle_path)?;
    let processes = load_json::<ProcessesJson>(&state.config.artifacts.processes_path)?;
    let requested_vm = request.vm.clone();

    let statuses = manifest
        .iter()
        .filter(|(name, _)| !name.starts_with('_'))
        .filter(|(name, _)| requested_vm.as_ref().map(|vm| vm == *name).unwrap_or(true))
        .map(|(name, manifest_entry)| {
            let closure = load_closure(&state.config.artifacts.closures_dir, name).ok();
            let process_nodes = processes
                .vms
                .iter()
                .find(|vm| vm.vm == *name)
                .map(|vm| vm.nodes.len())
                .unwrap_or(0);
            json!({
                "vm": name,
                "env": manifest_entry.get("env").cloned().unwrap_or(Value::Null),
                "staticIp": manifest_entry.get("staticIp").cloned().unwrap_or(Value::Null),
                "bundleVersion": bundle.bundle_version,
                "processNodes": process_nodes,
                "runnerParityOk": closure.as_ref().map(|value| value.runner_parity_ok).unwrap_or(false),
                "runtime": "unknown (daemon-experimental, W4 not landed)",
                "checkBridges": request.check_bridges,
            })
        })
        .collect::<Vec<_>>();

    Ok(wire::status_response(json!({ "entries": statuses })))
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
            vec!["list", "status", "audit", "host check", "auth status"],
            Vec::new(),
        )
    } else {
        (
            AuthRole::Launcher,
            vec!["list", "status", "host check", "auth status"],
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

fn load_closure(closures_dir: &Path, vm: &str) -> Result<ClosureMetadata, TypedError> {
    load_json(&closures_dir.join(format!("{vm}.json")))
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
            })
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
            })
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
    //! W3fu5 H3 (security-1): regression tests for the public-socket
    //! ACL + lock-parent shape under the non-root daemon contract.
    //!
    //! Coverage of the production deployment topology
    //! (`User=nixlingd`, `SupplementaryGroups=nixling-launchers`,
    //! tmpfile `d /run/nixling 0750 nixlingd nixling-launchers -`,
    //! socket `mode 0660 group nixling-launchers`) is split across
    //! these focused unit tests because the real system identities
    //! (`nixlingd`, `nixling-launchers`) only exist on the deployed
    //! NixOS host. Here we simulate `expect_root_owned_parent=true`
    //! with the caller's own uid+gid so the chown succeeds under
    //! `cargo test`, and assert the produced shape (owner / group /
    //! mode) matches what the production deployment will produce.

    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::PathBuf;

    use nix::unistd::{self, Gid, Uid};

    use super::{bind_public_socket, validate_lock_parent, RuntimeIdentity};

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

    /// W3fu5 H4 (test-1, R6): pick a supplementary group different from
    /// the caller's primary gid so we can prove the
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
        // `nixling-launchers`. With the gate removed and `chown(path,
        // None, Some(public_socket_gid))`, the socket must always
        // pick up the requested group when
        // `expect_root_owned_parent` is true.
        //
        // W3fu5 H4 (test-1, R6): the assertion is only meaningful if
        // the socket's natural (umask-inherited) gid differs from
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
        // nixling-launchers -`. With expect_root_owned_parent=true,
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
mod broker_dispatch_tests {
    use std::fs::File;
    use std::io::{self, IoSlice, Read, Write};
    use std::net::TcpListener;
    use std::os::fd::{AsRawFd, RawFd};
    use std::os::unix::net::UnixListener;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::{fs, thread};

    use nix::sys::socket::{
        accept4, bind, listen, recv, sendmsg, socket, AddressFamily, Backlog, ControlMessage,
        MsgFlags, SockFlag, SockType, UnixAddr,
    };
    use nix::unistd::close;
    use nixling_core::processes::ProcessRole;
    use nixling_ipc::broker_wire::{
        BrokerRequestEnvelope, BrokerResponse, RunnerRole, SpawnRunnerResponse,
    };
    use nixling_ipc::public_wire::{
        ActivationRequest, GcRequest, HostDestroyRequest, HostInstallRequest, HostPrepareRequest,
        KeysRotateRequest, MigrateRequest, MutationFlags, RotateKnownHostRequest, TrustRequest,
        VmLifecycleRequest,
    };
    use serde::Serialize;
    use serde_json::json;

    use super::supervisor::pidfd_table::{PidfdEntry, PidfdTable, WaitTermination};
    use super::supervisor::state::{
        parse_proc_stat_starttime, FilesystemSnapshotStore, PidfdOpener, ProcReader,
        RunnerSnapshotRecord, SnapshotStore,
    };
    use super::{
        adopt_orphaned_runners_on_startup_with, dispatch_broker_boot, dispatch_broker_gc,
        dispatch_broker_host_destroy, dispatch_broker_host_prepare, dispatch_broker_keys_rotate,
        dispatch_broker_rollback, dispatch_broker_rotate_known_host,
        dispatch_broker_run_host_install, dispatch_broker_run_migrate, dispatch_broker_switch,
        dispatch_broker_test, dispatch_broker_trust, dispatch_broker_vm_restart,
        dispatch_broker_vm_start, dispatch_broker_vm_stop, dispatch_broker_vm_stop_with_timeout,
        dispatch_request, redact_broker_dispatch_failure_for_launcher,
        redact_broker_error_for_launcher, vm_start_node_mode, ArtifactPaths, DaemonConfig,
        PeerIdentity, PeerRole, ServerState, VmStartNodeMode, VM_RUNNER_ROLE_ID,
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
        ServerState {
            config: DaemonConfig {
                broker_socket_path: path,
                artifacts: write_minimal_vm_start_bundle_artifacts(&daemon_state_dir),
                ..DaemonConfig::default()
            },
            daemon_uid: 0,
            daemon_state_dir: daemon_state_dir.clone(),
            pidfd_table: Arc::new(PidfdTable::new(daemon_state_dir.join("pidfd-table.json"))),
        }
    }

    fn test_state_with_broker_socket_and_host(path: PathBuf, host_path: PathBuf) -> ServerState {
        let daemon_state_dir = test_daemon_state_dir("broker-host");
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
            daemon_state_dir: daemon_state_dir.clone(),
            pidfd_table: Arc::new(PidfdTable::new(daemon_state_dir.join("pidfd-table.json"))),
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
                "_manifest": { "manifestVersion": 3 },
                "_observability": {
                    "chExporter": { "listenPort": 9100 },
                    "enabled": false,
                    "grafanaUrl": "http://127.0.0.1:3000",
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
                "_manifest": { "manifestVersion": 3 },
                "_observability": {
                    "chExporter": { "listenPort": 9100 },
                    "enabled": false,
                    "grafanaUrl": "http://127.0.0.1:3000",
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
        assert!(response
            .get("summary")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|summary| summary.starts_with(expected_summary)));
        assert_eq!(
            response
                .get("remediation")
                .and_then(serde_json::Value::as_str),
            Some(expected_remediation)
        );
        assert!(response
            .get("remediation")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| !message.contains("broker.sock")));
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
            let (summary, remediation) = redact_broker_error_for_launcher("Op", Some("W15"), kind);
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
        assert!(response
            .get("remediation")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| !message.contains("broker.sock")));
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
        }
    }

    fn admin_peer() -> PeerIdentity {
        PeerIdentity {
            role: PeerRole::Admin,
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
            (ProcessRole::VsockRelay, RunnerRole::VsockRelay),
            (ProcessRole::Usbip, RunnerRole::Usbip),
        ];
        for (role, expected_runner_role) in cases {
            match vm_start_node_mode(&role) {
                VmStartNodeMode::LongLived(actual) => assert_eq!(actual, expected_runner_role),
                other => panic!("expected LongLived for {role:?}, got {other:?}"),
            }
        }
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
            },
        )
        .expect("vm start response");
        let expected_remediation =
            "Supervisor DAG aborted before every readiness deadline passed. Admin: inspect `journalctl -u nixlingd` for the per-node supervisor audit.";

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
        let state = ServerState {
            config: DaemonConfig {
                broker_socket_path: socket_path.clone(),
                artifacts,
                ..DaemonConfig::default()
            },
            daemon_uid: 0,
            daemon_state_dir: daemon_state_dir.clone(),
            pidfd_table: Arc::new(PidfdTable::new(daemon_state_dir.join("pidfd-table.json"))),
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
            },
        )
        .expect("vm start response");

        assert_eq!(
            response.get("outcome").and_then(serde_json::Value::as_str),
            Some("applied")
        );
        assert!(response
            .get("summary")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .contains("registered in pidfd_table"));
        assert!(state.pidfd_table.contains("vm-a", VM_RUNNER_ROLE_ID));

        let child = broker.join().expect("join broker thread");
        state
            .pidfd_table
            .signal("vm-a", VM_RUNNER_ROLE_ID, libc::SIGKILL)
            .expect("cleanup signal");
        assert_eq!(
            state
                .pidfd_table
                .wait_terminated("vm-a", VM_RUNNER_ROLE_ID, std::time::Duration::from_secs(5))
                .expect("cleanup wait"),
            WaitTermination::Terminated
        );
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
        let state = ServerState {
            config: DaemonConfig {
                broker_socket_path: socket_path.clone(),
                artifacts,
                ..DaemonConfig::default()
            },
            daemon_uid: 0,
            daemon_state_dir: daemon_state_dir.clone(),
            pidfd_table: Arc::new(PidfdTable::new(daemon_state_dir.join("pidfd-table.json"))),
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
            },
        )
        .expect("vm start response");

        assert_eq!(
            response.get("outcome").and_then(serde_json::Value::as_str),
            Some("applied")
        );
        assert!(response
            .get("summary")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .contains("registered in pidfd_table"));
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
            assert_eq!(
                state
                    .pidfd_table
                    .wait_terminated("vm-a", role, std::time::Duration::from_secs(5))
                    .expect("cleanup wait"),
                WaitTermination::Terminated
            );
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

        let state = ServerState {
            config: DaemonConfig::default(),
            daemon_uid: 0,
            daemon_state_dir: daemon_state_dir.clone(),
            pidfd_table: Arc::new(PidfdTable::new(daemon_state_dir.join("pidfd-table.json"))),
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
            },
        )
        .expect("vm stop response");

        assert_eq!(
            response.get("outcome").and_then(serde_json::Value::as_str),
            Some("applied")
        );
        assert!(response
            .get("summary")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .contains("pidfd_table"));
        assert!(!state.pidfd_table.contains("vm-a", VM_RUNNER_ROLE_ID));
        let store = FilesystemSnapshotStore::new(&state.daemon_state_dir);
        assert!(SnapshotStore::list(&store)
            .expect("list runner snapshots")
            .is_empty());
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
        assert!(summary.contains("drained 8 pidfd_table entries in reverse DAG order"));
        assert!(summary.contains(
            "ch-runner, gpu, audio, video, vsock-relay, swtpm, virtiofsd-nl-meta, virtiofsd-ro-store"
        ));
        assert!(state.pidfd_table.list_for_vm("vm-a").is_empty());
        let store = FilesystemSnapshotStore::new(&state.daemon_state_dir);
        assert!(SnapshotStore::list(&store)
            .expect("list runner snapshots")
            .is_empty());
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
            },
            std::time::Duration::from_millis(100),
            std::time::Duration::from_secs(5),
        )
        .expect("vm stop response");

        assert_eq!(
            response.get("outcome").and_then(serde_json::Value::as_str),
            Some("applied")
        );
        assert!(response
            .get("summary")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .contains("SIGTERM timeout"));
        assert!(!state.pidfd_table.contains("vm-a", VM_RUNNER_ROLE_ID));
        let status = child.wait();
        assert!(!status.success());
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
            },
        )
        .expect("vm restart response");
        let expected_remediation =
            "Supervisor DAG aborted before every readiness deadline passed. Admin: inspect `journalctl -u nixlingd` for the per-node supervisor audit.";

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
            accept4, bind, listen, recv, send, socket, AddressFamily, Backlog, MsgFlags, SockFlag,
            SockType, UnixAddr,
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
}
