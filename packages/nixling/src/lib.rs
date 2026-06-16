use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::OsString,
    fmt::Write as _,
    fs,
    io::{self, IsTerminal as _, Write as _},
    os::fd::{AsRawFd as _, OwnedFd},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use nix::sys::socket::{
    connect, recv, send, socket, AddressFamily, MsgFlags, SockFlag, SockType, UnixAddr,
};
use nix::unistd::Uid;
use nixling_core::{
    bundle::Bundle, bundle_resolver::HostRuntime, closures::ClosureMetadata,
    error::Error as CoreError, host::HostJson, host_check, processes::ProcessesJson,
};
use nixling_ipc::{
    broker_wire::{
        ExportBrokerAuditResponse, StoreVerifyResponse as IpcStoreVerifyResponse,
        StoreVerifyStatus as IpcStoreVerifyStatus,
    },
    public_wire::{
        AuditFormat as IpcAuditFormat, AuditRequest as IpcAuditRequest, KeyEntry as IpcKeyEntry,
        KeysShowRequest as IpcKeysShowRequest, KeysShowResponse as IpcKeysShowResponse,
        ReadGuestConfigRequest, UsbipProbeEntry as IpcUsbipProbeEntry,
        UsbipProbeStatus as IpcUsbipProbeStatus,
    },
    Hello as IpcHello, HelloOk as IpcHelloOk, HelloRejected as IpcHelloRejected, KnownFeatureFlag,
    SemverRange,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

mod doctor;
mod exec_client;
mod host_validate;

use exec_client::ExecOwnerTransport as _;

const DEFAULT_MANIFEST_PATH: &str = "/run/current-system/sw/share/nixling/vms.json";
const DEFAULT_BUNDLE_PATH: &str = "/etc/nixling/bundle.json";
const DEFAULT_PUBLIC_SOCKET: &str = "/run/nixling/public.sock";
const DEFAULT_BROKER_SOCKET: &str = "/run/nixling/priv.sock";
const DEFAULT_HOST_RUNTIME_PATH: &str = "/var/lib/nixling/runtime/host-runtime.json";
const DEFAULT_CLIENT_VERSION_RANGE: &str = ">=0.4.0, <0.5.0";
const RUNTIME_UNKNOWN: &str = "unknown";
const MAX_FRAME_BYTES: usize = 1024 * 1024;
/// Location of daemon-persisted state files (`pidfd-table.json`,
/// `kernel-module-report.json`, `autostart-report.json`) that
/// `nixling host doctor --read-only` inspects. Mirrors
/// `nixlingd::DEFAULT_DAEMON_STATE_DIR`.
const DEFAULT_DAEMON_STATE_DIR: &str = "/var/lib/nixling/daemon-state";
/// Canonical Prometheus scrape URL the doctor probes for reachability.
/// See `docs/reference/daemon-metrics.md`.
const DEFAULT_METRICS_URL: &str = "http://127.0.0.1:9101/metrics";
/// Exit code for api-ready timeout in strict mode.
pub const EXIT_API_TIMEOUT: i32 = 33;
/// Default in-guest path of the editable guest config working copy. Only the
/// legacy operator SSH transport honors a custom path; the guest-control
/// transport reads the VM's canonical guest config working copy by file id.
const DEFAULT_GUEST_CONFIG_PATH: &str = "/var/lib/nixling-guest/guest-config.nix";
/// Exit code surfaced for every guest-control config-read failure on the CLI.
const EXIT_GUEST_CONTROL_CONFIG: i32 = 70;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ListOutputV2(pub Vec<ListItemOutputV2>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListItemOutputV2 {
    pub name: String,
    pub env: Option<String>,
    pub graphics: bool,
    pub tpm: bool,
    pub usbip: bool,
    pub static_ip: Option<String>,
    pub status: String,
    pub is_net_vm: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runner_parity_ok: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmExecCreateOutputV1 {
    pub command: String,
    pub vm: String,
    pub exec_id: String,
    pub state: nixling_ipc::guest_wire::ExecState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmExecListOutputV1 {
    pub command: String,
    pub vm: String,
    pub execs: Vec<VmExecListEntryOutputV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmExecListEntryOutputV1 {
    pub exec_id: String,
    pub state: nixling_ipc::guest_wire::ExecState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<u32>,
    pub started_at: String,
    pub start_offset: u64,
    pub end_offset: u64,
    pub stdout_start_offset: u64,
    pub stdout_end_offset: u64,
    pub stderr_start_offset: u64,
    pub stderr_end_offset: u64,
    pub dropped_bytes: u64,
    pub stdout_dropped_bytes: u64,
    pub stderr_dropped_bytes: u64,
    pub truncated: bool,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmExecStatusOutputV1 {
    pub command: String,
    pub vm: String,
    pub exec_id: String,
    pub state: nixling_ipc::guest_wire::ExecState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<u32>,
    pub start_offset: u64,
    pub end_offset: u64,
    pub dropped_bytes: u64,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmExecLogsOutputV1 {
    pub command: String,
    pub vm: String,
    pub exec_id: String,
    pub stdout_base64: String,
    pub stderr_base64: String,
    pub start_offset: u64,
    pub end_offset: u64,
    pub dropped_bytes: u64,
    pub truncated: bool,
    pub stdout_start_offset: u64,
    pub stdout_end_offset: u64,
    pub stdout_next_offset: u64,
    pub stdout_eof: bool,
    pub stdout_dropped_bytes: u64,
    pub stdout_truncated: bool,
    pub stderr_start_offset: u64,
    pub stderr_end_offset: u64,
    pub stderr_next_offset: u64,
    pub stderr_eof: bool,
    pub stderr_dropped_bytes: u64,
    pub stderr_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmExecKillOutputV1 {
    pub command: String,
    pub vm: String,
    pub exec_id: String,
    pub result: nixling_ipc::public_wire::ExecDetachedKillOutcome,
    pub state: nixling_ipc::guest_wire::ExecState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum StatusOutputV2 {
    Vm(Box<StatusVmOutputV2>),
    Inventory(Box<StatusInventoryOutputV2>),
    CheckBridges(Box<StatusBridgeCheckOutputV2>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatusInventoryOutputV2 {
    pub runtime: String,
    pub vms: Vec<StatusVmOutputV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ApiReadyStatusV1 {
    Simple(ApiReadySimple),
    WithError(ApiReadyErrorV1),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApiReadyErrorV1 {
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ApiReadySimple {
    Yes,
    Pending,
    Timeout,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatusVmOutputV2 {
    pub name: String,
    pub env: Option<String>,
    pub services: StatusServicesOutputV2,
    pub current: Option<String>,
    pub booted: Option<String>,
    pub pending_restart: bool,
    pub runtime: String,
    pub declared_roles: Vec<String>,
    pub readiness: Vec<String>,
    /// api-ready state from the last vm start in split mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_ready: Option<ApiReadyStatusV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runner_parity: Option<RunnerParityOutputV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_pool_integrity: Option<LivePoolIntegrityOutputV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LivePoolIntegrityOutputV1 {
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unknown_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_ref: Option<String>,
    pub repair_attempted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatusServicesOutputV2 {
    pub nixling: String,
    pub microvm: String,
    pub virtiofsd: String,
    pub gpu: Option<String>,
    pub video: Option<String>,
    pub snd: Option<String>,
    pub swtpm: Option<String>,
}

/// Per-VM service-state map (V3) — broker-spawn-aware status output.
///
/// All fields are optional so emitters can omit a role when the VM
/// doesn't enable it. The wire shape uses camelCase
/// + `deny_unknown_fields` to keep schema-drift gates honest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatusServicesOutputV3 {
    /// Cloud Hypervisor runner state (broker-spawned).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hypervisor: Option<String>,
    /// Per-share virtiofsd state, keyed by share `tag`.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty", default)]
    pub virtiofsd_per_share: std::collections::BTreeMap<String, String>,
    /// crosvm GPU sidecar state (broker-spawned).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu: Option<String>,
    /// vhost-device-sound audio sidecar state (broker-spawned).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio: Option<String>,
    /// swtpm sidecar state (broker-spawned).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swtpm: Option<String>,
    /// Per-VM OtelGuestRelay state (broker-spawned).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub otel_relay: Option<String>,
    /// Host-scoped OtelHostBridge state (broker-spawned).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub otel_host_bridge: Option<String>,
    /// Per-env USBIP backend state, keyed by env name.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty", default)]
    pub usbip_backend_per_env: std::collections::BTreeMap<String, String>,
    /// Per-env USBIP proxy state, keyed by env name.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty", default)]
    pub usbip_proxy_per_env: std::collections::BTreeMap<String, String>,
}

impl StatusServicesOutputV3 {
    /// Conversion shim: takes a V2 record and projects it into V3
    /// by applying the documented rename map. Used so callers
    /// consuming the legacy V2 shape can be migrated incrementally
    /// without breaking the bundle-resolver / status-output contract.
    pub fn from_v2(v2: &StatusServicesOutputV2) -> Self {
        let mut virtiofsd_per_share = std::collections::BTreeMap::new();
        // V2 had a single `virtiofsd` slot; we expose it under the
        // synthetic share tag `default` so the V3 consumer can read
        // it without losing data. v1.1.2+ wire bumps populate the
        // map per-share via the broker's per-share spawn records.
        virtiofsd_per_share.insert("default".to_owned(), v2.virtiofsd.clone());
        Self {
            hypervisor: Some(v2.microvm.clone()),
            virtiofsd_per_share,
            gpu: v2.gpu.clone(),
            // V3 has no dedicated video field yet; keep V2 authoritative
            // until a negotiated schema revision adds one.
            audio: v2.snd.clone(),
            swtpm: v2.swtpm.clone(),
            otel_relay: None,
            otel_host_bridge: None,
            usbip_backend_per_env: std::collections::BTreeMap::new(),
            usbip_proxy_per_env: std::collections::BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunnerParityOutputV2 {
    pub declared_runner: String,
    pub runner_parity_path: String,
    pub runner_parity_ok: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatusBridgeCheckOutputV2 {
    pub mode: String,
    pub status: String,
    pub message: String,
    pub runtime: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditOutputV2 {
    pub kvm_dev_mode: String,
    pub wayland_user_in_kvm: bool,
    pub store_delivery: BTreeMap<String, String>,
    pub virtiofsd: BTreeMap<String, AuditVirtiofsdOutputV2>,
    pub ssh: BTreeMap<String, AuditSshOutputV2>,
    pub bridge_isolation: BTreeMap<String, AuditBridgeIsolationOutputV2>,
    #[serde(rename = "autoUpgrade_commits_lock")]
    pub auto_upgrade_commits_lock: bool,
    pub ch_version: String,
    pub crosvm_rev: String,
    pub seccomp_rev: String,
    pub ch_crosvm_pair_ok: bool,
    pub fail2ban_active: bool,
    pub sidecars_per_vm: BTreeMap<String, AuditSidecarsOutputV2>,
    pub usbipd_per_env_isolation: BTreeMap<String, AuditUsbipEnvOutputV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditVirtiofsdOutputV2 {
    pub user: String,
    pub caps_dropped: Vec<String>,
    pub readonly_flag: bool,
    pub marker_ok: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditSshOutputV2 {
    #[serde(rename = "PasswordAuthentication")]
    pub password_authentication: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditBridgeIsolationOutputV2 {
    pub bridge: String,
    pub tap: String,
    pub state: String,
    pub isolated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditSidecarsOutputV2 {
    pub gpu_active: bool,
    pub snd_active: bool,
    pub gpu_user: String,
    pub snd_user: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditUsbipEnvOutputV2 {
    pub socket_active: bool,
    pub backend_active: bool,
    pub lock_present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostCheckOutputV2 {
    pub mode: String,
    pub strict: bool,
    pub summary: HostCheckSummaryV2,
    pub exit_code: u8,
    pub findings: Vec<HostCheckFindingV2>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostCheckSummaryV2 {
    pub pass: u32,
    pub warn: u32,
    pub fail: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostCheckFindingV2 {
    pub id: String,
    pub severity: HostCheckSeverityV2,
    pub message: String,
    pub remediation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub details: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HostCheckSeverityV2 {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthStatusOutputV2 {
    pub role: AuthRoleV2,
    pub effective_uid: u32,
    pub sockets: Vec<AuthSocketStatusV2>,
    pub allowed_subcommands: Vec<String>,
    pub denied_subcommands: Vec<AuthDeniedSubcommandV2>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AuthRoleV2 {
    None,
    Launcher,
    Admin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthSocketStatusV2 {
    pub name: String,
    pub path: String,
    pub reachable: bool,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthDeniedSubcommandV2 {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Parser)]
#[command(
    version,
    about = "nixling — opinionated NixOS desktop microVM CLI.",
    long_about = "nixling — daemon-native CLI for nixling microVMs.\n\nAll mutating verbs dispatch through nixlingd and nixling-priv-broker. \
        Read-only verbs (list, status, audit, host check) query the daemon or \
        the static manifest. See `nixling <COMMAND> --help` for per-verb usage."
)]
struct NativeCli {
    #[command(subcommand)]
    command: NativeCommand,
}

#[derive(Debug, Subcommand)]
enum NativeCommand {
    /// List declared VMs from the static manifest.
    List(ListArgs),
    /// Show per-VM runtime status plus bridge health.
    Status(StatusArgs),
    /// USBIP attach / detach / probe.
    Usb(UsbArgs),
    /// Foreground serial console bridge for headless VMs (not yet implemented).
    Console(ConsoleArgs),
    /// Per-VM audio grant bridge (not yet implemented).
    Audio(AudioArgs),
    /// Tail the broker audit log.
    Audit(AuditArgs),
    /// Host-side preflight, install, doctor, and reconcile verbs.
    Host(HostArgs),
    /// Authorisation introspection.
    Auth(AuthArgs),
    /// Per-VM lifecycle verbs (start / stop / restart / list / status) plus the
    /// admin-only guest-control sub-verb `exec`, which runs commands or an
    /// interactive session inside a VM over the authenticated
    /// guest-control transport (no SSH).
    Vm(VmArgs),
    /// Alias for `vm start <vm>`.
    Up(VmStartArgs),
    /// Alias for `vm stop <vm>`.
    Down(VmStopArgs),
    /// Alias for `vm restart <vm>`.
    Restart(VmRestartArgs),
    /// Non-destructive eval + build of the per-VM toplevel.
    Build(BuildArgs),
    /// List current / booted / numbered generations for a VM.
    Generations(GenerationsArgs),
    /// Atomically activate a new per-VM closure.
    Switch(SwitchArgs),
    /// Stage a per-VM closure for the next boot only.
    Boot(BootArgs),
    /// Activate a per-VM closure with rollback on reboot.
    Test(TestArgs),
    /// Roll a VM back to its previous generation.
    Rollback(RollbackArgs),
    /// Garbage-collect the per-VM /nix/store hardlink farm.
    Gc(GcArgs),
    /// Store-view maintenance and verification.
    Store(StoreArgs),
    /// Managed-key lifecycle (list / show / rotate).
    Keys(KeysArgs),
    /// Trust a VM's host key on first use (TOFU).
    Trust(KeysTrustArgs),
    /// Rotate the consumer's recorded known-host entry for a VM.
    #[command(name = "rotate-known-host")]
    RotateKnownHost(KeysRotateKnownHostArgs),
    /// Analyse the host config and emit a migration plan.
    Migrate(MigrateArgs),
    /// Sync / review / approve a VM's guest-editable config
    /// (`guestConfigFile`): pull the operator's in-VM edits to a
    /// host-side staging file, diff them, and approve them.
    Config(ConfigArgs),
}

#[derive(Debug, Args)]
struct ListArgs {
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct StatusArgs {
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
    #[arg(long)]
    check_bridges: bool,
    #[arg(long = "vm")]
    vm_flag: Option<String>,
    vm: Option<String>,
}

#[derive(Debug, Args)]
struct UsbArgs {
    #[command(subcommand)]
    command: UsbCommand,
}

#[derive(Debug, Subcommand)]
enum UsbCommand {
    /// Bind a host USB busid to a VM via the native daemon path.
    Attach(UsbAttachArgs),
    /// Unbind a host USB busid from a VM via the native daemon path.
    Detach(UsbDetachArgs),
    /// List daemon-declared USBIP busid claims and lock owners.
    Probe(UsbProbeArgs),
}

#[derive(Debug, Args)]
struct UsbAttachArgs {
    vm: String,
    busid: String,
    #[arg(long, conflicts_with = "apply")]
    dry_run: bool,
    #[arg(long, conflicts_with = "dry_run")]
    apply: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct UsbDetachArgs {
    vm: String,
    busid: String,
    #[arg(long, conflicts_with = "apply")]
    dry_run: bool,
    #[arg(long, conflicts_with = "dry_run")]
    apply: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct UsbProbeArgs {
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct AuditArgs {
    #[arg(long)]
    strict: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct HostArgs {
    #[command(subcommand)]
    command: HostCommand,
}

#[derive(Debug, Subcommand)]
enum HostCommand {
    /// Read-only preflight: inventories host posture without mutation.
    Check(HostCheckArgs),
    /// Reconcile host-side state (bridges, nftables, sysctls). --apply mutates.
    Prepare(HostPrepareArgs),
    /// Tear down host-side state owned by nixling. --apply mutates.
    Destroy(HostDestroyArgs),
    /// Read-only deep diagnostics for the daemon + broker state.
    Doctor(HostDoctorArgs),
    /// Install nixlingd + broker units onto the host. --apply mutates.
    Install(HostInstallArgs),
    /// Recover host network state after the daemon engaged operator-only mode.
    Reconcile(HostReconcileArgs),
    /// Run the host-side validator suite and write evidence records.
    Validate(HostValidateArgs),
}

#[derive(Debug, Args)]
struct HostValidateArgs {
    /// Plan: report which readiness validators WOULD be attested.
    /// No evidence is written.
    #[arg(long, conflicts_with = "apply")]
    dry_run: bool,
    /// Apply: write `/var/lib/nixling/validated/<wave>.json` for
    /// every wave whose declared validators are present on disk.
    #[arg(long, conflicts_with = "dry_run")]
    apply: bool,
    /// Restrict to a single wave. Other waves are reported as `skipped`.
    #[arg(long)]
    wave: Option<String>,
    /// Override the per-wave operator signature. When unset, the
    /// verb derives a deterministic sha256 signature from
    /// `hostname|wave|scripts_dir|timestamp`.
    #[arg(long, value_name = "SIGNATURE")]
    operator_signature: Option<String>,
    /// Override the evidence directory. Default: `/var/lib/nixling/validated`.
    #[arg(long, value_name = "PATH")]
    evidence_dir: Option<PathBuf>,
    /// Override the scripts directory. Default: best-effort
    /// discovery of the installed `tests/` share, then `./tests`.
    #[arg(long, value_name = "PATH")]
    scripts_dir: Option<PathBuf>,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct HostCheckArgs {
    #[arg(long)]
    read_only: bool,
    #[arg(long)]
    strict: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct HostPrepareArgs {
    /// Plan the reconcile without mutating host state.
    #[arg(long, conflicts_with = "apply")]
    dry_run: bool,
    /// Apply the reconcile (mutates host state).
    #[arg(long, conflicts_with = "dry_run")]
    apply: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct HostDestroyArgs {
    #[arg(long, conflicts_with = "apply")]
    dry_run: bool,
    #[arg(long, conflicts_with = "dry_run")]
    apply: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct HostDoctorArgs {
    /// Mandatory: doctor is read-only. Mutating forms are separate verbs.
    #[arg(long)]
    read_only: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct HostInstallArgs {
    /// Report the planned install steps without mutating.
    #[arg(long, conflicts_with_all = ["apply", "enable", "start", "no_start"])]
    dry_run: bool,
    /// Perform the install through the daemon → broker `RunHostInstall` path.
    #[arg(long, conflicts_with = "dry_run")]
    apply: bool,
    /// After `--apply`, enable nixlingd.service via systemctl.
    #[arg(long, conflicts_with = "dry_run", requires = "apply")]
    enable: bool,
    /// After `--apply --enable`, start nixlingd.service.
    #[arg(long, conflicts_with_all = ["dry_run", "no_start"], requires = "apply")]
    start: bool,
    /// Explicitly do NOT start nixlingd.service post-install.
    #[arg(long, conflicts_with_all = ["dry_run", "start"], requires = "apply")]
    no_start: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct HostReconcileArgs {
    /// Re-run the network slice of `host prepare` and clear the
    /// daemon's net-route preflight counter. Currently the only
    /// available scope.
    #[arg(long)]
    network: bool,
    /// Plan the reconcile without mutating host state.
    #[arg(long, conflicts_with = "apply")]
    dry_run: bool,
    /// Apply the reconcile (mutates host state).
    #[arg(long, conflicts_with = "dry_run")]
    apply: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct AuthArgs {
    #[command(subcommand)]
    command: AuthCommand,
}

#[derive(Debug, Args)]
struct VmArgs {
    #[command(subcommand)]
    command: VmCommand,
}

#[derive(Debug, Subcommand)]
enum VmCommand {
    /// Start the per-VM DAG (virtiofsd → CH → readiness probes).
    Start(VmStartArgs),
    /// Stop the per-VM DAG in reverse topo order.
    Stop(VmStopArgs),
    /// Stop then start; same envelope contract as start.
    Restart(VmRestartArgs),
    /// Daemon-side runtime view (different from `nixling list`, which
    /// is the static manifest view).
    List(VmListArgs),
    /// Daemon-side readiness state for a VM (api-ready phase).
    Status(VmStatusArgs),
    /// Run or manage commands inside a running VM. Use
    /// `nixling vm exec <vm> -- <cmd...>` for a non-interactive command,
    /// `nixling vm exec -it <vm> -- bash` for an interactive shell, `-d` for
    /// a detached command, and `nixling vm exec <vm> {list|logs|status|kill}`
    /// to manage detached execs.
    Exec(VmExecArgs),
}

/// `nixling vm exec [-d] [-it] [-i] [-t] <vm> [--env K=V]... [--cwd DIR] -- <cmd...>`
/// Run a command inside a VM. Use `--` before the command, `-it` for an
/// interactive guest PTY, and `-d` to create a detached exec. Detached execs
/// are managed with `nixling vm exec <vm> list`, `logs <id>`, `status <id>`,
/// and `kill <id>`.
#[derive(Debug, Args)]
struct VmExecArgs {
    /// Start the command detached and print its exec id. Incompatible with
    /// `-i`/`-t`; detached execs are managed with
    /// `nixling vm exec <vm> {list|logs|status|kill}`.
    #[arg(short = 'd', long = "detach")]
    detach: bool,
    /// Forward host stdin into the guest command (`-i`). Requires
    /// `-t`/`--tty`; use `-it` for an interactive shell.
    #[arg(short = 'i', long = "interactive")]
    interactive: bool,
    /// Allocate a PTY in the guest and put the host terminal in raw mode
    /// (`-t`). Implies stdin forwarding. Human-only (incompatible with
    /// `--json`).
    #[arg(short = 't', long = "tty")]
    tty: bool,
    /// Set an environment variable in the guest command (`KEY=VALUE`).
    /// Repeatable.
    #[arg(long = "env", value_name = "KEY=VALUE")]
    env: Vec<String>,
    /// Working directory for the guest command.
    #[arg(long = "cwd", value_name = "DIR")]
    cwd: Option<String>,
    /// VM name as declared in `nixling.vms.<name>`.
    vm: String,
    /// Emit a single terminal JSON envelope (exit code + source/reason +
    /// bounded captured output). Non-interactive only.
    #[arg(long, conflicts_with = "human", global = true)]
    json: bool,
    /// Force human output.
    #[arg(long, conflicts_with = "json", global = true)]
    human: bool,
    /// Optional detached exec management form: `list`,
    /// `logs <id> [--stdout-offset N|--stdout-offset=N]
    /// [--stderr-offset N|--stderr-offset=N] [--max-len N|--max-len=N]`,
    /// `status <id>`, or `kill <id>`. Command execs never use this position:
    /// pass a command after `--` instead.
    #[arg(value_name = "MANAGEMENT", num_args = 0.., allow_hyphen_values = true)]
    management: Vec<OsString>,
    /// The guest command and its arguments, after `--`.
    #[arg(last = true)]
    command: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum VmExecManagementCommand {
    List,
    Logs(VmExecLogsArgs),
    Status(VmExecIdArgs),
    Kill(VmExecIdArgs),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VmExecIdArgs {
    /// Detached exec id returned by `nixling vm exec -d`.
    exec_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VmExecLogsArgs {
    /// Detached exec id returned by `nixling vm exec -d`.
    exec_id: String,
    /// Resume stdout from this byte offset. The daemon clamps stale offsets.
    stdout_offset: Option<u64>,
    /// Resume stderr from this byte offset. The daemon clamps stale offsets.
    stderr_offset: Option<u64>,
    /// Maximum retained bytes to request per stream.
    max_len: Option<u64>,
}

#[derive(Debug, Args)]
struct VmStartArgs {
    /// VM name as declared in `nixling.vms.<name>`.
    vm: String,
    /// Plan the DAG without spawning any role.
    #[arg(long, conflicts_with = "apply")]
    dry_run: bool,
    /// Apply the DAG (drives the supervisor).
    #[arg(long, conflicts_with = "dry_run")]
    apply: bool,
    /// Exit 0 on process-alive success without waiting for api-ready.
    /// Default behavior is --strict (wait for both process-alive and api-ready).
    #[arg(long, requires = "apply")]
    no_wait_api: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct VmStopArgs {
    vm: String,
    #[arg(long, conflicts_with = "apply")]
    dry_run: bool,
    #[arg(long, conflicts_with = "dry_run")]
    apply: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct VmRestartArgs {
    vm: String,
    #[arg(long, conflicts_with = "apply")]
    dry_run: bool,
    #[arg(long, conflicts_with = "dry_run")]
    apply: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct VmListArgs {
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct VmStatusArgs {
    /// VM name.
    vm: String,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

// ---- store-lifecycle verbs ----

#[derive(Debug, Args)]
struct BuildArgs {
    vm: String,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct GenerationsArgs {
    vm: String,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct SwitchArgs {
    vm: String,
    #[arg(long, conflicts_with = "apply")]
    dry_run: bool,
    #[arg(long, conflicts_with = "dry_run")]
    apply: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct BootArgs {
    vm: String,
    #[arg(long, conflicts_with = "apply")]
    dry_run: bool,
    #[arg(long, conflicts_with = "dry_run")]
    apply: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct TestArgs {
    vm: String,
    #[arg(long, conflicts_with = "apply")]
    dry_run: bool,
    #[arg(long, conflicts_with = "dry_run")]
    apply: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct RollbackArgs {
    vm: String,
    #[arg(long, conflicts_with = "apply")]
    dry_run: bool,
    #[arg(long, conflicts_with = "dry_run")]
    apply: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct GcArgs {
    #[arg(long, conflicts_with = "apply")]
    dry_run: bool,
    #[arg(long, conflicts_with = "dry_run")]
    apply: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct StoreArgs {
    #[command(subcommand)]
    command: StoreCommand,
}

#[derive(Debug, Subcommand)]
enum StoreCommand {
    /// Verify a VM's hardlink-backed live store-view.
    Verify(StoreVerifyArgs),
}

#[derive(Debug, Args)]
struct StoreVerifyArgs {
    vm: String,
    #[arg(long)]
    repair: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

// ---- keys + trust verbs ----

#[derive(Debug, Args)]
struct KeysArgs {
    #[command(subcommand)]
    command: KeysCommand,
}

#[derive(Debug, Subcommand)]
enum KeysCommand {
    /// List managed keys (per-VM SSH keypair fingerprints).
    List(KeysListArgs),
    /// Show details for a specific VM's managed key.
    Show(KeysShowArgs),
    /// Rotate the framework-managed per-VM SSH keypair. --apply mutates.
    Rotate(KeysRotateArgs),
}

#[derive(Debug, Args)]
struct KeysListArgs {
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct KeysShowArgs {
    vm: String,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct KeysRotateArgs {
    vm: String,
    #[arg(long, conflicts_with = "apply")]
    dry_run: bool,
    #[arg(long, conflicts_with = "dry_run")]
    apply: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct KeysRotateKnownHostArgs {
    vm: String,
    #[arg(long, conflicts_with = "apply")]
    dry_run: bool,
    #[arg(long, conflicts_with = "dry_run")]
    apply: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct KeysTrustArgs {
    vm: String,
    #[arg(long, conflicts_with = "apply")]
    dry_run: bool,
    #[arg(long, conflicts_with = "dry_run")]
    apply: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

// ---- migrate verb ----

#[derive(Debug, Args)]
struct MigrateArgs {
    #[arg(long, conflicts_with = "apply")]
    dry_run: bool,
    #[arg(long, conflicts_with = "dry_run")]
    apply: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct ConsoleArgs {
    vm: String,
}

#[derive(Debug, Args)]
struct AudioArgs {
    #[command(subcommand)]
    command: Option<AudioCommand>,
}

#[derive(Debug, Subcommand)]
enum AudioCommand {
    /// Show current grant state. With no VM, lists every audio-enabled VM.
    Status(AudioStatusArgs),
    /// Grant or revoke microphone access.
    Mic(AudioToggleArgs),
    /// Grant or revoke speaker access.
    Speaker(AudioToggleArgs),
    /// Revoke both mic and speaker access.
    Off(AudioOffArgs),
}

#[derive(Debug, Args)]
struct AudioStatusArgs {
    vm: Option<String>,
}

#[derive(Debug, Args)]
struct AudioToggleArgs {
    #[arg(value_enum)]
    state: AudioGrantState,
    vm: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum AudioGrantState {
    On,
    Off,
}

#[derive(Debug, Args)]
struct AudioOffArgs {
    vm: String,
}

#[derive(Debug, Subcommand)]
enum AuthCommand {
    Status(AuthStatusArgs),
}

#[derive(Debug, Args)]
struct AuthStatusArgs {
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
    #[arg(long, hide = true)]
    test_uid: Option<u32>,
}

#[derive(Debug)]
struct CliFailure {
    exit_code: i32,
    message: String,
    rendered_stderr: Option<String>,
}

impl CliFailure {
    fn new(exit_code: i32, message: impl Into<String>) -> Self {
        Self {
            exit_code,
            message: message.into(),
            rendered_stderr: None,
        }
    }

    fn host_check_probe_error(error: host_check::ProbeError) -> Self {
        let operator_error = CoreError::internal_io(error.opaque_reason);
        Self {
            exit_code: 1,
            message: operator_error.message(),
            rendered_stderr: render_operator_error(&operator_error, Some("host check")),
        }
    }
}

#[derive(Debug, Clone)]
struct Context {
    manifest_path: PathBuf,
    bundle_path: PathBuf,
    public_socket: PathBuf,
    broker_socket: PathBuf,
    state_root: Option<PathBuf>,
    host_runtime_path: PathBuf,
    system_state_fixture: Option<SystemStateFixture>,
    auth_status_fixture: Option<AuthStatusFixture>,
    /// Daemon-persisted state dir (pidfd-table.json,
    /// kernel-module-report.json, autostart-report.json).
    /// Override via `NIXLING_DAEMON_STATE_DIR`.
    daemon_state_dir: PathBuf,
    /// Prometheus scrape URL the doctor probes for reachability.
    /// Override via `NIXLING_METRICS_URL`.
    metrics_url: String,
}

impl Context {
    fn from_env() -> Result<Self, CliFailure> {
        Ok(Self {
            manifest_path: env_path("NIXLING_MANIFEST_PATH", DEFAULT_MANIFEST_PATH),
            bundle_path: env_path("NIXLING_BUNDLE_PATH", DEFAULT_BUNDLE_PATH),
            public_socket: env_path("NIXLING_PUBLIC_SOCKET", DEFAULT_PUBLIC_SOCKET),
            broker_socket: env_path("NIXLING_BROKER_SOCKET", DEFAULT_BROKER_SOCKET),
            state_root: env::var_os("NIXLING_STATE_ROOT").map(PathBuf::from),
            host_runtime_path: env_path("NIXLING_HOST_RUNTIME_PATH", DEFAULT_HOST_RUNTIME_PATH),
            system_state_fixture: maybe_load_json_env("NIXLING_TEST_SYSTEM_STATE_JSON")?,
            auth_status_fixture: maybe_load_json_env("NIXLING_AUTH_STATUS_FIXTURE")?,
            daemon_state_dir: env_path("NIXLING_DAEMON_STATE_DIR", DEFAULT_DAEMON_STATE_DIR),
            metrics_url: env::var("NIXLING_METRICS_URL")
                .unwrap_or_else(|_| DEFAULT_METRICS_URL.to_owned()),
        })
    }

    fn load_manifest(&self) -> Result<ManifestDocument, CliFailure> {
        read_json_file(&self.manifest_path).map_err(|err| {
            CliFailure::new(
                1,
                format!("failed to read {}: {err}", self.manifest_path.display()),
            )
        })
    }

    fn load_bundle_context(&self) -> Result<Option<BundleContext>, CliFailure> {
        match self.bundle_path.try_exists() {
            Ok(true) => {}
            Ok(false) => return Ok(None),
            Err(err) => {
                return Err(CliFailure::new(
                    1,
                    format!("failed to inspect {}: {err}", self.bundle_path.display()),
                ));
            }
        }
        let bundle: Bundle = read_json_file(&self.bundle_path).map_err(|err| {
            CliFailure::new(
                1,
                format!("failed to read {}: {err}", self.bundle_path.display()),
            )
        })?;
        let base_dir = self
            .bundle_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("/"));
        let host = read_bundle_json::<HostJson>(&base_dir, &bundle.host_path)?;
        let processes = read_bundle_json::<ProcessesJson>(&base_dir, &bundle.processes_path)?;
        let mut closures = BTreeMap::new();
        for closure_ref in &bundle.closures {
            if let Some(closure) =
                read_bundle_json::<ClosureMetadata>(&base_dir, &closure_ref.path)?
            {
                closures.insert(closure_ref.vm.clone(), closure);
            }
        }
        let host_runtime = if self.host_runtime_path.exists() {
            read_json_file::<HostRuntime>(&self.host_runtime_path).ok()
        } else {
            None
        };
        Ok(Some(BundleContext {
            host,
            processes,
            closures,
            host_runtime,
        }))
    }
}

#[derive(Debug)]
struct BundleContext {
    host: Option<HostJson>,
    processes: Option<ProcessesJson>,
    closures: BTreeMap<String, ClosureMetadata>,
    host_runtime: Option<HostRuntime>,
}

#[derive(Debug, Deserialize)]
struct ManifestDocument {
    #[serde(rename = "_manifest", default)]
    _manifest: Option<Value>,
    #[serde(rename = "_observability", default)]
    _observability: Option<Value>,
    #[serde(flatten)]
    entries: BTreeMap<String, ManifestVm>,
}

impl ManifestDocument {
    fn vms(&self) -> Vec<&ManifestVm> {
        self.entries
            .iter()
            .filter(|(name, _)| !name.starts_with('_'))
            .map(|(_, vm)| vm)
            .collect()
    }

    fn get_vm(&self, name: &str) -> Option<&ManifestVm> {
        self.entries.get(name).filter(|_| !name.starts_with('_'))
    }

    fn bridge_names(&self) -> BTreeSet<String> {
        self.vms()
            .iter()
            .map(|vm| vm.bridge.clone())
            .collect::<BTreeSet<_>>()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestVm {
    name: String,
    env: Option<String>,
    graphics: bool,
    tpm: bool,
    audio: bool,
    usbip_yubikey: bool,
    static_ip: Option<String>,
    usbipd_host_ip: Option<String>,
    is_net_vm: bool,
    state_dir: String,
    bridge: String,
    ssh_user: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
struct SystemStateFixture {
    units: BTreeMap<String, String>,
    bridges: BTreeMap<String, BridgeHealthFixture>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BridgeHealthFixture {
    state: String,
    admin: String,
    expected_carrier: String,
    result: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
struct AuthStatusFixture {
    public_reachable: Option<bool>,
    public_version: Option<String>,
    broker_reachable: Option<bool>,
    broker_version: Option<String>,
}

#[derive(Debug, Clone)]
struct BridgeHealthRow {
    name: String,
    state: String,
    admin: String,
    expected_carrier: String,
    result: String,
}

#[derive(Debug, Clone)]
struct SocketProbe {
    reachable: bool,
    version: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HelloOkFrame {
    #[serde(rename = "type")]
    _type_name: String,
    #[serde(flatten)]
    payload: IpcHelloOk,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HelloRejectedFrame {
    #[serde(rename = "type")]
    _type_name: String,
    #[serde(flatten)]
    _payload: IpcHelloRejected,
    error: DaemonErrorEnvelope,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ErrorFrame {
    #[serde(rename = "type")]
    _type_name: String,
    error: DaemonErrorEnvelope,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuditResponseFrame {
    #[serde(rename = "type")]
    _type_name: String,
    #[serde(flatten)]
    payload: ExportBrokerAuditResponse,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DaemonErrorEnvelope {
    kind: String,
    #[serde(alias = "exitCode", alias = "code")]
    exit_code: u8,
    message: String,
    remediation: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KeysListResponseFrame {
    #[serde(rename = "type")]
    _type_name: String,
    entries: Vec<IpcKeyEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KeysShowResponseFrame {
    #[serde(rename = "type")]
    _type_name: String,
    #[serde(flatten)]
    payload: IpcKeysShowResponse,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsbipProbeResponseFrame {
    #[serde(rename = "type")]
    _type_name: String,
    entries: Vec<IpcUsbipProbeEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoreVerifyResponseFrame {
    #[serde(rename = "type")]
    _type_name: String,
    #[serde(flatten)]
    payload: IpcStoreVerifyResponse,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadGuestConfigResponseFrame {
    #[serde(rename = "type")]
    _type_name: String,
    content_base64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StoreVerifyOutputV2 {
    pub vm: String,
    pub status: IpcStoreVerifyStatus,
    pub checked: u32,
    pub drifted: u32,
    pub repaired: u32,
    pub unknown_reason: Option<String>,
    pub audit_ref: Option<String>,
    pub remediation: Option<String>,
}

#[derive(Debug, Clone)]
enum AuditSocketOutcome {
    Unreachable,
    Lines(Vec<String>),
}

#[derive(Debug, Clone)]
enum KeysSocketOutcome {
    Unavailable,
    List(Vec<IpcKeyEntry>),
    Show(IpcKeysShowResponse),
}

#[derive(Debug, Clone)]
enum UsbProbeSocketOutcome {
    Unavailable,
    Entries(Vec<IpcUsbipProbeEntry>),
}

#[derive(Debug, Clone)]
enum StoreVerifySocketOutcome {
    Unavailable,
    Response(IpcStoreVerifyResponse),
}

#[derive(Debug, Clone)]
enum PublicSocketOutcome {
    Unavailable,
    Unsupported,
    Reply(Vec<u8>),
}

fn encode_type_tagged_message<T>(
    type_name: &str,
    message: &T,
    context: &str,
) -> Result<Vec<u8>, CliFailure>
where
    T: Serialize,
{
    let mut value = serde_json::to_value(message)
        .map_err(|err| CliFailure::new(1, format!("failed to encode {context}: {err}")))?;
    value
        .as_object_mut()
        .ok_or_else(|| {
            CliFailure::new(
                1,
                format!("failed to encode {context}: JSON object required"),
            )
        })?
        .insert("type".to_owned(), Value::String(type_name.to_owned()));
    serde_json::to_vec(&value)
        .map_err(|err| CliFailure::new(1, format!("failed to encode {context}: {err}")))
}

fn daemon_supported_features() -> Vec<nixling_ipc::FeatureFlag> {
    vec![
        KnownFeatureFlag::TypedErrors.wire_value(),
        KnownFeatureFlag::ExportBrokerAudit.wire_value(),
    ]
}

fn daemon_hello_frame(type_name: &str) -> Result<Vec<u8>, CliFailure> {
    let hello = IpcHello {
        client_version: SemverRange::new(DEFAULT_CLIENT_VERSION_RANGE).map_err(|err| {
            CliFailure::new(1, format!("failed to build hello version range: {err}"))
        })?,
        supported_features: daemon_supported_features(),
    };
    encode_type_tagged_message(type_name, &hello, "hello request")
}

fn daemon_audit_frame(type_name: &str, json_mode: bool) -> Result<Vec<u8>, CliFailure> {
    let request = IpcAuditRequest {
        filter: None,
        format: if json_mode {
            IpcAuditFormat::Json
        } else {
            IpcAuditFormat::Human
        },
        since: None,
    };
    encode_type_tagged_message(type_name, &request, "audit request")
}

fn is_daemon_unreachable(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

fn cli_failure_from_daemon_error(error: DaemonErrorEnvelope) -> CliFailure {
    let message = if error.remediation.is_empty() {
        format!("{}: {}", error.kind, error.message)
    } else {
        format!("{}: {} ({})", error.kind, error.message, error.remediation)
    };
    CliFailure::new(i32::from(error.exit_code), message)
}

fn decode_daemon_frame(response: &[u8], context: &str) -> Result<Value, CliFailure> {
    serde_json::from_slice(response)
        .map_err(|err| CliFailure::new(1, format!("failed to decode {context}: {err}")))
}

fn parse_hello_reply(response: &[u8]) -> Result<IpcHelloOk, CliFailure> {
    let value = decode_daemon_frame(response, "hello reply")?;
    let Some(type_name) = value.get("type").and_then(Value::as_str) else {
        return Err(CliFailure::new(
            1,
            "daemon hello reply was missing a type discriminator",
        ));
    };
    match type_name {
        "helloOk" => serde_json::from_value::<HelloOkFrame>(value)
            .map(|frame| frame.payload)
            .map_err(|err| CliFailure::new(1, format!("failed to decode helloOk reply: {err}"))),
        "helloRejected" => {
            let frame: HelloRejectedFrame = serde_json::from_value(value).map_err(|err| {
                CliFailure::new(1, format!("failed to decode helloRejected reply: {err}"))
            })?;
            Err(cli_failure_from_daemon_error(frame.error))
        }
        "error" => {
            let frame: ErrorFrame = serde_json::from_value(value).map_err(|err| {
                CliFailure::new(1, format!("failed to decode error reply: {err}"))
            })?;
            Err(cli_failure_from_daemon_error(frame.error))
        }
        other => Err(CliFailure::new(
            1,
            format!("unexpected hello reply type {other}"),
        )),
    }
}

fn parse_audit_reply(response: &[u8]) -> Result<Vec<String>, CliFailure> {
    let value = decode_daemon_frame(response, "audit reply")?;
    let Some(type_name) = value.get("type").and_then(Value::as_str) else {
        return Err(CliFailure::new(
            1,
            "daemon audit reply was missing a type discriminator",
        ));
    };
    match type_name {
        "auditResponse" => serde_json::from_value::<AuditResponseFrame>(value)
            .map(|frame| frame.payload.lines)
            .map_err(|err| CliFailure::new(1, format!("failed to decode auditResponse: {err}"))),
        "error" => {
            let frame: ErrorFrame = serde_json::from_value(value).map_err(|err| {
                CliFailure::new(1, format!("failed to decode error reply: {err}"))
            })?;
            Err(cli_failure_from_daemon_error(frame.error))
        }
        other => Err(CliFailure::new(
            1,
            format!("unexpected audit reply type {other}"),
        )),
    }
}

fn render_daemon_audit_lines(lines: &[String], json_mode: bool) -> Result<(), CliFailure> {
    if json_mode {
        if let [line] = lines {
            let trimmed = line.trim_start();
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                if line.ends_with('\n') {
                    print_stdout(line);
                } else {
                    print_stdout(&(line.to_owned() + "\n"));
                }
                return Ok(());
            }
        }
        print_json(&serde_json::json!({ "lines": lines }))?;
    } else if lines.is_empty() {
        print_stdout("");
    } else {
        print_stdout(&(lines.join("\n") + "\n"));
    }
    Ok(())
}

pub fn cli_command() -> clap::Command {
    let mut command = NativeCli::command();
    command.set_bin_name("nixling");
    command
}

pub fn run<I>(args: I) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    let raw_args: Vec<OsString> = args.into_iter().collect();
    if raw_args.is_empty() {
        return 1;
    }

    if raw_args.len() == 1 {
        print_stdout("nixling 0.0.0-bootstrap (bootstrap stub)\n");
        print_stdout("Rust-native CLI shim active; run `nixling --help` for subcommands.\n");
        return 0;
    }

    let cli = match NativeCli::try_parse_from(raw_args.clone()) {
        Ok(cli) => cli,
        Err(err) => {
            let is_host_usage = raw_args
                .get(1)
                .and_then(|arg| arg.to_str())
                .map(|arg| arg == "host")
                .unwrap_or(false)
                && raw_args
                    .get(2)
                    .and_then(|arg| arg.to_str())
                    .map(|arg| arg == "check")
                    .unwrap_or(false);
            let _ = err.print();
            return if is_host_usage { 3 } else { err.exit_code() };
        }
    };

    let context = match Context::from_env() {
        Ok(context) => context,
        Err(err) => return report_failure(err),
    };

    match dispatch(&context, &cli, &raw_args[1..]) {
        Ok(code) => code,
        Err(err) => report_failure(err),
    }
}

fn dispatch(
    context: &Context,
    cli: &NativeCli,
    original_args: &[OsString],
) -> Result<i32, CliFailure> {
    match &cli.command {
        NativeCommand::List(args) => cmd_list(context, args),
        NativeCommand::Status(args) => cmd_status(context, args),
        NativeCommand::Usb(args) => match &args.command {
            UsbCommand::Attach(args) => cmd_usb_attach(context, args),
            UsbCommand::Detach(args) => cmd_usb_detach(context, args),
            UsbCommand::Probe(args) => cmd_usb_probe(context, args),
        },
        NativeCommand::Console(args) => cmd_console(context, args, original_args),
        NativeCommand::Audio(args) => cmd_audio(context, args, original_args),
        NativeCommand::Audit(args) => cmd_audit(context, args, original_args),
        NativeCommand::Host(args) => match &args.command {
            HostCommand::Check(args) => cmd_host_check(context, args),
            HostCommand::Prepare(args) => cmd_host_prepare(context, args),
            HostCommand::Destroy(args) => cmd_host_destroy(context, args),
            HostCommand::Doctor(args) => cmd_host_doctor(context, args),
            HostCommand::Install(args) => cmd_host_install(context, args, original_args),
            HostCommand::Reconcile(args) => cmd_host_reconcile(context, args, original_args),
            HostCommand::Validate(args) => cmd_host_validate(context, args),
        },
        NativeCommand::Auth(args) => match &args.command {
            AuthCommand::Status(args) => cmd_auth_status(context, args),
        },
        NativeCommand::Vm(args) => match &args.command {
            VmCommand::Start(args) => cmd_vm_start(context, args),
            VmCommand::Stop(args) => cmd_vm_stop(context, args),
            VmCommand::Restart(args) => cmd_vm_restart(context, args),
            VmCommand::List(args) => cmd_vm_list(context, args),
            VmCommand::Status(args) => cmd_vm_status(context, args),
            VmCommand::Exec(args) => cmd_vm_exec(context, args),
        },
        NativeCommand::Up(args) => cmd_vm_start(context, args),
        NativeCommand::Down(args) => cmd_vm_stop(context, args),
        NativeCommand::Restart(args) => cmd_vm_restart(context, args),
        NativeCommand::Build(args) => cmd_build(context, args),
        NativeCommand::Generations(args) => cmd_generations(context, args),
        NativeCommand::Switch(args) => cmd_switch(context, args, original_args),
        NativeCommand::Boot(args) => cmd_boot(context, args, original_args),
        NativeCommand::Test(args) => cmd_test(context, args, original_args),
        NativeCommand::Rollback(args) => cmd_rollback(context, args, original_args),
        NativeCommand::Gc(args) => cmd_gc(context, args, original_args),
        NativeCommand::Store(args) => match &args.command {
            StoreCommand::Verify(args) => cmd_store_verify(context, args),
        },
        NativeCommand::Keys(args) => match &args.command {
            KeysCommand::List(args) => cmd_keys_list(context, args, original_args),
            KeysCommand::Show(args) => cmd_keys_show(context, args, original_args),
            KeysCommand::Rotate(args) => cmd_keys_rotate(context, args, original_args),
        },
        NativeCommand::Trust(args) => cmd_keys_trust(context, args, original_args),
        NativeCommand::RotateKnownHost(args) => {
            cmd_keys_rotate_known_host(context, args, original_args)
        }
        NativeCommand::Migrate(args) => cmd_migrate(context, args, original_args),
        NativeCommand::Config(args) => match &args.command {
            ConfigCommand::Sync(args) => cmd_config_sync(context, args),
            ConfigCommand::Diff(args) => cmd_config_diff(args),
            ConfigCommand::Approve(args) => cmd_config_approve(args),
            ConfigCommand::Reject(args) => cmd_config_reject(args),
            ConfigCommand::Status(args) => cmd_config_status(args),
        },
    }
}

// ============================================================
// `nixling config` — guest-editable config sync / review / approve
// ============================================================
//
// The per-VM `guestConfigFile` is the guest-editable OS layer. An
// operator edits it from inside the VM; these verbs move that edit
// host-side under review:
//
//   sync    pull the in-VM edited file into a host-side staging copy
//   diff    compare the staging copy against the live host-side file
//   approve write the staging copy onto an operator-chosen target file
//   reject  discard the staging copy
//   status  report whether a VM has a pending (un-approved) staging
//
// The CLI only ever writes to (a) its own user-local staging area and
// (b) an operator-specified `--to` target. It never auto-locates or
// writes the operator's config tree. The host treats the synced bytes
// as untrusted data; the real containment + eval gate is the per-VM
// `guestConfigFile` assertion that fires on `nixling switch`.

#[derive(Debug, Args)]
struct ConfigArgs {
    #[command(subcommand)]
    command: ConfigCommand,
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    /// Pull the VM's in-guest edited config into a host-side staging file.
    ///
    /// Reads the VM's canonical guest config working copy over the
    /// authenticated guest-control vsock (`readGuestConfig` -> guestd
    /// `ReadGuestFile`); there is no SSH. The pull fails closed when the VM's
    /// running generation does not declare the guest-control transport. The
    /// `--host`/`--user`/`--key`/`--known-hosts` overrides and a non-default
    /// `--guest-path` configure only a future operator SSH compatibility
    /// transport and are rejected on guest-control VMs.
    Sync(ConfigSyncArgs),
    /// Diff the staged guest config against a live host-side file.
    Diff(ConfigDiffArgs),
    /// Approve the staged guest config by writing it to a target file.
    Approve(ConfigApproveArgs),
    /// Discard the staged guest config.
    Reject(ConfigRejectArgs),
    /// Report whether a VM has a pending (un-approved) staged config.
    Status(ConfigStatusArgs),
}

#[derive(Debug, Args)]
struct ConfigSyncArgs {
    /// VM name (must match the static manifest).
    vm: String,
    /// Path of the editable guest config INSIDE the VM to pull. Honored only by
    /// the legacy operator SSH transport; on guest-control VMs the canonical
    /// guest config working copy is read by file id and this flag is rejected.
    #[arg(long, default_value = DEFAULT_GUEST_CONFIG_PATH)]
    guest_path: String,
    /// Override the SSH host (defaults to the manifest `static_ip`). SSH
    /// transport only; rejected on guest-control VMs.
    #[arg(long)]
    host: Option<String>,
    /// Override the SSH user (defaults to the manifest `ssh_user`). SSH
    /// transport only; rejected on guest-control VMs.
    #[arg(long)]
    user: Option<String>,
    /// Override the SSH private key path. SSH transport only; rejected on
    /// guest-control VMs.
    #[arg(long)]
    key: Option<PathBuf>,
    /// known_hosts file used to verify the VM's host key (defaults to
    /// the framework-managed `/var/lib/nixling/known_hosts.nixling`). SSH
    /// transport only; rejected on guest-control VMs.
    #[arg(long)]
    known_hosts: Option<PathBuf>,
    /// Print the planned action instead of running it.
    #[arg(long)]
    dry_run: bool,
    /// Emit a JSON envelope.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct ConfigDiffArgs {
    /// VM name (must match the static manifest).
    vm: String,
    /// The live host-side guest config file to compare the staging against.
    #[arg(long)]
    against: PathBuf,
    /// Emit a JSON envelope.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct ConfigApproveArgs {
    /// VM name (must match the static manifest).
    vm: String,
    /// The host-side file to write the approved staging copy onto. The
    /// operator chooses this (typically their `guestConfigFile` path).
    #[arg(long)]
    to: PathBuf,
    /// Emit a JSON envelope.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct ConfigRejectArgs {
    /// VM name (must match the static manifest).
    vm: String,
    /// Emit a JSON envelope.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct ConfigStatusArgs {
    /// VM name; omit together with `--all` to report every staged VM.
    vm: Option<String>,
    /// Report every VM that currently has a pending staging file.
    #[arg(long)]
    all: bool,
    /// Emit a JSON envelope.
    #[arg(long)]
    json: bool,
}

/// Base directory for host-side config staging. User-local by default
/// (no privileged surface). Overridable via `NIXLING_CONFIG_STAGING_DIR`
/// (used by tests) or `XDG_STATE_HOME`.
fn config_staging_base() -> PathBuf {
    if let Some(dir) = std::env::var_os("NIXLING_CONFIG_STAGING_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("/tmp/nixling-state"));
    base.join("nixling/config-staging")
}

fn config_staging_path_in(base: &Path, vm: &str) -> PathBuf {
    base.join(format!("{vm}.guest.nix"))
}

fn config_staging_path(vm: &str) -> PathBuf {
    config_staging_path_in(&config_staging_base(), vm)
}

/// Reject VM names that are not the framework's `^[a-z][a-z0-9-]*$`
/// shape, so a VM arg can never traverse out of the staging dir.
fn config_validate_vm_name(vm: &str) -> Result<(), CliFailure> {
    let ok = !vm.is_empty()
        && vm.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && vm
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if ok {
        Ok(())
    } else {
        Err(CliFailure::new(
            1,
            format!("config: invalid vm name '{vm}' (expected ^[a-z][a-z0-9-]*$)"),
        ))
    }
}

/// Validate a remote (in-guest) path passed to `config sync`: absolute
/// and restricted to safe path characters, so the remote `cat` cannot
/// be steered into shell metacharacters.
fn config_validate_remote_path(p: &str) -> Result<(), CliFailure> {
    let ok = p.starts_with('/')
        && p.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(CliFailure::new(
            1,
            format!("config sync: unsafe --guest-path '{p}' (absolute, [A-Za-z0-9._/-] only)"),
        ))
    }
}

/// Validate the bytes of a staging file before approval. Kept
/// deliberately light — the authoritative eval + containment gate is
/// the per-VM `guestConfigFile` assertion on `nixling switch`. Here we
/// only refuse an empty / non-UTF-8 file so approve cannot silently
/// land a truncated sync.
fn config_validate_staging_bytes(bytes: &[u8]) -> Result<(), CliFailure> {
    if bytes.is_empty() {
        return Err(CliFailure::new(
            1,
            "config approve: staged file is empty; re-run `nixling config sync`".to_owned(),
        ));
    }
    if std::str::from_utf8(bytes).is_err() {
        return Err(CliFailure::new(
            1,
            "config approve: staged file is not valid UTF-8".to_owned(),
        ));
    }
    if bytes.iter().all(|b| b.is_ascii_whitespace()) {
        return Err(CliFailure::new(
            1,
            "config approve: staged file is blank".to_owned(),
        ));
    }
    Ok(())
}

/// Core (testable) approve: validate the staging file, atomically write
/// it onto `target`, then remove the staging file. Returns the byte
/// count written.
fn config_approve_core(staging: &Path, target: &Path) -> Result<usize, CliFailure> {
    if !staging.exists() {
        return Err(CliFailure::new(
            1,
            format!(
                "config approve: nothing staged at {} (run `nixling config sync` first)",
                staging.display()
            ),
        ));
    }
    let bytes = std::fs::read(staging)
        .map_err(|e| CliFailure::new(1, format!("config approve: read staging: {e}")))?;
    config_validate_staging_bytes(&bytes)?;
    let parent = target.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(parent) = parent {
        if !parent.exists() {
            return Err(CliFailure::new(
                1,
                format!(
                    "config approve: target dir {} does not exist",
                    parent.display()
                ),
            ));
        }
    }
    // Atomic, collision-safe publish (unique O_EXCL temp + fsync +
    // rename); staging is only consumed after a successful publish.
    config_atomic_write(target, &bytes)?;
    let _ = std::fs::remove_file(staging);
    Ok(bytes.len())
}

/// Core (testable) reject: remove the staging file if present. Returns
/// whether anything was removed.
fn config_reject_core(staging: &Path) -> Result<bool, CliFailure> {
    if staging.exists() {
        std::fs::remove_file(staging)
            .map_err(|e| CliFailure::new(1, format!("config reject: {e}")))?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Emit a human-output (stderr) note when a VM has a pending,
/// un-approved staged guest config. Kept on stderr + gated by the
/// caller on `!json` so it never perturbs a JSON stdout envelope.
fn warn_pending_staged_config(vm: &str) {
    if config_staging_path(vm).exists() {
        eprintln!(
            "note: vm '{vm}' has a pending un-approved guest config edit \
             (`nixling config diff {vm} --against <live>` to review, \
             `nixling config approve {vm} --to <live>` to land, or \
             `nixling config reject {vm}` to discard)"
        );
    }
}

/// Emit a human-output (stderr) note listing every VM with a pending,
/// un-approved staged guest config.
fn warn_all_pending_staged_configs() {
    let base = config_staging_base();
    let mut pending: Vec<String> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&base) {
        for entry in rd.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if let Some(vm) = name.strip_suffix(".guest.nix") {
                    pending.push(vm.to_owned());
                }
            }
        }
    }
    pending.sort();
    if !pending.is_empty() {
        eprintln!(
            "note: pending un-approved guest config edit(s) for: {} \
             (`nixling config status --all`)",
            pending.join(", ")
        );
    }
}

/// Build the host→guest SSH argv for `config sync`. The remote command
/// is exactly `cat <guest_path>` placed AFTER the destination (where ssh
/// expects the remote command); no `--` separator is used (it would be
/// sent as part of the remote command). `guest_path` is validated by
/// [`config_validate_remote_path`] (absolute, metacharacter-free) before
/// reaching here, so it cannot inject into the remote shell. Host-key
/// integrity is verified against the framework-managed known_hosts with
/// `accept-new` (pins on first use; refuses a CHANGED key, so a same-env
/// peer cannot silently MITM the pulled config).
///
/// Retained for the operator SSH compatibility transport; the guest-control
/// config-sync path does not call it (it routes through the daemon's
/// authenticated `ReadGuestConfig` verb instead).
#[allow(dead_code)]
fn config_sync_ssh_argv(
    key_path: &Path,
    known_hosts: &Path,
    ssh_target: &str,
    guest_path: &str,
) -> Vec<String> {
    // nixling-ssh-allowlist begin: operator SSH compatibility transport
    vec![
        "ssh".to_owned(),
        "-i".to_owned(),
        key_path.display().to_string(),
        "-o".to_owned(),
        format!("UserKnownHostsFile={}", known_hosts.display()),
        "-o".to_owned(),
        "StrictHostKeyChecking=accept-new".to_owned(),
        "-o".to_owned(),
        "BatchMode=yes".to_owned(),
        ssh_target.to_owned(),
        "cat".to_owned(),
        guest_path.to_owned(),
    ]
    // nixling-ssh-allowlist end
}

/// Atomically publish `bytes` to `target`: write a UNIQUE sibling temp
/// (O_CREAT|O_EXCL so it never clobbers a concurrent writer's temp or a
/// stale leftover), fsync it, then rename over `target`. The rename is
/// atomic on the same filesystem, so a crash never leaves a partially
/// written file (and never a non-empty truncated one that `approve`
/// might later accept).
fn config_atomic_write(target: &Path, bytes: &[u8]) -> Result<(), CliFailure> {
    use std::io::Write as _;
    let parent = target.parent().filter(|p| !p.as_os_str().is_empty());
    let base = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("nixling-config");
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_name = format!(".{base}.nixling-tmp.{}.{nanos}", std::process::id());
    let tmp = match parent {
        Some(p) => p.join(tmp_name),
        None => PathBuf::from(tmp_name),
    };
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|e| CliFailure::new(1, format!("config: create temp {}: {e}", tmp.display())))?;
    let write_result = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&tmp);
        return Err(CliFailure::new(1, format!("config: write temp: {e}")));
    }
    std::fs::rename(&tmp, target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        CliFailure::new(1, format!("config: publish to {}: {e}", target.display()))
    })?;
    // fsync the parent directory so the rename (the directory-entry
    // update that publishes the new file) is itself durable. Without
    // this a power loss right after the rename can lose the approved
    // target update even though the staging file has already been
    // consumed.
    if let Some(p) = parent {
        if let Ok(dir) = std::fs::File::open(p) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

/// Run the `config sync` capture (testable, no `Context`): spawn
/// `argv[0]` with `argv[1..]`, STREAM its stdout into a bounded buffer
/// (hard byte cap + wall-clock timeout so a hostile guest cannot stream
/// an unbounded file — e.g. a symlink to `/dev/zero` — and OOM/hang the
/// host), fail on non-zero exit, validate the captured stdout
/// (non-empty/UTF-8), then atomically publish it to `staging`. Returns
/// the byte count. Spawning `argv[0]` (an absolute path or PATH-resolved
/// binary) makes this hermetically testable with a fake `ssh`.
///
/// Retained for the operator SSH compatibility transport; the guest-control
/// config-sync path does not spawn a subprocess.
#[allow(dead_code)]
fn config_sync_capture_to_staging(argv: &[String], staging: &Path) -> Result<usize, CliFailure> {
    // A guest config file is small; bound the untrusted pull on both
    // size and time. The guest controls the remote file, so both limits
    // are load-bearing security controls, not just hygiene.
    const MAX_BYTES: usize = 1 << 20; // 1 MiB
    const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
    config_sync_capture_to_staging_limited(argv, staging, MAX_BYTES, TIMEOUT)
}

/// Inner capture with injectable limits so the byte-cap AND timeout
/// paths are both hermetically testable.
#[allow(dead_code)]
fn config_sync_capture_to_staging_limited(
    argv: &[String],
    staging: &Path,
    max_bytes: usize,
    timeout: std::time::Duration,
) -> Result<usize, CliFailure> {
    use std::io::Read as _;

    // The deadline bounds the ENTIRE child lifetime, not just the stdout
    // read: a hostile endpoint could send a small valid payload, close
    // stdout (EOF), then linger to hang `child.wait()` forever.
    let deadline = std::time::Instant::now() + timeout;
    let timed_out = |child: &mut std::process::Child, reader: std::thread::JoinHandle<()>| {
        let _ = child.kill();
        let _ = child.wait();
        let _ = reader.join();
        CliFailure::new(
            1,
            format!(
                "config sync: timed out after {}ms pulling guest config",
                timeout.as_millis()
            ),
        )
    };

    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| CliFailure::new(1, format!("config sync: spawn {}: {e}", argv[0])))?;

    let mut stdout = child.stdout.take().expect("piped stdout");
    let (tx, rx) = std::sync::mpsc::channel::<Result<Vec<u8>, String>>();
    let reader = std::thread::spawn(move || {
        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 8192];
        let res = loop {
            match stdout.read(&mut chunk) {
                Ok(0) => break Ok(buf),
                Ok(n) => {
                    if buf.len() + n > max_bytes {
                        break Err(format!("guest config exceeds the {max_bytes}-byte limit"));
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                Err(e) => break Err(format!("read guest stdout: {e}")),
            }
        };
        let _ = tx.send(res);
    });

    let read_budget = deadline.saturating_duration_since(std::time::Instant::now());
    let stdout_bytes = match rx.recv_timeout(read_budget) {
        Ok(Ok(buf)) => buf,
        Ok(Err(msg)) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(CliFailure::new(1, format!("config sync: {msg}")));
        }
        Err(_) => return Err(timed_out(&mut child, reader)),
    };
    let _ = reader.join();

    // Bounded wait for the child to actually exit (covers the
    // stdout-closed-but-process-lingers case); kill on the deadline.
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(CliFailure::new(
                        1,
                        format!(
                            "config sync: timed out after {}ms pulling guest config",
                            timeout.as_millis()
                        ),
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(e) => {
                let _ = child.kill();
                return Err(CliFailure::new(1, format!("config sync: wait: {e}")));
            }
        }
    };
    if !status.success() {
        let mut stderr = String::new();
        if let Some(es) = child.stderr.take() {
            let mut raw = Vec::new();
            let _ = es.take(8192).read_to_end(&mut raw);
            stderr = String::from_utf8_lossy(&raw).trim().to_owned();
        }
        return Err(CliFailure::new(
            1,
            format!(
                "config sync: {} exited {}: {}",
                argv[0],
                status.code().unwrap_or(-1),
                stderr
            ),
        ));
    }

    config_validate_staging_bytes(&stdout_bytes)?;
    if let Some(parent) = staging.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CliFailure::new(1, format!("config sync: create staging dir: {e}")))?;
    }
    config_atomic_write(staging, &stdout_bytes)?;
    Ok(stdout_bytes.len())
}

/// Standard `sha256:<64-hex>` digest over `data`. Computed by the host from the
/// RECEIVED bytes; the guest-reported size/hash is never trusted.
fn sha256_hex(data: &[u8]) -> String {
    use sha2::Digest as _;
    use std::fmt::Write as _;
    let digest: [u8; 32] = sha2::Sha256::digest(data).into();
    let mut hex = String::with_capacity("sha256:".len() + 64);
    hex.push_str("sha256:");
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// True iff `vm`'s committed bundle declares a guest-control health node, i.e.
/// the VM exposes the authenticated guest-control transport. Old or partial
/// generations without the node return false and fall to the fail-closed
/// old-generation path (the operator SSH compatibility transport is wired in a
/// later milestone).
fn vm_uses_guest_control(context: &Context, vm: &str) -> Result<bool, CliFailure> {
    let Some(bundle) = context.load_bundle_context()? else {
        return Ok(false);
    };
    let Some(processes) = bundle.processes.as_ref() else {
        return Ok(false);
    };
    Ok(processes
        .vms
        .iter()
        .find(|entry| entry.vm == vm)
        .is_some_and(|entry| {
            entry.nodes.iter().any(|node| {
                matches!(
                    node.role,
                    nixling_core::processes::ProcessRole::GuestControlHealth
                )
            })
        }))
}

/// Build a consistent, leak-free CLI failure envelope for a guest-control
/// config-sync error. `observed_state`/`remediation` carry only daemon-supplied
/// closed-enum text — never guest content, paths, nonces, or transport detail.
fn guest_control_config_failure(
    kind: &str,
    what_was_checked: &str,
    observed_state: &str,
    remediation: &str,
    exit_code: i32,
    is_json: bool,
) -> CliFailure {
    let envelope = host_error_envelope(
        kind,
        kind,
        exit_code,
        what_was_checked,
        observed_state,
        remediation,
        "docs/reference/cli-contract.md#config-sync-guest-control-transport",
    );
    let rendered_stderr = if is_json {
        let mut rendered = serde_json::to_string_pretty(&envelope)
            .expect("serialize guest-control config failure envelope");
        rendered.push('\n');
        rendered
    } else {
        format!(
            "nixling: {} (code: {}, exit {})\n  what was checked : {}\n  observed         : {}\n  remediation      : {}\n  docs             : {}\n",
            envelope.kind,
            envelope.code,
            envelope.exit_code,
            envelope.what_was_checked,
            envelope.observed_state,
            envelope.remediation,
            envelope.docs_anchor,
        )
    };
    CliFailure {
        exit_code: envelope.exit_code,
        message: envelope.kind,
        rendered_stderr: Some(rendered_stderr),
    }
}

/// Surface a daemon-typed guest-control read error as a CLI failure, preserving
/// the daemon's closed-enum `kind`, human message, and remediation. The daemon
/// guarantees these fields never embed guest content (verified by its own
/// leak-free test), so they are safe to render verbatim.
fn guest_control_config_failure_from_daemon(
    error: DaemonErrorEnvelope,
    is_json: bool,
) -> CliFailure {
    let remediation = if error.remediation.is_empty() {
        "retry after the guest finishes booting, then check `nixling status <vm>`".to_owned()
    } else {
        error.remediation
    };
    guest_control_config_failure(
        &error.kind,
        "reading the guest config over the guest-control transport",
        &error.message,
        &remediation,
        i32::from(error.exit_code),
        is_json,
    )
}

/// Reject SSH-only overrides (and a non-default in-guest path) on the
/// guest-control path. These flags only configure the legacy operator SSH
/// transport; the guest-control transport reads the VM's canonical guest config
/// working copy over the authenticated channel.
fn reject_ssh_only_flags_on_guest_control(args: &ConfigSyncArgs) -> Result<(), CliFailure> {
    let mut offenders: Vec<&str> = Vec::new();
    if args.host.is_some() {
        offenders.push("--host");
    }
    if args.user.is_some() {
        offenders.push("--user");
    }
    if args.key.is_some() {
        offenders.push("--key");
    }
    if args.known_hosts.is_some() {
        offenders.push("--known-hosts");
    }
    if args.guest_path != DEFAULT_GUEST_CONFIG_PATH {
        offenders.push("--guest-path");
    }
    if offenders.is_empty() {
        return Ok(());
    }
    Err(guest_control_config_failure(
        "guest-control-ssh-flag-rejected",
        "validating the flags passed to config sync",
        &format!(
            "the {} flag(s) configure the legacy operator SSH transport, which is not used for guest-control VMs",
            offenders.join(", ")
        ),
        "omit these flags; the guest-control transport reads the VM's canonical guest config working copy over the authenticated channel",
        2,
        args.json,
    ))
}

/// Reply parsed from a `readGuestConfig` socket exchange.
enum GuestConfigReadOutcome {
    /// The daemon public socket was missing or not reachable.
    Unavailable,
    /// A raw daemon reply frame (success OR typed error frame).
    Reply(Vec<u8>),
}

/// Send an admin-only `readGuestConfig` request over the daemon public socket
/// and return the raw reply frame. Connection failures collapse to
/// `Unavailable`; any daemon reply (success or typed error) is returned verbatim
/// for [`finish_config_sync_from_reply`] to interpret.
fn read_guest_config_via_socket(
    context: &Context,
    vm: &str,
) -> Result<GuestConfigReadOutcome, CliFailure> {
    if !context.public_socket.exists() {
        return Ok(GuestConfigReadOutcome::Unavailable);
    }
    let mut socket = match SeqpacketUnixSocket::connect(&context.public_socket) {
        Ok(socket) => socket,
        Err(err) if is_daemon_unreachable(&err) => return Ok(GuestConfigReadOutcome::Unavailable),
        Err(err) => {
            return Err(CliFailure::new(
                1,
                format!(
                    "failed to connect to {}: {err}",
                    context.public_socket.display()
                ),
            ))
        }
    };
    let hello = daemon_hello_frame("hello")?;
    socket
        .send_frame(&hello)
        .map_err(|err| CliFailure::new(1, format!("failed to send hello frame: {err}")))?;
    let hello_response = socket
        .recv_frame()
        .map_err(|err| CliFailure::new(1, format!("failed to receive hello reply: {err}")))?;
    let _ = parse_hello_reply(&hello_response)?;
    let request = encode_type_tagged_message(
        "readGuestConfig",
        &ReadGuestConfigRequest { vm: vm.to_owned() },
        "readGuestConfig request",
    )?;
    socket.send_frame(&request).map_err(|err| {
        CliFailure::new(1, format!("failed to send readGuestConfig request: {err}"))
    })?;
    let response = socket.recv_frame().map_err(|err| {
        CliFailure::new(1, format!("failed to receive readGuestConfig reply: {err}"))
    })?;
    Ok(GuestConfigReadOutcome::Reply(response))
}

/// Result of staging a guest config pulled over the guest-control transport.
#[derive(Debug)]
struct ConfigSyncStaged {
    bytes: usize,
    sha256: String,
}

/// Interpret a `readGuestConfig` daemon reply: decode the base64 content,
/// re-enforce the raw size cap on the DECODED bytes, compute size + sha256 from
/// the received bytes (never a guest-reported value), and atomically stage the
/// result. On ANY error (daemon typed error frame, malformed reply, oversize, or
/// empty/non-UTF-8 content) this stages NOTHING and never echoes guest content
/// into the error.
fn finish_config_sync_from_reply(
    reply: &[u8],
    staging: &Path,
    is_json: bool,
) -> Result<ConfigSyncStaged, CliFailure> {
    let protocol_error = |observed: &str| {
        guest_control_config_failure(
            "guest-control-protocol-error",
            "decoding the daemon reply to config sync",
            observed,
            "retry; if it persists, restart nixlingd after switching to this generation",
            EXIT_GUEST_CONTROL_CONFIG,
            is_json,
        )
    };
    let value: Value = serde_json::from_slice(reply)
        .map_err(|_| protocol_error("the daemon returned a reply that was not valid JSON"))?;
    match value.get("type").and_then(Value::as_str).unwrap_or("") {
        "readGuestConfigResponse" => {
            let frame: ReadGuestConfigResponseFrame = serde_json::from_value(value)
                .map_err(|_| protocol_error("the daemon reply was missing contentBase64"))?;
            let bytes = nixling_core::base64_codec::decode(&frame.content_base64)
                .map_err(|_| protocol_error("the daemon returned a malformed base64 payload"))?;
            // Defense in depth: the daemon already bounds the encoded payload,
            // but the host re-enforces the raw cap and never trusts a
            // guest-reported size.
            if bytes.len() as u64 > nixling_ipc::guest_wire::READ_GUEST_FILE_MAX_BYTES {
                return Err(guest_control_config_failure(
                    "guest-control-file-too-large",
                    "validating the received guest config size",
                    "the received guest config exceeded the read cap",
                    "shrink the guest config working copy below the read cap and retry",
                    EXIT_GUEST_CONTROL_CONFIG,
                    is_json,
                ));
            }
            config_validate_staging_bytes(&bytes)?;
            let sha256 = sha256_hex(&bytes);
            if let Some(parent) = staging.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    CliFailure::new(1, format!("config sync: create staging dir: {e}"))
                })?;
            }
            config_atomic_write(staging, &bytes)?;
            Ok(ConfigSyncStaged {
                bytes: bytes.len(),
                sha256,
            })
        }
        "error" => {
            let frame: ErrorFrame = serde_json::from_value(value)
                .map_err(|_| protocol_error("the daemon returned a malformed error reply"))?;
            Err(guest_control_config_failure_from_daemon(
                frame.error,
                is_json,
            ))
        }
        other => Err(protocol_error(&format!(
            "the daemon returned an unexpected reply type '{other}'"
        ))),
    }
}

fn cmd_config_sync(context: &Context, args: &ConfigSyncArgs) -> Result<i32, CliFailure> {
    config_validate_vm_name(&args.vm)?;
    require_known_vm(context, &args.vm, args.json)?;

    if !vm_uses_guest_control(context, &args.vm)? {
        // Operator SSH-compatibility transport (wired in a later wave):
        // the in-guest path is meaningful there, so validate it before
        // reporting that the guest-control transport is unavailable.
        config_validate_remote_path(&args.guest_path)?;
        return Err(guest_control_config_failure(
            "guest-control-unavailable-old-generation",
            "selecting the config-sync transport for the VM",
            &format!(
                "vm '{}' does not declare the guest-control transport (old or partial generation)",
                args.vm
            ),
            "rebuild and switch the VM to a generation that enables guest control, then retry; the operator SSH compatibility transport is not yet wired into this command",
            EXIT_GUEST_CONTROL_CONFIG,
            args.json,
        ));
    }

    // Guest-control VMs: SSH-only flags (including a non-default
    // --guest-path) are rejected with the stable
    // guest-control-ssh-flag-rejected envelope (exit 2) BEFORE any generic
    // unsafe-path validation, so flag-rejection wins on the guest-control
    // path rather than collapsing to the exit-1 unsafe-path error.
    reject_ssh_only_flags_on_guest_control(args)?;

    let staging = config_staging_path(&args.vm);

    if args.dry_run {
        if args.json {
            let body = serde_json::json!({
                "command": "config sync",
                "mode": "dry-run",
                "vm": args.vm,
                "transport": "guest-control",
                "staging": staging.display().to_string(),
                "guestFile": "guest-config",
            });
            print_json(&body)?;
        } else {
            print_stdout(&format!(
                "config sync --dry-run: would read the canonical guest config working copy of {} \
                 over the authenticated guest-control transport and stage it to {}\n",
                args.vm,
                staging.display()
            ));
        }
        return Ok(0);
    }

    let staged = match read_guest_config_via_socket(context, &args.vm)? {
        GuestConfigReadOutcome::Unavailable => {
            return Err(guest_control_config_failure(
                "guest-control-transport-unavailable",
                "connecting to the nixling daemon for config sync",
                "the nixling daemon public socket was not reachable",
                "ensure nixlingd is running (`systemctl status nixlingd`) and retry",
                EXIT_GUEST_CONTROL_CONFIG,
                args.json,
            ));
        }
        GuestConfigReadOutcome::Reply(reply) => {
            finish_config_sync_from_reply(&reply, &staging, args.json)?
        }
    };

    if args.json {
        let body = serde_json::json!({
            "command": "config sync",
            "vm": args.vm,
            "transport": "guest-control",
            "staging": staging.display().to_string(),
            "bytes": staged.bytes,
            "sha256": staged.sha256,
        });
        print_json(&body)?;
    } else {
        print_stdout(&format!(
            "config sync: staged {} bytes (sha256 {}) from the guest-control transport of {} to {}\n\
             Review with `nixling config diff {} --against <guestConfigFile>` then \
             `nixling config approve {} --to <guestConfigFile>` \
             (the host-side nixling.vms.{}.guestConfigFile path).\n",
            staged.bytes,
            staged.sha256,
            args.vm,
            staging.display(),
            args.vm,
            args.vm,
            args.vm
        ));
    }
    Ok(0)
}

fn cmd_config_diff(args: &ConfigDiffArgs) -> Result<i32, CliFailure> {
    config_validate_vm_name(&args.vm)?;
    let staging = config_staging_path(&args.vm);
    if !staging.exists() {
        return Err(CliFailure::new(
            1,
            format!(
                "config diff: nothing staged for '{}' (run `nixling config sync` first)",
                args.vm
            ),
        ));
    }
    // `diff -u <live> <staged>`: exit 0 = identical, 1 = differ, >1 = error.
    let output = Command::new("diff")
        .arg("-u")
        .arg(&args.against)
        .arg(&staging)
        .output()
        .map_err(|e| CliFailure::new(1, format!("config diff: spawn diff: {e}")))?;
    let code = output.status.code().unwrap_or(-1);
    if code > 1 {
        return Err(CliFailure::new(
            1,
            format!(
                "config diff: diff failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    let differ = code == 1;
    let diff_text = String::from_utf8_lossy(&output.stdout).to_string();
    if args.json {
        let body = serde_json::json!({
            "command": "config diff",
            "vm": args.vm,
            "against": args.against.display().to_string(),
            "staging": staging.display().to_string(),
            "differs": differ,
            "diff": diff_text,
        });
        print_json(&body)?;
    } else if differ {
        print_stdout(&diff_text);
    } else {
        print_stdout(&format!(
            "config diff: staged config for '{}' is identical to {}\n",
            args.vm,
            args.against.display()
        ));
    }
    Ok(0)
}

fn cmd_config_approve(args: &ConfigApproveArgs) -> Result<i32, CliFailure> {
    config_validate_vm_name(&args.vm)?;
    let staging = config_staging_path(&args.vm);
    let n = config_approve_core(&staging, &args.to)?;
    if args.json {
        let body = serde_json::json!({
            "command": "config approve",
            "vm": args.vm,
            "target": args.to.display().to_string(),
            "bytes": n,
        });
        print_json(&body)?;
    } else {
        print_stdout(&format!(
            "config approve: wrote {n} bytes to {}. Review the change in your config tree, \
             then `nixling switch {}` to build + activate it (the guestConfigFile containment \
             assertion runs during that eval).\n",
            args.to.display(),
            args.vm
        ));
    }
    Ok(0)
}

fn cmd_config_reject(args: &ConfigRejectArgs) -> Result<i32, CliFailure> {
    config_validate_vm_name(&args.vm)?;
    let staging = config_staging_path(&args.vm);
    let removed = config_reject_core(&staging)?;
    if args.json {
        let body = serde_json::json!({
            "command": "config reject",
            "vm": args.vm,
            "removed": removed,
        });
        print_json(&body)?;
    } else if removed {
        print_stdout(&format!(
            "config reject: discarded staged config for '{}'\n",
            args.vm
        ));
    } else {
        print_stdout(&format!(
            "config reject: nothing staged for '{}'\n",
            args.vm
        ));
    }
    Ok(0)
}

fn cmd_config_status(args: &ConfigStatusArgs) -> Result<i32, CliFailure> {
    let base = config_staging_base();
    let pending: Vec<String> = if args.all || args.vm.is_none() {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&base) {
            for entry in rd.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if let Some(vm) = name.strip_suffix(".guest.nix") {
                        out.push(vm.to_owned());
                    }
                }
            }
        }
        out.sort();
        out
    } else {
        let vm = args.vm.as_deref().unwrap();
        config_validate_vm_name(vm)?;
        if config_staging_path_in(&base, vm).exists() {
            vec![vm.to_owned()]
        } else {
            Vec::new()
        }
    };
    if args.json {
        let body = serde_json::json!({
            "command": "config status",
            "pending": pending,
        });
        print_json(&body)?;
    } else if pending.is_empty() {
        match &args.vm {
            Some(vm) => print_stdout(&format!(
                "config status: no pending staged config for '{vm}'\n"
            )),
            None => print_stdout("config status: no pending staged guest configs\n"),
        }
    } else {
        print_stdout(&format!(
            "config status: pending (un-approved) staged config for: {}\n",
            pending.join(", ")
        ));
    }
    Ok(0)
}

fn cmd_list(context: &Context, args: &ListArgs) -> Result<i32, CliFailure> {
    let manifest = context.load_manifest()?;
    let bundle = context.load_bundle_context()?;
    let output = ListOutputV2(
        manifest
            .vms()
            .into_iter()
            .map(|vm| {
                let current = current_symlink(context, vm);
                let booted = booted_symlink(context, vm);
                let process_vm = bundle
                    .as_ref()
                    .and_then(|bundle| bundle.processes.as_ref())
                    .and_then(|processes| processes.vms.iter().find(|entry| entry.vm == vm.name));
                let services = vm_service_states(context, vm, process_vm);
                let pending_restart =
                    is_pending_restart(vm, &services, current.as_deref(), booted.as_deref());
                ListItemOutputV2 {
                    name: vm.name.clone(),
                    env: vm.env.clone(),
                    graphics: vm.graphics,
                    tpm: vm.tpm,
                    usbip: vm.usbip_yubikey,
                    static_ip: vm.static_ip.clone(),
                    status: list_status_label(vm, &services, pending_restart),
                    is_net_vm: vm.is_net_vm,
                    runner_parity_ok: bundle
                        .as_ref()
                        .and_then(|bundle| bundle.closures.get(&vm.name))
                        .map(|closure| closure.runner_parity_ok),
                }
            })
            .collect(),
    );

    if args.json {
        print_json(&output)?;
    } else {
        print_stdout(&render_list_human(&output));
    }
    Ok(0)
}

fn cmd_status(context: &Context, args: &StatusArgs) -> Result<i32, CliFailure> {
    let manifest = context.load_manifest()?;
    let bundle = context.load_bundle_context()?;

    if args.check_bridges {
        if args.vm.is_some() || args.vm_flag.is_some() {
            return Err(CliFailure::new(
                2,
                "status --check-bridges cannot be combined with a VM selector",
            ));
        }
        let output = StatusBridgeCheckOutputV2 {
            mode: "check-bridges".to_owned(),
            status: "not-yet-implemented".to_owned(),
            message: "bridge reconciliation is not yet wired; use `nixling host check --read-only` for advisory bridge-related probes".to_owned(),
            runtime: RUNTIME_UNKNOWN.to_owned(),
        };
        if args.json {
            print_json(&StatusOutputV2::CheckBridges(Box::new(output)))?;
        } else {
            print_stdout(&(output.message.clone() + "\n"));
        }
        return Ok(0);
    }

    let selected_vm = resolve_selected_vm(args)?;
    if !args.json {
        match &selected_vm {
            // Single-VM status only warns about THAT VM's pending edit,
            // never unrelated VMs.
            Some(vm) => warn_pending_staged_config(vm),
            None => warn_all_pending_staged_configs(),
        }
    }
    if let Some(vm_name) = selected_vm {
        let vm = manifest
            .get_vm(&vm_name)
            .ok_or_else(|| CliFailure::new(1, format!("unknown VM '{vm_name}'")))?;
        let output = build_vm_status_output(context, vm, bundle.as_ref());
        if args.json {
            print_json(&StatusOutputV2::Vm(Box::new(output)))?;
        } else {
            print_stdout(&render_status_vm_human(
                &output,
                vm,
                collect_bridge_rows(context, &manifest, bundle.as_ref()),
            ));
        }
    } else {
        let output = StatusInventoryOutputV2 {
            runtime: RUNTIME_UNKNOWN.to_owned(),
            vms: manifest
                .vms()
                .into_iter()
                .map(|vm| build_vm_status_output(context, vm, bundle.as_ref()))
                .collect(),
        };
        if args.json {
            print_json(&StatusOutputV2::Inventory(Box::new(output)))?;
        } else {
            print_stdout(&render_status_inventory_human(
                &output,
                &manifest,
                context,
                bundle.as_ref(),
            ));
        }
    }

    Ok(0)
}

fn cmd_audit(
    context: &Context,
    args: &AuditArgs,
    _original_args: &[OsString],
) -> Result<i32, CliFailure> {
    let json_mode = if args.human {
        false
    } else if args.json {
        true
    } else {
        !stdout_is_tty()
    };
    if args.strict {
        return emit_host_error(&not_yet_implemented_envelope("audit --strict"), json_mode);
    }
    match try_audit_via_socket(context, json_mode)? {
        AuditSocketOutcome::Lines(lines) => {
            render_daemon_audit_lines(&lines, json_mode)?;
            Ok(0)
        }
        AuditSocketOutcome::Unreachable => {
            emit_host_error(&daemon_down_envelope("audit"), json_mode)
        }
    }
}

fn cmd_console(
    _context: &Context,
    _args: &ConsoleArgs,
    _original_args: &[OsString],
) -> Result<i32, CliFailure> {
    emit_host_error(&not_yet_implemented_envelope("console"), false)
}

fn cmd_audio(
    _context: &Context,
    _args: &AudioArgs,
    _original_args: &[OsString],
) -> Result<i32, CliFailure> {
    emit_host_error(&not_yet_implemented_envelope("audio"), false)
}

fn cmd_host_check(context: &Context, args: &HostCheckArgs) -> Result<i32, CliFailure> {
    let bundle = context.load_bundle_context()?.ok_or_else(|| {
        CliFailure::new(
            1,
            format!(
                "{} is required for host check",
                context.bundle_path.display()
            ),
        )
    })?;
    let host = bundle
        .host
        .as_ref()
        .ok_or_else(|| CliFailure::new(1, "bundle did not include host.json"))?;
    let report = host_check::run(host, bundle.closures.values(), args.strict)
        .map_err(CliFailure::host_check_probe_error)?;
    let output = map_host_check_report(report);

    if args.json {
        print_json(&output)?;
    } else {
        print_stdout(&render_host_check_human(&output));
    }

    Ok(i32::from(output.exit_code))
}

fn map_host_check_report(report: host_check::HostCheckReport) -> HostCheckOutputV2 {
    HostCheckOutputV2 {
        mode: "read-only".to_owned(),
        strict: report.strict,
        summary: HostCheckSummaryV2 {
            pass: report.summary.pass,
            warn: report.summary.warn,
            fail: report.summary.fail,
        },
        exit_code: report.exit_code(),
        findings: report
            .findings
            .into_iter()
            .map(map_host_check_finding)
            .collect(),
    }
}

fn map_host_check_finding(finding: host_check::HostCheckFinding) -> HostCheckFindingV2 {
    HostCheckFindingV2 {
        id: finding.id,
        severity: map_host_check_severity(finding.severity),
        message: finding.message,
        remediation: finding.remediation,
        vm: finding.vm,
        detail: finding.detail,
        details: finding.details,
    }
}

fn map_host_check_severity(severity: host_check::HostCheckSeverity) -> HostCheckSeverityV2 {
    match severity {
        host_check::HostCheckSeverity::Pass => HostCheckSeverityV2::Pass,
        host_check::HostCheckSeverity::Warn => HostCheckSeverityV2::Warn,
        host_check::HostCheckSeverity::Fail => HostCheckSeverityV2::Fail,
    }
}

/// Standard JSON error envelope. Every native host-verb refusal
/// emits this shape on stdout (JSON mode) or as a human-readable
/// summary on stderr (default mode).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostErrorEnvelope {
    kind: String,
    code: String,
    exit_code: i32,
    what_was_checked: String,
    observed_state: String,
    remediation: String,
    docs_anchor: String,
}

fn host_error_envelope(
    kind: &str,
    code: &str,
    exit_code: i32,
    what_was_checked: &str,
    observed_state: &str,
    remediation: &str,
    docs_anchor: &str,
) -> HostErrorEnvelope {
    HostErrorEnvelope {
        kind: kind.to_owned(),
        code: code.to_owned(),
        exit_code,
        what_was_checked: what_was_checked.to_owned(),
        observed_state: observed_state.to_owned(),
        remediation: remediation.to_owned(),
        docs_anchor: docs_anchor.to_owned(),
    }
}

fn emit_host_error(env: &HostErrorEnvelope, json: bool) -> Result<i32, CliFailure> {
    if json {
        let mut rendered = serde_json::to_string_pretty(env).map_err(|err| {
            CliFailure::new(1, format!("failed to serialize host error envelope: {err}"))
        })?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        let _ = writeln!(
            io::stderr().lock(),
            "nixling: {} (code: {}, exit {})\n  what was checked : {}\n  observed         : {}\n  remediation      : {}\n  docs             : {}",
            env.kind,
            env.code,
            env.exit_code,
            env.what_was_checked,
            env.observed_state,
            env.remediation,
            env.docs_anchor,
        );
    }
    Ok(env.exit_code)
}

/// Typed `daemon-down` envelope (exit 1) for verbs whose
/// daemon-backed path cannot be reached. The Rust CLI never executes
/// bash; verbs surface this envelope when the daemon is unreachable.
fn daemon_down_envelope(verb: &str) -> HostErrorEnvelope {
    host_error_envelope(
        &format!("nixling {verb} requires nixlingd"),
        "daemon-down",
        1,
        "Daemon connectivity at /run/nixling/public.sock.",
        "nixlingd is unreachable; the daemon is the only operator surface for mutating verbs.",
        "Start nixlingd (systemctl start nixlingd nixling-priv-broker.socket) and re-run the same command. See docs/how-to/migrate-nixling-v1-0-to-v1-1.md#recovery-broker-bring-up-troubleshooting for the full bring-up checklist.",
        "docs/reference/error-codes.md#daemon-down",
    )
}

/// Typed `not-yet-implemented` envelope (exit 78) for verbs whose
/// daemon-native handler has not landed yet. No bash fallback ever
/// satisfies these — operators receive the typed envelope and the
/// migration-guide cross-link.
fn not_yet_implemented_envelope(verb: &str) -> HostErrorEnvelope {
    host_error_envelope(
        &format!("nixling {verb} has no daemon-native handler yet"),
        "not-yet-implemented",
        78,
        &format!("Native daemon dispatch for `nixling {verb}`"),
        "The daemon-native handler has not landed yet; the typed envelope contract is the only operator path until the native handler ships.",
        "Track the surface schedule in CHANGELOG.md \"Unreleased\"; the typed envelope is the only operator path until the native handler ships.",
        "docs/reference/error-codes.md#not-yet-implemented",
    )
}

/// Bundle-derived deployment shape used by the `host prepare` /
/// `host destroy` per-tier routing logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeploymentShape {
    /// Legacy Tier-0 all-legacy shape: no daemon-owned VMs. The
    /// per-VM `supervisor` option was removed in v1.1, so a real
    /// bundle never resolves here; only the
    /// `NIXLING_TEST_DEPLOYMENT_SHAPE` test override can select it.
    Tier0AllLegacy,
    /// Mixed: some VMs daemon-owned, some systemd-owned.
    Tier0Mixed,
    /// Every VM is daemon-owned, or the bundle is Tier 1+.
    AllDaemon,
}

fn detect_deployment_shape(context: &Context) -> Result<DeploymentShape, CliFailure> {
    // Test override (used by goldens + cli-legacy-bash-dispatch).
    if let Ok(value) = env::var("NIXLING_TEST_DEPLOYMENT_SHAPE") {
        return Ok(match value.as_str() {
            "tier0-all-legacy" => DeploymentShape::Tier0AllLegacy,
            "tier0-mixed" => DeploymentShape::Tier0Mixed,
            "all-daemon" | "tier1" => DeploymentShape::AllDaemon,
            other => {
                return Err(CliFailure::new(
                    1,
                    format!("unknown NIXLING_TEST_DEPLOYMENT_SHAPE value: {other}"),
                ));
            }
        });
    }
    // Default to Tier-0 all-legacy when we can't load a bundle —
    // safest fail-closed shape for the `--apply` refusal contract.
    let bundle = context.load_bundle_context().ok().flatten();
    let Some(_bundle) = bundle else {
        return Ok(DeploymentShape::Tier0AllLegacy);
    };
    // The per-VM `supervisor` option was removed in v1.1: every
    // enabled VM is daemon-supervised, so a real bundle always
    // resolves to all-daemon. The Tier-0 shapes remain reachable only
    // through the `NIXLING_TEST_DEPLOYMENT_SHAPE` override above.
    Ok(DeploymentShape::AllDaemon)
}

fn cmd_host_prepare(context: &Context, args: &HostPrepareArgs) -> Result<i32, CliFailure> {
    let flags =
        require_explicit_mutation_flag("host prepare", args.dry_run, args.apply, args.json)?;
    let shape = detect_deployment_shape(context)?;
    match (shape, flags.apply) {
        (DeploymentShape::Tier0AllLegacy, true) => emit_host_error(
            &host_error_envelope(
                "Tier 0 all-legacy refused: use the NixOS module path",
                "tier-0-legacy-uses-nixos-module",
                78,
                "Whether this host resolves to the legacy Tier-0 all-legacy shape, which has no daemon-owned resources for the broker to reconcile.",
                "tier-0-all-legacy",
                "This legacy Tier-0 shape is unreachable on a daemon-only host: the per-VM `supervisor` option was removed in v1.1, so every enabled VM is daemon-supervised. Host-shared reconciliation on a genuine legacy host is owned by the nixling NixOS module; run `host prepare --dry-run` to inspect the plan.",
                "docs/reference/error-codes.md#tier-0-legacy-uses-nixos-module",
            ),
            args.json,
        ),
        (DeploymentShape::Tier0Mixed, true) => emit_host_error(
            &host_error_envelope(
                "Single-writer conflict refused",
                "single-writer-conflict",
                78,
                "At least one host-shared resource (bridge / TAP / nft chain / NM unmanaged file / /etc/hosts entry / sysctl) is claimed by both the NixOS module path and a daemon-owned VM.",
                "tier-0-mixed",
                "Move the conflicting resource exclusively to the daemon path or exclusively to the NixOS module path, then re-run host prepare --apply.",
                "docs/reference/error-codes.md#single-writer-conflict",
            ),
            args.json,
        ),
        (_, true) => {
            // Broker dispatch is staged in the privileged broker, but
            // the daemon path that wires the typed bundle intents through
            // `nixlingd` is not yet shipping in
            // bootstrap mode. Surface the same pending-impl envelope
            // the broker would emit so the human / JSON contract
            // stays stable.
            emit_host_error(
                &host_error_envelope(
                    "Daemon-backed prepare staged but the public-socket dispatch path is pending",
                    "daemon-down",
                    1,
                    "Daemon connectivity at /run/nixling/public.sock and broker dispatch readiness.",
                    "nixlingd is reachable, but the daemon-side typed-intent dispatch and bundle resolver that back host prepare --apply are not yet wired through nixlingd; the broker op is staged but not yet reachable from the public socket.",
                    "Re-run with --dry-run for now; production --apply lands together with the daemon-side bundle resolver.",
                    "docs/reference/error-codes.md#daemon-down",
                ),
                args.json,
            )
        }
        (_, false) => {
            // --dry-run: report the planned reconciliation. The
            // bash dispatch test exercises this path via a mock,
            // and the per-tier behavior table mandates `dry-run`
            // reports without mutation on every tier.
            let summary = serde_json::json!({
                "command": "host prepare",
                "mode": "dry-run",
                "tier": match shape {
                    DeploymentShape::Tier0AllLegacy => "tier-0-all-legacy",
                    DeploymentShape::Tier0Mixed => "tier-0-mixed",
                    DeploymentShape::AllDaemon => "all-daemon",
                },
                "planned": [],
                "notes": "host-prepare dry-run reports the planned reconcile without mutation; --apply mutates host state.",
            });
            if args.json {
                let mut rendered = serde_json::to_string_pretty(&summary).map_err(|err| {
                    CliFailure::new(1, format!("failed to serialize dry-run summary: {err}"))
                })?;
                rendered.push('\n');
                print_stdout(&rendered);
            } else {
                print_stdout("host prepare --dry-run: would do nothing on this tier (no daemon-owned resources detected)\n");
            }
            Ok(0)
        }
    }
}

fn cmd_host_destroy(context: &Context, args: &HostDestroyArgs) -> Result<i32, CliFailure> {
    let flags =
        require_explicit_mutation_flag("host destroy", args.dry_run, args.apply, args.json)?;
    let shape = detect_deployment_shape(context)?;
    if flags.apply && matches!(shape, DeploymentShape::Tier0AllLegacy) {
        return emit_host_error(
            &host_error_envelope(
                "Tier 0 all-legacy refused: use the NixOS module path",
                "tier-0-legacy-uses-nixos-module",
                78,
                "Whether this host resolves to the legacy Tier-0 all-legacy shape; host destroy only acts on daemon-owned resources.",
                "tier-0-all-legacy",
                "This legacy Tier-0 shape is unreachable on a daemon-only host: the per-VM `supervisor` option was removed in v1.1, so every enabled VM is daemon-supervised. The historical `--legacy` bash-destroy escape hatch was retired in v1.0 (per ADR 0015); run `host destroy --dry-run` to inspect nixling-owned resources.",
                "docs/reference/error-codes.md#tier-0-legacy-uses-nixos-module",
            ),
            args.json,
        );
    }
    if flags.apply {
        return emit_host_error(
            &host_error_envelope(
                "Daemon-backed destroy staged but the public-socket dispatch path is pending",
                "daemon-down",
                1,
                "Daemon connectivity and broker destroy dispatch readiness.",
                "nixlingd is reachable, but the daemon-side typed-intent dispatch and bundle resolver that back host destroy --apply are not yet wired through nixlingd; the broker op is staged but not yet reachable from the public socket.",
                "Re-run with --dry-run for now; production --apply lands together with the daemon-side bundle resolver.",
                "docs/reference/error-codes.md#daemon-down",
            ),
            args.json,
        );
    }
    let summary = serde_json::json!({
        "command": "host destroy",
        "mode": "dry-run",
        "tier": match shape {
            DeploymentShape::Tier0AllLegacy => "tier-0-all-legacy",
            DeploymentShape::Tier0Mixed => "tier-0-mixed",
            DeploymentShape::AllDaemon => "all-daemon",
        },
        "planned": [],
        "notes": "host destroy --dry-run reports nixling-owned resources only; foreign resources are never touched.",
    });
    if args.json {
        let mut rendered = serde_json::to_string_pretty(&summary).map_err(|err| {
            CliFailure::new(1, format!("failed to serialize dry-run summary: {err}"))
        })?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        print_stdout("host destroy --dry-run: no nixling-owned resources to remove\n");
    }
    Ok(0)
}

fn cmd_host_doctor(context: &Context, args: &HostDoctorArgs) -> Result<i32, CliFailure> {
    if !args.read_only {
        return emit_host_error(
            &host_error_envelope(
                "host doctor requires the explicit --read-only flag",
                "--read-only-required",
                78,
                "host doctor invocation flags.",
                "--read-only flag missing",
                "Re-run as `nixling host doctor --read-only`. The doctor verb is read-only; mutation forms are future deliverables.",
                "docs/reference/error-codes.md#--read-only-required",
            ),
            args.json,
        );
    }

    let report = doctor::run_doctor(context);
    let summary = doctor::render_summary(&report);
    let exit_code = report.exit_code();

    if args.json {
        let mut rendered = serde_json::to_string_pretty(&summary).map_err(|err| {
            CliFailure::new(1, format!("failed to serialize doctor summary: {err}"))
        })?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        print_stdout(&doctor::render_human(&report));
    }
    Ok(exit_code)
}

fn cmd_host_validate(_context: &Context, args: &HostValidateArgs) -> Result<i32, CliFailure> {
    let flags =
        require_explicit_mutation_flag("host validate", args.dry_run, args.apply, args.json)?;
    let mode = if flags.apply {
        host_validate::ValidateMode::Apply
    } else {
        host_validate::ValidateMode::DryRun
    };
    let mut req = host_validate::ValidateRequest::from_env_defaults(mode);
    if let Some(dir) = &args.evidence_dir {
        req.evidence_dir = dir.clone();
    }
    if let Some(dir) = &args.scripts_dir {
        req.scripts_dir = dir.clone();
    }
    if let Some(wave) = &args.wave {
        req.only_wave = Some(wave.clone());
    }
    if let Some(sig) = &args.operator_signature {
        req.operator_signature = Some(sig.clone());
    }

    // Validate `--wave` value against the catalog before doing any
    // filesystem work — surface a typed envelope instead of a silent
    // empty report.
    if let Some(only) = &req.only_wave {
        let known: bool = host_validate::WAVE_CATALOG.iter().any(|w| w.wave == only);
        if !known {
            let known_list: Vec<&str> =
                host_validate::WAVE_CATALOG.iter().map(|w| w.wave).collect();
            return emit_host_error(
                &host_error_envelope(
                    "host validate --wave value is not a known readiness wave",
                    "unknown-wave",
                    78,
                    "host validate --wave argument.",
                    &format!("--wave {only} is not in the readiness-wave catalog"),
                    &format!(
                        "Re-run with one of: {}. The catalog mirrors readinessWaveSpecs in nixos-modules/options-daemon.nix.",
                        known_list.join(", ")
                    ),
                    "docs/reference/host-validate.md#waves",
                ),
                args.json,
            );
        }
    }

    let report = host_validate::run_host_validate(&req);
    let exit_code = host_validate::exit_code(&report);
    if args.json {
        let mut rendered = serde_json::to_string_pretty(&host_validate::render_summary(&report))
            .map_err(|err| {
                CliFailure::new(
                    1,
                    format!("failed to serialize host validate summary: {err}"),
                )
            })?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        print_stdout(&host_validate::render_human(&report));
    }
    Ok(exit_code)
}

fn cmd_host_install(
    context: &Context,
    args: &HostInstallArgs,
    _original_args: &[OsString],
) -> Result<i32, CliFailure> {
    // host install --dry-run/--apply/--enable/--start/--no-start
    // skeleton. --dry-run returns the planned 5-step install:
    // (1) place units, (2) write daemon-config.json, (3) bind sockets,
    // (4) optionally enable + start nixlingd.service, (5) emit smoke.
    if !args.dry_run && !args.apply {
        return emit_host_error(
            &host_error_envelope(
                "host install requires either --dry-run or --apply",
                "--apply-or-dry-run-required",
                78,
                "host install invocation flags.",
                "Neither --dry-run nor --apply was provided.",
                "Re-run as `nixling host install --dry-run` to plan or `nixling host install --apply` (optionally with --enable / --start | --no-start) to install.",
                "docs/reference/error-codes.md#--apply-or-dry-run-required",
            ),
            args.json,
        );
    }
    if args.apply {
        return dispatch_mutating_verb(
            context,
            "hostInstall",
            serde_json::json!({
                "enable": args.enable,
                "start": args.start,
                "noStart": args.no_start,
            }),
            args.dry_run,
            args.apply,
            args.json,
        );
    }
    // --dry-run path
    let summary = serde_json::json!({
        "command": "host install",
        "mode": "dry-run",
        "planned_steps": [
            { "step": 1, "what": "place systemd units at /etc/systemd/system/nixlingd.service + nixling-priv-broker.socket" },
            { "step": 2, "what": "write daemon-config.json to /etc/nixling/daemon-config.json with paths matching the daemon's compiled-in defaults" },
            { "step": 3, "what": "bind /run/nixling/public.sock + /run/nixling/priv.sock with socket ACLs (launcher / admin groups)" },
            { "step": 4, "what": if args.enable && args.start { "systemctl enable --now nixlingd.service" } else if args.enable { "systemctl enable nixlingd.service" } else if args.no_start { "do NOT enable; operator starts manually" } else { "neither --enable nor --start specified: leave service inactive" } },
            { "step": 5, "what": "smoke: nixling auth status against /run/nixling/public.sock" },
        ],
        "notes": "dry-run preview; --apply routes through the daemon → broker RunHostInstall path.",
    });
    if args.json {
        let mut rendered = serde_json::to_string_pretty(&summary)
            .map_err(|e| CliFailure::new(1, format!("serialize: {e}")))?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        print_stdout(
            "host install --dry-run: would install nixlingd at /etc/systemd/system/ and bind /run/nixling/public.sock (the live --apply path routes through the daemon → broker RunHostInstall path)\n",
        );
    }
    Ok(0)
}

fn cmd_host_reconcile(
    context: &Context,
    args: &HostReconcileArgs,
    _original_args: &[OsString],
) -> Result<i32, CliFailure> {
    // SOLE mutating recovery verb for the daemon's net-route preflight
    // degraded mode.
    // Mandatory flag pair (--dry-run XOR --apply) matches the rest
    // of the mutating verbs. `--network` is required because it is
    // the only scope today; routing without a scope flag would be
    // ambiguous.
    if !args.dry_run && !args.apply {
        return emit_host_error(
            &host_error_envelope(
                "host reconcile requires either --dry-run or --apply",
                "--apply-or-dry-run-required",
                78,
                "host reconcile invocation flags.",
                "Neither --dry-run nor --apply was provided.",
                "Re-run as `nixling host reconcile --network --dry-run` to plan or `nixling host reconcile --network --apply` to apply.",
                "docs/reference/error-codes.md#--apply-or-dry-run-required",
            ),
            args.json,
        );
    }
    if !args.network {
        return emit_host_error(
            &host_error_envelope(
                "host reconcile requires --network (at least one scope must be selected)",
                "--scope-required",
                78,
                "host reconcile invocation flags.",
                "No reconcile scope was provided.",
                "Re-run with `--network` (the only scope available today); future scopes will be added in later releases.",
                "docs/explanation/host-prepare.md",
            ),
            args.json,
        );
    }
    dispatch_mutating_verb(
        context,
        "hostReconcile",
        serde_json::json!({
            "network": args.network,
        }),
        args.dry_run,
        args.apply,
        args.json,
    )
}

fn require_known_vm(context: &Context, vm: &str, json: bool) -> Result<(), CliFailure> {
    let manifest = context.load_manifest()?;
    if manifest.vms().iter().any(|v| v.name == vm) {
        return Ok(());
    }
    let exit_code = emit_host_error(
        &host_error_envelope(
            &format!("vm '{vm}' is not declared in the loaded manifest"),
            "not-found",
            70,
            "Whether the VM name appears in `nixling.vms.<name>` in the active manifest.",
            "VM name unknown",
            "Run `nixling list` to see declared VMs, then re-run with a name from that list.",
            "docs/reference/error-codes.md#not-found",
        ),
        json,
    )?;
    Err(CliFailure::new(exit_code, format!("unknown vm: {vm}")))
}

fn vm_dag_dry_run_summary(verb: &str, vm: &str) -> serde_json::Value {
    // The DAG the supervisor would drive. Mirrors the structure emitted
    // by the processes::VmProcessDag exporter — for the headless alpha
    // shape (host-reconcile → store-preflight → virtiofsd-ro-store → ch
    // → guest-control-health) we summarize the node ids and the
    // topological edges. The full per-role argv preview is a follow-up
    // gate.
    //
    // `vm stop` walks the DAG in REVERSE topo order (terminate ch first,
    // then virtiofsd, etc).
    // The dry-run summary reflects the current apply order so the
    // operator sees the same DAG the daemon bridge will drive.
    let stopping = matches!(verb, "stop");
    let restarting = matches!(verb, "restart");
    let forward_nodes: Vec<serde_json::Value> = vec![
        serde_json::json!({"id": "host-reconcile",        "role": "host-reconcile"}),
        serde_json::json!({"id": "store-preflight",       "role": "store-virtiofs-preflight"}),
        serde_json::json!({"id": "virtiofsd-ro-store",    "role": "virtiofsd"}),
        serde_json::json!({"id": "ch",                    "role": "cloud-hypervisor-runner"}),
        serde_json::json!({"id": "guest-control-health",  "role": "guest-control-health"}),
    ];
    let forward_edges = serde_json::json!([
        {"from": "host-reconcile",     "to": "store-preflight"},
        {"from": "store-preflight",    "to": "virtiofsd-ro-store"},
        {"from": "virtiofsd-ro-store", "to": "ch"},
        {"from": "ch",                 "to": "guest-control-health"},
    ]);
    let stop_order = serde_json::json!([
        "guest-control-health",
        "ch",
        "virtiofsd-ro-store",
        "store-preflight",
        "host-reconcile",
    ]);
    serde_json::json!({
        "command": format!("vm {verb}"),
        "mode": "dry-run",
        "vm": vm,
        "dag": {
            "nodes": forward_nodes,
            "edges": forward_edges,
        },
        "stopOrder": if stopping || restarting { Some(stop_order) } else { None::<serde_json::Value> },
        "notes": "vm dry-run reports the DAG the supervisor would drive (start: topo order; stop: reverse topo). --apply routes through nixlingd → broker (v1.0 daemon-only per ADR 0015).",
    })
}

fn cmd_vm_lifecycle_verb(
    context: &Context,
    verb: &str,
    vm: &str,
    dry_run: bool,
    apply: bool,
    no_wait_api: bool,
    json: bool,
) -> Result<i32, CliFailure> {
    let flags = require_explicit_mutation_flag(&format!("vm {verb}"), dry_run, apply, json)?;
    require_known_vm(context, vm, json)?;
    if (verb == "start" || verb == "restart") && !json {
        warn_pending_staged_config(vm);
    }
    if flags.apply {
        // VM lifecycle verbs are daemon-only. The bash-translation
        // bridge has been removed; any failure mode
        // surfaces as a typed envelope via `dispatch_mutating_verb`.
        let request_type = match verb {
            "start" => "vmStart",
            "stop" => "vmStop",
            "restart" => "vmRestart",
            other => other,
        };
        let extra_fields = if no_wait_api {
            serde_json::json!({ "vm": vm, "noWaitApi": true })
        } else {
            serde_json::json!({ "vm": vm })
        };
        return dispatch_mutating_verb(
            context,
            request_type,
            extra_fields,
            flags.dry_run,
            flags.apply,
            json,
        );
    }
    let summary = vm_dag_dry_run_summary(verb, vm);
    if json {
        let mut rendered = serde_json::to_string_pretty(&summary).map_err(|err| {
            CliFailure::new(1, format!("failed to serialize vm dry-run summary: {err}"))
        })?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        print_stdout(&format!(
            "vm {verb} --dry-run: would drive the 5-node DAG for vm '{vm}' (host-reconcile → store-preflight → virtiofsd-ro-store → ch → guest-control-health)\n"
        ));
    }
    Ok(0)
}

fn cmd_vm_start(context: &Context, args: &VmStartArgs) -> Result<i32, CliFailure> {
    cmd_vm_lifecycle_verb(
        context,
        "start",
        &args.vm,
        args.dry_run,
        args.apply,
        args.no_wait_api,
        args.json,
    )
}

fn cmd_vm_stop(context: &Context, args: &VmStopArgs) -> Result<i32, CliFailure> {
    cmd_vm_lifecycle_verb(
        context,
        "stop",
        &args.vm,
        args.dry_run,
        args.apply,
        false,
        args.json,
    )
}

fn cmd_vm_restart(context: &Context, args: &VmRestartArgs) -> Result<i32, CliFailure> {
    cmd_vm_lifecycle_verb(
        context,
        "restart",
        &args.vm,
        args.dry_run,
        args.apply,
        false,
        args.json,
    )
}

fn cmd_vm_list(_context: &Context, args: &VmListArgs) -> Result<i32, CliFailure> {
    // `vm list` already has its stable JSON shape, but this shim still
    // returns a placeholder empty inventory rather than a live daemon
    // runner table. Keep the empty list explicit so callers do not
    // misread it as proof that no VMs exist.
    let body = serde_json::json!({
        "command": "vm list",
        "entries": [],
        "notes": "vm list placeholder: live daemon runner inventory is not wired through this surface yet; use `nixling status <vm>` for per-VM truth.",
    });
    if args.json {
        let mut rendered = serde_json::to_string_pretty(&body)
            .map_err(|err| CliFailure::new(1, format!("failed to serialize vm list: {err}")))?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        print_stdout(
            "vm list: daemon runner inventory not yet exposed here; use `nixling status <vm>`\n",
        );
    }
    Ok(0)
}

fn cmd_vm_status(context: &Context, args: &VmStatusArgs) -> Result<i32, CliFailure> {
    cmd_status(
        context,
        &StatusArgs {
            json: args.json,
            human: args.human,
            check_bridges: false,
            vm_flag: None,
            vm: Some(args.vm.clone()),
        },
    )
}

/// The owner-connection transport: one op per round trip over the held
/// public.sock seqpacket connection. The daemon multiplexes a single
/// authenticated guest-control session behind this connection.
struct OwnerSocketTransport {
    socket: SeqpacketUnixSocket,
    next_op_id: u64,
}

impl exec_client::ExecOwnerTransport for OwnerSocketTransport {
    fn round_trip(
        &mut self,
        op: &nixling_ipc::public_wire::ExecOp,
    ) -> Result<nixling_ipc::public_wire::ExecOpResponse, exec_client::ExecClientError> {
        let op_id = self.next_op_id;
        self.next_op_id = self.next_op_id.wrapping_add(1);
        let frame = exec_client::encode_exec_op_frame(op, op_id)?;
        self.socket.send_frame(&frame).map_err(|err| {
            exec_client::ExecClientError::transport(format!("exec op send failed: {err}"))
        })?;
        let reply = self.socket.recv_frame().map_err(|err| {
            exec_client::ExecClientError::transport(format!("exec op recv failed: {err}"))
        })?;
        exec_client::decode_exec_response_frame(&reply)
    }
}

/// Typed transport error for an unreachable daemon on the exec path: there is
/// no SSH fallback, so an absent/unreachable daemon is a transport failure.
fn exec_daemon_unavailable_error() -> exec_client::ExecClientError {
    exec_client::ExecClientError::transport(
        "vm exec: the nixling daemon is not reachable on its public socket; \
         start nixlingd and retry (nixling does not fall back to SSH)",
    )
}

fn exec_owner_transport(
    context: &Context,
) -> Result<OwnerSocketTransport, exec_client::ExecClientError> {
    if !context.public_socket.exists() {
        return Err(exec_daemon_unavailable_error());
    }
    let mut socket =
        SeqpacketUnixSocket::connect(&context.public_socket).map_err(|err| match err {
            err if is_daemon_unreachable(&err) => exec_daemon_unavailable_error(),
            err => exec_client::ExecClientError::transport(format!(
                "vm exec: failed to connect to the daemon: {err}"
            )),
        })?;
    let hello = daemon_hello_frame("hello")
        .map_err(|failure| exec_client::ExecClientError::internal(failure.message))?;
    socket.send_frame(&hello).map_err(|err| {
        exec_client::ExecClientError::transport(format!(
            "vm exec: failed to send hello frame: {err}"
        ))
    })?;
    let hello_reply = socket.recv_frame().map_err(|err| {
        exec_client::ExecClientError::transport(format!(
            "vm exec: failed to receive hello reply: {err}"
        ))
    })?;
    parse_hello_reply(&hello_reply)
        .map_err(|failure| exec_client::ExecClientError::protocol(failure.message))?;
    Ok(OwnerSocketTransport {
        socket,
        next_op_id: 0,
    })
}

/// Render a typed exec-client error as a CliFailure carrying the CLI exec
/// exit-code contract. The wire `kind` slug + message + remediation are
/// redaction-safe (no argv/env/output bytes).
fn exec_error_to_failure(error: exec_client::ExecClientError) -> CliFailure {
    let message = if error.remediation.is_empty() {
        format!("vm exec: {}: {}", error.kind, error.message)
    } else {
        format!(
            "vm exec: {}: {} ({})",
            error.kind, error.message, error.remediation
        )
    };
    CliFailure::new(error.exit_code, message)
}

/// Terminate `vm exec` on a typed exec-client failure. For `--json`, emit the
/// single terminal JSON document on STDOUT and return the CLI exit code (so
/// nothing reaches stderr and there is exactly one JSON document on stdout).
/// For human runs, return the plain `CliFailure` rendered to stderr.
fn exec_terminate(
    args: &VmExecArgs,
    error: exec_client::ExecClientError,
) -> Result<i32, CliFailure> {
    if exec_effective_json(args) {
        let exit_code = error.exit_code;
        print_exec_json(&exec_json_failure_value(args, &error))?;
        Ok(exit_code)
    } else {
        Err(exec_error_to_failure(error))
    }
}

/// Terminate `vm exec` on a usage error (exit 2, `source: "cli"`). For `--json`
/// this still emits one terminal JSON document on STDOUT; otherwise it is
/// a plain stderr failure.
fn exec_usage_terminate(args: &VmExecArgs, message: impl Into<String>) -> Result<i32, CliFailure> {
    let message = message.into();
    if exec_effective_json(args) {
        let mut map = exec_json_base(args);
        map.insert("source".to_owned(), Value::String("cli".to_owned()));
        map.insert("reason".to_owned(), Value::String("usage".to_owned()));
        map.insert("exitCode".to_owned(), Value::from(2));
        map.insert("message".to_owned(), Value::String(message));
        print_exec_json(&Value::Object(map))?;
        Ok(2)
    } else {
        Err(CliFailure::new(2, message))
    }
}

#[derive(Debug)]
struct VmExecParsedAction {
    json: bool,
    management: Option<VmExecManagementCommand>,
}

fn exec_effective_json(args: &VmExecArgs) -> bool {
    args.json
        || args
            .management
            .iter()
            .any(|value| value.to_str() == Some("--json"))
}

fn parse_vm_exec_action(args: &VmExecArgs) -> Result<VmExecParsedAction, String> {
    let mut json = args.json;
    let mut human = args.human;
    let mut words = Vec::new();
    for value in &args.management {
        let Some(value) = value.to_str() else {
            return Err("vm exec: management arguments must be valid UTF-8".to_owned());
        };
        match value {
            "--json" => json = true,
            "--human" => human = true,
            other => words.push(other.to_owned()),
        }
    }
    if json && human {
        return Err("vm exec: --json cannot be combined with --human".to_owned());
    }
    if words.is_empty() {
        return Ok(VmExecParsedAction {
            json,
            management: None,
        });
    }

    let management = match words[0].as_str() {
        "list" => {
            if words.len() != 1 {
                return Err(
                    "vm exec list: expected no arguments after `list`; use `--` to run a command"
                        .to_owned(),
                );
            }
            VmExecManagementCommand::List
        }
        "status" => {
            if words.len() != 2 {
                return Err(
                    "vm exec status: expected exactly one detached exec id after `status`"
                        .to_owned(),
                );
            }
            VmExecManagementCommand::Status(VmExecIdArgs {
                exec_id: words[1].clone(),
            })
        }
        "kill" => {
            if words.len() != 2 {
                return Err(
                    "vm exec kill: expected exactly one detached exec id after `kill`".to_owned(),
                );
            }
            VmExecManagementCommand::Kill(VmExecIdArgs {
                exec_id: words[1].clone(),
            })
        }
        "logs" => VmExecManagementCommand::Logs(parse_vm_exec_logs_args(&words)?),
        _ => {
            return Err(
                "vm exec: use `--` to run a command, or choose management verb \
                 {list|logs|status|kill} after the VM name"
                    .to_owned(),
            )
        }
    };
    Ok(VmExecParsedAction {
        json,
        management: Some(management),
    })
}

fn parse_vm_exec_logs_args(words: &[String]) -> Result<VmExecLogsArgs, String> {
    if words.len() < 2 {
        return Err("vm exec logs: expected a detached exec id after `logs`".to_owned());
    }
    let mut logs = VmExecLogsArgs {
        exec_id: words[1].clone(),
        stdout_offset: None,
        stderr_offset: None,
        max_len: None,
    };
    let mut index = 2;
    while index < words.len() {
        let word = words[index].as_str();
        match word {
            "--stdout-offset" => {
                index += 1;
                let value = words.get(index).ok_or_else(|| {
                    "vm exec logs: --stdout-offset requires a byte offset".to_owned()
                })?;
                logs.stdout_offset = Some(parse_vm_exec_u64_flag("--stdout-offset", value)?);
            }
            "--stderr-offset" => {
                index += 1;
                let value = words.get(index).ok_or_else(|| {
                    "vm exec logs: --stderr-offset requires a byte offset".to_owned()
                })?;
                logs.stderr_offset = Some(parse_vm_exec_u64_flag("--stderr-offset", value)?);
            }
            "--max-len" => {
                index += 1;
                let value = words
                    .get(index)
                    .ok_or_else(|| "vm exec logs: --max-len requires a byte length".to_owned())?;
                logs.max_len = Some(parse_vm_exec_u64_flag("--max-len", value)?);
            }
            other if other.strip_prefix("--stdout-offset=").is_some() => {
                let value = other
                    .strip_prefix("--stdout-offset=")
                    .expect("prefix checked");
                logs.stdout_offset = Some(parse_vm_exec_u64_flag("--stdout-offset", value)?);
            }
            other if other.strip_prefix("--stderr-offset=").is_some() => {
                let value = other
                    .strip_prefix("--stderr-offset=")
                    .expect("prefix checked");
                logs.stderr_offset = Some(parse_vm_exec_u64_flag("--stderr-offset", value)?);
            }
            other if other.strip_prefix("--max-len=").is_some() => {
                let value = other.strip_prefix("--max-len=").expect("prefix checked");
                logs.max_len = Some(parse_vm_exec_u64_flag("--max-len", value)?);
            }
            other if other.starts_with('-') => {
                return Err(
                    "vm exec logs: unknown flag; expected --stdout-offset, --stderr-offset, or --max-len"
                        .to_owned(),
                );
            }
            _ => {
                return Err(
                    "vm exec logs: unexpected argument after log options; use `--` to run a command"
                        .to_owned(),
                );
            }
        }
        index += 1;
    }
    Ok(logs)
}

fn parse_vm_exec_u64_flag(flag: &str, value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("vm exec logs: {flag} must be a non-negative integer"))
}

/// Run a command inside a guest-control VM (FSM). Establishes the
/// daemon-held authenticated session over `public.sock` (admin-only), then
/// multiplexes stdin/stdout/stderr/signals over one owner connection. The
/// guest owns the PTY; the CLI only manages host terminal state.
fn cmd_vm_exec(context: &Context, args: &VmExecArgs) -> Result<i32, CliFailure> {
    use nixling_ipc::public_wire::{ExecEnvVar, ExecOp, ExecStartArgs, ExecTermSize};

    // 1. Validate flags BEFORE touching host terminal state or the daemon.
    let action = match parse_vm_exec_action(args) {
        Ok(action) => action,
        Err(message) => return exec_usage_terminate(args, message),
    };
    if let Some(management) = action.management.as_ref() {
        return cmd_vm_exec_management(context, args, management);
    }
    if args.detach && (args.tty || args.interactive) {
        return exec_usage_terminate(
            args,
            "vm exec: -d/--detach cannot be combined with -i/-t; detached exec has no attached terminal",
        );
    }
    //    `--json` is machine output: reject it together with ANY interactive /
    //    TTY mode (which streams raw bytes to stdout) before raw mode.
    if action.json && (args.tty || args.interactive) {
        return exec_usage_terminate(
            args,
            "vm exec: --json cannot be combined with -i/-t; an interactive \
             session streams raw output and is human-only",
        );
    }
    // guestd forwards guest stdin only in PTY mode: its non-TTY validators
    // reject an open stdin, so `-i`/`--interactive` without `-t`/`--tty`
    // would create a stdin-closed exec the CLI then tries to write to
    // (guestd rejects the writes as StdinClosed). Require a PTY for stdin
    // forwarding rather than fail deterministically once stdin is piped.
    if args.interactive && !args.tty {
        return exec_usage_terminate(
            args,
            "vm exec: -i/--interactive requires -t/--tty; the guest-control \
             transport forwards stdin only in PTY mode. Use `-it`, or drop \
             `-i` to run a stdin-closed command.",
        );
    }
    if args.command.is_empty() {
        return exec_usage_terminate(
            args,
            "vm exec: missing command; pass it after `--` (e.g. `nixling vm exec myvm -- ls`)",
        );
    }
    let tty = args.tty;
    let interactive = args.interactive || args.tty;

    let mut env_vars = Vec::with_capacity(args.env.len());
    for (idx, entry) in args.env.iter().enumerate() {
        // Redaction: never echo the raw --env entry — it may carry a
        // secret value (e.g. `TOKEN=...` or `=secret`). Report the 1-based
        // position only.
        let position = idx + 1;
        let Some((key, value)) = entry.split_once('=') else {
            return exec_usage_terminate(
                args,
                format!("vm exec: --env entry #{position} is not KEY=VALUE"),
            );
        };
        if key.is_empty() {
            return exec_usage_terminate(
                args,
                format!("vm exec: --env entry #{position} has an empty key (expected KEY=VALUE)"),
            );
        }
        env_vars.push(ExecEnvVar {
            key: key.to_owned(),
            value: value.to_owned(),
        });
    }

    if tty && !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
        return exec_usage_terminate(
            args,
            "vm exec: -t/--tty requires stdin and stdout to be a terminal",
        );
    }
    let term_size = if tty {
        exec_client::current_window_size().map(|(rows, cols)| ExecTermSize { rows, cols })
    } else {
        None
    };

    // 2. Connect + hello + Start (establish) BEFORE entering raw mode, so an
    //    establishment failure leaves the host terminal untouched. Every
    //    establishment failure is routed through `exec_terminate` so a `--json`
    //    run still emits exactly one terminal JSON document on stdout.
    let start_op = ExecOp::Start(ExecStartArgs {
        vm: args.vm.clone(),
        argv: args.command.clone(),
        tty,
        detached: args.detach,
        env: (!env_vars.is_empty()).then_some(env_vars),
        cwd: args.cwd.clone(),
        term_size,
    });
    let mut transport = match exec_owner_transport(context) {
        Ok(transport) => transport,
        Err(err) => return exec_terminate(args, err),
    };
    let start_response = match transport.round_trip(&start_op) {
        Ok(response) => response,
        Err(err) => {
            return exec_terminate(args, err);
        }
    };
    if args.detach {
        let create = match exec_client::expect_detached_create(start_response) {
            Ok(result) => result,
            Err(err) => return exec_terminate(args, err),
        };
        return exec_render_detached_create(args, &create);
    }
    let start_result = match exec_client::expect_start(start_response) {
        Ok(result) => result,
        Err(err) => {
            return exec_terminate(args, err);
        }
    };

    // 3. Enter host terminal state (raw mode for -t, non-blocking stdin for
    //    -i) + install the forwarded-signal source. The guard restores termios
    //    + O_NONBLOCK on EVERY return path below (including panics). `--json`
    //    rejects -i/-t up front, so this only runs for human sessions.
    let guard = if tty {
        match exec_client::FdStateGuard::enter(true, true) {
            Ok(guard) => Some(guard),
            Err(err) => {
                return exec_terminate(
                    args,
                    exec_client::ExecClientError::internal(format!(
                        "vm exec: failed to enter raw mode: {err}"
                    )),
                )
            }
        }
    } else if interactive {
        match exec_client::FdStateGuard::enter(false, true) {
            Ok(guard) => Some(guard),
            Err(err) => {
                return exec_terminate(
                    args,
                    exec_client::ExecClientError::internal(format!(
                        "vm exec: failed to set stdin non-blocking: {err}"
                    )),
                )
            }
        }
    } else {
        None
    };
    let mut signals = match exec_client::install_signals() {
        Ok(signals) => signals,
        Err(err) => {
            drop(guard);
            return exec_terminate(
                args,
                exec_client::ExecClientError::internal(format!(
                    "vm exec: failed to install signal handlers: {err}"
                )),
            );
        }
    };

    let config = exec_client::ExecFsmConfig {
        tty,
        interactive,
        poll_timeout_ms: if interactive { 40 } else { 200 },
        max_chunk: exec_client::EXEC_CLI_CHUNK_BYTES,
    };
    // 4. Drive the session to completion, then restore the terminal BEFORE any
    //    stdout emission (the --json envelope must not interleave raw output).
    if action.json {
        let mut host = exec_client::CapturingHostIo::new(interactive, 1024 * 1024);
        let result = exec_client::run_exec_fsm(
            &mut transport,
            &mut host,
            &mut signals,
            &start_result,
            &config,
        );
        drop(guard);
        match result {
            Ok(outcome) => exec_json_success(args, &outcome, &host),
            // Failure envelopes carry NO captured stdio bytes; they are
            // printed to stdout as the single terminal JSON document.
            Err(err) => exec_terminate(args, err),
        }
    } else {
        let mut host = exec_client::RealHostIo;
        let result = exec_client::run_exec_fsm(
            &mut transport,
            &mut host,
            &mut signals,
            &start_result,
            &config,
        );
        drop(guard);
        match result {
            Ok(outcome) => Ok(exec_client::exit_code_for_terminal(&outcome.terminal)),
            Err(err) => Err(exec_error_to_failure(err)),
        }
    }
}

fn cmd_vm_exec_management(
    context: &Context,
    args: &VmExecArgs,
    management: &VmExecManagementCommand,
) -> Result<i32, CliFailure> {
    use nixling_ipc::public_wire::{
        ExecDetachedKillArgs, ExecDetachedListArgs, ExecDetachedLogsArgs, ExecDetachedStatusArgs,
        ExecOp,
    };

    if args.detach
        || args.interactive
        || args.tty
        || !args.env.is_empty()
        || args.cwd.is_some()
        || !args.command.is_empty()
    {
        return exec_usage_terminate(
            args,
            "vm exec: detached management verbs do not accept -d/-i/-t, --env, --cwd, or a command; use `--` to run a command",
        );
    }

    match management {
        VmExecManagementCommand::List => {
            let response = match exec_send_one_op(
                context,
                ExecOp::List(ExecDetachedListArgs {
                    vm: args.vm.clone(),
                }),
            ) {
                Ok(response) => response,
                Err(err) => return exec_terminate(args, err),
            };
            let result = match exec_client::expect_detached_list(response) {
                Ok(result) => result,
                Err(err) => return exec_terminate(args, err),
            };
            exec_render_detached_list(args, &result)
        }
        VmExecManagementCommand::Logs(logs_args) => {
            let response = match exec_send_one_op(
                context,
                ExecOp::Logs(ExecDetachedLogsArgs {
                    vm: args.vm.clone(),
                    exec_id: logs_args.exec_id.clone(),
                    stdout_offset: logs_args.stdout_offset,
                    stderr_offset: logs_args.stderr_offset,
                    max_len: logs_args.max_len,
                }),
            ) {
                Ok(response) => response,
                Err(err) => return exec_terminate(args, err),
            };
            let result = match exec_client::expect_detached_logs(response) {
                Ok(result) => result,
                Err(err) => return exec_terminate(args, err),
            };
            exec_render_detached_logs(args, &result)
        }
        VmExecManagementCommand::Status(status_args) => {
            let response = match exec_send_one_op(
                context,
                ExecOp::Status(ExecDetachedStatusArgs {
                    vm: args.vm.clone(),
                    exec_id: status_args.exec_id.clone(),
                }),
            ) {
                Ok(response) => response,
                Err(err) => return exec_terminate(args, err),
            };
            let result = match exec_client::expect_detached_status(response) {
                Ok(result) => result,
                Err(err) => return exec_terminate(args, err),
            };
            exec_render_detached_status(args, &result)
        }
        VmExecManagementCommand::Kill(kill_args) => {
            let response = match exec_send_one_op(
                context,
                ExecOp::Kill(ExecDetachedKillArgs {
                    vm: args.vm.clone(),
                    exec_id: kill_args.exec_id.clone(),
                }),
            ) {
                Ok(response) => response,
                Err(err) => return exec_terminate(args, err),
            };
            let result = match exec_client::expect_detached_kill(response) {
                Ok(result) => result,
                Err(err) => return exec_terminate(args, err),
            };
            exec_render_detached_kill(args, &result)
        }
    }
}

fn exec_send_one_op(
    context: &Context,
    op: nixling_ipc::public_wire::ExecOp,
) -> Result<nixling_ipc::public_wire::ExecOpResponse, exec_client::ExecClientError> {
    let mut transport = exec_owner_transport(context)?;
    transport.round_trip(&op)
}

fn exec_render_detached_create(
    args: &VmExecArgs,
    result: &nixling_ipc::public_wire::ExecDetachedCreateResult,
) -> Result<i32, CliFailure> {
    if exec_effective_json(args) {
        exec_print_json(&VmExecCreateOutputV1 {
            command: "vm exec".to_owned(),
            vm: args.vm.clone(),
            exec_id: result.exec_id.clone(),
            state: result.state,
        })?;
    } else {
        print_stdout(&(result.exec_id.clone() + "\n"));
    }
    Ok(0)
}

fn exec_render_detached_list(
    args: &VmExecArgs,
    result: &nixling_ipc::public_wire::ExecDetachedListResult,
) -> Result<i32, CliFailure> {
    if exec_effective_json(args) {
        let execs = result
            .execs
            .iter()
            .map(|entry| VmExecListEntryOutputV1 {
                exec_id: entry.exec_id.clone(),
                state: entry.state,
                exit_code: entry.exit_code,
                signal: entry.signal,
                started_at: entry.started_at.clone(),
                start_offset: entry.start_offset,
                end_offset: entry.end_offset,
                stdout_start_offset: entry.stdout_start_offset,
                stdout_end_offset: entry.stdout_end_offset,
                stderr_start_offset: entry.stderr_start_offset,
                stderr_end_offset: entry.stderr_end_offset,
                dropped_bytes: entry.dropped_bytes,
                stdout_dropped_bytes: entry.stdout_dropped_bytes,
                stderr_dropped_bytes: entry.stderr_dropped_bytes,
                truncated: entry.truncated,
                stdout_truncated: entry.stdout_truncated,
                stderr_truncated: entry.stderr_truncated,
            })
            .collect();
        exec_print_json(&VmExecListOutputV1 {
            command: "vm exec list".to_owned(),
            vm: args.vm.clone(),
            execs,
        })?;
    } else {
        let mut rendered = String::new();
        let _ = writeln!(
            rendered,
            "{:<24} {:<22} {:<25} {:<14} {:<42} DROPPED/TRUNCATED",
            "EXEC ID", "STATE", "STARTED AT", "EXIT/SIGNAL", "OFFSETS"
        );
        for entry in &result.execs {
            let _ = writeln!(
                rendered,
                "{:<24} {:<22} {:<25} {:<14} {:<42} {}",
                entry.exec_id,
                exec_state_label(entry.state),
                entry.started_at,
                exec_terminal_summary(entry.exit_code, entry.signal, None),
                exec_list_offsets_summary(entry),
                exec_list_loss_summary(entry)
            );
        }
        print_stdout(&rendered);
    }
    Ok(0)
}

fn exec_render_detached_status(
    args: &VmExecArgs,
    result: &nixling_ipc::public_wire::ExecDetachedStatusResult,
) -> Result<i32, CliFailure> {
    if exec_effective_json(args) {
        exec_print_json(&VmExecStatusOutputV1 {
            command: "vm exec status".to_owned(),
            vm: args.vm.clone(),
            exec_id: result.exec_id.clone(),
            state: result.state,
            reason: result.reason.clone(),
            exit_code: result.exit_code,
            signal: result.signal,
            start_offset: result.start_offset,
            end_offset: result.end_offset,
            dropped_bytes: result.dropped_bytes,
            truncated: result.truncated,
        })?;
    } else {
        let mut rendered = String::new();
        let _ = writeln!(
            rendered,
            "{}: {}",
            result.exec_id,
            exec_state_label(result.state)
        );
        let _ = writeln!(
            rendered,
            "terminal: {}",
            exec_terminal_summary(result.exit_code, result.signal, result.reason.as_deref())
        );
        let _ = writeln!(
            rendered,
            "logs: startOffset={} endOffset={} droppedBytes={} truncated={}",
            result.start_offset, result.end_offset, result.dropped_bytes, result.truncated
        );
        print_stdout(&rendered);
    }
    Ok(0)
}

fn exec_render_detached_logs(
    args: &VmExecArgs,
    result: &nixling_ipc::public_wire::ExecDetachedLogsResult,
) -> Result<i32, CliFailure> {
    let (stdout, stderr) = match exec_decode_detached_logs(result) {
        Ok(decoded) => decoded,
        Err(err) => return exec_terminate(args, err),
    };
    if exec_effective_json(args) {
        exec_print_json(&VmExecLogsOutputV1 {
            command: "vm exec logs".to_owned(),
            vm: args.vm.clone(),
            exec_id: result.exec_id.clone(),
            stdout_base64: result.stdout_base64.clone(),
            stderr_base64: result.stderr_base64.clone(),
            start_offset: result.start_offset,
            end_offset: result.end_offset,
            dropped_bytes: result.dropped_bytes,
            truncated: result.truncated,
            stdout_start_offset: result.stdout_start_offset,
            stdout_end_offset: result.stdout_end_offset,
            stdout_next_offset: result.stdout_next_offset,
            stdout_eof: result.stdout_eof,
            stdout_dropped_bytes: result.stdout_dropped_bytes,
            stdout_truncated: result.stdout_truncated,
            stderr_start_offset: result.stderr_start_offset,
            stderr_end_offset: result.stderr_end_offset,
            stderr_next_offset: result.stderr_next_offset,
            stderr_eof: result.stderr_eof,
            stderr_dropped_bytes: result.stderr_dropped_bytes,
            stderr_truncated: result.stderr_truncated,
        })?;
        return Ok(0);
    }

    write_stdout_bytes(&stdout).map_err(|err| {
        CliFailure::new(1, format!("vm exec logs: failed to write stdout: {err}"))
    })?;
    write_stderr_bytes(&stderr).map_err(|err| {
        CliFailure::new(1, format!("vm exec logs: failed to write stderr: {err}"))
    })?;
    if exec_logs_incomplete(result) {
        if !stderr.is_empty() && !stderr.ends_with(b"\n") {
            write_stderr_bytes(b"\n").map_err(|err| {
                CliFailure::new(1, format!("vm exec logs: failed to write warning: {err}"))
            })?;
        }
        write_stderr_bytes(exec_logs_warning(result).as_bytes()).map_err(|err| {
            CliFailure::new(1, format!("vm exec logs: failed to write warning: {err}"))
        })?;
    }
    Ok(0)
}

fn exec_decode_detached_logs(
    result: &nixling_ipc::public_wire::ExecDetachedLogsResult,
) -> Result<(Vec<u8>, Vec<u8>), exec_client::ExecClientError> {
    let stdout = match nixling_core::base64_codec::decode(&result.stdout_base64) {
        Ok(bytes) => bytes,
        Err(_) => {
            return Err(exec_client::ExecClientError::protocol(
                "daemon returned malformed base64 for detached stdout",
            ));
        }
    };
    let stderr = match nixling_core::base64_codec::decode(&result.stderr_base64) {
        Ok(bytes) => bytes,
        Err(_) => {
            return Err(exec_client::ExecClientError::protocol(
                "daemon returned malformed base64 for detached stderr",
            ));
        }
    };
    Ok((stdout, stderr))
}

fn exec_render_detached_kill(
    args: &VmExecArgs,
    result: &nixling_ipc::public_wire::ExecDetachedKillResult,
) -> Result<i32, CliFailure> {
    let outcome = exec_kill_outcome_label(result.result);
    if exec_effective_json(args) {
        exec_print_json(&VmExecKillOutputV1 {
            command: "vm exec kill".to_owned(),
            vm: args.vm.clone(),
            exec_id: result.exec_id.clone(),
            result: result.result,
            state: result.state,
        })?;
    } else {
        print_stdout(&format!(
            "{}: {} (state={})\n",
            result.exec_id,
            outcome,
            exec_state_label(result.state)
        ));
    }
    Ok(0)
}

fn exec_print_json<T: Serialize>(value: &T) -> Result<(), CliFailure> {
    let value = serde_json::to_value(value)
        .map_err(|err| CliFailure::new(1, format!("vm exec: failed to serialize JSON: {err}")))?;
    print_exec_json(&value)
}

fn exec_state_label(state: nixling_ipc::guest_wire::ExecState) -> &'static str {
    use nixling_ipc::guest_wire::ExecState;

    match state {
        ExecState::Created => "created",
        ExecState::Running => "running",
        ExecState::Exited => "exited",
        ExecState::Signaled => "signaled",
        ExecState::Cancelled => "cancelled",
        ExecState::SlowConsumerCancelled => "slow-consumer-cancelled",
        ExecState::ProtocolError => "protocol-error",
        ExecState::LostGuestd => "lost-guestd",
        ExecState::Reaped => "reaped",
    }
}

fn exec_kill_outcome_label(
    outcome: nixling_ipc::public_wire::ExecDetachedKillOutcome,
) -> &'static str {
    use nixling_ipc::public_wire::ExecDetachedKillOutcome;

    match outcome {
        ExecDetachedKillOutcome::Cancelling => "cancelling",
        ExecDetachedKillOutcome::AlreadyTerminal => "already-terminal",
    }
}

fn exec_terminal_summary(
    exit_code: Option<i32>,
    signal: Option<u32>,
    reason: Option<&str>,
) -> String {
    if let Some(code) = exit_code {
        format!("exit={code}")
    } else if let Some(signal) = signal {
        format!("signal={signal}")
    } else if let Some(reason) = reason {
        reason.to_owned()
    } else {
        "-".to_owned()
    }
}

fn exec_loss_summary(dropped_bytes: u64, truncated: bool) -> String {
    format!(
        "{dropped_bytes}/{}",
        if truncated { "truncated" } else { "complete" }
    )
}

fn exec_list_offsets_summary(entry: &nixling_ipc::public_wire::ExecDetachedListEntry) -> String {
    format!(
        "all={}..{} stdout={}..{} stderr={}..{}",
        entry.start_offset,
        entry.end_offset,
        entry.stdout_start_offset,
        entry.stdout_end_offset,
        entry.stderr_start_offset,
        entry.stderr_end_offset
    )
}

fn exec_list_loss_summary(entry: &nixling_ipc::public_wire::ExecDetachedListEntry) -> String {
    format!(
        "all={} stdout={} stderr={}",
        exec_loss_summary(entry.dropped_bytes, entry.truncated),
        exec_loss_summary(entry.stdout_dropped_bytes, entry.stdout_truncated),
        exec_loss_summary(entry.stderr_dropped_bytes, entry.stderr_truncated)
    )
}

fn exec_logs_incomplete(result: &nixling_ipc::public_wire::ExecDetachedLogsResult) -> bool {
    result.dropped_bytes > 0
        || result.truncated
        || result.stdout_dropped_bytes > 0
        || result.stderr_dropped_bytes > 0
        || result.stdout_truncated
        || result.stderr_truncated
}

fn exec_logs_warning(result: &nixling_ipc::public_wire::ExecDetachedLogsResult) -> String {
    format!(
        "nixling: vm exec logs: retained output incomplete (startOffset={} endOffset={} droppedBytes={} truncated={} stdoutStartOffset={} stdoutEndOffset={} stdoutNextOffset={} stdoutEof={} stdoutDroppedBytes={} stdoutTruncated={} stderrStartOffset={} stderrEndOffset={} stderrNextOffset={} stderrEof={} stderrDroppedBytes={} stderrTruncated={})\n",
        result.start_offset,
        result.end_offset,
        result.dropped_bytes,
        result.truncated,
        result.stdout_start_offset,
        result.stdout_end_offset,
        result.stdout_next_offset,
        result.stdout_eof,
        result.stdout_dropped_bytes,
        result.stdout_truncated,
        result.stderr_start_offset,
        result.stderr_end_offset,
        result.stderr_next_offset,
        result.stderr_eof,
        result.stderr_dropped_bytes,
        result.stderr_truncated
    )
}

/// Build the terminal `--json` envelope fields shared by success and failure.
fn exec_json_base(args: &VmExecArgs) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    map.insert("command".to_owned(), Value::String("vm exec".to_owned()));
    map.insert("vm".to_owned(), Value::String(args.vm.clone()));
    map
}

/// Append the bounded, charset-safe captured guest output to a JSON envelope.
fn exec_json_attach_output(
    map: &mut serde_json::Map<String, Value>,
    host: &exec_client::CapturingHostIo,
) {
    map.insert(
        "stdoutBase64".to_owned(),
        Value::String(nixling_core::base64_codec::encode(host.stdout())),
    );
    map.insert(
        "stderrBase64".to_owned(),
        Value::String(nixling_core::base64_codec::encode(host.stderr())),
    );
    map.insert(
        "stdoutTruncated".to_owned(),
        Value::Bool(host.stdout_truncated()),
    );
    map.insert(
        "stderrTruncated".to_owned(),
        Value::Bool(host.stderr_truncated()),
    );
}

/// Build the success `--json` envelope value + CLI exit code. `source` is
/// always `guest`; `guestExitCode`/`signal` disambiguate a code that collides
/// with a reserved transport code. The FSM resolves only true guest
/// `WIFEXITED`/`WIFSIGNALED` terminals as a success; abnormal terminal
/// kinds surface through [`exec_terminate`] as transport/protocol failures.
fn exec_json_success_value(
    args: &VmExecArgs,
    outcome: &exec_client::ExecOutcome,
    host: &exec_client::CapturingHostIo,
) -> (Value, i32) {
    use nixling_ipc::public_wire::ExecTerminalStatus;

    let exit_code = exec_client::exit_code_for_terminal(&outcome.terminal);
    let mut map = exec_json_base(args);
    map.insert("source".to_owned(), Value::String("guest".to_owned()));
    map.insert("exitCode".to_owned(), Value::from(exit_code));
    match &outcome.terminal {
        ExecTerminalStatus::Exited { code } => {
            map.insert("reason".to_owned(), Value::String("exited".to_owned()));
            map.insert("guestExitCode".to_owned(), Value::from(*code));
        }
        ExecTerminalStatus::Signaled { signal } => {
            map.insert("reason".to_owned(), Value::String("signaled".to_owned()));
            map.insert("signal".to_owned(), Value::from(*signal));
        }
        // Defensive: the FSM never resolves an abnormal terminal as a success.
        ExecTerminalStatus::Error { slug: _ } => {
            map.insert("reason".to_owned(), Value::String("abnormal".to_owned()));
        }
    }
    exec_json_attach_output(&mut map, host);
    (Value::Object(map), exit_code)
}

/// Emit the success `--json` envelope and return the CLI exit code.
fn exec_json_success(
    args: &VmExecArgs,
    outcome: &exec_client::ExecOutcome,
    host: &exec_client::CapturingHostIo,
) -> Result<i32, CliFailure> {
    let (value, exit_code) = exec_json_success_value(args, outcome, host);
    print_exec_json(&value)?;
    Ok(exit_code)
}

/// Build the failure `--json` envelope value. Transport/protocol/internal
/// failures carry `transportExitCode` + a non-`guest` `source`. A failure
/// envelope NEVER carries captured stdio bytes.
fn exec_json_failure_value(args: &VmExecArgs, error: &exec_client::ExecClientError) -> Value {
    let mut map = exec_json_base(args);
    map.insert(
        "source".to_owned(),
        Value::String(error.source.as_str().to_owned()),
    );
    map.insert("reason".to_owned(), Value::String(error.kind.clone()));
    map.insert("exitCode".to_owned(), Value::from(error.exit_code));
    map.insert("transportExitCode".to_owned(), Value::from(error.exit_code));
    map.insert("message".to_owned(), Value::String(error.message.clone()));
    if !error.remediation.is_empty() {
        map.insert(
            "remediation".to_owned(),
            Value::String(error.remediation.clone()),
        );
    }
    Value::Object(map)
}

/// Print a single pretty JSON document to stdout with a trailing newline.
fn print_exec_json(value: &Value) -> Result<(), CliFailure> {
    let mut rendered = serde_json::to_string_pretty(value)
        .map_err(|err| CliFailure::new(1, format!("vm exec: failed to serialize JSON: {err}")))?;
    rendered.push('\n');
    print_stdout(&rendered);
    Ok(())
}

// ---- store-lifecycle CLI verbs ----

fn w7_dry_run_summary(verb: &str, vm: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "command": verb,
        "mode": "dry-run",
        "vm": vm,
        "planned": [],
        "notes": format!("nixling {verb} --dry-run reports the planned operation; --apply routes through nixlingd → broker."),
    })
}

fn cmd_build(context: &Context, args: &BuildArgs) -> Result<i32, CliFailure> {
    // build is non-destructive — always allowed; never returns
    // daemon-down. The non-destructive scope (build / generations
    // / richer status) ships dry-run-shaped output today even
    // without --dry-run.
    require_known_vm(context, &args.vm, args.json)?;
    let summary = serde_json::json!({
        "command": "build",
        "vm": args.vm,
        "planned": {
            "drv_path": format!("/nix/store/<placeholder>-nixos-system-{}.drv", args.vm),
            "out_path": format!("/nix/store/<placeholder>-nixos-system-{}", args.vm),
        },
        "notes": "build evaluates and builds the per-VM toplevel only; hardlink-farm materialization happens on activation and gc paths.",
    });
    if args.json {
        let mut rendered = serde_json::to_string_pretty(&summary)
            .map_err(|e| CliFailure::new(1, format!("serialize: {e}")))?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        print_stdout(&format!(
            "nixling build {}: would evaluate and build the toplevel (hardlink-farm materialization happens on activation/gc)\n",
            args.vm
        ));
    }
    Ok(0)
}

fn cmd_generations(context: &Context, args: &GenerationsArgs) -> Result<i32, CliFailure> {
    require_known_vm(context, &args.vm, args.json)?;
    let manifest = context.load_manifest()?;
    let vm = manifest
        .vms()
        .into_iter()
        .find(|v| v.name == args.vm)
        .ok_or_else(|| CliFailure::new(70, format!("unknown vm: {}", args.vm)))?;
    let current = current_symlink(context, vm);
    let booted = booted_symlink(context, vm);
    let summary = serde_json::json!({
        "command": "generations",
        "vm": args.vm,
        "current": current,
        "booted": booted,
        "entries": [],
        "notes": "generations currently reports the current/booted symlink targets only; full on-disk generation enumeration is not exposed on this surface yet.",
    });
    if args.json {
        let mut rendered = serde_json::to_string_pretty(&summary)
            .map_err(|e| CliFailure::new(1, format!("serialize: {e}")))?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        print_stdout(&format!(
            "nixling generations {}: current={}  booted={}\n",
            args.vm,
            current.as_deref().unwrap_or("<none>"),
            booted.as_deref().unwrap_or("<none>"),
        ));
    }
    Ok(0)
}

fn w7_mutating_verb(
    context: &Context,
    verb: &str,
    vm: &str,
    dry_run: bool,
    apply: bool,
    json: bool,
    _original_args: &[OsString],
) -> Result<i32, CliFailure> {
    let flags = require_mutation_flag(verb, dry_run, apply, json)?;
    require_known_vm(context, vm, json)?;
    // `switch`/`boot`/`test` build + activate from the host-side
    // guestConfigFile; warn if a synced edit is staged-but-unapproved so
    // the operator doesn't silently activate the old config.
    if matches!(verb, "switch" | "boot" | "test") && !json {
        warn_pending_staged_config(vm);
    }
    if flags.apply {
        // Daemon-first dispatch is live for activation verbs.
        // The CLI only reaches the legacy bash surface when the daemon
        // explicitly defers or is unavailable.
        return dispatch_mutating_verb(
            context,
            verb,
            serde_json::json!({ "vm": vm }),
            flags.dry_run,
            flags.apply,
            json,
        );
    }
    let summary = w7_dry_run_summary(verb, Some(vm));
    if json {
        let mut rendered = serde_json::to_string_pretty(&summary)
            .map_err(|e| CliFailure::new(1, format!("serialize: {e}")))?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        print_stdout(&format!(
            "nixling {verb} --dry-run: would activate the planned generation for vm '{vm}'\n"
        ));
    }
    Ok(0)
}

fn cmd_switch(
    context: &Context,
    args: &SwitchArgs,
    original_args: &[OsString],
) -> Result<i32, CliFailure> {
    w7_mutating_verb(
        context,
        "switch",
        &args.vm,
        args.dry_run,
        args.apply,
        args.json,
        original_args,
    )
}

fn cmd_boot(
    context: &Context,
    args: &BootArgs,
    original_args: &[OsString],
) -> Result<i32, CliFailure> {
    w7_mutating_verb(
        context,
        "boot",
        &args.vm,
        args.dry_run,
        args.apply,
        args.json,
        original_args,
    )
}

fn cmd_test(
    context: &Context,
    args: &TestArgs,
    original_args: &[OsString],
) -> Result<i32, CliFailure> {
    w7_mutating_verb(
        context,
        "test",
        &args.vm,
        args.dry_run,
        args.apply,
        args.json,
        original_args,
    )
}

fn cmd_rollback(
    context: &Context,
    args: &RollbackArgs,
    original_args: &[OsString],
) -> Result<i32, CliFailure> {
    w7_mutating_verb(
        context,
        "rollback",
        &args.vm,
        args.dry_run,
        args.apply,
        args.json,
        original_args,
    )
}

fn cmd_gc(
    context: &Context,
    args: &GcArgs,
    _original_args: &[OsString],
) -> Result<i32, CliFailure> {
    let flags = require_mutation_flag("gc", args.dry_run, args.apply, args.json)?;
    if flags.apply {
        // v1.0 daemon-only: --apply routes through nixlingd → broker
        // (ADR 0015). The historical bash fallback was retired in v1.0;
        // daemon-unreachable + native-handler-deferred surface typed
        // envelopes (exit-1 / exit-78).
        return dispatch_mutating_verb(
            context,
            "gc",
            serde_json::json!({}),
            flags.dry_run,
            flags.apply,
            args.json,
        );
    }
    let summary = w7_dry_run_summary("gc", None);
    if args.json {
        let mut rendered = serde_json::to_string_pretty(&summary)
            .map_err(|e| CliFailure::new(1, format!("serialize: {e}")))?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        print_stdout("nixling gc --dry-run: would prune unreachable store paths in /var/lib/nixling/vms/<vm>/store/\n");
    }
    Ok(0)
}

fn cmd_store_verify(context: &Context, args: &StoreVerifyArgs) -> Result<i32, CliFailure> {
    let json_mode = if args.human { false } else { args.json };
    let manifest = context.load_manifest()?;
    if !manifest.vms().iter().any(|vm| vm.name == args.vm) {
        let response = IpcStoreVerifyResponse {
            vm: args.vm.clone(),
            status: IpcStoreVerifyStatus::NotFound,
            checked: 0,
            drifted: 0,
            repaired: 0,
            unknown_reason: None,
            audit_ref: None,
            remediation: Some("check the VM name, declaration, and authorization".to_owned()),
        };
        if json_mode {
            let envelope = store_verify_cli_envelope(&response);
            print_json(&envelope)?;
        } else {
            print_stdout(&render_store_verify_human(&response));
        }
        return Ok(70);
    }
    let response = match try_store_verify_via_socket(context, &args.vm, args.repair)? {
        StoreVerifySocketOutcome::Response(response) => response,
        StoreVerifySocketOutcome::Unavailable => {
            return emit_host_error(&daemon_down_envelope("store verify"), json_mode);
        }
    };
    if json_mode {
        let envelope = store_verify_cli_envelope(&response);
        print_json(&envelope)?;
    } else {
        print_stdout(&render_store_verify_human(&response));
    }
    Ok(store_verify_exit_code(response.status))
}

fn store_verify_exit_code(status: IpcStoreVerifyStatus) -> i32 {
    match status {
        IpcStoreVerifyStatus::Ok | IpcStoreVerifyStatus::Repaired => 0,
        IpcStoreVerifyStatus::Drift | IpcStoreVerifyStatus::Unknown => 4,
        IpcStoreVerifyStatus::NotFound => 70,
        IpcStoreVerifyStatus::Failed => 78,
    }
}

fn store_verify_cli_envelope(response: &IpcStoreVerifyResponse) -> StoreVerifyOutputV2 {
    StoreVerifyOutputV2 {
        vm: response.vm.clone(),
        status: response.status,
        checked: response.checked,
        drifted: response.drifted,
        repaired: response.repaired,
        unknown_reason: response
            .unknown_reason
            .map(|reason| serde_json::to_value(reason).unwrap_or(Value::Null))
            .and_then(|value| value.as_str().map(str::to_owned)),
        audit_ref: response.audit_ref.clone(),
        remediation: response.remediation.clone(),
    }
}

fn render_store_verify_human(response: &IpcStoreVerifyResponse) -> String {
    let status = serde_json::to_value(response.status)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "failed".to_owned());
    let mut out = format!(
        "store verify {}: status={status} checked={} drifted={} repaired={}\n",
        response.vm, response.checked, response.drifted, response.repaired
    );
    if let Some(reason) = response.unknown_reason {
        let reason = serde_json::to_value(reason)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| "unknown".to_owned());
        let _ = writeln!(out, "unknown_reason={reason}");
    }
    if let Some(remediation) = &response.remediation {
        let _ = writeln!(out, "remediation={remediation}");
    }
    out
}

// ---- native usb CLI ----

fn usb_json_mode(json: bool, human: bool) -> bool {
    if human {
        false
    } else {
        json
    }
}

fn cmd_usb_attach(context: &Context, args: &UsbAttachArgs) -> Result<i32, CliFailure> {
    usb_mutating_verb(
        context,
        "usb attach",
        "usbipBind",
        &args.vm,
        &args.busid,
        args.dry_run,
        args.apply,
        args.json,
        args.human,
    )
}

fn cmd_usb_detach(context: &Context, args: &UsbDetachArgs) -> Result<i32, CliFailure> {
    usb_mutating_verb(
        context,
        "usb detach",
        "usbipUnbind",
        &args.vm,
        &args.busid,
        args.dry_run,
        args.apply,
        args.json,
        args.human,
    )
}

#[allow(clippy::too_many_arguments)]
fn usb_mutating_verb(
    context: &Context,
    verb: &str,
    request_type: &str,
    vm: &str,
    bus_id: &str,
    dry_run: bool,
    apply: bool,
    json: bool,
    human: bool,
) -> Result<i32, CliFailure> {
    let json_mode = usb_json_mode(json, human);
    let flags = require_mutation_flag(verb, dry_run, apply, json_mode)?;
    require_known_vm(context, vm, json_mode)?;
    if flags.apply {
        if verb == "usb attach" {
            let guest_plan = usb_guest_attach_plan(context, vm, bus_id, json_mode)?;
            let outcome = try_daemon_mutating_verb(
                context,
                request_type,
                serde_json::json!({
                    "vm": vm,
                    "busId": bus_id,
                }),
                flags.dry_run,
                flags.apply,
                json_mode,
            )?;
            match outcome {
                DaemonVerbOutcome::Applied { summary } => {
                    run_usb_guest_attach(&guest_plan, json_mode)?;
                    print_stdout(&format!("{summary}\n"));
                    if !json_mode {
                        print_stdout(&format!(
                            "nixling usb attach --apply: imported busid '{}' inside vm '{}'\n",
                            guest_plan.bus_id, guest_plan.vm_name
                        ));
                    }
                    return Ok(0);
                }
                other => return emit_daemon_mutating_outcome(other, json_mode),
            }
        }
        return dispatch_mutating_verb(
            context,
            request_type,
            serde_json::json!({
                "vm": vm,
                "busId": bus_id,
            }),
            flags.dry_run,
            flags.apply,
            json_mode,
        );
    }
    let planned: Vec<&str> = if verb == "usb attach" {
        vec![
            "UsbipBind",
            "UsbipBindFirewallRule",
            "SpawnRunner(sys-<env>-usbipd/backend)",
            "SpawnRunner(sys-<env>-usbipd/proxy)",
            "UsbipProxyReconcile",
            "GuestUsbipAttach(ssh sudo -n usbip attach)",
        ]
    } else {
        vec!["UsbipUnbind", "UsbipProxyReconcile"]
    };
    let summary = serde_json::json!({
        "command": verb,
        "mode": "dry-run",
        "vm": vm,
        "busId": bus_id,
        "planned": planned,
        "notes": if verb == "usb attach" {
            "USBIP dry-run reports the daemon → broker bind/lock, firewall, backend/proxy ensurement, reconcile plan, and guest import without mutating host or guest state."
        } else {
            "USBIP dry-run reports the daemon → broker unbind and reconcile plan without mutating host state."
        },
    });
    if json_mode {
        let mut rendered = serde_json::to_string_pretty(&summary)
            .map_err(|e| CliFailure::new(1, format!("serialize: {e}")))?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        let action = if verb == "usb attach" {
            "bind and lock, apply the USBIP firewall carve-out, ensure the per-env backend/proxy for"
        } else {
            "unbind"
        };
        if verb == "usb attach" {
            print_stdout(&format!(
                "nixling {verb} --dry-run: would {action} busid '{bus_id}' for vm '{vm}', reconcile the USBIP proxy, and SSH into the guest to run sudo -n usbip attach\n"
            ));
        } else {
            print_stdout(&format!(
                "nixling {verb} --dry-run: would {action} busid '{bus_id}' for vm '{vm}', and reconcile the USBIP proxy\n"
            ));
        }
    }
    Ok(0)
}

#[derive(Debug, Clone)]
struct UsbGuestAttachPlan {
    vm_name: String,
    bus_id: String,
    host: String,
    user: String,
    usbip_host: String,
    key_path: PathBuf,
    known_hosts: PathBuf,
}

fn usb_guest_attach_ssh_argv(
    key_path: &Path,
    known_hosts: &Path,
    remote: &str,
    usbip_host: &str,
    bus_id: &str,
) -> Vec<String> {
    // nixling-ssh-allowlist begin: usb-connect guest attach convenience
    vec![
        "ssh".to_owned(),
        "-i".to_owned(),
        key_path.display().to_string(),
        "-o".to_owned(),
        format!("UserKnownHostsFile={}", known_hosts.display()),
        "-o".to_owned(),
        "BatchMode=yes".to_owned(),
        "-o".to_owned(),
        "ConnectTimeout=8".to_owned(),
        "-o".to_owned(),
        "StrictHostKeyChecking=accept-new".to_owned(),
        remote.to_owned(),
        "sudo".to_owned(),
        "-n".to_owned(),
        "usbip".to_owned(),
        "attach".to_owned(),
        "-r".to_owned(),
        usbip_host.to_owned(),
        "-b".to_owned(),
        bus_id.to_owned(),
    ]
    // nixling-ssh-allowlist end
}

fn usb_attach_cli_failure(
    kind: &str,
    code: &str,
    what_was_checked: &str,
    observed_state: &str,
    remediation: &str,
    is_json: bool,
) -> CliFailure {
    let envelope = host_error_envelope(
        kind,
        code,
        1,
        what_was_checked,
        observed_state,
        remediation,
        "docs/reference/components-usbip.md#common-gotchas--failure-modes",
    );
    let rendered_stderr = if is_json {
        let mut rendered =
            serde_json::to_string_pretty(&envelope).expect("serialize usb attach failure envelope");
        rendered.push('\n');
        rendered
    } else {
        format!(
            "nixling: {} (code: {}, exit {})\n  what was checked : {}\n  observed         : {}\n  remediation      : {}\n  docs             : {}\n",
            envelope.kind,
            envelope.code,
            envelope.exit_code,
            envelope.what_was_checked,
            envelope.observed_state,
            envelope.remediation,
            envelope.docs_anchor,
        )
    };
    CliFailure {
        exit_code: envelope.exit_code,
        message: envelope.kind,
        rendered_stderr: Some(rendered_stderr),
    }
}

fn usb_resolve_bundle_key_path(
    bundle_path: &Path,
    vm_name: &str,
    is_json: bool,
) -> Result<Option<PathBuf>, CliFailure> {
    match bundle_path.try_exists() {
        Ok(true) => {
            let bundle: Bundle = match read_json_file(bundle_path) {
                Ok(bundle) => bundle,
                Err(err) if err.kind() == io::ErrorKind::PermissionDenied => return Ok(None),
                Err(err) => {
                    return Err(usb_attach_cli_failure(
                        "nixling usb attach --apply failed to read the trusted bundle",
                        "usb-guest-import-prerequisite",
                        "Whether the CLI can resolve the framework-managed VM SSH key before mutating host USBIP state.",
                        &format!("Failed to read bundle {}: {err}", bundle_path.display()),
                        "Rebuild the host so the bundle is readable to the launcher, or ensure the framework-managed key exists at /var/lib/nixling/keys/<vm>_ed25519.",
                        is_json,
                    ));
                }
            };
            Ok(Some(bundle.managed_keys.effective_key_path(vm_name)))
        }
        Ok(false) => Ok(None),
        Err(err) if err.kind() == io::ErrorKind::PermissionDenied => Ok(None),
        Err(err) => Err(usb_attach_cli_failure(
            "nixling usb attach --apply failed to inspect the trusted bundle",
            "usb-guest-import-prerequisite",
            "Whether the CLI can resolve the framework-managed VM SSH key before mutating host USBIP state.",
            &format!("Failed to inspect bundle {}: {err}", bundle_path.display()),
            "Fix the bundle path or filesystem permissions, then retry `nixling usb attach`.",
            is_json,
        )),
    }
}

fn usb_validate_key_exists(key_path: &Path, is_json: bool) -> Result<(), CliFailure> {
    match key_path.try_exists() {
        Ok(true) => Ok(()),
        Ok(false) => Err(usb_attach_cli_failure(
            "nixling usb attach --apply cannot find the VM SSH key",
            "usb-guest-import-prerequisite",
            "Whether the framework-managed VM SSH key exists before mutating host USBIP state.",
            &format!("SSH key not found at {}", key_path.display()),
            "Rebuild the host or run `nixling keys rotate <vm>` for the target VM, then retry `nixling usb attach`.",
            is_json,
        )),
        Err(err) => {
            let remediation = if err.kind() == io::ErrorKind::PermissionDenied {
                "Verify your shell session is a member of the `nixling` group (`id -nG | tr ' ' '\\n' | grep -x nixling`) and that /var/lib/nixling grants launcher traversal, then retry `nixling usb attach`."
            } else {
                "Fix the SSH key path or filesystem error, then retry `nixling usb attach`."
            };
            Err(usb_attach_cli_failure(
                "nixling usb attach --apply cannot access the VM SSH key",
                "usb-guest-import-prerequisite",
                "Whether the framework-managed VM SSH key is accessible before mutating host USBIP state.",
                &format!("Failed to inspect SSH key {}: {err}", key_path.display()),
                remediation,
                is_json,
            ))
        }
    }
}

fn usb_guest_attach_plan(
    context: &Context,
    vm_name: &str,
    bus_id: &str,
    is_json: bool,
) -> Result<UsbGuestAttachPlan, CliFailure> {
    let manifest = context.load_manifest()?;
    let vm = manifest.entries.get(vm_name).ok_or_else(|| {
        usb_attach_cli_failure(
            "nixling usb attach --apply target VM is not in the manifest",
            "usage",
            "Whether the requested VM exists in the active manifest before mutating host USBIP state.",
            &format!("Unknown VM '{vm_name}' in manifest"),
            "Run `nixling list` and retry with a declared VM name.",
            is_json,
        )
    })?;
    let host = vm.static_ip.clone().ok_or_else(|| {
        usb_attach_cli_failure(
            "nixling usb attach --apply target VM has no static IP",
            "usb-guest-import-prerequisite",
            "Whether the target VM has guest SSH metadata before mutating host USBIP state.",
            &format!("VM '{vm_name}' has no staticIp in the manifest"),
            "Start from a VM that belongs to a nixling env and has a generated static IP, then retry `nixling usb attach`.",
            is_json,
        )
    })?;
    let user = vm.ssh_user.clone().ok_or_else(|| {
        usb_attach_cli_failure(
            "nixling usb attach --apply target VM has no SSH user",
            "usb-guest-import-prerequisite",
            "Whether the target VM has guest SSH metadata before mutating host USBIP state.",
            &format!("VM '{vm_name}' has no sshUser in the manifest"),
            "Set `nixling.vms.<vm>.ssh.user`, rebuild the host, and retry `nixling usb attach`.",
            is_json,
        )
    })?;
    let usbip_host = vm.usbipd_host_ip.clone().ok_or_else(|| {
        usb_attach_cli_failure(
            "nixling usb attach --apply target VM has no USBIP proxy IP",
            "usb-guest-import-prerequisite",
            "Whether the target VM has per-env USBIP proxy metadata before mutating host USBIP state.",
            &format!("VM '{vm_name}' has no usbipdHostIp in the manifest"),
            "Rebuild the host with a current nixling module so the manifest includes usbipdHostIp, then retry `nixling usb attach`.",
            is_json,
        )
    })?;
    let key_path = usb_resolve_bundle_key_path(&context.bundle_path, vm_name, is_json)?
        .unwrap_or_else(|| PathBuf::from(format!("/var/lib/nixling/keys/{vm_name}_ed25519")));
    usb_validate_key_exists(&key_path, is_json)?;

    Ok(UsbGuestAttachPlan {
        vm_name: vm_name.to_owned(),
        bus_id: bus_id.to_owned(),
        host,
        user,
        usbip_host,
        key_path,
        known_hosts: PathBuf::from("/var/lib/nixling/known_hosts.nixling"),
    })
}

fn run_usb_guest_attach(plan: &UsbGuestAttachPlan, is_json: bool) -> Result<(), CliFailure> {
    let remote = format!("{}@{}", plan.user, plan.host);
    let argv = usb_guest_attach_ssh_argv(
        &plan.key_path,
        &plan.known_hosts,
        &remote,
        &plan.usbip_host,
        &plan.bus_id,
    );
    let status = Command::new(&argv[0])
        .args(&argv[1..])
        .status()
        .map_err(|err| {
            usb_attach_cli_failure(
                "nixling usb attach --apply failed to SSH into the guest",
                "usb-guest-import-failed",
                "Guest-side USBIP import after the host-side daemon → broker attach succeeded.",
                &format!("SSH guest import failed for vm '{}': {err}", plan.vm_name),
                &format!("The host-side USBIP attach may already be active. Check VM reachability and guest sudo/usbip availability, then run `nixling usb detach {} {} --apply` before retrying if needed.", plan.vm_name, plan.bus_id),
                is_json,
            )
        })?;
    if !status.success() {
        return Err(usb_attach_cli_failure(
            "nixling usb attach --apply guest import command failed",
            "usb-guest-import-failed",
            "Guest-side `sudo -n usbip attach` after the host-side daemon → broker attach succeeded.",
            &format!(
                "Guest import in vm '{}' exited {}",
                plan.vm_name,
                status.code().unwrap_or(-1)
            ),
            &format!("The host-side USBIP attach may already be active. Check guest sudo permissions and `usbip`/`vhci_hcd`, then run `nixling usb detach {} {} --apply` before retrying if needed.", plan.vm_name, plan.bus_id),
            is_json,
        ));
    }
    Ok(())
}

fn cmd_usb_probe(context: &Context, args: &UsbProbeArgs) -> Result<i32, CliFailure> {
    let json_mode = usb_json_mode(args.json, args.human);
    match try_usb_probe_via_socket(context)? {
        UsbProbeSocketOutcome::Entries(entries) => {
            if json_mode {
                let body = serde_json::json!({
                    "command": "usb probe",
                    "entries": entries,
                });
                let mut rendered = serde_json::to_string_pretty(&body)
                    .map_err(|e| CliFailure::new(1, format!("serialize: {e}")))?;
                rendered.push('\n');
                print_stdout(&rendered);
            } else {
                print_stdout(&render_usb_probe_human(&entries));
            }
            Ok(0)
        }
        UsbProbeSocketOutcome::Unavailable => emit_host_error(
            &host_error_envelope(
                "USBIP probe requires a reachable nixlingd",
                "daemon-down",
                1,
                "Daemon connectivity at /run/nixling/public.sock and USBIP probe support.",
                "nixlingd is unreachable or does not expose the native USBIP probe request.",
                "Start nixlingd on the host, then re-run `nixling usb probe`.",
                "docs/reference/error-codes.md#daemon-down",
            ),
            json_mode,
        ),
    }
}

fn render_usb_probe_human(entries: &[IpcUsbipProbeEntry]) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<24} {:<12} {:<12} {:<8} OWNER",
        "VM", "ENV", "BUSID", "STATUS"
    );
    for entry in entries {
        let status = match entry.status {
            IpcUsbipProbeStatus::Bound => "bound",
            IpcUsbipProbeStatus::Unbound => "unbound",
        };
        let _ = writeln!(
            out,
            "{:<24} {:<12} {:<12} {:<8} {}",
            entry.vm,
            entry.env,
            entry.bus_id,
            status,
            entry.owner_vm.as_deref().unwrap_or("-"),
        );
    }
    out
}

// ---- managed-keys + trust verbs ----

fn cmd_keys_list(
    context: &Context,
    args: &KeysListArgs,
    _original_args: &[OsString],
) -> Result<i32, CliFailure> {
    let json_mode = if args.human { false } else { args.json };
    match try_keys_list_via_socket(context)? {
        KeysSocketOutcome::List(entries) => {
            if json_mode {
                let body = serde_json::json!({
                    "command": "keys list",
                    "entries": entries,
                });
                let mut rendered = serde_json::to_string_pretty(&body)
                    .map_err(|e| CliFailure::new(1, format!("serialize: {e}")))?;
                rendered.push('\n');
                print_stdout(&rendered);
            } else {
                print_stdout(&render_keys_list_human(&entries));
            }
            Ok(0)
        }
        KeysSocketOutcome::Unavailable => {
            emit_host_error(&daemon_down_envelope("keys list"), json_mode)
        }
        KeysSocketOutcome::Show(_) => Err(CliFailure::new(
            1,
            "internal keysList/keysShow response mismatch".to_owned(),
        )),
    }
}

fn render_keys_list_human(entries: &[IpcKeyEntry]) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<24} {:<12} {:<64} MANAGED KEY",
        "VM", "ENV", "FINGERPRINT"
    );
    for entry in entries {
        let _ = writeln!(
            out,
            "{:<24} {:<12} {:<64} {}",
            entry.vm,
            entry.env.as_deref().unwrap_or("-"),
            entry.fingerprint,
            entry.managed_key_path,
        );
    }
    out
}

fn cmd_keys_show(
    context: &Context,
    args: &KeysShowArgs,
    original_args: &[OsString],
) -> Result<i32, CliFailure> {
    let json_mode = if args.human { false } else { args.json };
    require_known_vm(context, &args.vm, json_mode)?;
    match try_keys_show_via_socket(context, &args.vm)? {
        KeysSocketOutcome::Show(response) => {
            if json_mode {
                let body = serde_json::json!({
                    "command": "keys show",
                    "vm": response.vm,
                    "env": response.env,
                    "managedKeyPath": response.managed_key_path,
                    "publicKey": response.public_key,
                    "fingerprint": response.fingerprint,
                    "knownHostsEntry": response.known_hosts_entry,
                });
                let mut rendered = serde_json::to_string_pretty(&body)
                    .map_err(|e| CliFailure::new(1, format!("serialize: {e}")))?;
                rendered.push('\n');
                print_stdout(&rendered);
            } else {
                print_stdout(&format!("{}\n", response.public_key));
            }
            Ok(0)
        }
        KeysSocketOutcome::Unavailable => {
            let _ = original_args;
            emit_host_error(&daemon_down_envelope("keys show"), json_mode)
        }
        KeysSocketOutcome::List(_) => Err(CliFailure::new(
            1,
            "internal keysShow/keysList response mismatch".to_owned(),
        )),
    }
}

fn w8_mutating_verb(
    context: &Context,
    verb: &str,
    vm: &str,
    dry_run: bool,
    apply: bool,
    json: bool,
    _original_args: &[OsString],
) -> Result<i32, CliFailure> {
    let flags = require_mutation_flag(&format!("keys {verb}"), dry_run, apply, json)?;
    require_known_vm(context, vm, json)?;
    if flags.apply {
        // v1.0 daemon-only: --apply routes through nixlingd → broker
        // (ADR 0015). The historical bash fallback was retired in v1.0.
        let request_type = match verb {
            "rotate" => "keysRotate",
            "trust" => "trust",
            "rotate-known-host" => "rotateKnownHost",
            other => other,
        };
        return dispatch_mutating_verb(
            context,
            request_type,
            serde_json::json!({ "vm": vm }),
            flags.dry_run,
            flags.apply,
            json,
        );
    }
    let summary = serde_json::json!({
        "command": format!("keys {verb}"),
        "mode": "dry-run",
        "vm": vm,
        "planned": [],
        "notes": format!("nixling keys {verb} --dry-run: planned operation. --apply routes through nixlingd → broker RunKeysRotate with broker audit."),
    });
    if json {
        let mut rendered = serde_json::to_string_pretty(&summary)
            .map_err(|e| CliFailure::new(1, format!("serialize: {e}")))?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        print_stdout(&format!(
            "nixling keys {verb} --dry-run: planned operation for vm '{vm}'\n"
        ));
    }
    Ok(0)
}

fn cmd_keys_rotate(
    context: &Context,
    args: &KeysRotateArgs,
    original_args: &[OsString],
) -> Result<i32, CliFailure> {
    w8_mutating_verb(
        context,
        "rotate",
        &args.vm,
        args.dry_run,
        args.apply,
        args.json,
        original_args,
    )
}

fn cmd_keys_rotate_known_host(
    context: &Context,
    args: &KeysRotateKnownHostArgs,
    original_args: &[OsString],
) -> Result<i32, CliFailure> {
    w8_mutating_verb(
        context,
        "rotate-known-host",
        &args.vm,
        args.dry_run,
        args.apply,
        args.json,
        original_args,
    )
}

fn cmd_keys_trust(
    context: &Context,
    args: &KeysTrustArgs,
    original_args: &[OsString],
) -> Result<i32, CliFailure> {
    w8_mutating_verb(
        context,
        "trust",
        &args.vm,
        args.dry_run,
        args.apply,
        args.json,
        original_args,
    )
}

// ---- nixling migrate ----

fn cmd_migrate(
    context: &Context,
    args: &MigrateArgs,
    _original_args: &[OsString],
) -> Result<i32, CliFailure> {
    let flags = require_explicit_mutation_flag("migrate", args.dry_run, args.apply, args.json)?;
    let manifest = context.load_manifest()?;
    let shape = detect_deployment_shape(context)?;
    let vms: Vec<&ManifestVm> = manifest.vms();

    // Migrate planner. Per-VM supervisor classification needs the consumer
    // flake's `nixling.vms.<vm>.supervisor` setting, which the public
    // manifest still does not expose. The prior shape always claimed
    // every VM needed migration, which is materially misleading on a
    // fully-daemon-managed host. The planner now honestly reports
    // "per-VM classification unavailable" and uses the
    // detect_deployment_shape() tier as the operative summary.
    let tier_str = match shape {
        DeploymentShape::Tier0AllLegacy => "tier-0-all-legacy",
        DeploymentShape::Tier0Mixed => "tier-0-mixed",
        DeploymentShape::AllDaemon => "all-daemon",
    };

    if flags.apply {
        // v1.0 daemon-only: --apply routes through nixlingd → broker
        // `RunMigrate` (ADR 0015). The historical bash fallback was
        // retired in v1.0; daemon-unreachable surfaces a typed daemon-down
        // envelope (exit-1).
        let _ = vms;
        let _ = tier_str;
        return dispatch_mutating_verb(
            context,
            "migrate",
            serde_json::json!({}),
            flags.dry_run,
            flags.apply,
            args.json,
        );
    }

    let summary = serde_json::json!({
        "command": "migrate",
        "mode": "dry-run",
        "currentTier": tier_str,
        "classificationAvailable": false,
        "perVmClassificationNote": "v1.1 (per ADR 0015) made every enabled VM daemon-supervised by default; the `nixling.vms.<vm>.supervisor` option was removed in v1.1. Per-VM systemd-unit inspection still uses `nixling status <vm>`.",
        "totalVms": vms.len(),
        "vms": vms.iter().map(|vm| serde_json::json!({
            "name": vm.name,
            "env": vm.env,
            "classification": "unknown-not-in-public-manifest",
        })).collect::<Vec<_>>(),
        "plannedSteps": [
            "v1.1 daemon-only: every enabled VM is daemon-supervised by default; no consumer-flake action is required for supervisor classification.",
            "Per migrating VM: verify per-VM state under `/var/lib/nixling/vms/<vm>/` is owned root:nixlingd 0750.",
            "Run `nixos-rebuild switch` so the daemon module materializes the per-VM broker SpawnRunner state.",
            "Verify each migrated VM via `nixling status <vm>`; `vm list` is still a placeholder inventory surface.",
            "After all VMs migrate cleanly, keep the default-switch readiness gates aligned with the rollout evidence."
        ],
        "notes": "migrate reports the deployment-shape tier today; v1.1 retired the per-VM supervisor option, so per-VM classification is uniformly daemon-supervised. `--apply` routes through nixlingd → broker RunMigrate.",
    });

    if args.json {
        let mut rendered = serde_json::to_string_pretty(&summary)
            .map_err(|e| CliFailure::new(1, format!("serialize: {e}")))?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        print_stdout(&format!(
            "nixling migrate --dry-run: deployment shape = {tier_str}, {} VM(s) in manifest.\n",
            vms.len()
        ));
        print_stdout(
            "v1.1 daemon-only: every enabled VM is daemon-supervised; the per-VM\n\
             `supervisor` option was removed in v1.1 (ADR 0015). Use\n\
             `nixling status <vm>` to inspect each VM directly; `nixling migrate --apply`\n\
             is the live mutation path when you are ready.\n",
        );
    }
    Ok(0)
}

// Legacy bash parity verbs keep the flag-less entrypoint by
// defaulting to --dry-run; native-only host/vm/migrate verbs keep
// using `require_explicit_mutation_flag`.
const DEFAULT_DRY_RUN_NOTICE: &str =
    "nixling: NOTICE: defaulting to --dry-run; nixling 1.0 will require explicit --dry-run or --apply (v0.4 bash CLI had no flag requirement).";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MutationFlags {
    dry_run: bool,
    apply: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MutationFlagResolution {
    flags: MutationFlags,
    notice: Option<&'static str>,
}

fn resolve_mutation_flags(
    dry_run: bool,
    apply: bool,
    default_to_dry_run: bool,
) -> Option<MutationFlagResolution> {
    if dry_run || apply {
        return Some(MutationFlagResolution {
            flags: MutationFlags { dry_run, apply },
            notice: None,
        });
    }
    if default_to_dry_run {
        return Some(MutationFlagResolution {
            flags: MutationFlags {
                dry_run: true,
                apply: false,
            },
            notice: Some(DEFAULT_DRY_RUN_NOTICE),
        });
    }
    None
}

fn require_mutation_flag(
    verb: &str,
    dry_run: bool,
    apply: bool,
    json: bool,
) -> Result<MutationFlags, CliFailure> {
    require_mutation_flag_impl(verb, dry_run, apply, json, true)
}

fn require_explicit_mutation_flag(
    verb: &str,
    dry_run: bool,
    apply: bool,
    json: bool,
) -> Result<MutationFlags, CliFailure> {
    require_mutation_flag_impl(verb, dry_run, apply, json, false)
}

fn require_mutation_flag_impl(
    verb: &str,
    dry_run: bool,
    apply: bool,
    json: bool,
    default_to_dry_run: bool,
) -> Result<MutationFlags, CliFailure> {
    if let Some(resolution) = resolve_mutation_flags(dry_run, apply, default_to_dry_run) {
        if let Some(notice) = resolution.notice {
            let _ = writeln!(io::stderr().lock(), "{notice}");
        }
        return Ok(resolution.flags);
    }
    let exit_code = emit_host_error(
        &host_error_envelope(
            &format!("{verb} requires either --dry-run or --apply"),
            "--apply-or-dry-run-required",
            78,
            &format!("{verb} invocation flags."),
            "Neither --dry-run nor --apply was provided.",
            &format!(
                "Re-run as `nixling {verb} --dry-run` to plan or `nixling {verb} --apply` to mutate.",
            ),
            "docs/reference/error-codes.md#--apply-or-dry-run-required",
        ),
        json,
    )?;
    Err(CliFailure::new(
        exit_code,
        format!("{verb} refused without --dry-run or --apply"),
    ))
}

fn cmd_auth_status(context: &Context, args: &AuthStatusArgs) -> Result<i32, CliFailure> {
    let uid = args.test_uid.unwrap_or_else(effective_uid);
    let launcher_uids = parse_uid_env("NIXLING_TEST_LAUNCHER_UIDS");
    let admin_uids = parse_uid_env("NIXLING_TEST_ADMIN_UIDS");
    let role = if admin_uids.contains(&uid) {
        AuthRoleV2::Admin
    } else if launcher_uids.contains(&uid) {
        AuthRoleV2::Launcher
    } else {
        AuthRoleV2::None
    };

    let public_probe = match context.auth_status_fixture.clone() {
        Some(fixture) => SocketProbe {
            reachable: fixture.public_reachable.unwrap_or(false),
            version: fixture.public_version,
        },
        None => probe_socket(&context.public_socket).unwrap_or(SocketProbe {
            reachable: false,
            version: None,
        }),
    };
    let broker_probe = match context.auth_status_fixture.clone() {
        Some(fixture) => SocketProbe {
            reachable: fixture.broker_reachable.unwrap_or(false),
            version: fixture.broker_version,
        },
        None => SocketProbe {
            reachable: false,
            version: None,
        },
    };

    let all_commands = all_known_subcommands();
    let allowed = allowed_subcommands(role);
    let denied = all_commands
        .into_iter()
        .filter(|command| !allowed.contains(command))
        .map(|name| AuthDeniedSubcommandV2 {
            reason: denied_reason(role, &name).to_owned(),
            name,
        })
        .collect::<Vec<_>>();
    let output = AuthStatusOutputV2 {
        role,
        effective_uid: uid,
        sockets: vec![
            AuthSocketStatusV2 {
                name: "public".to_owned(),
                path: context.public_socket.display().to_string(),
                reachable: public_probe.reachable,
                version: public_probe.version,
            },
            AuthSocketStatusV2 {
                name: "broker".to_owned(),
                path: context.broker_socket.display().to_string(),
                reachable: broker_probe.reachable,
                version: broker_probe.version,
            },
        ],
        allowed_subcommands: allowed.into_iter().collect(),
        denied_subcommands: denied,
    };

    if args.json {
        print_json(&output)?;
    } else {
        print_stdout(&render_auth_status_human(&output));
    }

    Ok(0)
}

fn resolve_selected_vm(args: &StatusArgs) -> Result<Option<String>, CliFailure> {
    match (&args.vm, &args.vm_flag) {
        (Some(positional), Some(flagged)) if positional != flagged => Err(CliFailure::new(
            2,
            "status received conflicting VM selectors",
        )),
        (Some(positional), _) => Ok(Some(positional.clone())),
        (_, Some(flagged)) => Ok(Some(flagged.clone())),
        (None, None) => Ok(None),
    }
}

/// Read the per-VM api-ready state file written by nixlingd on each DAG run.
///
/// The file lives at `{daemon_state_dir}/{vm_name}/api-ready.json` and contains
/// `{"apiReady": <value>}` where the value mirrors `ApiReadyState`'s serialization:
/// `"yes"` | `"pending"` | `"timeout"` | `{"error":"<reason>"}`.
fn read_vm_api_ready(daemon_state_dir: &Path, vm_name: &str) -> Option<ApiReadyStatusV1> {
    let path = daemon_state_dir.join(vm_name).join("api-ready.json");
    let bytes = fs::read(&path).ok()?;
    let obj: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let raw = obj.get("apiReady")?;
    match raw {
        serde_json::Value::String(s) => match s.as_str() {
            "yes" => Some(ApiReadyStatusV1::Simple(ApiReadySimple::Yes)),
            "pending" => Some(ApiReadyStatusV1::Simple(ApiReadySimple::Pending)),
            "timeout" => Some(ApiReadyStatusV1::Simple(ApiReadySimple::Timeout)),
            _ => None,
        },
        serde_json::Value::Object(map) => map.get("error").and_then(|v| v.as_str()).map(|e| {
            ApiReadyStatusV1::WithError(ApiReadyErrorV1 {
                error: e.to_owned(),
            })
        }),
        _ => None,
    }
}

fn live_pool_integrity_unknown(reason: &str, remediation: String) -> LivePoolIntegrityOutputV1 {
    LivePoolIntegrityOutputV1 {
        status: "unknown".to_owned(),
        unknown_reason: Some(reason.to_owned()),
        audit_ref: None,
        repair_attempted: false,
        remediation: Some(remediation),
    }
}

fn live_pool_integrity_suspect(
    repair_attempted: bool,
    audit_ref: Option<String>,
    remediation: String,
) -> LivePoolIntegrityOutputV1 {
    LivePoolIntegrityOutputV1 {
        status: "suspect".to_owned(),
        unknown_reason: None,
        audit_ref,
        repair_attempted,
        remediation: Some(remediation),
    }
}

fn marker_status_for_integrity(store_root: &Path, vm: &str) -> Result<(), &'static str> {
    let marker = store_root
        .join("live")
        .join(format!(".nixling-marker-{vm}"));
    match std::fs::symlink_metadata(&marker) {
        Ok(meta) if meta.is_file() && meta.len() == 0 => Ok(()),
        Ok(_) => Err("suspect"),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Err("marker_or_manifest_missing"),
        Err(_) => Err("marker_or_manifest_unreadable"),
    }
}

fn read_live_pool_integrity(
    context: &Context,
    vm: &ManifestVm,
) -> Option<LivePoolIntegrityOutputV1> {
    let store_root = vm_state_dir(context, vm).join("store-view");
    let state_dir = store_root.join("state");
    let generation_id = match std::fs::read_link(state_dir.join("current")) {
        Ok(target) => target
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned),
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(_) => {
            return Some(live_pool_integrity_unknown(
                "generation_identity_unavailable",
                "restore state/current or activate a new generation, then rerun verify".to_owned(),
            ));
        }
    };
    let Some(generation_id) = generation_id else {
        let vm_unknown = state_dir.join("integrity-unknown.json");
        if let Ok(raw) = std::fs::read_to_string(&vm_unknown) {
            if let Ok(value) = serde_json::from_str::<Value>(&raw) {
                if value.get("state").and_then(Value::as_str) == Some("unknown") {
                    let reason = value
                        .get("unknown_reason")
                        .and_then(Value::as_str)
                        .unwrap_or("generation_identity_unavailable");
                    return Some(live_pool_integrity_unknown(
                        reason,
                        "restore state/current or activate a new generation, then rerun verify"
                            .to_owned(),
                    ));
                }
            }
        }
        return Some(live_pool_integrity_unknown(
            "generation_identity_unavailable",
            "restore state/current or activate a new generation, then rerun verify".to_owned(),
        ));
    };

    let integrity_path = state_dir
        .join("generations")
        .join(&generation_id)
        .join("integrity.json");
    let raw = match std::fs::read_to_string(&integrity_path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Some(live_pool_integrity_unknown(
                "marker_or_manifest_missing",
                format!(
                    "run `nixling store verify {}` to establish live-pool integrity",
                    vm.name
                ),
            ));
        }
        Err(_) => {
            return Some(live_pool_integrity_unknown(
                "marker_or_manifest_unreadable",
                "fix permissions or storage errors, then rerun verify".to_owned(),
            ));
        }
    };
    let value: Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(_) => {
            return Some(live_pool_integrity_unknown(
                "marker_or_manifest_unreadable",
                "fix permissions or storage errors, then rerun verify".to_owned(),
            ));
        }
    };
    let audit_ref = value
        .get("audit_ref")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let repair_attempted = value
        .get("repair_attempted")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    match value.get("state").and_then(Value::as_str) {
        Some("ok") => match marker_status_for_integrity(&store_root, &vm.name) {
            Ok(()) => Some(LivePoolIntegrityOutputV1 {
                status: "ok".to_owned(),
                unknown_reason: None,
                audit_ref,
                repair_attempted,
                remediation: None,
            }),
            Err("suspect") => Some(live_pool_integrity_suspect(
                repair_attempted,
                audit_ref,
                format!("run `nixling store verify {} --repair`", vm.name),
            )),
            Err(reason) => Some(live_pool_integrity_unknown(
                reason,
                format!(
                    "run `nixling store verify {}` to re-establish live-pool integrity",
                    vm.name
                ),
            )),
        },
        Some("suspect") => {
            let remediation = if repair_attempted {
                if audit_ref.is_some() {
                    "repair already attempted; inspect audit_ref and broker logs".to_owned()
                } else {
                    "repair already attempted; inspect broker logs".to_owned()
                }
            } else {
                format!("run `nixling store verify {} --repair`", vm.name)
            };
            Some(live_pool_integrity_suspect(
                repair_attempted,
                audit_ref,
                remediation,
            ))
        }
        Some("unknown") => {
            let reason = value
                .get("unknown_reason")
                .and_then(Value::as_str)
                .unwrap_or("marker_or_manifest_unreadable");
            Some(live_pool_integrity_unknown(
                reason,
                format!("run `nixling store verify {}`", vm.name),
            ))
        }
        _ => Some(live_pool_integrity_unknown(
            "marker_or_manifest_unreadable",
            "fix permissions or storage errors, then rerun verify".to_owned(),
        )),
    }
}

fn build_vm_status_output(
    context: &Context,
    vm: &ManifestVm,
    bundle: Option<&BundleContext>,
) -> StatusVmOutputV2 {
    let process_vm = bundle
        .and_then(|bundle| bundle.processes.as_ref())
        .and_then(|processes| processes.vms.iter().find(|entry| entry.vm == vm.name));
    let service_states = vm_service_states(context, vm, process_vm);
    let current = current_symlink(context, vm);
    let booted = booted_symlink(context, vm);
    let pending_restart =
        is_pending_restart(vm, &service_states, current.as_deref(), booted.as_deref());
    let declared_roles = process_vm
        .map(|entry| {
            entry
                .nodes
                .iter()
                .map(|node| process_role_name(&node.role))
                .collect()
        })
        .unwrap_or_default();
    let readiness: Vec<String> = process_vm
        .map(|entry| {
            entry
                .nodes
                .iter()
                .flat_map(|node| node.readiness.iter().map(readiness_name))
                .collect()
        })
        .unwrap_or_default();
    let runner_parity = bundle
        .and_then(|bundle| bundle.closures.get(&vm.name))
        .map(|closure| RunnerParityOutputV2 {
            declared_runner: closure.declared_runner.clone(),
            runner_parity_path: closure.runner_parity_path.clone(),
            runner_parity_ok: closure.runner_parity_ok,
        });

    StatusVmOutputV2 {
        name: vm.name.clone(),
        env: vm.env.clone(),
        services: service_states,
        current,
        booted,
        pending_restart,
        runtime: RUNTIME_UNKNOWN.to_owned(),
        declared_roles,
        readiness,
        api_ready: read_vm_api_ready(&context.daemon_state_dir, &vm.name),
        runner_parity,
        live_pool_integrity: read_live_pool_integrity(context, vm),
    }
}

fn vm_service_states(
    context: &Context,
    vm: &ManifestVm,
    process_vm: Option<&nixling_core::processes::VmProcessDag>,
) -> StatusServicesOutputV2 {
    let has_role = |role: nixling_core::processes::ProcessRole| {
        process_vm
            .map(|entry| entry.nodes.iter().any(|node| node.role == role))
            .unwrap_or(false)
    };
    let gpu_role_id = if has_role(nixling_core::processes::ProcessRole::GpuRenderNode) {
        Some("gpu-render-node")
    } else if has_role(nixling_core::processes::ProcessRole::Gpu) || vm.graphics {
        Some("gpu")
    } else {
        None
    };
    StatusServicesOutputV2 {
        nixling: systemctl_state(context, "nixlingd.service"),
        microvm: pidfd_role_state(context, &vm.name, "ch-runner"),
        virtiofsd: pidfd_role_prefix_state(context, &vm.name, "virtiofsd"),
        gpu: gpu_role_id.map(|role| pidfd_role_state(context, &vm.name, role)),
        video: has_role(nixling_core::processes::ProcessRole::Video)
            .then(|| pidfd_role_state(context, &vm.name, "video")),
        snd: (has_role(nixling_core::processes::ProcessRole::Audio) || vm.audio)
            .then(|| pidfd_role_state(context, &vm.name, "audio")),
        swtpm: (has_role(nixling_core::processes::ProcessRole::Swtpm) || vm.tpm)
            .then(|| pidfd_role_state(context, &vm.name, "swtpm")),
    }
}

fn pidfd_role_state(context: &Context, vm: &str, role: &str) -> String {
    pidfd_role_state_matching(context, vm, |candidate| candidate == role)
}

fn pidfd_role_prefix_state(context: &Context, vm: &str, prefix: &str) -> String {
    pidfd_role_state_matching(context, vm, |candidate| candidate.starts_with(prefix))
}

fn pidfd_role_state_matching<F>(context: &Context, vm: &str, role_matches: F) -> String
where
    F: Fn(&str) -> bool,
{
    let path = context.daemon_state_dir.join("pidfd-table.json");
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return "stopped".to_owned(),
        Err(_) => return "unknown".to_owned(),
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return "unknown".to_owned();
    };
    let running = value
        .get("entries")
        .and_then(Value::as_array)
        .map(|entries| {
            entries.iter().any(|entry| {
                entry.get("vm").and_then(Value::as_str) == Some(vm)
                    && entry
                        .get("role")
                        .and_then(Value::as_str)
                        .map(&role_matches)
                        .unwrap_or(false)
            })
        })
        .unwrap_or(false);
    if running { "running" } else { "stopped" }.to_owned()
}

fn current_symlink(context: &Context, vm: &ManifestVm) -> Option<String> {
    read_symlink_target(&vm_state_dir(context, vm).join("current"))
}

fn booted_symlink(context: &Context, vm: &ManifestVm) -> Option<String> {
    read_symlink_target(&vm_state_dir(context, vm).join("booted"))
}

fn vm_state_dir(context: &Context, vm: &ManifestVm) -> PathBuf {
    context
        .state_root
        .as_ref()
        .map(|state_root| state_root.join(&vm.name))
        .unwrap_or_else(|| PathBuf::from(&vm.state_dir))
}

fn is_pending_restart(
    vm: &ManifestVm,
    services: &StatusServicesOutputV2,
    current: Option<&str>,
    booted: Option<&str>,
) -> bool {
    current
        .zip(booted)
        .map(|(current, booted)| current != booted)
        .unwrap_or(false)
        && vm_counts_as_running(vm, services)
}

fn vm_counts_as_running(vm: &ManifestVm, services: &StatusServicesOutputV2) -> bool {
    if vm.is_net_vm {
        return true;
    }
    [
        Some(services.nixling.as_str()),
        Some(services.microvm.as_str()),
        services.gpu.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(service_state_counts_as_running)
}

fn service_state_counts_as_running(state: &str) -> bool {
    matches!(state, "active" | "activating" | "reloading" | "running")
}

fn list_status_label(
    vm: &ManifestVm,
    services: &StatusServicesOutputV2,
    pending_restart: bool,
) -> String {
    if vm.is_net_vm {
        "running".to_owned()
    } else if pending_restart {
        "pending-restart".to_owned()
    } else if services.microvm == "unknown" {
        "unknown".to_owned()
    } else if vm_counts_as_running(vm, services) {
        "running".to_owned()
    } else {
        "stopped".to_owned()
    }
}

fn process_role_name(role: &nixling_core::processes::ProcessRole) -> String {
    match role {
        nixling_core::processes::ProcessRole::HostReconcile => "host-reconcile",
        nixling_core::processes::ProcessRole::StoreVirtiofsPreflight => "store-virtiofs-preflight",
        nixling_core::processes::ProcessRole::SwtpmPreStartFlush => "swtpm-pre-start-flush",
        nixling_core::processes::ProcessRole::Swtpm => "swtpm",
        nixling_core::processes::ProcessRole::Virtiofsd => "virtiofsd",
        nixling_core::processes::ProcessRole::Video => "video",
        nixling_core::processes::ProcessRole::Gpu => "gpu",
        nixling_core::processes::ProcessRole::GpuRenderNode => "gpu-render-node",
        nixling_core::processes::ProcessRole::Audio => "audio",
        nixling_core::processes::ProcessRole::CloudHypervisorRunner => "cloud-hypervisor-runner",
        nixling_core::processes::ProcessRole::VsockRelay => "vsock-relay",
        nixling_core::processes::ProcessRole::OtelHostBridge => "otel-host-bridge",
        nixling_core::processes::ProcessRole::GuestSshReadiness => "guest-ssh-readiness",
        nixling_core::processes::ProcessRole::GuestControlHealth => "guest-control-health",
        nixling_core::processes::ProcessRole::Usbip => "usbip",
        nixling_core::processes::ProcessRole::WaylandProxy => "wayland-proxy",
    }
    .to_owned()
}

fn readiness_name(readiness: &nixling_core::processes::ReadinessPredicate) -> String {
    match readiness {
        nixling_core::processes::ReadinessPredicate::ApiSocketInfo(value) => {
            format!("api-socket-info:{value}")
        }
        nixling_core::processes::ReadinessPredicate::VsockNotify(value) => {
            format!("vsock-notify:{value}")
        }
        nixling_core::processes::ReadinessPredicate::UnixSocketExists(value) => {
            format!("unix-socket-exists:{value}")
        }
        nixling_core::processes::ReadinessPredicate::UnixSocketListening(value) => {
            format!("unix-socket-listening:{value}")
        }
        nixling_core::processes::ReadinessPredicate::TcpPort { host, port } => {
            format!("tcp-port:{host}:{port}")
        }
        nixling_core::processes::ReadinessPredicate::Command(argv) => {
            format!("command:{}", argv.join(" "))
        }
        nixling_core::processes::ReadinessPredicate::ComponentSpecific(value) => {
            format!("component-specific:{value}")
        }
        nixling_core::processes::ReadinessPredicate::GuestControlHealth { .. } => {
            "guest-control-health".to_owned()
        }
    }
}

fn render_list_human(output: &ListOutputV2) -> String {
    let mut text = String::from(
        "NAME               ENV       GRAPHICS  TPM   USBIP   STATIC_IP       STATUS\n",
    );
    for item in &output.0 {
        let status = if item.is_net_vm {
            "systemd (net-vm)".to_owned()
        } else {
            item.status.clone()
        };
        let static_ip = item.static_ip.clone().unwrap_or_else(|| "-".to_owned());
        let _ = writeln!(
            text,
            "{:<18} {:<9} {:<9} {:<5} {:<7} {:<15} {}",
            item.name,
            item.env.clone().unwrap_or_else(|| "-".to_owned()),
            item.graphics,
            item.tpm,
            item.usbip,
            static_ip,
            status,
        );
    }
    text
}

fn render_status_vm_human(
    output: &StatusVmOutputV2,
    manifest_vm: &ManifestVm,
    bridge_rows: Vec<BridgeHealthRow>,
) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "=== {} ===", output.name);
    if let Some(env) = &output.env {
        let _ = writeln!(text, "env: {env}");
    }
    let _ = writeln!(text, "runtime: {}", output.runtime);
    let _ = writeln!(text, "nixling@{}: {}", output.name, output.services.nixling);
    let _ = writeln!(
        text,
        "microvm@{} (backend): {}",
        output.name, output.services.microvm
    );
    let _ = writeln!(text, "virtiofsd: {}", output.services.virtiofsd);
    let _ = writeln!(
        text,
        "interactive: {}",
        output
            .services
            .gpu
            .clone()
            .unwrap_or_else(|| "stopped".to_owned())
    );
    if let Some(video) = &output.services.video {
        let _ = writeln!(text, "video: {video}");
    }
    if manifest_vm.ssh_user.is_some() && manifest_vm.static_ip.is_some() {
        let _ = writeln!(text, "ssh: declared");
    }
    let _ = writeln!(
        text,
        "pending-restart: {}",
        if output.pending_restart { "yes" } else { "no" }
    );
    let _ = writeln!(
        text,
        "current: {}",
        output
            .current
            .clone()
            .unwrap_or_else(|| "(missing)".to_owned())
    );
    let _ = writeln!(
        text,
        "booted: {}",
        output
            .booted
            .clone()
            .unwrap_or_else(|| "(missing)".to_owned())
    );
    if !output.declared_roles.is_empty() {
        let _ = writeln!(text, "declared roles: {}", output.declared_roles.join(", "));
    }
    if !output.readiness.is_empty() {
        let _ = writeln!(text, "readiness: {}", output.readiness.join(", "));
    }
    if let Some(runner_parity) = &output.runner_parity {
        let _ = writeln!(
            text,
            "runner parity: {} ({})",
            if runner_parity.runner_parity_ok {
                "ok"
            } else {
                "drift"
            },
            runner_parity.runner_parity_path,
        );
    }
    if let Some(integrity) = &output.live_pool_integrity {
        let _ = writeln!(text, "live-pool integrity: {}", integrity.status);
        if let Some(reason) = &integrity.unknown_reason {
            let _ = writeln!(text, "live-pool unknown reason: {reason}");
        }
        if let Some(remediation) = &integrity.remediation {
            let _ = writeln!(text, "live-pool remediation: {remediation}");
        }
    }
    text.push_str("\n=== Bridge health ===\n");
    text.push_str("BRIDGE               STATE      ADMIN   EXPECTED     RESULT\n");
    for row in bridge_rows {
        let _ = writeln!(
            text,
            "{:<20} {:<10} {:<7} {:<12} {}",
            row.name, row.state, row.admin, row.expected_carrier, row.result
        );
    }
    text
}

fn render_status_inventory_human(
    output: &StatusInventoryOutputV2,
    manifest: &ManifestDocument,
    context: &Context,
    bundle: Option<&BundleContext>,
) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "runtime: {}", output.runtime);
    text.push('\n');
    for vm in &output.vms {
        if let Some(manifest_vm) = manifest.get_vm(&vm.name) {
            text.push_str(&render_status_vm_human(
                vm,
                manifest_vm,
                collect_bridge_rows(context, manifest, bundle),
            ));
            text.push('\n');
        }
    }
    text
}

fn render_host_check_human(output: &HostCheckOutputV2) -> String {
    let mut text = String::new();
    let _ = writeln!(
        text,
        "mode: {}\nstrict: {}\nsummary: pass={} warn={} fail={}\nexit-code: {}\n",
        output.mode,
        output.strict,
        output.summary.pass,
        output.summary.warn,
        output.summary.fail,
        output.exit_code
    );
    for severity in [
        HostCheckSeverityV2::Pass,
        HostCheckSeverityV2::Warn,
        HostCheckSeverityV2::Fail,
    ] {
        let label = match severity {
            HostCheckSeverityV2::Pass => "PASS",
            HostCheckSeverityV2::Warn => "WARN",
            HostCheckSeverityV2::Fail => "FAIL",
        };
        let matching = output
            .findings
            .iter()
            .filter(|finding| finding.severity == severity)
            .collect::<Vec<_>>();
        if matching.is_empty() {
            continue;
        }
        let _ = writeln!(text, "{label}");
        for finding in matching {
            if let Some(vm) = &finding.vm {
                let _ = writeln!(text, "- [{}] {}: {}", vm, finding.id, finding.message);
            } else {
                let _ = writeln!(text, "- {}: {}", finding.id, finding.message);
            }
            let _ = writeln!(text, "  hint: {}", finding.remediation);
        }
        text.push('\n');
    }
    text
}

fn render_auth_status_human(output: &AuthStatusOutputV2) -> String {
    let mut text = String::new();
    let _ = writeln!(
        text,
        "role: {}",
        match output.role {
            AuthRoleV2::None => "none",
            AuthRoleV2::Launcher => "launcher",
            AuthRoleV2::Admin => "admin",
        }
    );
    let _ = writeln!(text, "effective uid: {}", output.effective_uid);
    text.push_str("sockets:\n");
    for socket in &output.sockets {
        let _ = writeln!(
            text,
            "- {}: {}{}",
            socket.name,
            if socket.reachable {
                "reachable"
            } else {
                "unreachable"
            },
            socket
                .version
                .as_ref()
                .map(|version| format!(" (version {version})"))
                .unwrap_or_default(),
        );
    }
    let _ = writeln!(
        text,
        "allowed subcommands: {}",
        output.allowed_subcommands.join(", ")
    );
    if !output.denied_subcommands.is_empty() {
        text.push_str("denied subcommands:\n");
        for denied in &output.denied_subcommands {
            let _ = writeln!(text, "- {}: {}", denied.name, denied.reason);
        }
    }
    text
}

fn collect_bridge_rows(
    context: &Context,
    manifest: &ManifestDocument,
    bundle: Option<&BundleContext>,
) -> Vec<BridgeHealthRow> {
    manifest
        .bridge_names()
        .into_iter()
        .map(|bridge| bridge_health_row(context, bundle, &bridge))
        .collect()
}

fn resolve_bridge_probe_name(bundle: Option<&BundleContext>, bridge: &str) -> String {
    if let Some(runtime) = bundle.and_then(|bundle| bundle.host_runtime.as_ref()) {
        if let Some(ifname) = runtime
            .ifnames
            .iter()
            .find(|row| row.vm.is_none() && row.user_visible_name == bridge)
        {
            return ifname.derived_ifname.clone();
        }
    }
    if let Some(host) = bundle.and_then(|bundle| bundle.host.as_ref()) {
        if let Some(mapping) = host
            .if_name_mappings
            .iter()
            .find(|row| row.vm.is_none() && row.user_visible_name == bridge)
        {
            return mapping.derived_ifname.as_str().to_owned();
        }
    }
    bridge.to_owned()
}

fn bridge_health_row(
    context: &Context,
    bundle: Option<&BundleContext>,
    bridge: &str,
) -> BridgeHealthRow {
    if let Some(fixture) = context
        .system_state_fixture
        .as_ref()
        .and_then(|fixture| fixture.bridges.get(bridge))
    {
        return BridgeHealthRow {
            name: bridge.to_owned(),
            state: fixture.state.clone(),
            admin: fixture.admin.clone(),
            expected_carrier: fixture.expected_carrier.clone(),
            result: fixture.result.clone(),
        };
    }

    let probe_bridge = resolve_bridge_probe_name(bundle, bridge);
    let output = Command::new("ip")
        .args(["-j", "link", "show", "dev", probe_bridge.as_str()])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    let mut row = BridgeHealthRow {
        name: bridge.to_owned(),
        state: "unknown".to_owned(),
        admin: "unknown".to_owned(),
        expected_carrier: "UNKNOWN".to_owned(),
        result: "unavailable".to_owned(),
    };
    if let Ok(output) = output {
        if output.status.success() {
            if let Ok(value) = serde_json::from_slice::<Value>(&output.stdout) {
                if let Some(link) = value.as_array().and_then(|items| items.first()) {
                    row.state = link
                        .get("operstate")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_owned();
                    row.admin = link
                        .get("flags")
                        .and_then(Value::as_array)
                        .map(|flags| {
                            if flags.iter().any(|flag| flag.as_str() == Some("UP")) {
                                "up"
                            } else {
                                "down"
                            }
                        })
                        .unwrap_or("unknown")
                        .to_owned();
                    row.expected_carrier = if row.state == "UP" {
                        "UP"
                    } else {
                        "NO-CARRIER"
                    }
                    .to_owned();
                    row.result = "ok".to_owned();
                }
            }
        }
    }
    row
}

fn systemctl_state(context: &Context, unit: &str) -> String {
    if let Some(state) = context
        .system_state_fixture
        .as_ref()
        .and_then(|fixture| fixture.units.get(unit))
    {
        return state.clone();
    }
    let output = Command::new("systemctl")
        .args(["--no-pager", "is-active", unit])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    match output {
        Ok(output) if !output.stdout.is_empty() => {
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        }
        Ok(output) if output.status.code() == Some(3) => "inactive".to_owned(),
        Ok(_) => "inactive".to_owned(),
        Err(_) => "inactive".to_owned(),
    }
}

fn effective_uid() -> u32 {
    Uid::effective().as_raw()
}

fn all_known_subcommands() -> Vec<String> {
    vec![
        "list",
        "status",
        "audit",
        "host check",
        "auth status",
        "up",
        "down",
        "restart",
        "boot",
        "build",
        "switch",
        "test",
        "rollback",
        "generations",
        "gc",
        "usb",
        "console",
        "audio",
        "keys list",
        "rotate-known-host",
        "trust",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn allowed_subcommands(role: AuthRoleV2) -> BTreeSet<String> {
    match role {
        AuthRoleV2::Admin => all_known_subcommands().into_iter().collect(),
        AuthRoleV2::Launcher => all_known_subcommands()
            .into_iter()
            .filter(|command| command != "audit")
            .collect(),
        AuthRoleV2::None => ["list", "status", "host check", "auth status"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
    }
}

fn denied_reason(role: AuthRoleV2, command: &str) -> &'static str {
    match (role, command) {
        (AuthRoleV2::Admin, _) => "allowed",
        (_, "audit") => "audit requires admin role in `nixling.site.adminUsers`.",
        (AuthRoleV2::Launcher, _) => "allowed",
        (AuthRoleV2::None, _) => {
            "this subcommand requires launcher membership or daemon-admin privileges."
        }
    }
}

fn parse_uid_env(name: &str) -> BTreeSet<u32> {
    env::var(name)
        .ok()
        .map(|value| {
            value
                .split(',')
                .filter_map(|part| part.trim().parse::<u32>().ok())
                .collect()
        })
        .unwrap_or_default()
}

fn env_path(name: &str, default: &str) -> PathBuf {
    env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default))
}

fn maybe_load_json_env<T>(name: &str) -> Result<Option<T>, CliFailure>
where
    T: for<'de> Deserialize<'de>,
{
    match env::var_os(name) {
        Some(path) => read_json_file::<T>(&PathBuf::from(path))
            .map(Some)
            .map_err(|err| CliFailure::new(1, format!("failed to read {name}: {err}"))),
        None => Ok(None),
    }
}

fn read_json_file<T>(path: &Path) -> Result<T, io::Error>
where
    T: for<'de> Deserialize<'de>,
{
    let data = fs::read(path)?;
    serde_json::from_slice(&data).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

fn read_bundle_json<T>(base_dir: &Path, raw_path: &str) -> Result<Option<T>, CliFailure>
where
    T: for<'de> Deserialize<'de>,
{
    let raw = Path::new(raw_path);
    let path = if raw.is_absolute() && raw.exists() {
        raw.to_path_buf()
    } else if raw.is_absolute() {
        raw.file_name()
            .map(|name| base_dir.join(name))
            .unwrap_or_else(|| raw.to_path_buf())
    } else {
        base_dir.join(raw)
    };
    if !path.exists() {
        return Ok(None);
    }
    read_json_file(&path)
        .map(Some)
        .map_err(|err| CliFailure::new(1, format!("failed to read {}: {err}", path.display())))
}

fn print_json<T>(value: &T) -> Result<(), CliFailure>
where
    T: Serialize,
{
    let mut data = serde_json::to_string_pretty(value)
        .map_err(|err| CliFailure::new(1, format!("failed to render JSON: {err}")))?;
    data.push('\n');
    print_stdout(&data);
    Ok(())
}

// Per-thread stdout capture for tests: a thread-local buffer so concurrently
// running tests never pollute one another's captured output. A prior global
// `Mutex<Option<Vec<u8>>>` let any parallel test's `print_stdout` append into
// whichever test currently had capture active, racing the `--json` envelope
// assertions.
#[cfg(test)]
thread_local! {
    static TEST_STDOUT_CAPTURE: std::cell::RefCell<Option<Vec<u8>>> =
        const { std::cell::RefCell::new(None) };
    static TEST_STDERR_CAPTURE: std::cell::RefCell<Option<Vec<u8>>> =
        const { std::cell::RefCell::new(None) };
}
// Process-wide serialization for `with_test_stdout_capture`. The thread-local
// buffer above isolates captured BYTES, but the capturing tests also mutate
// process-global state (an `EnvVarGuard` over `NIXLING_CONFIG_STAGING_DIR`,
// `PATH`, ...). Holding this lock across the closure serializes those tests so
// their env mutations cannot race each other under cargo's parallel harness.
#[cfg(test)]
static TEST_STDOUT_CAPTURE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
fn with_test_stdout_capture<T>(f: impl FnOnce() -> T) -> (T, Vec<u8>) {
    // Recover a poisoned lock: a panicking capturing test must not cascade into
    // every later test failing to acquire the serialization lock.
    let _guard = TEST_STDOUT_CAPTURE_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    TEST_STDOUT_CAPTURE.with(|capture| {
        *capture.borrow_mut() = Some(Vec::new());
    });
    let result = f();
    let stdout = TEST_STDOUT_CAPTURE
        .with(|capture| capture.borrow_mut().take())
        .expect("stdout capture active");
    (result, stdout)
}

#[cfg(test)]
fn with_test_output_capture<T>(f: impl FnOnce() -> T) -> (T, Vec<u8>, Vec<u8>) {
    let _guard = TEST_STDOUT_CAPTURE_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    TEST_STDOUT_CAPTURE.with(|capture| {
        *capture.borrow_mut() = Some(Vec::new());
    });
    TEST_STDERR_CAPTURE.with(|capture| {
        *capture.borrow_mut() = Some(Vec::new());
    });
    let result = f();
    let stdout = TEST_STDOUT_CAPTURE
        .with(|capture| capture.borrow_mut().take())
        .expect("stdout capture active");
    let stderr = TEST_STDERR_CAPTURE
        .with(|capture| capture.borrow_mut().take())
        .expect("stderr capture active");
    (result, stdout, stderr)
}

fn print_stdout(text: &str) {
    let _ = write_stdout_bytes(text.as_bytes());
}

fn write_stdout_bytes(bytes: &[u8]) -> io::Result<()> {
    #[cfg(test)]
    {
        let captured = TEST_STDOUT_CAPTURE.with(|capture| {
            if let Some(buffer) = capture.borrow_mut().as_mut() {
                buffer.extend_from_slice(bytes);
                true
            } else {
                false
            }
        });
        if captured {
            return Ok(());
        }
    }
    let mut stdout = io::stdout().lock();
    stdout.write_all(bytes)?;
    stdout.flush()
}

fn write_stderr_bytes(bytes: &[u8]) -> io::Result<()> {
    #[cfg(test)]
    {
        let captured = TEST_STDERR_CAPTURE.with(|capture| {
            if let Some(buffer) = capture.borrow_mut().as_mut() {
                buffer.extend_from_slice(bytes);
                true
            } else {
                false
            }
        });
        if captured {
            return Ok(());
        }
    }
    let mut stderr = io::stderr().lock();
    stderr.write_all(bytes)?;
    stderr.flush()
}

fn report_failure(err: CliFailure) -> i32 {
    let mut stderr = io::stderr().lock();
    if let Some(rendered_stderr) = err.rendered_stderr {
        let _ = stderr.write_all(rendered_stderr.as_bytes());
    } else {
        let _ = writeln!(stderr, "nixling: {}", err.message);
    }
    err.exit_code
}

fn render_operator_error(error: &CoreError, owning_command: Option<&str>) -> Option<String> {
    let mut value = serde_json::to_value(error).ok()?;
    if let Some(owning_command) = owning_command {
        value.as_object_mut()?.insert(
            "owningCommand".to_owned(),
            Value::String(owning_command.to_owned()),
        );
    }
    let mut rendered = serde_json::to_string_pretty(&value).ok()?;
    rendered.push('\n');
    Some(rendered)
}

fn stdout_is_tty() -> bool {
    io::stdout().is_terminal()
}

// ADR 0017: the `should_fallback_to_legacy` /
// `exec_legacy_passthrough` pair were removed wholesale. Every verb
// the Rust CLI accepts dispatches to clap → typed-envelope; verbs
// clap rejects fall through to the parse-error path. No bash exec
// site survives in the binary crate.

/// Daemon mutating-verb outcome from
/// [`try_daemon_mutating_verb`]. The CLI uses this to decide whether
/// to (a) print the daemon's plan and exit, (b) surface a typed
/// `not-yet-implemented` envelope (exit 78 per ADR 0015), or (c)
/// surface a `daemon-down` envelope (exit 1).
#[derive(Debug)]
enum DaemonVerbOutcome {
    /// The daemon's native handler ran the verb end-to-end.
    Applied { summary: String },
    /// The daemon returned a rust-native dry-run plan.
    DryRunPlanned { summary: String },
    /// The daemon kept the VM process alive but the api-ready phase
    /// timed out in strict mode.
    ApiReadyTimeout { summary: Option<String> },
    /// The daemon has the wire variant + dispatch row, but the
    /// per-verb native backend has not yet landed. CLI surfaces a
    /// typed `not-yet-implemented` envelope and exits 78 (v1.0
    /// daemon-only contract per ADR 0015; no bash fallback).
    NotYetImplemented {
        verb: String,
        target_wave: Option<String>,
        remediation: Option<String>,
    },
    /// The daemon reached the live broker executor but the broker
    /// refused or failed the request. CLI must surface the error and
    /// MUST NOT fall back to bash.
    BrokerError {
        verb: String,
        summary: Option<String>,
        target_wave: Option<String>,
        broker_error_kind: Option<String>,
        remediation: Option<String>,
    },
    /// The daemon refused the request (e.g. missing --dry-run /
    /// --apply pair). CLI surfaces the remediation + exits 2.
    InvalidRequest { remediation: Option<String> },
    /// The daemon socket is not present / reachable. CLI surfaces
    /// a typed `daemon-down` envelope and exits 1 (v1.0 daemon-only
    /// contract per ADR 0015; no bash fallback).
    Unreachable,
}

fn daemon_mutating_verb_frame(
    request_type: &str,
    extra_fields: serde_json::Value,
    dry_run: bool,
    apply: bool,
    json: bool,
) -> Result<Vec<u8>, CliFailure> {
    let mut payload = serde_json::Map::new();
    payload.insert(
        "type".to_owned(),
        serde_json::Value::String(request_type.to_owned()),
    );
    payload.insert("dryRun".to_owned(), serde_json::Value::Bool(dry_run));
    payload.insert("apply".to_owned(), serde_json::Value::Bool(apply));
    payload.insert("json".to_owned(), serde_json::Value::Bool(json));
    if let serde_json::Value::Object(extra) = extra_fields {
        for (k, v) in extra {
            payload.insert(k, v);
        }
    }
    serde_json::to_vec(&serde_json::Value::Object(payload))
        .map_err(|err| CliFailure::new(1, format!("failed to serialize daemon frame: {err}")))
}

/// Send a mutating-verb request frame to the daemon and parse
/// the typed envelope reply.
///
/// `request_type` is the daemon wire `type` discriminant (e.g.
/// `"vmStart"`, `"switch"`, `"hostInstall"`); `extra_fields` is the
/// JSON payload merged with the daemon `MutationFlags` block. The
/// daemon's `dispatch_mutating_verb` validates the flag pair and
/// dispatches the per-verb readiness row.
fn try_daemon_mutating_verb(
    context: &Context,
    request_type: &str,
    extra_fields: serde_json::Value,
    dry_run: bool,
    apply: bool,
    json: bool,
) -> Result<DaemonVerbOutcome, CliFailure> {
    if !context.public_socket.exists() {
        return Ok(DaemonVerbOutcome::Unreachable);
    }
    let mut socket = match SeqpacketUnixSocket::connect(&context.public_socket) {
        Ok(s) => s,
        Err(err) if is_daemon_unreachable(&err) => return Ok(DaemonVerbOutcome::Unreachable),
        Err(err) => {
            return Err(CliFailure::new(
                1,
                format!(
                    "failed to connect to {}: {err}",
                    context.public_socket.display()
                ),
            ))
        }
    };
    let hello = daemon_hello_frame("hello")?;
    socket
        .send_frame(&hello)
        .map_err(|err| CliFailure::new(1, format!("failed to send hello frame: {err}")))?;
    let hello_response = socket
        .recv_frame()
        .map_err(|err| CliFailure::new(1, format!("failed to receive hello reply: {err}")))?;
    let _ = parse_hello_reply(&hello_response)?;

    let frame_bytes = daemon_mutating_verb_frame(request_type, extra_fields, dry_run, apply, json)?;
    socket
        .send_frame(&frame_bytes)
        .map_err(|err| CliFailure::new(1, format!("failed to send mutating verb frame: {err}")))?;
    let response_bytes = socket
        .recv_frame()
        .map_err(|err| CliFailure::new(1, format!("failed to receive verb reply: {err}")))?;

    let response: serde_json::Value = serde_json::from_slice(&response_bytes)
        .map_err(|err| CliFailure::new(1, format!("failed to parse verb reply: {err}")))?;
    let response_type = response
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if response_type == "error" {
        let frame: ErrorFrame = serde_json::from_value(response).map_err(|err| {
            CliFailure::new(1, format!("failed to decode daemon error frame: {err}"))
        })?;
        return Err(cli_failure_from_daemon_error(frame.error));
    }
    let outcome_str = response
        .get("outcome")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let verb = response
        .get("verb")
        .and_then(|v| v.as_str())
        .unwrap_or(request_type)
        .to_owned();
    let summary = response
        .get("summary")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let target_wave = response
        .get("targetWave")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let broker_error_kind = ["brokerErrorKind", "brokerKind", "errorKind", "kind"]
        .iter()
        .find_map(|field| response.get(field).and_then(|v| v.as_str()))
        .map(str::to_owned);
    let remediation = response
        .get("remediation")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    match outcome_str {
        "applied" => Ok(DaemonVerbOutcome::Applied {
            summary: summary.unwrap_or_else(|| format!("nixling {verb} --apply ok")),
        }),
        "dry-run-planned" => Ok(DaemonVerbOutcome::DryRunPlanned {
            summary: summary
                .unwrap_or_else(|| format!("nixling {verb} --dry-run: plan synthesized by daemon")),
        }),
        "api-ready-timeout" => Ok(DaemonVerbOutcome::ApiReadyTimeout { summary }),
        "not-yet-implemented" => Ok(DaemonVerbOutcome::NotYetImplemented {
            verb,
            target_wave,
            remediation,
        }),
        "broker-error" => Ok(DaemonVerbOutcome::BrokerError {
            verb,
            summary,
            target_wave,
            broker_error_kind,
            remediation,
        }),
        "invalid-request" => Ok(DaemonVerbOutcome::InvalidRequest { remediation }),
        other => Err(CliFailure::new(
            1,
            format!("daemon returned unknown mutating-verb outcome: {other}"),
        )),
    }
}

fn redact_broker_error_for_cli(
    op_name: &str,
    broker_error_kind: &str,
) -> Option<(String, String, String)> {
    Some(match broker_error_kind {
        "Broker.BundleResolverUnavailable" => (
            format!("{op_name} failed: broker bundle resolver unavailable"),
            "The daemon reached the broker, but the broker was still starting up or had not loaded the trusted bundle yet.".to_owned(),
            "broker is starting up / bundle not yet loaded; retry shortly. Admin: confirm the bundle path is populated.".to_owned(),
        ),
        "Broker.BundleIntentMissing" => (
            format!("{op_name} failed: trusted bundle intent missing"),
            "The daemon reached the broker, but the trusted bundle did not contain the requested intent row.".to_owned(),
            format!(
                "{op_name} references a bundle intent that the broker did not find. Admin: ask `journalctl -u nixling-priv-broker` for the intent id."
            ),
        ),
        "Broker.StoreViewFilesystemMismatch" => (
            format!("{op_name} refused: store-view filesystem mismatch"),
            "The daemon reached the broker, but the per-VM store view is not on the same filesystem as /nix/store.".to_owned(),
            format!(
                "{op_name} refused: the per-VM store view is not on the same filesystem as /nix/store. Admin: check the VM state dir layout and retry."
            ),
        ),
        "Broker.StoreViewMarkerMissing" => (
            format!("{op_name} refused: store-view marker missing"),
            "The daemon reached the broker, but the prepared store-view generation was missing its marker file.".to_owned(),
            format!(
                "{op_name} refused: the prepared store-view generation is missing its marker. Admin: rebuild the store view and retry."
            ),
        ),
        "Broker.LiveHandlerFailed" => (
            format!("{op_name} failed at the broker live handler"),
            "The daemon reached the broker and the privileged live handler started, but the underlying host mutation failed.".to_owned(),
            format!(
                "{op_name} failed at the broker live handler. Admin: inspect `journalctl -u nixling-priv-broker` for the underlying syscall/exit code."
            ),
        ),
        "Broker.CoexistenceRefused" => (
            format!("{op_name} refused by firewall coexistence policy"),
            "The daemon reached the broker, but another firewall manager still owns the live table described by the trusted bundle.".to_owned(),
            format!(
                "{op_name} refused: another firewall manager owns the table per FirewallCoexistencePolicy. Admin: check nixling.site.firewallCoexistencePolicy."
            ),
        ),
        "Broker.NftScriptParseFailed" => (
            format!("{op_name} failed: bundle nft script parse error"),
            "The daemon reached the broker, but the nftables batch embedded in the trusted bundle could not be parsed.".to_owned(),
            format!(
                "{op_name} failed: bundle nft script could not be parsed. Admin: inspect `journalctl -u nixling-priv-broker` for the parse error."
            ),
        ),
        "Broker.CarveoutOrderingViolation" => (
            format!("{op_name} refused: USBIP firewall carve-out ordering violation"),
            "The daemon reached the broker, but the USBIP carve-out rules were out of order relative to the broad allow/drop rules.".to_owned(),
            "USBIP firewall carve-out rules are out of order relative to broad allow/drop. Admin: inspect the bundle's nft batch ordering.".to_owned(),
        ),
        "Broker.NftablesDriftDetected" => (
            format!("{op_name} refused: live nftables drift detected"),
            "The daemon reached the broker, but the live nftables table hash no longer matched the trusted bundle.".to_owned(),
            "the live nft table hash differs from the bundle's expected hash; someone modified the table out-of-band. Admin: investigate before reapplying.".to_owned(),
        ),
        "Broker.ValidateBundleFailed" => (
            format!("{op_name} failed: trusted bundle validation failed"),
            "The daemon reached the broker, but trusted bundle validation failed before the live handler ran.".to_owned(),
            "trusted bundle validation failed; Admin: re-render the bundle and retry.".to_owned(),
        ),
        "Broker.Protocol" => (
            format!("{op_name} failed: daemon/broker protocol mismatch"),
            "The daemon reached the broker path, but the daemon and broker disagreed on the private wire protocol.".to_owned(),
            "broker protocol error; retry after admin checks broker logs".to_owned(),
        ),
        "Broker.Unimplemented" => (
            format!("{op_name} refused: broker operation unimplemented"),
            "The daemon reached the broker, but this build does not implement the requested broker operation.".to_owned(),
            "broker operation is not implemented in this build; Admin: use the supported fallback path for this wave.".to_owned(),
        ),
        "unknown-operation" => (
            format!("{op_name} refused: broker rejected unknown operation"),
            "The daemon reached the broker, but the broker rejected an unknown private operation id.".to_owned(),
            "broker rejected an unknown operation; Admin: verify daemon and broker versions match.".to_owned(),
        ),
        "authz-audit-requires-admin" => (
            format!("{op_name} refused: admin role required"),
            "The daemon reached the broker, but the broker requires an authorized admin role for this request.".to_owned(),
            "broker audit export requires an authorized admin user.".to_owned(),
        ),
        _ => return None,
    })
}

fn broker_error_envelope(
    verb: &str,
    summary: Option<&str>,
    target_wave: Option<&str>,
    broker_error_kind: Option<&str>,
    remediation: Option<&str>,
) -> HostErrorEnvelope {
    let op_name = format!("nixling {verb} --apply");
    let default_observed_state = if target_wave.is_some() {
        format!(
            "The daemon reached the broker for `{op_name}`, but the broker refused or failed the request (operation not yet implemented in this build)."
        )
    } else {
        format!(
            "The daemon reached the broker for `{op_name}`, but the broker refused or failed the request."
        )
    };
    let (kind, observed_state, remediation) = broker_error_kind
        .and_then(|kind| redact_broker_error_for_cli(&op_name, kind))
        .unwrap_or_else(|| {
            (
                summary
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("{op_name} failed in the daemon → broker path")),
                default_observed_state,
                remediation
                    .unwrap_or(
                        "Review the broker error, fix the host-side prerequisite, and re-run the same command once the daemon → broker path is healthy.",
                    )
                    .to_owned(),
            )
        });
    host_error_envelope(
        &kind,
        "broker-error",
        78,
        &format!("Daemon → broker execution for `{op_name}`"),
        &observed_state,
        &remediation,
        "docs/reference/error-codes.md#broker-error",
    )
}

fn emit_daemon_mutating_outcome(outcome: DaemonVerbOutcome, json: bool) -> Result<i32, CliFailure> {
    match outcome {
        DaemonVerbOutcome::Applied { summary } => {
            print_stdout(&format!("{summary}\n"));
            Ok(0)
        }
        DaemonVerbOutcome::DryRunPlanned { summary } => {
            print_stdout(&format!("{summary}\n"));
            Ok(0)
        }
        DaemonVerbOutcome::ApiReadyTimeout { summary } => {
            let msg = summary.unwrap_or_else(|| "vm start: api-ready timeout".to_owned());
            print_stdout(&format!("{msg}\n"));
            Ok(EXIT_API_TIMEOUT)
        }
        DaemonVerbOutcome::InvalidRequest { remediation } => {
            let msg = remediation.unwrap_or_else(|| "invalid mutating-verb request".to_owned());
            let _ = io::stderr().lock().write_all(msg.as_bytes());
            let _ = io::stderr().lock().write_all(b"\n");
            Ok(2)
        }
        DaemonVerbOutcome::BrokerError {
            verb,
            summary,
            target_wave,
            broker_error_kind,
            remediation,
        } => emit_host_error(
            &broker_error_envelope(
                &verb,
                summary.as_deref(),
                target_wave.as_deref(),
                broker_error_kind.as_deref(),
                remediation.as_deref(),
            ),
            json,
        ),
        DaemonVerbOutcome::NotYetImplemented {
            verb,
            target_wave,
            remediation,
        } => {
            // Bash fallback removed. Surface the typed envelope
            // unconditionally.
            let tw = target_wave
                .as_deref()
                .unwrap_or("the matching W*-fu deferral");
            let remediation_line = remediation.as_deref().unwrap_or(
                "Upgrade nixlingd to a build that includes the requested native handler, then retry.",
            );
            emit_host_error(
                &host_error_envelope(
                    &format!("nixling {verb} --apply requires a daemon-native handler"),
                    "not-yet-implemented",
                    78,
                    &format!("Daemon-native execution for `nixling {verb} --apply` (target: {tw})"),
                    "The daemon reported the requested native handler as not yet implemented; the v1.0 daemon-only contract (ADR 0015) returns the typed `not-yet-implemented` envelope with exit 78.",
                    remediation_line,
                    "docs/reference/error-codes.md#not-yet-implemented",
                ),
                json,
            )
        }
        DaemonVerbOutcome::Unreachable => {
            // Daemon-only. No bash fallback.
            emit_host_error(
                &host_error_envelope(
                    "Daemon required for native --apply",
                    "daemon-down",
                    1,
                    "Daemon connectivity at /run/nixling/public.sock.",
                    "nixlingd is unreachable; v1.1 daemon-only (ADR 0015 + ADR 0017) surfaces the typed `daemon-down` envelope with exit 1.",
                    "Start nixlingd on the host, then re-run the same command.",
                    "docs/reference/error-codes.md#daemon-down",
                ),
                json,
            )
        }
    }
}

/// Top-level dispatcher for mutating verbs. Runs the native daemon
/// path; failure modes surface as typed envelopes (daemon-down
/// exit-1, broker-error exit-78, not-yet-implemented exit-78). The
/// Rust CLI dispatching through nixlingd → broker is the only
/// operator path — no bash fallback.
fn dispatch_mutating_verb(
    context: &Context,
    request_type: &str,
    extra_fields: serde_json::Value,
    dry_run: bool,
    apply: bool,
    json: bool,
) -> Result<i32, CliFailure> {
    let outcome =
        try_daemon_mutating_verb(context, request_type, extra_fields, dry_run, apply, json)?;
    emit_daemon_mutating_outcome(outcome, json)
}

fn probe_socket(path: &Path) -> Result<SocketProbe, CliFailure> {
    let mut socket = SeqpacketUnixSocket::connect(path).map_err(|err| {
        CliFailure::new(1, format!("failed to connect to {}: {err}", path.display()))
    })?;
    let payload = daemon_hello_frame("hello")?;
    socket
        .send_frame(&payload)
        .map_err(|err| CliFailure::new(1, format!("failed to send hello frame: {err}")))?;
    let response = socket
        .recv_frame()
        .map_err(|err| CliFailure::new(1, format!("failed to receive hello reply: {err}")))?;
    let hello = parse_hello_reply(&response)?;
    Ok(SocketProbe {
        reachable: true,
        version: Some(hello.selected_version.as_str().to_owned()),
    })
}

fn try_audit_via_socket(
    context: &Context,
    json_mode: bool,
) -> Result<AuditSocketOutcome, CliFailure> {
    if !context.public_socket.exists() {
        return Ok(AuditSocketOutcome::Unreachable);
    }
    let mut socket = match SeqpacketUnixSocket::connect(&context.public_socket) {
        Ok(socket) => socket,
        Err(err) if is_daemon_unreachable(&err) => return Ok(AuditSocketOutcome::Unreachable),
        Err(err) => {
            return Err(CliFailure::new(
                1,
                format!(
                    "failed to connect to {}: {err}",
                    context.public_socket.display()
                ),
            ))
        }
    };
    let hello = daemon_hello_frame("hello")?;
    socket
        .send_frame(&hello)
        .map_err(|err| CliFailure::new(1, format!("failed to send hello frame: {err}")))?;
    let hello_response = socket
        .recv_frame()
        .map_err(|err| CliFailure::new(1, format!("failed to receive hello reply: {err}")))?;
    let _ = parse_hello_reply(&hello_response)?;
    let request = daemon_audit_frame("audit", json_mode)?;
    socket
        .send_frame(&request)
        .map_err(|err| CliFailure::new(1, format!("failed to send audit request: {err}")))?;
    let response = socket
        .recv_frame()
        .map_err(|err| CliFailure::new(1, format!("failed to receive audit reply: {err}")))?;
    parse_audit_reply(&response).map(AuditSocketOutcome::Lines)
}

fn try_keys_list_via_socket(context: &Context) -> Result<KeysSocketOutcome, CliFailure> {
    let request =
        encode_type_tagged_message("keysList", &serde_json::json!({}), "keysList request")?;
    match try_public_socket_request(context, &request, "keysList")? {
        PublicSocketOutcome::Reply(response) => {
            parse_keys_list_reply(&response).map(KeysSocketOutcome::List)
        }
        PublicSocketOutcome::Unavailable | PublicSocketOutcome::Unsupported => {
            Ok(KeysSocketOutcome::Unavailable)
        }
    }
}

fn try_keys_show_via_socket(context: &Context, vm: &str) -> Result<KeysSocketOutcome, CliFailure> {
    let request = encode_type_tagged_message(
        "keysShow",
        &IpcKeysShowRequest { vm: vm.to_owned() },
        "keysShow request",
    )?;
    match try_public_socket_request(context, &request, "keysShow")? {
        PublicSocketOutcome::Reply(response) => {
            parse_keys_show_reply(&response).map(KeysSocketOutcome::Show)
        }
        PublicSocketOutcome::Unavailable | PublicSocketOutcome::Unsupported => {
            Ok(KeysSocketOutcome::Unavailable)
        }
    }
}

fn try_usb_probe_via_socket(context: &Context) -> Result<UsbProbeSocketOutcome, CliFailure> {
    let request =
        encode_type_tagged_message("usbipProbe", &serde_json::json!({}), "usbipProbe request")?;
    match try_public_socket_request(context, &request, "usbipProbe")? {
        PublicSocketOutcome::Reply(response) => {
            parse_usb_probe_reply(&response).map(UsbProbeSocketOutcome::Entries)
        }
        PublicSocketOutcome::Unavailable | PublicSocketOutcome::Unsupported => {
            Ok(UsbProbeSocketOutcome::Unavailable)
        }
    }
}

fn try_store_verify_via_socket(
    context: &Context,
    vm: &str,
    repair: bool,
) -> Result<StoreVerifySocketOutcome, CliFailure> {
    let request = encode_type_tagged_message(
        "storeVerify",
        &nixling_ipc::public_wire::StoreVerifyRequest {
            vm: vm.to_owned(),
            repair,
        },
        "storeVerify request",
    )?;
    match try_public_socket_request(context, &request, "storeVerify")? {
        PublicSocketOutcome::Reply(response) => {
            parse_store_verify_reply(&response).map(StoreVerifySocketOutcome::Response)
        }
        PublicSocketOutcome::Unavailable | PublicSocketOutcome::Unsupported => {
            Ok(StoreVerifySocketOutcome::Unavailable)
        }
    }
}

fn try_public_socket_request(
    context: &Context,
    request: &[u8],
    request_label: &str,
) -> Result<PublicSocketOutcome, CliFailure> {
    if !context.public_socket.exists() {
        return Ok(PublicSocketOutcome::Unavailable);
    }
    let mut socket = match SeqpacketUnixSocket::connect(&context.public_socket) {
        Ok(socket) => socket,
        Err(err) if is_daemon_unreachable(&err) => return Ok(PublicSocketOutcome::Unavailable),
        Err(err) => {
            return Err(CliFailure::new(
                1,
                format!(
                    "failed to connect to {}: {err}",
                    context.public_socket.display()
                ),
            ))
        }
    };
    let hello = daemon_hello_frame("hello")?;
    socket
        .send_frame(&hello)
        .map_err(|err| CliFailure::new(1, format!("failed to send hello frame: {err}")))?;
    let hello_response = socket
        .recv_frame()
        .map_err(|err| CliFailure::new(1, format!("failed to receive hello reply: {err}")))?;
    let _ = parse_hello_reply(&hello_response)?;
    socket.send_frame(request).map_err(|err| {
        CliFailure::new(1, format!("failed to send {request_label} request: {err}"))
    })?;
    let response = socket.recv_frame().map_err(|err| {
        CliFailure::new(1, format!("failed to receive {request_label} reply: {err}"))
    })?;
    let value: Value = serde_json::from_slice(&response).map_err(|err| {
        CliFailure::new(1, format!("failed to parse {request_label} reply: {err}"))
    })?;
    if value.get("type").and_then(Value::as_str) == Some("error") {
        let frame: ErrorFrame = serde_json::from_value(value).map_err(|err| {
            CliFailure::new(
                1,
                format!("failed to decode {request_label} error reply: {err}"),
            )
        })?;
        if frame.error.kind == "wire-unsupported-request" {
            return Ok(PublicSocketOutcome::Unsupported);
        }
        return Err(CliFailure::new(
            i32::from(frame.error.exit_code),
            format!("{}: {}", request_label, frame.error.message),
        ));
    }
    Ok(PublicSocketOutcome::Reply(response))
}

fn parse_keys_list_reply(bytes: &[u8]) -> Result<Vec<IpcKeyEntry>, CliFailure> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|err| CliFailure::new(1, format!("failed to parse keysList reply: {err}")))?;
    if value.get("type").and_then(Value::as_str) != Some("keysListResponse") {
        return Err(CliFailure::new(
            1,
            "daemon returned an unexpected reply to keysList".to_owned(),
        ));
    }
    serde_json::from_value::<KeysListResponseFrame>(value)
        .map(|frame| frame.entries)
        .map_err(|err| CliFailure::new(1, format!("failed to decode keysList reply: {err}")))
}

fn parse_keys_show_reply(bytes: &[u8]) -> Result<IpcKeysShowResponse, CliFailure> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|err| CliFailure::new(1, format!("failed to parse keysShow reply: {err}")))?;
    if value.get("type").and_then(Value::as_str) != Some("keysShowResponse") {
        return Err(CliFailure::new(
            1,
            "daemon returned an unexpected reply to keysShow".to_owned(),
        ));
    }
    serde_json::from_value::<KeysShowResponseFrame>(value)
        .map(|frame| frame.payload)
        .map_err(|err| CliFailure::new(1, format!("failed to decode keysShow reply: {err}")))
}

fn parse_usb_probe_reply(bytes: &[u8]) -> Result<Vec<IpcUsbipProbeEntry>, CliFailure> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|err| CliFailure::new(1, format!("failed to parse usbipProbe reply: {err}")))?;
    match value.get("type").and_then(Value::as_str) {
        Some("usbipProbeResponse") => serde_json::from_value::<UsbipProbeResponseFrame>(value)
            .map(|frame| frame.entries)
            .map_err(|err| CliFailure::new(1, format!("failed to decode usbipProbe reply: {err}"))),
        Some("mutatingVerbResponse") => {
            let message = value
                .get("summary")
                .and_then(Value::as_str)
                .or_else(|| value.get("remediation").and_then(Value::as_str))
                .unwrap_or("nixling usb probe failed in the daemon → broker path")
                .to_owned();
            let exit_code = if value.get("outcome").and_then(Value::as_str) == Some("broker-error")
            {
                78
            } else {
                1
            };
            Err(CliFailure::new(exit_code, message))
        }
        _ => Err(CliFailure::new(
            1,
            "daemon returned an unexpected reply to usbipProbe".to_owned(),
        )),
    }
}

fn parse_store_verify_reply(bytes: &[u8]) -> Result<IpcStoreVerifyResponse, CliFailure> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|err| CliFailure::new(1, format!("failed to parse storeVerify reply: {err}")))?;
    match value.get("type").and_then(Value::as_str) {
        Some("storeVerifyResponse") => serde_json::from_value::<StoreVerifyResponseFrame>(value)
            .map(|frame| frame.payload)
            .map_err(|err| {
                CliFailure::new(1, format!("failed to decode storeVerify reply: {err}"))
            }),
        _ => Err(CliFailure::new(
            1,
            "daemon returned an unexpected reply to storeVerify".to_owned(),
        )),
    }
}

struct SeqpacketUnixSocket {
    fd: OwnedFd,
}

impl SeqpacketUnixSocket {
    fn connect(path: &Path) -> io::Result<Self> {
        let fd = socket(
            AddressFamily::Unix,
            SockType::SeqPacket,
            SockFlag::SOCK_CLOEXEC,
            None,
        )
        .map_err(nix_err_to_io)?;
        let addr = UnixAddr::new(path).map_err(nix_err_to_io)?;
        connect(fd.as_raw_fd(), &addr).map_err(nix_err_to_io)?;
        Ok(Self { fd })
    }

    fn send_frame(&mut self, payload: &[u8]) -> io::Result<()> {
        if payload.len() > MAX_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "frame exceeds 1 MiB limit",
            ));
        }
        let mut frame = Vec::with_capacity(payload.len() + 4);
        frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        frame.extend_from_slice(payload);
        let sent = send(self.fd.as_raw_fd(), &frame, MsgFlags::empty()).map_err(nix_err_to_io)?;
        if sent != frame.len() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "short write on seqpacket socket",
            ));
        }
        Ok(())
    }

    fn recv_frame(&mut self) -> io::Result<Vec<u8>> {
        let mut buffer = vec![0_u8; MAX_FRAME_BYTES + 4];
        let received =
            recv(self.fd.as_raw_fd(), &mut buffer, MsgFlags::empty()).map_err(nix_err_to_io)?;
        if received < 4 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "short frame from seqpacket socket",
            ));
        }
        let expected = u32::from_le_bytes(buffer[..4].try_into().expect("frame prefix")) as usize;
        if expected > MAX_FRAME_BYTES || expected + 4 > received {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "malformed seqpacket frame",
            ));
        }
        Ok(buffer[4..4 + expected].to_vec())
    }
}

fn read_symlink_target(path: &Path) -> Option<String> {
    fs::read_link(path)
        .ok()
        .map(|target| target.display().to_string())
}

fn nix_err_to_io(err: nix::errno::Errno) -> io::Error {
    io::Error::from_raw_os_error(err as i32)
}

#[cfg(test)]
mod host_install_dispatch_tests {
    use clap::Parser;
    use std::{
        env,
        ffi::{OsStr, OsString},
        io,
        os::{
            fd::{AsRawFd as _, RawFd},
            unix::fs::PermissionsExt,
        },
        path::PathBuf,
        sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc, Mutex,
        },
        thread,
        time::Duration,
    };

    use nix::{
        sys::socket::{accept4, bind, listen, Backlog},
        unistd::close,
    };
    use serde_json::{json, Value};

    use super::{
        broker_error_envelope, cmd_host_install, cmd_vm_exec, cmd_vm_start,
        daemon_supported_features, encode_type_tagged_message, nix_err_to_io, parse_vm_exec_action,
        send, socket, AddressFamily, ApiReadySimple, ApiReadyStatusV1, Context, HostInstallArgs,
        IpcHelloOk, MsgFlags, NativeCli, SockFlag, SockType, UnixAddr, UsbAttachArgs, VmExecArgs,
        VmStartArgs, MAX_FRAME_BYTES,
    };
    use nixling_ipc::Version;

    static ENV_MUTEX: Mutex<()> = Mutex::new(());
    static TEST_SOCKET_COUNTER: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn resolve_mutation_flags_defaults_to_dry_run() {
        assert_eq!(
            super::resolve_mutation_flags(false, false, true),
            Some(super::MutationFlagResolution {
                flags: super::MutationFlags {
                    dry_run: true,
                    apply: false,
                },
                notice: Some(super::DEFAULT_DRY_RUN_NOTICE),
            })
        );
    }

    #[test]
    fn resolve_mutation_flags_requires_explicit_flag_when_requested() {
        assert_eq!(super::resolve_mutation_flags(false, false, false), None);
    }

    #[test]
    fn resolve_mutation_flags_preserves_explicit_apply() {
        assert_eq!(
            super::resolve_mutation_flags(false, true, true),
            Some(super::MutationFlagResolution {
                flags: super::MutationFlags {
                    dry_run: false,
                    apply: true,
                },
                notice: None,
            })
        );
    }

    #[allow(dead_code)] // EnvVarGuard is utility code used by tests that toggle env vars
    struct EnvVarGuard {
        key: &'static str,
        old: Option<OsString>,
    }

    #[allow(dead_code)]
    impl EnvVarGuard {
        fn set(key: &'static str, value: impl AsRef<OsStr>) -> Self {
            let old = env::var_os(key);
            env::set_var(key, value);
            Self { key, old }
        }

        fn remove(key: &'static str) -> Self {
            let old = env::var_os(key);
            env::remove_var(key);
            Self { key, old }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.old {
                env::set_var(self.key, value);
            } else {
                env::remove_var(self.key);
            }
        }
    }

    fn recv_test_frame(fd: RawFd) -> io::Result<Vec<u8>> {
        let mut buffer = vec![0_u8; MAX_FRAME_BYTES + 4];
        let received = super::recv(fd, &mut buffer, MsgFlags::empty()).map_err(nix_err_to_io)?;
        if received < 4 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "short frame from seqpacket socket",
            ));
        }
        let expected = u32::from_le_bytes(buffer[..4].try_into().expect("frame prefix")) as usize;
        if expected > MAX_FRAME_BYTES || expected + 4 > received {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "malformed seqpacket frame",
            ));
        }
        Ok(buffer[4..4 + expected].to_vec())
    }

    fn send_test_frame(fd: RawFd, payload: &[u8]) -> io::Result<()> {
        if payload.len() > MAX_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "frame exceeds 1 MiB limit",
            ));
        }
        let mut frame = Vec::with_capacity(payload.len() + 4);
        frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        frame.extend_from_slice(payload);
        let sent = send(fd, &frame, MsgFlags::empty()).map_err(nix_err_to_io)?;
        if sent != frame.len() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "short write on seqpacket socket",
            ));
        }
        Ok(())
    }

    fn test_socket_path(test_name: &str, suffix: &str) -> PathBuf {
        let counter = TEST_SOCKET_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::current_dir()
            .expect("current dir")
            .join("target")
            .join(format!(
                "{test_name}-{}-{counter}{suffix}",
                std::process::id()
            ))
    }

    fn host_install_original_args(args: &HostInstallArgs) -> Vec<OsString> {
        let mut original_args = vec![OsString::from("host"), OsString::from("install")];
        if args.dry_run {
            original_args.push(OsString::from("--dry-run"));
        }
        if args.apply {
            original_args.push(OsString::from("--apply"));
        }
        if args.enable {
            original_args.push(OsString::from("--enable"));
        }
        if args.start {
            original_args.push(OsString::from("--start"));
        }
        if args.no_start {
            original_args.push(OsString::from("--no-start"));
        }
        if args.json {
            original_args.push(OsString::from("--json"));
        }
        if args.human {
            original_args.push(OsString::from("--human"));
        }
        original_args
    }

    fn write_test_manifest(path: &PathBuf, vm: &str) {
        let manifest = json!({
            (vm): {
                "name": vm,
                "env": "dev",
                "graphics": false,
                "tpm": false,
                "audio": false,
                "audioService": format!("nixling-{vm}-audio.service"),
                "usbipYubikey": false,
                "staticIp": null,
                "isNetVm": false,
                "stateDir": format!("/var/lib/nixling/vms/{vm}"),
                "bridge": "nl-dev",
                "sshUser": "alice"
            }
        });
        std::fs::write(
            path,
            serde_json::to_vec(&manifest).expect("serialize manifest"),
        )
        .expect("write manifest");
    }

    fn write_usb_attach_manifest(path: &PathBuf, vm: &str) {
        let manifest = json!({
            (vm): {
                "name": vm,
                "env": "dev",
                "graphics": false,
                "tpm": false,
                "audio": false,
                "usbipYubikey": true,
                "staticIp": "10.20.0.10",
                "usbipdHostIp": "192.0.2.1",
                "isNetVm": false,
                "stateDir": format!("/var/lib/nixling/vms/{vm}"),
                "bridge": "nl-dev",
                "sshUser": "alice"
            }
        });
        std::fs::write(
            path,
            serde_json::to_vec(&manifest).expect("serialize usb attach manifest"),
        )
        .expect("write usb attach manifest");
    }

    fn run_vm_start_with_mock_daemon(
        args: VmStartArgs,
        response: Value,
    ) -> (Result<i32, super::CliFailure>, Value) {
        let socket_path = test_socket_path("vm-start", ".sock");
        let manifest_path = test_socket_path("vm-start", ".manifest.json");
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent).expect("create test socket dir");
        }
        let _ = std::fs::remove_file(&socket_path);
        let _ = std::fs::remove_file(&manifest_path);
        write_test_manifest(&manifest_path, &args.vm);
        let listener = socket(
            AddressFamily::Unix,
            SockType::SeqPacket,
            SockFlag::SOCK_CLOEXEC,
            None,
        )
        .expect("listener socket");
        let addr = UnixAddr::new(&socket_path).expect("unix addr");
        bind(listener.as_raw_fd(), &addr).expect("bind listener");
        listen(&listener, Backlog::new(1).expect("backlog")).expect("listen");

        let (request_tx, request_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let accepted = accept4(listener.as_raw_fd(), SockFlag::SOCK_CLOEXEC).expect("accept");
            let exchange_result = (|| -> io::Result<()> {
                let hello_bytes = recv_test_frame(accepted)?;
                let hello: Value = serde_json::from_slice(&hello_bytes)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                assert_eq!(hello.get("type").and_then(Value::as_str), Some("hello"));

                let hello_reply = encode_type_tagged_message(
                    "helloOk",
                    &IpcHelloOk {
                        server_version: Version::new("0.4.0").expect("server version"),
                        selected_version: Version::new("0.4.0").expect("selected version"),
                        capabilities: daemon_supported_features(),
                    },
                    "test hello reply",
                )
                .expect("encode hello reply");
                send_test_frame(accepted, &hello_reply)?;

                let request_bytes = recv_test_frame(accepted)?;
                let request: Value = serde_json::from_slice(&request_bytes)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                request_tx
                    .send(request)
                    .expect("send request to test thread");

                let response_bytes = serde_json::to_vec(&response)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                send_test_frame(accepted, &response_bytes)
            })();
            close(accepted).expect("close accepted socket");
            exchange_result.expect("mock daemon exchange");
        });

        let context = Context {
            manifest_path: manifest_path.clone(),
            bundle_path: manifest_path.with_extension("bundle.json"),
            public_socket: socket_path.clone(),
            broker_socket: PathBuf::from("/dev/null"),
            state_root: None,
            host_runtime_path: PathBuf::from("/dev/null"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: PathBuf::from("/dev/null"),
            metrics_url: "http://127.0.0.1:1/metrics".to_owned(),
        };
        let result = cmd_vm_start(&context, &args);
        let request = request_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("receive daemon request");
        server.join().expect("join mock daemon thread");
        let _ = std::fs::remove_file(&socket_path);
        let _ = std::fs::remove_file(&manifest_path);
        (result, request)
    }

    fn gc_sync_args(vm: &str) -> super::ConfigSyncArgs {
        super::ConfigSyncArgs {
            vm: vm.to_owned(),
            guest_path: super::DEFAULT_GUEST_CONFIG_PATH.to_owned(),
            host: None,
            user: None,
            key: None,
            known_hosts: None,
            dry_run: false,
            json: false,
        }
    }

    /// PATH-based `ssh`/`scp` trap: prepends a sentinel bin holding scripts
    /// that touch a marker file when invoked. Restores PATH on drop. Guarded by
    /// `ENV_MUTEX` so it never races a concurrent env-mutating test.
    struct ExecSshTrap {
        marker: PathBuf,
        old_path: Option<OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl ExecSshTrap {
        fn install(dir: &std::path::Path) -> Self {
            let lock = ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
            let bin = dir.join("trap-bin");
            std::fs::create_dir_all(&bin).expect("create trap bin");
            let marker = dir.join("ssh-spawned.marker");
            for tool in ["ssh", "scp"] {
                let script = bin.join(tool);
                std::fs::write(
                    &script,
                    format!("#!/bin/sh\necho spawned > {}\nexit 0\n", marker.display()),
                )
                .expect("write trap script");
                let mut perms = std::fs::metadata(&script).expect("stat").permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(&script, perms).expect("chmod trap script");
            }
            let old_path = env::var_os("PATH");
            let mut entries = vec![bin];
            if let Some(existing) = &old_path {
                entries.extend(env::split_paths(existing));
            }
            env::set_var("PATH", env::join_paths(entries).expect("join PATH"));
            Self {
                marker,
                old_path,
                _lock: lock,
            }
        }

        fn ssh_was_spawned(&self) -> bool {
            self.marker.exists()
        }
    }

    impl Drop for ExecSshTrap {
        fn drop(&mut self) {
            match &self.old_path {
                Some(value) => env::set_var("PATH", value),
                None => env::remove_var("PATH"),
            }
        }
    }

    /// Drive `cmd_vm_exec` (json) against a mock daemon that completes the
    /// hello handshake, accepts the `Start` op, and replies with the daemon
    /// `error` frame whose `kind` is supplied. Returns the CLI result plus the
    /// list of post-hello frames the daemon received (the first MUST be the
    /// `Start`; any further frame would be an illegitimate proxied op).
    fn run_vm_exec_with_mock_daemon_response(
        args: VmExecArgs,
        response_frame: Value,
    ) -> (Result<i32, super::CliFailure>, Vec<Value>, Vec<u8>) {
        let (result, frames, stdout, _stderr) =
            run_vm_exec_with_mock_daemon_response_and_stderr(args, response_frame);
        (result, frames, stdout)
    }

    fn run_vm_exec_with_mock_daemon_response_and_stderr(
        args: VmExecArgs,
        response_frame: Value,
    ) -> (Result<i32, super::CliFailure>, Vec<Value>, Vec<u8>, Vec<u8>) {
        let socket_path = test_socket_path("vm-exec", ".sock");
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent).expect("create test socket dir");
        }
        let _ = std::fs::remove_file(&socket_path);
        let listener = socket(
            AddressFamily::Unix,
            SockType::SeqPacket,
            SockFlag::SOCK_CLOEXEC,
            None,
        )
        .expect("listener socket");
        let addr = UnixAddr::new(&socket_path).expect("unix addr");
        bind(listener.as_raw_fd(), &addr).expect("bind listener");
        listen(&listener, Backlog::new(1).expect("backlog")).expect("listen");

        let (frames_tx, frames_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let accepted = accept4(listener.as_raw_fd(), SockFlag::SOCK_CLOEXEC).expect("accept");
            let exchange = (|| -> io::Result<()> {
                let hello_bytes = recv_test_frame(accepted)?;
                let hello: Value = serde_json::from_slice(&hello_bytes)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                assert_eq!(hello.get("type").and_then(Value::as_str), Some("hello"));
                let hello_reply = encode_type_tagged_message(
                    "helloOk",
                    &IpcHelloOk {
                        server_version: Version::new("0.4.0").expect("server version"),
                        selected_version: Version::new("0.4.0").expect("selected version"),
                        capabilities: daemon_supported_features(),
                    },
                    "test hello reply",
                )
                .expect("encode hello reply");
                send_test_frame(accepted, &hello_reply)?;

                // First post-hello frame: the Start op.
                let start_bytes = recv_test_frame(accepted)?;
                let start: Value = serde_json::from_slice(&start_bytes)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                frames_tx.send(start).expect("send start frame");

                let response_frame = serde_json::to_vec(&response_frame)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                send_test_frame(accepted, &response_frame)?;

                // Any further frame is an illegitimate proxied op; record it.
                if let Ok(extra_bytes) = recv_test_frame(accepted) {
                    if !extra_bytes.is_empty() {
                        if let Ok(extra) = serde_json::from_slice::<Value>(&extra_bytes) {
                            frames_tx.send(extra).expect("send extra frame");
                        }
                    }
                }
                Ok(())
            })();
            close(accepted).expect("close accepted socket");
            exchange.expect("mock daemon exchange");
        });

        let context = Context {
            manifest_path: PathBuf::from("/dev/null"),
            bundle_path: PathBuf::from("/dev/null"),
            public_socket: socket_path.clone(),
            broker_socket: PathBuf::from("/dev/null"),
            state_root: None,
            host_runtime_path: PathBuf::from("/dev/null"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: PathBuf::from("/dev/null"),
            metrics_url: "http://127.0.0.1:1/metrics".to_owned(),
        };
        let (result, stdout, stderr) =
            super::with_test_output_capture(|| cmd_vm_exec(&context, &args));
        server.join().expect("join mock daemon thread");
        let frames: Vec<Value> = frames_rx.try_iter().collect();
        let _ = std::fs::remove_file(&socket_path);
        (result, frames, stdout, stderr)
    }

    fn run_vm_exec_with_mock_daemon(
        args: VmExecArgs,
        error_kind: &'static str,
    ) -> (Result<i32, super::CliFailure>, Vec<Value>, Vec<u8>) {
        run_vm_exec_with_mock_daemon_response(
            args,
            json!({
                "type": "error",
                "error": {
                    "kind": error_kind,
                    "message": "this VM generation does not support guest-control exec",
                    "remediation": "rebuild the VM with a current nixling generation",
                },
            }),
        )
    }

    fn missing_daemon_context() -> Context {
        Context {
            manifest_path: PathBuf::from("/dev/null"),
            bundle_path: PathBuf::from("/dev/null"),
            public_socket: PathBuf::from("/dev/null"),
            broker_socket: PathBuf::from("/dev/null"),
            state_root: None,
            host_runtime_path: PathBuf::from("/dev/null"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: PathBuf::from("/dev/null"),
            metrics_url: "http://127.0.0.1:1/metrics".to_owned(),
        }
    }

    fn parse_vm_exec(argv: &[&str]) -> VmExecArgs {
        let cli = NativeCli::try_parse_from(argv).expect("vm exec argv parses");
        match cli.command {
            super::NativeCommand::Vm(super::VmArgs {
                command: super::VmCommand::Exec(args),
            }) => args,
            other => panic!("expected vm exec parse, got {other:?}"),
        }
    }

    #[test]
    fn vm_exec_old_generation_fails_closed_without_proxy_or_ssh() {
        // Binding fail-closed invariant: `vm exec` against a VM whose
        // generation lacks the guest-control transport must surface exit 70 +
        // `guest-control-unavailable-old-generation`, MUST NOT proxy any exec
        // op beyond the rejected `Start`, and MUST NOT fall back to SSH. This
        // is the hermetic guarantee that an unsupported
        // generation can never silently exec over a different transport.
        let dir = test_socket_path("vm-exec-oldgen", ".dir");
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let trap = ExecSshTrap::install(&dir);

        let args = VmExecArgs {
            vm: "oldgenvm".to_owned(),
            detach: false,
            interactive: false,
            tty: false,
            env: Vec::new(),
            cwd: None,
            json: true,
            human: false,
            management: Vec::new(),
            command: vec!["ls".to_owned()],
        };
        let (result, frames, stdout) =
            run_vm_exec_with_mock_daemon(args, "guest-control-unavailable-old-generation");

        // A `--json` run emits exactly ONE terminal JSON document on
        // STDOUT for ALL outcomes (incl this old-generation establishment
        // reject) and returns the CLI exit code — nothing goes to stderr.
        let exit_code = result.expect("json exec returns the exit code, not a stderr failure");
        assert_eq!(exit_code, 70, "old generation maps to exit 70");
        let envelope: Value =
            serde_json::from_slice(&stdout).expect("exactly one JSON document on stdout");
        assert_eq!(
            envelope.get("reason").and_then(Value::as_str),
            Some("guest-control-unavailable-old-generation"),
            "old-generation surfaces its fail-closed slug: {envelope}"
        );
        assert_eq!(
            envelope.get("source").and_then(Value::as_str),
            Some("guest-control"),
            "old-generation is a guest-control source, never guest"
        );
        assert_eq!(envelope.get("exitCode").and_then(Value::as_i64), Some(70));
        assert_eq!(
            envelope.get("transportExitCode").and_then(Value::as_i64),
            Some(70),
            "a non-guest failure carries transportExitCode"
        );
        assert!(
            envelope.get("stdoutBase64").is_none() && envelope.get("stderrBase64").is_none(),
            "a failure envelope never carries captured stdio bytes: {envelope}"
        );
        // The daemon received exactly ONE post-hello frame (the Start). A
        // second frame would mean the CLI proxied an exec op after the reject.
        assert_eq!(
            frames.len(),
            1,
            "exactly the rejected Start may be sent; no proxied op may follow"
        );
        assert_eq!(
            frames[0].get("op").and_then(Value::as_str),
            Some("start"),
            "the single proxied frame is the Start op"
        );
        // No SSH/SCP client may be spawned on the fail-closed exec path.
        assert!(
            !trap.ssh_was_spawned(),
            "old-generation exec fail-closed must never spawn an SSH client"
        );

        drop(trap);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn vm_exec_env_validation_redacts_supplied_value() {
        // A malformed `--env` entry may carry a secret (e.g. `=secret`
        // or `TOKEN=hunter2`). The operator error must report the offending
        // position only — never the raw entry, key, or value.
        const SECRET: &str = "sentinel-env-secret-7f3a";
        let context = Context {
            manifest_path: PathBuf::from("/dev/null"),
            bundle_path: PathBuf::from("/dev/null"),
            public_socket: PathBuf::from("/dev/null"),
            broker_socket: PathBuf::from("/dev/null"),
            state_root: None,
            host_runtime_path: PathBuf::from("/dev/null"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: PathBuf::from("/dev/null"),
            metrics_url: "http://127.0.0.1:1/metrics".to_owned(),
        };

        // Human path: the CliFailure message must not leak the value. Env
        // validation runs before any daemon connection, so /dev/null is fine.
        let human_args = VmExecArgs {
            vm: "work".to_owned(),
            detach: false,
            interactive: false,
            tty: false,
            env: vec![format!("={SECRET}")],
            cwd: None,
            json: false,
            human: false,
            management: Vec::new(),
            command: vec!["true".to_owned()],
        };
        let failure = cmd_vm_exec(&context, &human_args)
            .expect_err("an empty-key --env entry is a usage failure");
        assert_eq!(failure.exit_code, 2);
        assert!(
            !failure.message.contains(SECRET),
            "human --env error leaked the secret value: {}",
            failure.message
        );
        assert!(
            failure.message.contains("#1"),
            "human --env error reports the offending position: {}",
            failure.message
        );

        // JSON path: the single stdout envelope must not leak the value either.
        let json_args = VmExecArgs {
            vm: "work".to_owned(),
            detach: false,
            interactive: false,
            tty: false,
            env: vec![format!("not-a-pair-{SECRET}")],
            cwd: None,
            json: true,
            human: false,
            management: Vec::new(),
            command: vec!["true".to_owned()],
        };
        let (result, stdout) =
            super::with_test_stdout_capture(|| cmd_vm_exec(&context, &json_args));
        let exit_code = result.expect("json usage failure returns the exit code");
        assert_eq!(exit_code, 2);
        let envelope: Value = serde_json::from_slice(&stdout).expect("one JSON document on stdout");
        let rendered = envelope.to_string();
        assert!(
            !rendered.contains(SECRET),
            "json --env envelope leaked the secret value: {rendered}"
        );
        assert_eq!(
            envelope.get("reason").and_then(Value::as_str),
            Some("usage")
        );
        assert_eq!(envelope.get("exitCode").and_then(Value::as_i64), Some(2));
    }

    #[test]
    fn vm_exec_missing_command_emits_usage_envelope() {
        // A missing command is validated inside `cmd_vm_exec` (the
        // clap arg is NOT `required`), so a `--json` run emits a single stdout
        // usage envelope (source: cli, reason: usage, exit 2) and the human run
        // is a plain stderr usage failure — both matching error-codes.md and
        // cli-contract.md.
        let context = Context {
            manifest_path: PathBuf::from("/dev/null"),
            bundle_path: PathBuf::from("/dev/null"),
            public_socket: PathBuf::from("/dev/null"),
            broker_socket: PathBuf::from("/dev/null"),
            state_root: None,
            host_runtime_path: PathBuf::from("/dev/null"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: PathBuf::from("/dev/null"),
            metrics_url: "http://127.0.0.1:1/metrics".to_owned(),
        };

        let json_args = VmExecArgs {
            vm: "work".to_owned(),
            detach: false,
            interactive: false,
            tty: false,
            env: Vec::new(),
            cwd: None,
            json: true,
            human: false,
            management: Vec::new(),
            command: Vec::new(),
        };
        let (result, stdout) =
            super::with_test_stdout_capture(|| cmd_vm_exec(&context, &json_args));
        let exit_code = result.expect("json missing-command usage returns the exit code");
        assert_eq!(exit_code, 2);
        let envelope: Value = serde_json::from_slice(&stdout).expect("one JSON document on stdout");
        assert_eq!(
            envelope.get("command").and_then(Value::as_str),
            Some("vm exec")
        );
        assert_eq!(envelope.get("source").and_then(Value::as_str), Some("cli"));
        assert_eq!(
            envelope.get("reason").and_then(Value::as_str),
            Some("usage")
        );
        assert_eq!(envelope.get("exitCode").and_then(Value::as_i64), Some(2));

        let human_args = VmExecArgs {
            json: false,
            ..json_args
        };
        let failure = cmd_vm_exec(&context, &human_args)
            .expect_err("missing command is a human usage failure");
        assert_eq!(failure.exit_code, 2);
        assert!(
            failure.message.contains("missing command"),
            "human missing-command error is actionable: {}",
            failure.message
        );
    }

    #[test]
    fn vm_exec_detach_rejects_interactive_and_requires_command() {
        let context = missing_daemon_context();

        for argv in [
            ["nixling", "vm", "exec", "-d", "-i", "work", "--", "id"].as_slice(),
            ["nixling", "vm", "exec", "-d", "-t", "work", "--", "id"].as_slice(),
        ] {
            let args = parse_vm_exec(argv);
            let failure = cmd_vm_exec(&context, &args).expect_err("-d with -i/-t is usage");
            assert_eq!(failure.exit_code, 2);
            assert!(
                failure.message.contains("cannot be combined"),
                "detach usage error is actionable: {}",
                failure.message
            );
        }

        let args = parse_vm_exec(&["nixling", "vm", "exec", "-d", "work", "--json"]);
        let (result, stdout) = super::with_test_stdout_capture(|| cmd_vm_exec(&context, &args));
        assert_eq!(result.expect("json usage returns exit code"), 2);
        let envelope: Value = serde_json::from_slice(&stdout).expect("usage JSON");
        assert_eq!(
            envelope.get("reason").and_then(Value::as_str),
            Some("usage")
        );
        assert!(
            envelope
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .contains("missing command"),
            "detach missing command stays actionable: {envelope}"
        );
    }

    #[test]
    fn vm_exec_vm_first_management_grammar_parses_verbs_and_verb_named_vms() {
        let list = parse_vm_exec(&["nixling", "vm", "exec", "work", "list"]);
        assert_eq!(list.vm, "work");
        let list_action = parse_vm_exec_action(&list).expect("list action parses");
        assert!(matches!(
            list_action.management,
            Some(super::VmExecManagementCommand::List)
        ));

        let logs = parse_vm_exec(&[
            "nixling",
            "vm",
            "exec",
            "work",
            "logs",
            "exec-1",
            "--stdout-offset",
            "11",
            "--stderr-offset",
            "22",
            "--max-len",
            "33",
        ]);
        let logs_action = parse_vm_exec_action(&logs).expect("logs action parses");
        assert!(matches!(
            logs_action.management,
            Some(super::VmExecManagementCommand::Logs(super::VmExecLogsArgs {
                exec_id,
                stdout_offset: Some(11),
                stderr_offset: Some(22),
                max_len: Some(33),
            })) if exec_id == "exec-1"
        ));

        let logs_equals = parse_vm_exec(&[
            "nixling",
            "vm",
            "exec",
            "work",
            "logs",
            "exec-1",
            "--stdout-offset=44",
        ]);
        let logs_equals_action =
            parse_vm_exec_action(&logs_equals).expect("logs equals action parses");
        assert!(matches!(
            logs_equals_action.management,
            Some(super::VmExecManagementCommand::Logs(
                super::VmExecLogsArgs {
                    stdout_offset: Some(44),
                    ..
                }
            ))
        ));

        let status = parse_vm_exec(&["nixling", "vm", "exec", "list", "status", "exec-2"]);
        assert_eq!(status.vm, "list");
        let status_action = parse_vm_exec_action(&status).expect("status action parses");
        assert!(matches!(
            status_action.management,
            Some(super::VmExecManagementCommand::Status(super::VmExecIdArgs { exec_id }))
                if exec_id == "exec-2"
        ));

        let kill = parse_vm_exec(&["nixling", "vm", "exec", "kill", "kill", "exec-3"]);
        assert_eq!(kill.vm, "kill");
        let kill_action = parse_vm_exec_action(&kill).expect("kill action parses");
        assert!(matches!(
            kill_action.management,
            Some(super::VmExecManagementCommand::Kill(super::VmExecIdArgs { exec_id }))
                if exec_id == "exec-3"
        ));

        let command = parse_vm_exec(&["nixling", "vm", "exec", "logs", "--", "status", "exec-4"]);
        assert_eq!(command.vm, "logs");
        let command_action = parse_vm_exec_action(&command).expect("command action parses");
        assert!(command_action.management.is_none());
        assert_eq!(
            command.command,
            vec!["status".to_owned(), "exec-4".to_owned()]
        );

        let status_named_vm = parse_vm_exec(&[
            "nixling",
            "vm",
            "exec",
            "status",
            "logs",
            "exec-status-logs",
        ]);
        assert_eq!(status_named_vm.vm, "status");
        let status_named_action =
            parse_vm_exec_action(&status_named_vm).expect("status-named VM logs action parses");
        assert!(matches!(
            status_named_action.management,
            Some(super::VmExecManagementCommand::Logs(super::VmExecLogsArgs {
                exec_id,
                ..
            })) if exec_id == "exec-status-logs"
        ));

        let logs_named_vm = parse_vm_exec(&[
            "nixling",
            "vm",
            "exec",
            "logs",
            "status",
            "exec-logs-status",
        ]);
        assert_eq!(logs_named_vm.vm, "logs");
        let logs_named_action =
            parse_vm_exec_action(&logs_named_vm).expect("logs-named VM status action parses");
        assert!(matches!(
            logs_named_action.management,
            Some(super::VmExecManagementCommand::Status(super::VmExecIdArgs { exec_id }))
                if exec_id == "exec-logs-status"
        ));
    }

    #[test]
    fn vm_exec_unknown_management_word_is_usage_not_reserved_name() {
        let context = missing_daemon_context();
        const SECRET_TOKEN: &str = "secret-token-should-not-render";
        let args = parse_vm_exec(&["nixling", "vm", "exec", "work", SECRET_TOKEN]);
        let failure = cmd_vm_exec(&context, &args).expect_err("unknown no---word is usage failure");
        assert_eq!(failure.exit_code, 2);
        assert!(
            failure.message.contains("use `--` to run a command"),
            "unknown management error tells the operator how to run commands: {}",
            failure.message
        );
        assert!(
            !failure.message.contains(SECRET_TOKEN),
            "unknown management error leaked the would-be argv token: {}",
            failure.message
        );

        let json_args = parse_vm_exec(&["nixling", "vm", "exec", "work", SECRET_TOKEN, "--json"]);
        let (result, stdout) =
            super::with_test_stdout_capture(|| cmd_vm_exec(&context, &json_args));
        assert_eq!(result.expect("json usage returns exit code"), 2);
        let envelope: Value = serde_json::from_slice(&stdout).expect("usage JSON");
        assert_eq!(
            envelope.get("reason").and_then(Value::as_str),
            Some("usage")
        );
        let rendered = envelope.to_string();
        assert!(
            !rendered.contains(SECRET_TOKEN),
            "json usage envelope leaked the would-be argv token: {rendered}"
        );
    }

    #[test]
    fn vm_exec_invalid_program_daemon_error_exits_usage_without_stale_remediation() {
        let args = parse_vm_exec(&["nixling", "vm", "exec", "work", "--json", "--", "-foo"]);
        let (result, frames, stdout) = run_vm_exec_with_mock_daemon_response(
            args,
            json!({
                "type": "error",
                "error": {
                    "kind": "guest-control-invalid-program",
                    "message": "invalid program: pass a non-empty command after `--` that does not start with `-`",
                    "remediation": "insert `--` before the guest command and use a program name such as `bash` or `id`",
                },
            }),
        );
        assert_eq!(result.expect("json error returns code"), 2);
        assert_eq!(frames.len(), 1);
        let envelope: Value = serde_json::from_slice(&stdout).expect("invalid-program JSON");
        assert_eq!(
            envelope.get("reason").and_then(Value::as_str),
            Some("guest-control-invalid-program")
        );
        assert_eq!(envelope.get("exitCode").and_then(Value::as_i64), Some(2));
        let rendered = envelope.to_string();
        assert!(
            !rendered.contains("already exited"),
            "invalid-program must not use stale remediation: {rendered}"
        );
        assert!(
            rendered.contains("pass a non-empty command after"),
            "invalid-program JSON must carry the actionable daemon message: {rendered}"
        );
        assert!(
            rendered.contains("insert `--` before the guest command"),
            "invalid-program JSON must carry the actionable remediation: {rendered}"
        );

        let human_args = parse_vm_exec(&["nixling", "vm", "exec", "work", "--", "-foo"]);
        let (human_result, _, _) = run_vm_exec_with_mock_daemon_response(
            human_args,
            json!({
                "type": "error",
                "error": {
                    "kind": "guest-control-invalid-program",
                    "message": "invalid program: pass a non-empty command after `--` that does not start with `-`",
                    "remediation": "insert `--` before the guest command and use a program name such as `bash` or `id`",
                },
            }),
        );
        let failure = human_result.expect_err("human invalid-program is a usage failure");
        assert_eq!(failure.exit_code, 2);
        assert!(
            failure.message.contains("pass a non-empty command after"),
            "invalid-program human output must carry the actionable message: {}",
            failure.message
        );
    }

    #[test]
    fn vm_exec_detached_create_renders_human_and_json() {
        let human_args = parse_vm_exec(&["nixling", "vm", "exec", "-d", "work", "--", "id"]);
        let (human_result, human_frames, human_stdout) = run_vm_exec_with_mock_daemon_response(
            human_args,
            json!({
                "type": "execResponse",
                "op": "detachedCreate",
                "result": {"execId": "exec-abc", "state": "running"},
            }),
        );
        assert_eq!(human_result.expect("detached create human"), 0);
        assert_eq!(String::from_utf8(human_stdout).unwrap(), "exec-abc\n");
        assert_eq!(
            human_frames[0]
                .pointer("/args/detached")
                .and_then(Value::as_bool),
            Some(true)
        );

        let json_args =
            parse_vm_exec(&["nixling", "vm", "exec", "-d", "work", "--json", "--", "id"]);
        let (json_result, _json_frames, json_stdout) = run_vm_exec_with_mock_daemon_response(
            json_args,
            json!({
                "type": "execResponse",
                "op": "detachedCreate",
                "result": {"execId": "exec-json", "state": "created"},
            }),
        );
        assert_eq!(json_result.expect("detached create json"), 0);
        let envelope: Value = serde_json::from_slice(&json_stdout).expect("create JSON");
        assert_eq!(
            envelope.get("command").and_then(Value::as_str),
            Some("vm exec")
        );
        assert_eq!(
            envelope.get("execId").and_then(Value::as_str),
            Some("exec-json")
        );
        assert_eq!(
            envelope.get("state").and_then(Value::as_str),
            Some("created")
        );
    }

    #[test]
    fn vm_exec_detached_management_renders_json_shapes() {
        let list_args = parse_vm_exec(&["nixling", "vm", "exec", "work", "list", "--json"]);
        let (list_result, list_frames, list_stdout) = run_vm_exec_with_mock_daemon_response(
            list_args,
            json!({
                "type": "execResponse",
                "op": "list",
                "result": {
                    "execs": [{
                        "execId": "exec-1",
                        "state": "exited",
                        "exitCode": 0,
                        "startedAt": "2026-06-15T00:00:00Z",
                        "startOffset": 1,
                        "endOffset": 9,
                        "stdoutStartOffset": 1,
                        "stdoutEndOffset": 5,
                        "stderrStartOffset": 2,
                        "stderrEndOffset": 9,
                        "droppedBytes": 3,
                        "stdoutDroppedBytes": 1,
                        "stderrDroppedBytes": 2,
                        "truncated": true,
                        "stdoutTruncated": false,
                        "stderrTruncated": true
                    }]
                },
            }),
        );
        assert_eq!(list_result.expect("list json"), 0);
        assert_eq!(
            list_frames[0].get("op").and_then(Value::as_str),
            Some("list")
        );
        let list_envelope: Value = serde_json::from_slice(&list_stdout).expect("list JSON");
        assert_eq!(
            list_envelope.get("command").and_then(Value::as_str),
            Some("vm exec list")
        );
        assert_eq!(
            list_envelope
                .pointer("/execs/0/stdoutDroppedBytes")
                .and_then(Value::as_i64),
            Some(1)
        );

        let status_args = parse_vm_exec(&[
            "nixling", "vm", "exec", "work", "status", "exec-1", "--json",
        ]);
        let (status_result, _status_frames, status_stdout) = run_vm_exec_with_mock_daemon_response(
            status_args,
            json!({
                "type": "execResponse",
                "op": "status",
                "result": {
                    "execId": "exec-1",
                    "state": "signaled",
                    "reason": "operator-cancelled",
                    "signal": 15,
                    "startOffset": 4,
                    "endOffset": 44,
                    "droppedBytes": 0,
                    "truncated": false
                },
            }),
        );
        assert_eq!(status_result.expect("status json"), 0);
        let status_envelope: Value = serde_json::from_slice(&status_stdout).expect("status JSON");
        assert_eq!(
            status_envelope.get("command").and_then(Value::as_str),
            Some("vm exec status")
        );
        assert_eq!(
            status_envelope.get("signal").and_then(Value::as_i64),
            Some(15)
        );

        let logs_args = parse_vm_exec(&[
            "nixling",
            "vm",
            "exec",
            "work",
            "logs",
            "exec-1",
            "--stdout-offset",
            "4",
            "--stderr-offset",
            "8",
            "--max-len",
            "16",
            "--json",
        ]);
        let (logs_result, logs_frames, logs_stdout) = run_vm_exec_with_mock_daemon_response(
            logs_args,
            json!({
                "type": "execResponse",
                "op": "logs",
                "result": {
                    "execId": "exec-1",
                    "stdoutBase64": "T1VUCg==",
                    "stderrBase64": "RVJSCg==",
                    "startOffset": 4,
                    "endOffset": 12,
                    "droppedBytes": 0,
                    "truncated": false,
                    "stdoutStartOffset": 4,
                    "stdoutEndOffset": 8,
                    "stdoutNextOffset": 8,
                    "stdoutEof": true,
                    "stdoutDroppedBytes": 0,
                    "stdoutTruncated": false,
                    "stderrStartOffset": 8,
                    "stderrEndOffset": 12,
                    "stderrNextOffset": 12,
                    "stderrEof": true,
                    "stderrDroppedBytes": 0,
                    "stderrTruncated": false
                },
            }),
        );
        assert_eq!(logs_result.expect("logs json"), 0);
        assert_eq!(
            logs_frames[0]
                .pointer("/args/stdoutOffset")
                .and_then(Value::as_i64),
            Some(4)
        );
        assert_eq!(
            logs_frames[0]
                .pointer("/args/stderrOffset")
                .and_then(Value::as_i64),
            Some(8)
        );
        assert_eq!(
            logs_frames[0]
                .pointer("/args/maxLen")
                .and_then(Value::as_i64),
            Some(16)
        );
        let logs_envelope: Value = serde_json::from_slice(&logs_stdout).expect("logs JSON");
        assert_eq!(
            logs_envelope.get("stdoutBase64").and_then(Value::as_str),
            Some("T1VUCg==")
        );
        assert_eq!(
            logs_envelope
                .get("stderrNextOffset")
                .and_then(Value::as_i64),
            Some(12)
        );

        let kill_args =
            parse_vm_exec(&["nixling", "vm", "exec", "work", "kill", "exec-1", "--json"]);
        let (kill_result, _kill_frames, kill_stdout) = run_vm_exec_with_mock_daemon_response(
            kill_args,
            json!({
                "type": "execResponse",
                "op": "kill",
                "result": {
                    "execId": "exec-1",
                    "result": "cancelling",
                    "state": "running"
                },
            }),
        );
        assert_eq!(kill_result.expect("kill json"), 0);
        let kill_envelope: Value = serde_json::from_slice(&kill_stdout).expect("kill JSON");
        assert_eq!(
            kill_envelope.get("command").and_then(Value::as_str),
            Some("vm exec kill")
        );
        assert_eq!(
            kill_envelope.get("result").and_then(Value::as_str),
            Some("cancelling")
        );

        let kill_terminal_args = parse_vm_exec(&[
            "nixling",
            "vm",
            "exec",
            "work",
            "kill",
            "exec-terminal",
            "--json",
        ]);
        let (kill_terminal_result, _kill_terminal_frames, kill_terminal_stdout) =
            run_vm_exec_with_mock_daemon_response(
                kill_terminal_args,
                json!({
                    "type": "execResponse",
                    "op": "kill",
                    "result": {
                        "execId": "exec-terminal",
                        "result": "already-terminal",
                        "state": "exited"
                    },
                }),
            );
        assert_eq!(kill_terminal_result.expect("kill already-terminal json"), 0);
        let kill_terminal_envelope: Value =
            serde_json::from_slice(&kill_terminal_stdout).expect("kill terminal JSON");
        assert_eq!(
            kill_terminal_envelope.get("result").and_then(Value::as_str),
            Some("already-terminal")
        );
        assert_eq!(
            kill_terminal_envelope.get("state").and_then(Value::as_str),
            Some("exited")
        );
    }

    #[test]
    fn vm_exec_detached_management_renders_human_shapes_with_offsets() {
        let list_args = parse_vm_exec(&["nixling", "vm", "exec", "work", "list"]);
        let (list_result, _list_frames, list_stdout, _list_stderr) =
            run_vm_exec_with_mock_daemon_response_and_stderr(
                list_args,
                json!({
                    "type": "execResponse",
                    "op": "list",
                    "result": {
                        "execs": [{
                            "execId": "exec-1",
                            "state": "exited",
                            "exitCode": 0,
                            "startedAt": "2026-06-15T00:00:00Z",
                            "startOffset": 1,
                            "endOffset": 9,
                            "stdoutStartOffset": 1,
                            "stdoutEndOffset": 5,
                            "stderrStartOffset": 2,
                            "stderrEndOffset": 9,
                            "droppedBytes": 3,
                            "stdoutDroppedBytes": 1,
                            "stderrDroppedBytes": 2,
                            "truncated": true,
                            "stdoutTruncated": false,
                            "stderrTruncated": true
                        }]
                    },
                }),
            );
        assert_eq!(list_result.expect("list human"), 0);
        let list_rendered = String::from_utf8(list_stdout).expect("list stdout utf8");
        assert!(
            list_rendered.contains("OFFSETS"),
            "list human output labels retained offset windows: {list_rendered}"
        );
        assert!(
            list_rendered.contains("all=1..9 stdout=1..5 stderr=2..9"),
            "list human output includes aggregate and per-stream windows: {list_rendered}"
        );
        assert!(
            list_rendered.contains("all=3/truncated stdout=1/complete stderr=2/truncated"),
            "list human output includes aggregate and per-stream loss metadata: {list_rendered}"
        );

        let status_args = parse_vm_exec(&["nixling", "vm", "exec", "work", "status", "exec-1"]);
        let (status_result, _status_frames, status_stdout, _status_stderr) =
            run_vm_exec_with_mock_daemon_response_and_stderr(
                status_args,
                json!({
                    "type": "execResponse",
                    "op": "status",
                    "result": {
                        "execId": "exec-1",
                        "state": "signaled",
                        "reason": "operator-cancelled",
                        "signal": 15,
                        "startOffset": 4,
                        "endOffset": 44,
                        "droppedBytes": 2,
                        "truncated": true
                    },
                }),
            );
        assert_eq!(status_result.expect("status human"), 0);
        let status_rendered = String::from_utf8(status_stdout).expect("status stdout utf8");
        assert!(
            status_rendered.contains("terminal: signal=15"),
            "status human output includes terminal disposition: {status_rendered}"
        );
        assert!(
            status_rendered
                .contains("logs: startOffset=4 endOffset=44 droppedBytes=2 truncated=true"),
            "status human output includes retained window and loss metadata: {status_rendered}"
        );

        let logs_args = parse_vm_exec(&["nixling", "vm", "exec", "work", "logs", "exec-1"]);
        let (logs_result, _logs_frames, logs_stdout, logs_stderr) =
            run_vm_exec_with_mock_daemon_response_and_stderr(
                logs_args,
                json!({
                    "type": "execResponse",
                    "op": "logs",
                    "result": {
                        "execId": "exec-1",
                        "stdoutBase64": "T1VUCg==",
                        "stderrBase64": "RVJS",
                        "startOffset": 4,
                        "endOffset": 18,
                        "droppedBytes": 5,
                        "truncated": true,
                        "stdoutStartOffset": 4,
                        "stdoutEndOffset": 8,
                        "stdoutNextOffset": 10,
                        "stdoutEof": false,
                        "stdoutDroppedBytes": 2,
                        "stdoutTruncated": true,
                        "stderrStartOffset": 9,
                        "stderrEndOffset": 18,
                        "stderrNextOffset": 21,
                        "stderrEof": true,
                        "stderrDroppedBytes": 3,
                        "stderrTruncated": false
                    },
                }),
            );
        assert_eq!(logs_result.expect("logs human"), 0);
        assert_eq!(
            String::from_utf8(logs_stdout).expect("logs stdout utf8"),
            "OUT\n"
        );
        let logs_stderr_rendered = String::from_utf8(logs_stderr).expect("logs stderr utf8");
        assert!(
            logs_stderr_rendered
                .starts_with("ERR\nnixling: vm exec logs: retained output incomplete"),
            "logs human output writes stderr bytes then bounded warning: {logs_stderr_rendered}"
        );
        for expected in [
            "startOffset=4",
            "endOffset=18",
            "stdoutStartOffset=4",
            "stdoutEndOffset=8",
            "stdoutNextOffset=10",
            "stdoutEof=false",
            "stderrStartOffset=9",
            "stderrEndOffset=18",
            "stderrNextOffset=21",
            "stderrEof=true",
            "stdoutDroppedBytes=2",
            "stderrDroppedBytes=3",
        ] {
            assert!(
                logs_stderr_rendered.contains(expected),
                "logs warning missing {expected}: {logs_stderr_rendered}"
            );
        }

        for (wire_result, state) in [("cancelling", "running"), ("already-terminal", "exited")] {
            let kill_args = parse_vm_exec(&["nixling", "vm", "exec", "work", "kill", wire_result]);
            let (kill_result, _kill_frames, kill_stdout, _kill_stderr) =
                run_vm_exec_with_mock_daemon_response_and_stderr(
                    kill_args,
                    json!({
                        "type": "execResponse",
                        "op": "kill",
                        "result": {
                            "execId": wire_result,
                            "result": wire_result,
                            "state": state
                        },
                    }),
                );
            assert_eq!(kill_result.expect("kill human"), 0);
            let kill_rendered = String::from_utf8(kill_stdout).expect("kill stdout utf8");
            assert!(
                kill_rendered.contains(&format!("{wire_result}: {wire_result} (state={state})")),
                "kill human output includes outcome {wire_result}: {kill_rendered}"
            );
        }
    }

    #[test]
    fn vm_exec_logs_json_validates_base64_before_success_envelope() {
        let logs_args = parse_vm_exec(&[
            "nixling", "vm", "exec", "work", "logs", "exec-bad", "--json",
        ]);
        let (logs_result, _logs_frames, logs_stdout) = run_vm_exec_with_mock_daemon_response(
            logs_args,
            json!({
                "type": "execResponse",
                "op": "logs",
                "result": {
                    "execId": "exec-bad",
                    "stdoutBase64": "not-valid-base64!",
                    "stderrBase64": "RVJSCg==",
                    "startOffset": 0,
                    "endOffset": 0,
                    "droppedBytes": 0,
                    "truncated": false,
                    "stdoutStartOffset": 0,
                    "stdoutEndOffset": 0,
                    "stdoutNextOffset": 0,
                    "stdoutEof": false,
                    "stdoutDroppedBytes": 0,
                    "stdoutTruncated": false,
                    "stderrStartOffset": 0,
                    "stderrEndOffset": 0,
                    "stderrNextOffset": 0,
                    "stderrEof": false,
                    "stderrDroppedBytes": 0,
                    "stderrTruncated": false
                },
            }),
        );
        assert_eq!(
            logs_result.expect("malformed logs JSON returns protocol exit code"),
            76
        );
        let envelope: Value =
            serde_json::from_slice(&logs_stdout).expect("protocol error JSON envelope");
        assert_eq!(
            envelope.get("reason").and_then(Value::as_str),
            Some("guest-control-protocol-error")
        );
        assert_eq!(
            envelope.get("source").and_then(Value::as_str),
            Some("protocol")
        );
        assert_eq!(envelope.get("exitCode").and_then(Value::as_i64), Some(76));
        assert!(
            envelope.get("stdoutBase64").is_none(),
            "protocol failure must not serialize malformed stdout payload: {envelope}"
        );
        assert!(
            envelope
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .contains("malformed base64 for detached stdout"),
            "protocol failure names the malformed field: {envelope}"
        );
    }

    fn read_guest_config_reply(content: &[u8]) -> Vec<u8> {
        let encoded = nixling_core::base64_codec::encode(content);
        serde_json::to_vec(&json!({
            "type": "readGuestConfigResponse",
            "contentBase64": encoded,
        }))
        .expect("serialize reply")
    }

    fn guest_control_error_reply(kind: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "type": "error",
            "error": {
                "kind": kind,
                "exitCode": 70,
                "message": "guest-control read failed",
                "remediation": "retry after the guest finishes booting",
            },
        }))
        .expect("serialize error reply")
    }

    fn gc_test_role_profile() -> nixling_core::processes::RoleProfile {
        nixling_core::processes::RoleProfile {
            profile_id: "guest-control-health".to_owned(),
            uid: 1000,
            gid: 1000,
            adr_carve_out: None,
            caps: Vec::new(),
            namespaces: nixling_core::minijail_profile::NamespaceSet {
                mount: false,
                pid: false,
                net: false,
                ipc: false,
                uts: false,
                user: false,
            },
            seccomp_policy_ref: None,
            mount_policy: nixling_core::minijail_profile::MountPolicy {
                read_only_paths: Vec::new(),
                writable_paths: Vec::new(),
                device_binds: Vec::new(),
                bind_mounts: Vec::new(),
                nix_store_read_only: true,
                hide_device_nodes_by_default: true,
            },
            cgroup_placement: nixling_core::minijail_profile::CgroupPlacement {
                subtree: "nixling.slice/test".to_owned(),
                controllers: Vec::new(),
                delegated: false,
            },
            user_namespace: None,
            umask: None,
        }
    }

    /// Write a bundle whose processes DAG declares a `GuestControlHealth`
    /// node for `vm`, so `vm_uses_guest_control` resolves true and
    /// `cmd_config_sync` follows the guest-control transport path.
    fn write_guest_control_bundle(bundle_path: &std::path::Path, vm: &str) {
        let base_dir = bundle_path.parent().expect("bundle parent");
        std::fs::create_dir_all(base_dir).expect("create bundle dir");
        // Derive EVERY sibling artifact path from the unique bundle file
        // name. The bundle path is unique per test (a monotonic counter);
        // sharing a `<vm>.processes.json` across the parallel config-sync
        // tests caused torn reads (one test truncating the file while
        // another parsed it), so the file name MUST be per-bundle, not
        // per-vm.
        let unique = bundle_path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("bundle file name");
        let processes_path = base_dir.join(format!("{unique}.processes.json"));
        let processes = nixling_core::processes::ProcessesJson {
            schema_version: "v2".to_owned(),
            vms: vec![nixling_core::processes::VmProcessDag {
                vm: vm.to_owned(),
                nodes: vec![nixling_core::processes::ProcessNode {
                    id: nixling_core::processes::NodeId("guest-control-health".to_owned()),
                    role: nixling_core::processes::ProcessRole::GuestControlHealth,
                    unit: None,
                    binary_path: None,
                    argv: Vec::new(),
                    env: Vec::new(),
                    plan_ops: Vec::new(),
                    profile: gc_test_role_profile(),
                    readiness: Vec::new(),
                }],
                edges: Vec::new(),
                invariants: nixling_core::processes::VmProcessInvariants {
                    swtpm_pre_start_flush: false,
                    per_vm_audit_pipeline: false,
                    usbip_gating: false,
                    tpm_ownership_migration_without_running_vm_mutation: false,
                },
            }],
        };
        std::fs::write(
            &processes_path,
            serde_json::to_vec(&processes).expect("serialize processes"),
        )
        .expect("write processes.json");
        let bundle = json!({
            "bundleVersion": 4,
            "schemaVersion": "v2",
            "publicManifestPath": base_dir.join(format!("{unique}.vms.json")).to_string_lossy(),
            "hostPath": base_dir.join(format!("{unique}.host.json")).to_string_lossy(),
            "processesPath": processes_path.to_string_lossy(),
            "privilegesPath": base_dir.join(format!("{unique}.privileges.json")).to_string_lossy(),
            "closures": [],
            "minijailProfiles": [],
            "generation": { "generator": "test", "sourceRevision": null, "generatedAt": null },
        });
        std::fs::write(
            bundle_path,
            serde_json::to_vec(&bundle).expect("serialize bundle"),
        )
        .expect("write bundle.json");
    }

    /// Drive the real `cmd_config_sync` over a mock public.sock that
    /// performs the hello handshake then, if `serve` is `Some`, reads the
    /// `readGuestConfig` request (recording it) and replies with the given
    /// frame. When `serve` is `None`, no socket is created so the command
    /// observes the daemon as unavailable. Returns the command result, the
    /// recorded daemon request (if a server ran), and the captured stdout.
    fn run_config_sync_with_mock_daemon(
        args: super::ConfigSyncArgs,
        serve: Option<Vec<u8>>,
    ) -> (Result<i32, super::CliFailure>, Option<Value>, Vec<u8>) {
        let socket_path = test_socket_path("config-sync", ".sock");
        let manifest_path = test_socket_path("config-sync", ".manifest.json");
        let bundle_path = manifest_path.with_extension("bundle.json");
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent).expect("create test socket dir");
        }
        let _ = std::fs::remove_file(&socket_path);
        let _ = std::fs::remove_file(&manifest_path);
        write_test_manifest(&manifest_path, &args.vm);
        write_guest_control_bundle(&bundle_path, &args.vm);
        let staging_dir = test_socket_path("config-sync", ".staging");
        let _ = std::fs::remove_dir_all(&staging_dir);

        let server = serve.map(|reply| {
            let listener = socket(
                AddressFamily::Unix,
                SockType::SeqPacket,
                SockFlag::SOCK_CLOEXEC,
                None,
            )
            .expect("listener socket");
            let addr = UnixAddr::new(&socket_path).expect("unix addr");
            bind(listener.as_raw_fd(), &addr).expect("bind listener");
            listen(&listener, Backlog::new(1).expect("backlog")).expect("listen");
            let (request_tx, request_rx) = mpsc::channel();
            let join = thread::spawn(move || {
                let accepted =
                    accept4(listener.as_raw_fd(), SockFlag::SOCK_CLOEXEC).expect("accept");
                let exchange = (|| -> io::Result<()> {
                    let hello_bytes = recv_test_frame(accepted)?;
                    let hello: Value = serde_json::from_slice(&hello_bytes)
                        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                    assert_eq!(hello.get("type").and_then(Value::as_str), Some("hello"));
                    let hello_reply = encode_type_tagged_message(
                        "helloOk",
                        &IpcHelloOk {
                            server_version: Version::new("0.4.0").expect("server version"),
                            selected_version: Version::new("0.4.0").expect("selected version"),
                            capabilities: daemon_supported_features(),
                        },
                        "test hello reply",
                    )
                    .expect("encode hello reply");
                    send_test_frame(accepted, &hello_reply)?;
                    let request_bytes = recv_test_frame(accepted)?;
                    let request: Value = serde_json::from_slice(&request_bytes)
                        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                    request_tx
                        .send(request)
                        .expect("send request to test thread");
                    send_test_frame(accepted, &reply)
                })();
                close(accepted).expect("close accepted socket");
                exchange.expect("mock daemon exchange");
            });
            (join, request_rx)
        });

        let context = Context {
            manifest_path: manifest_path.clone(),
            bundle_path: bundle_path.clone(),
            public_socket: socket_path.clone(),
            broker_socket: PathBuf::from("/dev/null"),
            state_root: None,
            host_runtime_path: PathBuf::from("/dev/null"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: PathBuf::from("/dev/null"),
            metrics_url: "http://127.0.0.1:1/metrics".to_owned(),
        };

        let (result, stdout) = super::with_test_stdout_capture(|| {
            let _staging_guard =
                EnvVarGuard::set("NIXLING_CONFIG_STAGING_DIR", staging_dir.as_os_str());
            super::cmd_config_sync(&context, &args)
        });

        let recorded = server.map(|(join, request_rx)| {
            let request = request_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("receive daemon request");
            join.join().expect("join mock daemon thread");
            request
        });

        let _ = std::fs::remove_file(&socket_path);
        let _ = std::fs::remove_file(&manifest_path);
        let _ = std::fs::remove_file(&bundle_path);
        if let Some(name) = bundle_path.file_name().and_then(|n| n.to_str()) {
            if let Some(parent) = bundle_path.parent() {
                let _ = std::fs::remove_file(parent.join(format!("{name}.processes.json")));
            }
        }
        let _ = std::fs::remove_dir_all(&staging_dir);
        (result, recorded, stdout)
    }

    #[test]
    fn config_sync_dry_run_uses_guest_control_transport_and_reads_no_bytes() {
        let args = super::ConfigSyncArgs {
            dry_run: true,
            json: true,
            ..gc_sync_args("work-aad")
        };
        // serve = None: no socket, no server. A dry-run must select the
        // guest-control transport WITHOUT connecting or reading guest bytes.
        let (result, recorded, stdout) = run_config_sync_with_mock_daemon(args, None);
        assert_eq!(result.expect("dry-run succeeds"), 0);
        assert!(recorded.is_none(), "dry-run must not contact the daemon");
        let body: Value = serde_json::from_slice(&stdout).expect("dry-run json");
        assert_eq!(
            body.get("transport").and_then(Value::as_str),
            Some("guest-control")
        );
        assert_eq!(body.get("mode").and_then(Value::as_str), Some("dry-run"));
        let rendered = String::from_utf8_lossy(&stdout);
        // No SSH argv and no guest content may appear in a dry-run.
        assert!(!rendered.contains("ssh"));
        assert!(!rendered.contains("sudo"));
        assert!(!rendered.contains("contentBase64"));
    }

    #[test]
    fn config_sync_end_to_end_success_stages_received_bytes() {
        let content = b"{ environment.systemPackages = [ ]; }\n";
        let reply = read_guest_config_reply(content);
        let args = gc_sync_args("work-aad");
        let (result, recorded, stdout) = run_config_sync_with_mock_daemon(args, Some(reply));
        assert_eq!(result.expect("config sync succeeds"), 0);
        let request = recorded.expect("server recorded a request");
        assert_eq!(
            request.get("type").and_then(Value::as_str),
            Some("readGuestConfig")
        );
        assert_eq!(request.get("vm").and_then(Value::as_str), Some("work-aad"));
        let rendered = String::from_utf8_lossy(&stdout);
        assert!(rendered.contains("guest-control"));
        // The success summary reports byte count + sha256 but never the
        // raw guest config body.
        assert!(!rendered.contains("systemPackages"));
    }

    #[test]
    fn config_sync_end_to_end_failure_matrix_never_stages_or_leaks() {
        for kind in [
            "guest-control-transport-unavailable",
            "guest-control-auth-failed",
            "guest-control-protocol-error",
            "guest-control-capability-unavailable",
            "guest-control-file-not-found",
            "guest-control-file-too-large",
            "guest-control-path-unsafe",
            "guest-control-read-denied",
            "guest-control-timeout",
        ] {
            let reply = guest_control_error_reply(kind);
            let args = super::ConfigSyncArgs {
                json: true,
                ..gc_sync_args("work-aad")
            };
            let (result, recorded, _stdout) = run_config_sync_with_mock_daemon(args, Some(reply));
            let err = result.expect_err(&format!("kind {kind} must fail"));
            assert_eq!(err.exit_code, 70, "kind {kind} maps to exit 70");
            assert!(recorded.is_some(), "kind {kind} reached the daemon");
            let rendered = err.rendered_stderr.unwrap_or_default();
            assert!(rendered.contains(kind), "kind {kind} surfaces its slug");
            // No guest bytes, paths, or transport detail in the error.
            assert!(!rendered.contains("systemPackages"));
            assert!(!rendered.contains("contentBase64"));
        }
    }

    #[test]
    fn config_sync_daemon_unavailable_returns_transport_unavailable() {
        let args = super::ConfigSyncArgs {
            json: true,
            ..gc_sync_args("work-aad")
        };
        // serve = None: the socket file is absent, so the daemon is
        // unavailable and no guest bytes are read.
        let (result, recorded, _stdout) = run_config_sync_with_mock_daemon(args, None);
        let err = result.expect_err("missing daemon socket must fail");
        assert_eq!(err.exit_code, 70);
        assert!(recorded.is_none());
        let rendered = err.rendered_stderr.unwrap_or_default();
        assert!(rendered.contains("guest-control-transport-unavailable"));
    }

    fn run_host_install_with_mock_daemon(
        args: HostInstallArgs,
        response: Value,
    ) -> (Result<i32, super::CliFailure>, Value) {
        let socket_path = test_socket_path("host-install", ".sock");
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent).expect("create test socket dir");
        }
        let _ = std::fs::remove_file(&socket_path);
        let listener = socket(
            AddressFamily::Unix,
            SockType::SeqPacket,
            SockFlag::SOCK_CLOEXEC,
            None,
        )
        .expect("listener socket");
        let addr = UnixAddr::new(&socket_path).expect("unix addr");
        bind(listener.as_raw_fd(), &addr).expect("bind listener");
        listen(&listener, Backlog::new(1).expect("backlog")).expect("listen");

        let (request_tx, request_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let accepted = accept4(listener.as_raw_fd(), SockFlag::SOCK_CLOEXEC).expect("accept");
            let exchange_result = (|| -> io::Result<()> {
                let hello_bytes = recv_test_frame(accepted)?;
                let hello: Value = serde_json::from_slice(&hello_bytes)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                assert_eq!(hello.get("type").and_then(Value::as_str), Some("hello"));

                let hello_reply = encode_type_tagged_message(
                    "helloOk",
                    &IpcHelloOk {
                        server_version: Version::new("0.4.0").expect("server version"),
                        selected_version: Version::new("0.4.0").expect("selected version"),
                        capabilities: daemon_supported_features(),
                    },
                    "test hello reply",
                )
                .expect("encode hello reply");
                send_test_frame(accepted, &hello_reply)?;

                let request_bytes = recv_test_frame(accepted)?;
                let request: Value = serde_json::from_slice(&request_bytes)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                request_tx
                    .send(request)
                    .expect("send request to test thread");

                let response_bytes = serde_json::to_vec(&response)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                send_test_frame(accepted, &response_bytes)
            })();
            close(accepted).expect("close accepted socket");
            exchange_result.expect("mock daemon exchange");
        });

        let context = Context {
            manifest_path: PathBuf::from("/dev/null"),
            bundle_path: PathBuf::from("/dev/null"),
            public_socket: socket_path.clone(),
            broker_socket: PathBuf::from("/dev/null"),
            state_root: None,
            host_runtime_path: PathBuf::from("/dev/null"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: PathBuf::from("/dev/null"),
            metrics_url: "http://127.0.0.1:1/metrics".to_owned(),
        };
        let original_args = host_install_original_args(&args);
        let result = cmd_host_install(&context, &args, &original_args);
        let request = request_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("receive daemon request");
        server.join().expect("join mock daemon thread");
        let _ = std::fs::remove_file(&socket_path);
        (result, request)
    }

    #[test]
    fn host_install_apply_dispatches_host_install_request_frame_under_native_only() {
        let _env_lock = ENV_MUTEX.lock().expect("lock env mutex");
        let args = HostInstallArgs {
            dry_run: false,
            apply: true,
            enable: true,
            start: false,
            no_start: true,
            json: false,
            human: false,
        };
        let (result, request) = run_host_install_with_mock_daemon(
            args,
            json!({
                "type": "mutatingVerbResponse",
                "verb": "host install",
                "outcome": "applied",
                "summary": "host install ok",
            }),
        );

        assert_eq!(result.expect("host install result"), 0);
        assert_eq!(
            request.get("type").and_then(Value::as_str),
            Some("hostInstall")
        );
        assert_eq!(request.get("apply").and_then(Value::as_bool), Some(true));
        assert_eq!(request.get("dryRun").and_then(Value::as_bool), Some(false));
        assert_eq!(request.get("json").and_then(Value::as_bool), Some(false));
        assert_eq!(request.get("enable").and_then(Value::as_bool), Some(true));
        assert_eq!(request.get("start").and_then(Value::as_bool), Some(false));
        assert_eq!(request.get("noStart").and_then(Value::as_bool), Some(true));
    }

    #[test]
    fn native_help_requests_parse_natively_through_clap() {
        // `should_fallback_to_legacy` was deleted. The equivalent
        // invariant is now "clap accepts every help
        // request for a native root command without parse error".
        // We assert via `NativeCli::try_parse_from` directly.
        for argv in [
            vec!["nixling", "host", "--help"],
            vec!["nixling", "host", "help"],
            vec!["nixling", "vm", "--help"],
            vec!["nixling", "vm", "help"],
            vec!["nixling", "audio", "--help"],
            vec!["nixling", "help", "audio"],
            vec!["nixling", "console", "--help"],
            vec!["nixling", "up", "--help"],
            vec!["nixling", "down", "--help"],
            vec!["nixling", "restart", "--help"],
            vec!["nixling", "help", "up"],
            vec!["nixling", "help", "down"],
            vec!["nixling", "help", "restart"],
        ] {
            // clap's `--help` short-circuits with a `DisplayHelp`
            // error kind; either Ok or DisplayHelp is acceptable —
            // anything else means we lost native help routing.
            match NativeCli::try_parse_from(argv.clone()) {
                Ok(_) => {}
                Err(err) => {
                    let kind = err.kind();
                    assert!(
                        matches!(
                            kind,
                            clap::error::ErrorKind::DisplayHelp
                                | clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
                                | clap::error::ErrorKind::DisplayVersion
                        ),
                        "expected clap help/version short-circuit for {:?}, got {:?}",
                        argv,
                        kind
                    );
                }
            }
        }
    }

    #[test]
    fn audio_and_console_commands_parse_natively() {
        let audio = NativeCli::try_parse_from(["nixling", "audio", "mic", "on", "personal-dev"])
            .expect("audio parse");
        assert!(matches!(
            audio.command,
            super::NativeCommand::Audio(super::AudioArgs {
                command: Some(super::AudioCommand::Mic(super::AudioToggleArgs {
                    state: super::AudioGrantState::On,
                    vm,
                })),
            }) if vm == "personal-dev"
        ));

        let audio_default =
            NativeCli::try_parse_from(["nixling", "audio"]).expect("audio status parse");
        assert!(matches!(
            audio_default.command,
            super::NativeCommand::Audio(super::AudioArgs { command: None })
        ));

        let console = NativeCli::try_parse_from(["nixling", "console", "personal-dev"])
            .expect("console parse");
        assert!(matches!(
            console.command,
            super::NativeCommand::Console(super::ConsoleArgs { vm }) if vm == "personal-dev"
        ));
    }

    #[test]
    fn broker_error_envelope_uses_per_kind_redaction_when_kind_is_present() {
        let envelope = broker_error_envelope(
            "host install",
            Some("RunHostInstall failed"),
            Some("W15"),
            Some("Broker.ValidateBundleFailed"),
            Some("raw remediation should not win"),
        );

        assert_eq!(
            envelope.kind,
            "nixling host install --apply failed: trusted bundle validation failed"
        );
        assert_eq!(
            envelope.observed_state,
            "The daemon reached the broker, but trusted bundle validation failed before the live handler ran."
        );
        assert_eq!(
            envelope.remediation,
            "trusted bundle validation failed; Admin: re-render the bundle and retry."
        );
    }

    #[test]
    fn broker_error_envelope_keeps_daemon_summary_when_kind_is_absent() {
        let envelope = broker_error_envelope(
            "host install",
            Some("RunHostInstall failed"),
            Some("W15"),
            None,
            Some("generic remediation"),
        );

        assert_eq!(envelope.kind, "RunHostInstall failed");
        assert!(envelope
            .observed_state
            .contains("operation not yet implemented in this build"));
        assert_eq!(envelope.remediation, "generic remediation");
    }

    #[test]
    fn host_install_broker_error_returns_exit_78_without_bash_fallback() {
        let _env_lock = ENV_MUTEX.lock().expect("lock env mutex");
        let args = HostInstallArgs {
            dry_run: false,
            apply: true,
            enable: false,
            start: false,
            no_start: false,
            json: false,
            human: false,
        };
        let (result, request) = run_host_install_with_mock_daemon(
            args,
            json!({
                "type": "mutatingVerbResponse",
                "verb": "host install",
                "outcome": "broker-error",
                "targetWave": "W15",
                "summary": "RunHostInstall failed",
                "remediation": "RunHostInstall failed at the broker live handler. Admin: inspect `journalctl -u nixling-priv-broker` for the underlying syscall/exit code.",
            }),
        );

        assert_eq!(result.expect("host install result"), 78);
        assert_eq!(
            request.get("type").and_then(Value::as_str),
            Some("hostInstall")
        );
        assert_eq!(request.get("apply").and_then(Value::as_bool), Some(true));
    }

    #[test]
    fn host_install_authz_not_admin_error_uses_typed_envelope() {
        let _env_lock = ENV_MUTEX.lock().expect("lock env mutex");
        let args = HostInstallArgs {
            dry_run: false,
            apply: true,
            enable: false,
            start: false,
            no_start: false,
            json: false,
            human: false,
        };
        let (result, _request) = run_host_install_with_mock_daemon(
            args,
            json!({
                "type": "error",
                "error": {
                    "kind": "authz-not-admin",
                    "exitCode": 75,
                    "message": "hostInstall requires an admin role from nixling.site.adminUsers",
                    "remediation": "add the caller to nixling.site.adminUsers to use hostInstall"
                }
            }),
        );

        let err = result.expect_err("host install must surface the daemon authz envelope");
        assert_eq!(err.exit_code, 75);
        assert_eq!(
            err.message,
            "authz-not-admin: hostInstall requires an admin role from nixling.site.adminUsers (add the caller to nixling.site.adminUsers to use hostInstall)"
        );
    }

    #[test]
    fn usb_attach_prevalidates_guest_import_before_daemon_dispatch() {
        let manifest_path = test_socket_path("usb-attach-prevalidate", ".manifest.json");
        if let Some(parent) = manifest_path.parent() {
            std::fs::create_dir_all(parent).expect("create test manifest dir");
        }
        let vm = "unit-usb-missing-key";
        write_usb_attach_manifest(&manifest_path, vm);
        let context = Context {
            manifest_path: manifest_path.clone(),
            bundle_path: manifest_path.with_extension("bundle.json"),
            public_socket: test_socket_path("usb-attach-prevalidate", ".sock"),
            broker_socket: PathBuf::from("/dev/null"),
            state_root: None,
            host_runtime_path: PathBuf::from("/dev/null"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: PathBuf::from("/dev/null"),
            metrics_url: "http://127.0.0.1:1/metrics".to_owned(),
        };
        let args = UsbAttachArgs {
            vm: vm.to_owned(),
            busid: "1-2".to_owned(),
            dry_run: false,
            apply: true,
            json: true,
            human: false,
        };

        let err = super::cmd_usb_attach(&context, &args)
            .expect_err("missing key must fail before daemon dispatch");
        assert_eq!(err.exit_code, 1);
        let rendered = err.rendered_stderr.as_deref().unwrap_or("");
        assert!(rendered.contains("nixling usb attach --apply"));
        assert!(!rendered.contains("--key"));
        assert!(!rendered.contains("daemon-down"));

        let _ = std::fs::remove_file(&manifest_path);
    }

    #[test]
    fn start_apply_no_wait_api_exits_zero_on_process_alive() {
        let _env_lock = ENV_MUTEX.lock().expect("lock env mutex");
        let args = VmStartArgs {
            vm: "vm-a".to_owned(),
            dry_run: false,
            apply: true,
            no_wait_api: true,
            json: false,
            human: false,
        };
        let (result, request) = run_vm_start_with_mock_daemon(
            args,
            json!({
                "type": "mutatingVerbResponse",
                "verb": "vm start",
                "outcome": "applied",
                "summary": "vm.vm-a: process-alive: ok; api-ready: pending",
                "apiReady": "pending",
            }),
        );

        assert_eq!(result.expect("vm start result"), 0);
        assert_eq!(request.get("type").and_then(Value::as_str), Some("vmStart"));
        assert_eq!(request.get("apply").and_then(Value::as_bool), Some(true));
        assert_eq!(
            request.get("noWaitApi").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn start_apply_strict_default_exits_nonzero_on_api_timeout() {
        let _env_lock = ENV_MUTEX.lock().expect("lock env mutex");
        let args = VmStartArgs {
            vm: "vm-a".to_owned(),
            dry_run: false,
            apply: true,
            no_wait_api: false,
            json: false,
            human: false,
        };
        let (result, request) = run_vm_start_with_mock_daemon(
            args,
            json!({
                "type": "mutatingVerbResponse",
                "verb": "vm start",
                "outcome": "api-ready-timeout",
                "summary": "vm.vm-a: process-alive: ok; api-ready: timeout",
                "apiReady": "timeout",
            }),
        );

        assert_eq!(result.expect("vm start result"), super::EXIT_API_TIMEOUT);
        assert_eq!(request.get("type").and_then(Value::as_str), Some("vmStart"));
        assert!(request.get("noWaitApi").is_none());
    }

    #[test]
    fn vm_status_reads_api_ready_state_from_daemon_state_dir() {
        let counter = TEST_SOCKET_COUNTER.fetch_add(1, Ordering::Relaxed);
        let state_dir = std::env::current_dir()
            .expect("current dir")
            .join("target")
            .join(format!(
                "vm-status-api-ready-{}-{counter}",
                std::process::id()
            ));
        let vm_dir = state_dir.join("vm-a");
        std::fs::create_dir_all(&vm_dir).expect("create vm state dir");
        std::fs::write(vm_dir.join("api-ready.json"), br#"{"apiReady":"timeout"}"#)
            .expect("write api-ready.json");

        let manifest_path = state_dir.join("vms.json");
        write_test_manifest(&manifest_path, "vm-a");
        let context = Context {
            manifest_path: manifest_path.clone(),
            bundle_path: manifest_path.with_extension("bundle.json"),
            public_socket: PathBuf::from("/dev/null"),
            broker_socket: PathBuf::from("/dev/null"),
            state_root: None,
            host_runtime_path: PathBuf::from("/dev/null"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: state_dir.clone(),
            metrics_url: "http://127.0.0.1:1/metrics".to_owned(),
        };
        let manifest_bytes = std::fs::read(&manifest_path).expect("read manifest");
        let manifest: std::collections::BTreeMap<String, super::ManifestVm> =
            serde_json::from_slice(&manifest_bytes).expect("parse manifest");
        let vm = manifest.get("vm-a").expect("vm-a in manifest");
        let output = super::build_vm_status_output(&context, vm, None);

        assert_eq!(
            output.api_ready,
            Some(ApiReadyStatusV1::Simple(ApiReadySimple::Timeout))
        );
        // Verify it also serialises correctly (regression guard).
        let value = serde_json::to_value(&output).expect("serialize vm status");
        assert_eq!(
            value.get("apiReady").and_then(Value::as_str),
            Some("timeout")
        );

        // Cleanup.
        let _ = std::fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn vm_status_reports_live_pool_integrity_ok() {
        let counter = TEST_SOCKET_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::current_dir()
            .expect("current dir")
            .join("target")
            .join(format!(
                "vm-status-live-pool-{}-{counter}",
                std::process::id()
            ));
        let manifest_path = root.join("vms.json");
        std::fs::create_dir_all(&root).expect("create status root");
        write_test_manifest(&manifest_path, "vm-a");
        let vm_root = root.join("vm-a");
        let store_view = vm_root.join("store-view");
        let generation_id = "g-test";
        std::fs::create_dir_all(store_view.join("state/generations").join(generation_id))
            .expect("create state generation");
        std::fs::create_dir_all(store_view.join("live")).expect("create live");
        std::os::unix::fs::symlink(
            format!("generations/{generation_id}"),
            store_view.join("state/current"),
        )
        .expect("state current symlink");
        std::fs::write(store_view.join("live/.nixling-marker-vm-a"), b"")
            .expect("write zero marker");
        std::fs::write(
            store_view
                .join("state/generations")
                .join(generation_id)
                .join("integrity.json"),
            br#"{"generation_id":"g-test","state":"ok","repair_attempted":false}"#,
        )
        .expect("write integrity");
        let context = Context {
            manifest_path: manifest_path.clone(),
            bundle_path: manifest_path.with_extension("bundle.json"),
            public_socket: PathBuf::from("/dev/null"),
            broker_socket: PathBuf::from("/dev/null"),
            state_root: Some(root.clone()),
            host_runtime_path: PathBuf::from("/dev/null"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: root.join("daemon-state"),
            metrics_url: "http://127.0.0.1:1/metrics".to_owned(),
        };
        let manifest_bytes = std::fs::read(&manifest_path).expect("read manifest");
        let manifest: std::collections::BTreeMap<String, super::ManifestVm> =
            serde_json::from_slice(&manifest_bytes).expect("parse manifest");
        let vm = manifest.get("vm-a").expect("vm-a in manifest");
        let output = super::build_vm_status_output(&context, vm, None);

        let integrity = output
            .live_pool_integrity
            .expect("live pool integrity is reported");
        assert_eq!(integrity.status, "ok");
        assert_eq!(integrity.remediation, None);
        let value = serde_json::to_value(integrity).expect("serialize integrity");
        assert_eq!(value.get("status").and_then(Value::as_str), Some("ok"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn status_human_redacts_ssh_target_details() {
        let output = super::StatusVmOutputV2 {
            name: "vm-a".to_owned(),
            env: Some("dev".to_owned()),
            services: super::StatusServicesOutputV2 {
                nixling: "inactive".to_owned(),
                microvm: "inactive".to_owned(),
                virtiofsd: "inactive".to_owned(),
                gpu: Some("stopped".to_owned()),
                video: None,
                snd: None,
                swtpm: None,
            },
            current: None,
            booted: None,
            pending_restart: false,
            runtime: super::RUNTIME_UNKNOWN.to_owned(),
            declared_roles: vec!["gpu".to_owned()],
            readiness: Vec::new(),
            api_ready: None,
            runner_parity: None,
            live_pool_integrity: None,
        };
        let manifest_vm = super::ManifestVm {
            name: "vm-a".to_owned(),
            env: Some("dev".to_owned()),
            graphics: true,
            tpm: false,
            audio: false,
            usbip_yubikey: false,
            static_ip: Some("10.20.0.10".to_owned()),
            usbipd_host_ip: Some("192.0.2.1".to_owned()),
            is_net_vm: false,
            state_dir: "/var/lib/nixling/vms/vm-a".to_owned(),
            bridge: "nl-dev".to_owned(),
            ssh_user: Some("alice".to_owned()),
        };
        let rendered = super::render_status_vm_human(&output, &manifest_vm, Vec::new());
        assert!(rendered.contains("ssh: declared"));
        assert!(!rendered.contains("alice@"));
        assert!(!rendered.contains("10.20.0.10"));
        assert!(!rendered.contains("video:"));
        assert!(!rendered.contains("video-disabled:"));
    }

    #[test]
    fn vm_service_states_use_pidfd_roles_for_daemon_only_runners() {
        fn role_profile() -> nixling_core::processes::RoleProfile {
            nixling_core::processes::RoleProfile {
                profile_id: "test-profile".to_owned(),
                uid: 1000,
                gid: 1000,
                adr_carve_out: None,
                caps: Vec::new(),
                namespaces: nixling_core::minijail_profile::NamespaceSet {
                    mount: false,
                    pid: false,
                    net: false,
                    ipc: false,
                    uts: false,
                    user: false,
                },
                seccomp_policy_ref: None,
                mount_policy: nixling_core::minijail_profile::MountPolicy {
                    read_only_paths: Vec::new(),
                    writable_paths: Vec::new(),
                    device_binds: Vec::new(),
                    bind_mounts: Vec::new(),
                    nix_store_read_only: true,
                    hide_device_nodes_by_default: true,
                },
                cgroup_placement: nixling_core::minijail_profile::CgroupPlacement {
                    subtree: "nixling.slice/test".to_owned(),
                    controllers: Vec::new(),
                    delegated: false,
                },
                user_namespace: None,
                umask: None,
            }
        }

        let state_dir = test_socket_path("pidfd-status-running", "");
        std::fs::create_dir_all(&state_dir).expect("create daemon state dir");
        std::fs::write(
            state_dir.join("pidfd-table.json"),
            br#"{"entries":[
              {"vm":"vm-a","role":"ch-runner","pid":11,"startTimeTicks":1},
              {"vm":"vm-a","role":"virtiofsd-ro-store","pid":12,"startTimeTicks":1},
              {"vm":"vm-a","role":"gpu","pid":13,"startTimeTicks":1},
              {"vm":"vm-a","role":"video","pid":14,"startTimeTicks":1},
              {"vm":"vm-a","role":"audio","pid":15,"startTimeTicks":1}
            ]}"#,
        )
        .expect("write pidfd table");
        let context = Context {
            manifest_path: PathBuf::from("/dev/null"),
            bundle_path: PathBuf::from("/dev/null"),
            public_socket: PathBuf::from("/dev/null"),
            broker_socket: PathBuf::from("/dev/null"),
            state_root: None,
            host_runtime_path: PathBuf::from("/dev/null"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: state_dir.clone(),
            metrics_url: "http://127.0.0.1:1/metrics".to_owned(),
        };
        let vm = super::ManifestVm {
            name: "vm-a".to_owned(),
            env: Some("dev".to_owned()),
            graphics: true,
            tpm: false,
            audio: true,
            usbip_yubikey: false,
            static_ip: None,
            usbipd_host_ip: None,
            is_net_vm: false,
            state_dir: "/var/lib/nixling/vms/vm-a".to_owned(),
            bridge: "nl-dev".to_owned(),
            ssh_user: None,
        };
        let dag = nixling_core::processes::VmProcessDag {
            vm: "vm-a".to_owned(),
            nodes: vec![
                nixling_core::processes::ProcessNode {
                    id: nixling_core::processes::NodeId("ch-runner".to_owned()),
                    role: nixling_core::processes::ProcessRole::CloudHypervisorRunner,
                    unit: None,
                    binary_path: None,
                    argv: Vec::new(),
                    env: Vec::new(),
                    profile: role_profile(),
                    readiness: Vec::new(),
                    plan_ops: Vec::new(),
                },
                nixling_core::processes::ProcessNode {
                    id: nixling_core::processes::NodeId("video".to_owned()),
                    role: nixling_core::processes::ProcessRole::Video,
                    unit: None,
                    binary_path: None,
                    argv: Vec::new(),
                    env: Vec::new(),
                    profile: role_profile(),
                    readiness: Vec::new(),
                    plan_ops: Vec::new(),
                },
            ],
            edges: Vec::new(),
            invariants: nixling_core::processes::VmProcessInvariants {
                swtpm_pre_start_flush: false,
                per_vm_audit_pipeline: false,
                usbip_gating: false,
                tpm_ownership_migration_without_running_vm_mutation: false,
            },
        };
        let services = super::vm_service_states(&context, &vm, Some(&dag));
        assert_eq!(services.microvm, "running");
        assert_eq!(services.virtiofsd, "running");
        assert_eq!(services.gpu.as_deref(), Some("running"));
        assert_eq!(services.video.as_deref(), Some("running"));
        assert_eq!(services.snd.as_deref(), Some("running"));
        assert_eq!(super::list_status_label(&vm, &services, false), "running");
        let _ = std::fs::remove_dir_all(&state_dir);
    }

    #[test]
    #[cfg(unix)]
    fn pidfd_role_state_unreadable_returns_unknown_not_stopped() {
        let state_dir = test_socket_path("pidfd-unreadable", "");
        std::fs::create_dir_all(&state_dir).expect("create daemon state dir");
        std::fs::write(
            state_dir.join("pidfd-table.json"),
            br#"{"entries":[{"vm":"vm-a","role":"video","pid":123,"startTimeTicks":1}]}"#,
        )
        .expect("write pidfd table");
        let mut perms = std::fs::metadata(&state_dir)
            .expect("stat daemon state dir")
            .permissions();
        perms.set_mode(0o000);
        std::fs::set_permissions(&state_dir, perms).expect("make daemon state dir unreadable");

        let context = Context {
            manifest_path: PathBuf::from("/dev/null"),
            bundle_path: PathBuf::from("/dev/null"),
            public_socket: PathBuf::from("/dev/null"),
            broker_socket: PathBuf::from("/dev/null"),
            state_root: None,
            host_runtime_path: PathBuf::from("/dev/null"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: state_dir.clone(),
            metrics_url: "http://127.0.0.1:1/metrics".to_owned(),
        };

        let state = super::pidfd_role_state(&context, "vm-a", "video");

        let mut cleanup_perms = std::fs::metadata(&state_dir)
            .expect("stat daemon state dir for cleanup")
            .permissions();
        cleanup_perms.set_mode(0o700);
        let _ = std::fs::set_permissions(&state_dir, cleanup_perms);
        let _ = std::fs::remove_dir_all(&state_dir);

        if nix::unistd::Uid::effective().is_root() {
            assert_eq!(state, "running");
        } else {
            assert_eq!(state, "unknown");
        }
    }

    #[test]
    #[cfg(unix)]
    fn load_bundle_context_unreadable_path_returns_error_not_none() {
        let parent = test_socket_path("bundle-unreadable", "");
        std::fs::create_dir_all(&parent).expect("create bundle parent");
        let mut perms = std::fs::metadata(&parent)
            .expect("stat bundle parent")
            .permissions();
        perms.set_mode(0o000);
        std::fs::set_permissions(&parent, perms).expect("make bundle parent unreadable");

        let context = Context {
            manifest_path: PathBuf::from("/dev/null"),
            bundle_path: parent.join("bundle.json"),
            public_socket: PathBuf::from("/dev/null"),
            broker_socket: PathBuf::from("/dev/null"),
            state_root: None,
            host_runtime_path: PathBuf::from("/dev/null"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: PathBuf::from("/dev/null"),
            metrics_url: "http://127.0.0.1:1/metrics".to_owned(),
        };

        let result = context.load_bundle_context();

        let mut cleanup_perms = std::fs::metadata(&parent)
            .expect("stat bundle parent for cleanup")
            .permissions();
        cleanup_perms.set_mode(0o700);
        let _ = std::fs::set_permissions(&parent, cleanup_perms);
        let _ = std::fs::remove_dir_all(&parent);

        if nix::unistd::Uid::effective().is_root() {
            assert!(matches!(result, Ok(None)));
        } else {
            let err = result.expect_err("unreadable bundle path must error");
            assert!(err.message.contains("failed to inspect"));
        }
    }

    #[test]
    fn vm_status_subcommand_parses_natively() {
        let cli = NativeCli::try_parse_from(["nixling", "vm", "status", "vm-a"])
            .expect("vm status parse");
        assert!(matches!(
            cli.command,
            super::NativeCommand::Vm(super::VmArgs {
                command: super::VmCommand::Status(super::VmStatusArgs { vm, .. }),
            }) if vm == "vm-a"
        ));
    }

    #[test]
    fn daemon_mutating_verb_frame_serializes_host_install_flags() {
        let payload = super::daemon_mutating_verb_frame(
            "hostInstall",
            json!({
                "enable": true,
                "start": false,
                "noStart": true,
            }),
            false,
            true,
            false,
        )
        .expect("serialize hostInstall frame");
        let value: Value = serde_json::from_slice(&payload).expect("parse frame");

        assert_eq!(
            value.get("type").and_then(Value::as_str),
            Some("hostInstall")
        );
        assert_eq!(value.get("apply").and_then(Value::as_bool), Some(true));
        assert_eq!(value.get("enable").and_then(Value::as_bool), Some(true));
        assert_eq!(value.get("noStart").and_then(Value::as_bool), Some(true));
    }
}

#[cfg(test)]
mod exec_json_envelope_tests {
    //! The `vm exec --json` envelope disambiguates a guest exit code from a
    //! transport/old-generation failure that happens to share the same shell
    //! status number (the 70-vs-70 case): `source` + `reason` +
    //! `guestExitCode`/`transportExitCode` carry the distinction.

    use nixling_ipc::public_wire::ExecTerminalStatus;

    use super::{exec_client, exec_json_failure_value, exec_json_success_value, VmExecArgs};

    fn exec_args(vm: &str) -> VmExecArgs {
        VmExecArgs {
            vm: vm.to_owned(),
            detach: false,
            interactive: false,
            tty: false,
            env: Vec::new(),
            cwd: None,
            json: true,
            human: false,
            management: Vec::new(),
            command: vec!["true".to_owned()],
        }
    }

    #[test]
    fn guest_exit_70_envelope_is_sourced_to_the_guest() {
        let args = exec_args("work");
        let outcome = exec_client::ExecOutcome {
            terminal: ExecTerminalStatus::Exited { code: 70 },
        };
        let host = exec_client::CapturingHostIo::new(false, 1024);
        let (value, exit_code) = exec_json_success_value(&args, &outcome, &host);
        assert_eq!(exit_code, 70);
        assert_eq!(value["source"], "guest");
        assert_eq!(value["reason"], "exited");
        assert_eq!(value["guestExitCode"], 70);
        assert_eq!(value["exitCode"], 70);
        // A success envelope never carries a transportExitCode.
        assert!(value.get("transportExitCode").is_none());
    }

    #[test]
    fn old_generation_70_envelope_is_sourced_to_guest_control() {
        let args = exec_args("work");
        let error = exec_client::ExecClientError::from_daemon_error(
            "guest-control-unavailable-old-generation",
            "this VM generation does not support guest-control exec",
            "rebuild the VM with a current nixling generation",
        );
        assert_eq!(error.exit_code, 70);
        let value = exec_json_failure_value(&args, &error);
        assert_eq!(value["source"], "guest-control");
        assert_eq!(value["reason"], "guest-control-unavailable-old-generation");
        assert_eq!(value["exitCode"], 70);
        assert_eq!(value["transportExitCode"], 70);
        // A failure envelope never carries a guestExitCode.
        assert!(value.get("guestExitCode").is_none());
        // A failure envelope never carries captured stdio bytes.
        assert!(value.get("stdoutBase64").is_none());
        assert!(value.get("stderrBase64").is_none());
    }
}

#[cfg(test)]
mod config_cmd_tests {
    //! Host-side review/approve logic for `nixling config`. The SSH
    //! `sync` path needs a live VM (Layer-2); these unit tests cover
    //! the pure file-op core + the input validators.

    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{
        config_approve_core, config_atomic_write, config_reject_core, config_staging_path_in,
        config_sync_capture_to_staging, config_sync_capture_to_staging_limited,
        config_sync_ssh_argv, config_validate_remote_path, config_validate_staging_bytes,
        config_validate_vm_name,
    };

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn scratch(name: &str) -> PathBuf {
        let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::current_dir()
            .expect("cwd")
            .join("target")
            .join(format!(
                "config-cmd-{name}-{}-{counter}",
                std::process::id()
            ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch");
        dir
    }

    #[test]
    fn vm_name_validation_blocks_traversal_and_bad_shapes() {
        assert!(config_validate_vm_name("work-aad").is_ok());
        assert!(config_validate_vm_name("personal-dev").is_ok());
        assert!(config_validate_vm_name("a1").is_ok());
        for bad in [
            "", "../x", "..", "Work", "a/b", "1abc", "a_b", "a b", "sys/..",
        ] {
            assert!(
                config_validate_vm_name(bad).is_err(),
                "expected '{bad}' to be rejected"
            );
        }
    }

    #[test]
    fn remote_path_validation_requires_absolute_safe_path() {
        assert!(config_validate_remote_path("/var/lib/nixling-guest/guest-config.nix").is_ok());
        assert!(config_validate_remote_path("/etc/nixling/guest-config.nix").is_ok());
        for bad in [
            "guest.nix",   // not absolute
            "/a;rm -rf /", // shell metachar
            "/a b",        // space
            "/a$(x)",      // command substitution
            "/a`x`",       // backtick
            "/a\nb",       // newline
            "/a|b",        // pipe
        ] {
            assert!(
                config_validate_remote_path(bad).is_err(),
                "expected '{bad}' to be rejected"
            );
        }
    }

    #[test]
    fn staging_bytes_validation_rejects_empty_and_blank_and_non_utf8() {
        assert!(config_validate_staging_bytes(b"{ environment.systemPackages = []; }").is_ok());
        assert!(config_validate_staging_bytes(b"").is_err());
        assert!(config_validate_staging_bytes(b"   \n\t  ").is_err());
        assert!(config_validate_staging_bytes(&[0xff, 0xfe, 0x00]).is_err());
    }

    #[test]
    fn approve_writes_staging_to_target_atomically_and_clears_staging() {
        let dir = scratch("approve-ok");
        let staging = config_staging_path_in(&dir, "work-aad");
        let target = dir.join("work.guest.nix");
        let content = b"{ environment.systemPackages = [ ]; }\n";
        fs::write(&staging, content).expect("write staging");
        fs::write(&target, b"{ }\n").expect("seed target");

        let n = config_approve_core(&staging, &target).expect("approve ok");
        assert_eq!(n, content.len());
        assert_eq!(fs::read(&target).expect("read target"), content);
        // staging consumed
        assert!(!staging.exists());
        // no temp turds left behind (impl writes `.<base>.nixling-tmp.*`)
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("nixling-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "approve left a temp file behind");
    }

    #[test]
    fn atomic_write_publishes_whole_value_under_concurrency() {
        // Prove rename-atomicity: many threads each publish a DISTINCT
        // full value to the same target; the final target must equal
        // exactly ONE complete value (never a torn/mixed/truncated
        // result), and no temp files may be left behind.
        let dir = scratch("atomic-race");
        let target = dir.join("work.guest.nix");
        let values: Vec<Vec<u8>> = (0..16)
            .map(|i| format!("{{ environment.systemPackages = [ \"pkg-{i}\" ]; }}\n").into_bytes())
            .collect();
        let target_for = target.clone();
        let mut handles = Vec::new();
        for v in values.clone() {
            let t = target_for.clone();
            handles.push(std::thread::spawn(move || {
                config_atomic_write(&t, &v).expect("atomic write");
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let final_bytes = fs::read(&target).expect("read target");
        assert!(
            values.iter().any(|v| v == &final_bytes),
            "target was torn/mixed: not equal to any single complete value"
        );
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("nixling-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "atomic write left a temp file behind");
    }

    #[test]
    fn approve_errors_when_nothing_staged() {
        let dir = scratch("approve-nostage");
        let staging = config_staging_path_in(&dir, "work-aad");
        let target = dir.join("work.guest.nix");
        fs::write(&target, b"{ }\n").expect("seed target");
        let err = config_approve_core(&staging, &target).expect_err("must error");
        assert!(err.message.contains("nothing staged"));
        // target untouched
        assert_eq!(fs::read(&target).expect("read target"), b"{ }\n");
    }

    #[test]
    fn approve_refuses_empty_staging_and_leaves_target_intact() {
        let dir = scratch("approve-empty");
        let staging = config_staging_path_in(&dir, "work-aad");
        let target = dir.join("work.guest.nix");
        fs::write(&staging, b"").expect("write empty staging");
        fs::write(&target, b"{ keep = true; }\n").expect("seed target");
        let err = config_approve_core(&staging, &target).expect_err("must error on empty");
        assert!(err.message.contains("empty"));
        assert_eq!(
            fs::read(&target).expect("read target"),
            b"{ keep = true; }\n"
        );
    }

    #[test]
    fn approve_errors_when_target_dir_missing() {
        let dir = scratch("approve-nodir");
        let staging = config_staging_path_in(&dir, "work-aad");
        fs::write(&staging, b"{ ok = true; }\n").expect("write staging");
        let target = dir.join("does-not-exist").join("work.guest.nix");
        let err = config_approve_core(&staging, &target).expect_err("must error");
        assert!(err.message.contains("does not exist"));
        // staging preserved so the operator can retry
        assert!(staging.exists());
    }

    #[test]
    fn reject_removes_staging_and_reports_absence() {
        let dir = scratch("reject");
        let staging = config_staging_path_in(&dir, "work-aad");
        fs::write(&staging, b"{ }\n").expect("write staging");
        assert!(config_reject_core(&staging).expect("reject"));
        assert!(!staging.exists());
        // second reject: nothing to remove
        assert!(!config_reject_core(&staging).expect("reject-again"));
    }

    // Hermetic coverage of the real sync capture path via a fake `ssh`
    // script invoked through `/bin/sh` (read, not exec'd — avoids any
    // ETXTBSY race exec'ing a just-written binary under CI load).
    fn fake_ssh(dir: &std::path::Path, name: &str, body: &str) -> Vec<String> {
        let p = dir.join(name);
        fs::write(&p, body).expect("write fake ssh");
        vec!["/bin/sh".to_owned(), p.display().to_string()]
    }

    #[test]
    fn sync_capture_success_stages_stdout() {
        let dir = scratch("sync-ok");
        let mut argv = fake_ssh(
            &dir,
            "ssh",
            "printf '{ environment.systemPackages = []; }\\n'\n",
        );
        argv.push("ignored-arg".to_owned());
        let staging = dir.join("work-aad.guest.nix");
        let n = config_sync_capture_to_staging(&argv, &staging).expect("sync ok");
        assert_eq!(
            fs::read(&staging).unwrap(),
            b"{ environment.systemPackages = []; }\n"
        );
        assert_eq!(n, fs::read(&staging).unwrap().len());
    }

    #[test]
    fn sync_capture_nonzero_exit_errors_and_does_not_stage() {
        let dir = scratch("sync-fail");
        let argv = fake_ssh(&dir, "ssh", "echo 'permission denied' >&2\nexit 255\n");
        let staging = dir.join("work-aad.guest.nix");
        let err = config_sync_capture_to_staging(&argv, &staging).expect_err("must error");
        assert!(err.message.contains("exited 255"));
        assert!(!staging.exists(), "must not stage on ssh failure");
    }

    #[test]
    fn sync_capture_empty_stdout_is_rejected() {
        let dir = scratch("sync-empty");
        let argv = fake_ssh(&dir, "ssh", "exit 0\n");
        let staging = dir.join("work-aad.guest.nix");
        let err = config_sync_capture_to_staging(&argv, &staging).expect_err("empty rejected");
        assert!(err.message.contains("empty"));
        assert!(!staging.exists());
    }

    #[test]
    fn sync_capture_oversized_stdout_is_rejected_and_not_staged() {
        // A hostile guest streaming an unbounded file must be cut off by
        // the byte cap, not buffered until OOM, and must not stage.
        let dir = scratch("sync-oversized");
        // Emit ~2 MiB, well past the 1 MiB cap.
        let argv = fake_ssh(&dir, "ssh", "exec head -c 2097152 /dev/zero\n");
        let staging = dir.join("work-aad.guest.nix");
        let err = config_sync_capture_to_staging(&argv, &staging).expect_err("oversized rejected");
        assert!(
            err.message.contains("limit"),
            "expected a size-limit error, got: {}",
            err.message
        );
        assert!(!staging.exists(), "must not stage an oversized pull");
    }

    #[test]
    fn sync_capture_times_out_and_does_not_stage() {
        // A guest that stalls (writes nothing, never closes stdout) must
        // hit the wall-clock timeout, get killed, and not stage. Uses an
        // injected 200 ms timeout against a fake ssh that sleeps.
        let dir = scratch("sync-timeout");
        let argv = fake_ssh(&dir, "ssh", "exec sleep 30\n");
        let staging = dir.join("work-aad.guest.nix");
        let start = std::time::Instant::now();
        let err = config_sync_capture_to_staging_limited(
            &argv,
            &staging,
            1 << 20,
            std::time::Duration::from_millis(200),
        )
        .expect_err("must time out");
        assert!(
            err.message.contains("timed out"),
            "expected a timeout error, got: {}",
            err.message
        );
        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "timeout did not fire promptly"
        );
        assert!(!staging.exists(), "must not stage on timeout");
    }

    #[test]
    fn sync_capture_times_out_when_process_lingers_after_stdout_close() {
        // A hostile endpoint sends a small valid payload, CLOSES stdout
        // (EOF), then lingers. The deadline must cover the whole child
        // lifetime (not just the stdout read), so this still times out,
        // is killed, and does not stage.
        let dir = scratch("sync-linger");
        let argv = fake_ssh(
            &dir,
            "ssh",
            "printf '{ environment.systemPackages = []; }\\n'\nexec 1>&-\nsleep 30\n",
        );
        let staging = dir.join("work-aad.guest.nix");
        let start = std::time::Instant::now();
        let err = config_sync_capture_to_staging_limited(
            &argv,
            &staging,
            1 << 20,
            std::time::Duration::from_millis(200),
        )
        .expect_err("must time out on the lingering process");
        assert!(
            err.message.contains("timed out"),
            "expected a timeout error, got: {}",
            err.message
        );
        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "lingering-process timeout did not fire promptly"
        );
        assert!(!staging.exists(), "must not stage when the process lingers");
    }

    #[test]
    fn staging_path_in_is_per_vm() {
        let base = PathBuf::from("/x/state");
        assert_eq!(
            config_staging_path_in(&base, "work-aad"),
            PathBuf::from("/x/state/work-aad.guest.nix")
        );
    }

    #[test]
    fn sync_ssh_argv_remote_command_is_cat_after_destination() {
        let argv = config_sync_ssh_argv(
            &PathBuf::from("/var/lib/nixling/keys/work-aad_ed25519"),
            &PathBuf::from("/var/lib/nixling/known_hosts.nixling"),
            "alice@10.20.0.10",
            "/var/lib/nixling-guest/guest-config.nix",
        );
        assert_eq!(argv[0], "ssh");
        // key flag
        let i = argv.iter().position(|a| a == "-i").unwrap();
        assert_eq!(argv[i + 1], "/var/lib/nixling/keys/work-aad_ed25519");
        // host-key integrity: managed known_hosts + accept-new (NOT
        // StrictHostKeyChecking=no / UserKnownHostsFile=/dev/null).
        assert!(argv
            .iter()
            .any(|a| a == "UserKnownHostsFile=/var/lib/nixling/known_hosts.nixling"));
        assert!(argv.iter().any(|a| a == "StrictHostKeyChecking=accept-new"));
        assert!(!argv.iter().any(|a| a == "StrictHostKeyChecking=no"));
        assert!(!argv.iter().any(|a| a == "UserKnownHostsFile=/dev/null"));
        assert!(argv.iter().any(|a| a == "BatchMode=yes"));
        // No `--`: ssh would send it as part of the remote command.
        assert!(!argv.iter().any(|a| a == "--"), "`--` must not be present");
        // The remote command (everything after the destination) is
        // exactly `cat <guest_path>`.
        let target = argv.iter().position(|a| a == "alice@10.20.0.10").unwrap();
        assert_eq!(
            &argv[target + 1..],
            &["cat", "/var/lib/nixling-guest/guest-config.nix"]
        );
    }

    #[test]
    fn usb_guest_attach_ssh_argv_runs_guest_import_after_destination() {
        let argv = super::usb_guest_attach_ssh_argv(
            &PathBuf::from("/var/lib/nixling/keys/work-aad_ed25519"),
            &PathBuf::from("/var/lib/nixling/known_hosts.nixling"),
            "alice@10.20.0.10",
            "192.0.2.1",
            "1-2",
        );

        assert_eq!(argv[0], "ssh");
        let key_pos = argv.iter().position(|a| a == "-i").unwrap();
        assert_eq!(argv[key_pos + 1], "/var/lib/nixling/keys/work-aad_ed25519");
        assert!(argv
            .iter()
            .any(|a| a == "UserKnownHostsFile=/var/lib/nixling/known_hosts.nixling"));
        assert!(argv.iter().any(|a| a == "StrictHostKeyChecking=accept-new"));
        assert!(argv.iter().any(|a| a == "BatchMode=yes"));
        assert!(argv.iter().any(|a| a == "ConnectTimeout=8"));
        assert!(!argv.iter().any(|a| a == "StrictHostKeyChecking=no"));
        assert!(!argv.iter().any(|a| a == "UserKnownHostsFile=/dev/null"));

        let remote_pos = argv.iter().position(|a| a == "alice@10.20.0.10").unwrap();
        assert_eq!(
            &argv[remote_pos + 1..],
            &[
                "sudo",
                "-n",
                "usbip",
                "attach",
                "-r",
                "192.0.2.1",
                "-b",
                "1-2"
            ]
        );
    }

    fn gc_sync_args(vm: &str) -> super::ConfigSyncArgs {
        super::ConfigSyncArgs {
            vm: vm.to_owned(),
            guest_path: super::DEFAULT_GUEST_CONFIG_PATH.to_owned(),
            host: None,
            user: None,
            key: None,
            known_hosts: None,
            dry_run: false,
            json: false,
        }
    }

    #[test]
    fn ssh_only_flags_are_rejected_on_guest_control_path() {
        // Default args (no SSH overrides, default in-guest path) are accepted.
        assert!(super::reject_ssh_only_flags_on_guest_control(&gc_sync_args("work-aad")).is_ok());

        let with_host = super::ConfigSyncArgs {
            host: Some("10.0.0.5".to_owned()),
            ..gc_sync_args("work-aad")
        };
        assert!(super::reject_ssh_only_flags_on_guest_control(&with_host).is_err());

        let with_user = super::ConfigSyncArgs {
            user: Some("alice".to_owned()),
            ..gc_sync_args("work-aad")
        };
        assert!(super::reject_ssh_only_flags_on_guest_control(&with_user).is_err());

        let with_key = super::ConfigSyncArgs {
            key: Some(PathBuf::from("/tmp/k")),
            ..gc_sync_args("work-aad")
        };
        assert!(super::reject_ssh_only_flags_on_guest_control(&with_key).is_err());

        let with_known_hosts = super::ConfigSyncArgs {
            known_hosts: Some(PathBuf::from("/tmp/kh")),
            ..gc_sync_args("work-aad")
        };
        assert!(super::reject_ssh_only_flags_on_guest_control(&with_known_hosts).is_err());

        let with_custom_path = super::ConfigSyncArgs {
            guest_path: "/etc/other.nix".to_owned(),
            ..gc_sync_args("work-aad")
        };
        let err = super::reject_ssh_only_flags_on_guest_control(&with_custom_path)
            .expect_err("custom guest path must be rejected");
        // Flag rejection is a usage error, not a transport failure.
        assert_eq!(err.exit_code, 2);
    }

    fn read_guest_config_reply(content: &[u8]) -> Vec<u8> {
        let encoded = nixling_core::base64_codec::encode(content);
        serde_json::to_vec(&serde_json::json!({
            "type": "readGuestConfigResponse",
            "contentBase64": encoded,
        }))
        .expect("serialize reply")
    }

    fn guest_control_error_reply(kind: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "type": "error",
            "error": {
                "kind": kind,
                "exitCode": 70,
                "message": "guest-control read failed",
                "remediation": "retry after the guest finishes booting",
            },
        }))
        .expect("serialize error reply")
    }

    #[test]
    fn finish_config_sync_decodes_stages_and_hashes_received_bytes() {
        let dir = scratch("gc-sync-ok");
        let staging = dir.join("work.guest.nix");
        let content = b"{ environment.systemPackages = [ ]; }\n";
        let reply = read_guest_config_reply(content);

        let staged = super::finish_config_sync_from_reply(&reply, &staging, false)
            .expect("decode + stage ok");
        assert_eq!(staged.bytes, content.len());
        assert_eq!(staged.sha256, super::sha256_hex(content));
        assert_eq!(fs::read(&staging).expect("read staging"), content);
    }

    #[test]
    fn finish_config_sync_error_frames_never_stage_and_carry_no_guest_bytes() {
        // The full daemon error matrix: each kind must fail closed, leave NO
        // staging file, and surface exit code 70 (guest-control config read).
        for kind in [
            "guest-control-transport-unavailable",
            "guest-control-auth-failed",
            "guest-control-protocol-error",
            "guest-control-capability-unavailable",
            "guest-control-file-not-found",
            "guest-control-file-too-large",
            "guest-control-path-unsafe",
            "guest-control-read-denied",
            "guest-control-timeout",
        ] {
            let dir = scratch("gc-sync-err");
            let staging = dir.join("work.guest.nix");
            let reply = guest_control_error_reply(kind);
            let err = super::finish_config_sync_from_reply(&reply, &staging, true)
                .expect_err("error frame must fail");
            assert_eq!(err.exit_code, 70, "kind {kind} must map to exit 70");
            assert!(
                !staging.exists(),
                "kind {kind} must not create a staging file"
            );
            let rendered = err.rendered_stderr.unwrap_or_default();
            assert!(rendered.contains(kind), "kind {kind} must surface its slug");
            // No success content can appear on an error path: a sentinel that
            // only exists in a real config body must never leak here.
            assert!(!rendered.contains("systemPackages"));
        }
    }

    #[test]
    fn finish_config_sync_empty_content_is_rejected_and_not_staged() {
        let dir = scratch("gc-sync-empty");
        let staging = dir.join("work.guest.nix");
        let reply = read_guest_config_reply(b"   \n\t ");
        let err = super::finish_config_sync_from_reply(&reply, &staging, false)
            .expect_err("blank content must be rejected");
        assert!(!staging.exists(), "blank content must not be staged");
        // config_validate_staging_bytes rejects with a plain CliFailure.
        assert_ne!(err.exit_code, 0);
    }

    #[test]
    fn finish_config_sync_oversize_decoded_is_rejected_and_not_staged() {
        let dir = scratch("gc-sync-big");
        let staging = dir.join("work.guest.nix");
        let oversize =
            vec![b'a'; (nixling_ipc::guest_wire::READ_GUEST_FILE_MAX_BYTES as usize) + 1];
        let reply = read_guest_config_reply(&oversize);
        let err = super::finish_config_sync_from_reply(&reply, &staging, false)
            .expect_err("oversize must be rejected");
        assert_eq!(err.exit_code, 70);
        assert_eq!(err.message, "guest-control-file-too-large");
        assert!(!staging.exists());
    }

    #[test]
    fn finish_config_sync_malformed_base64_is_rejected_and_not_staged() {
        let dir = scratch("gc-sync-b64");
        let staging = dir.join("work.guest.nix");
        let reply = serde_json::to_vec(&serde_json::json!({
            "type": "readGuestConfigResponse",
            "contentBase64": "not valid base64!!!",
        }))
        .expect("serialize");
        let err = super::finish_config_sync_from_reply(&reply, &staging, false)
            .expect_err("malformed base64 must be rejected");
        assert_eq!(err.message, "guest-control-protocol-error");
        assert!(!staging.exists());
    }

    #[test]
    fn finish_config_sync_unexpected_reply_type_is_rejected() {
        let dir = scratch("gc-sync-type");
        let staging = dir.join("work.guest.nix");
        let reply =
            serde_json::to_vec(&serde_json::json!({ "type": "somethingElse" })).expect("serialize");
        let err = super::finish_config_sync_from_reply(&reply, &staging, false)
            .expect_err("unexpected type must be rejected");
        assert_eq!(err.message, "guest-control-protocol-error");
        assert!(!staging.exists());
    }
}

/// Fail-closed source gate: `ssh`/`scp` may only be launched from sanctioned
/// convenience/compatibility sites, each delimited by
/// `// nixling-ssh-allowlist begin/end`. The guest-control transport (config
/// sync, readiness) MUST NOT spawn an SSH client. This module scans the crate
/// source for SSH/SCP argv tokens outside the allowlist and proves at runtime
/// that the guest-control config path spawns no SSH client.
#[cfg(test)]
mod ssh_spawn_gate {
    use std::ffi::OsString;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::Mutex;

    use super::{
        cmd_config_sync, config_staging_path, finish_config_sync_from_reply,
        read_guest_config_via_socket, Context, GuestConfigReadOutcome, DEFAULT_GUEST_CONFIG_PATH,
    };

    /// Allowlist comment markers. Built so this scanner's own source never
    /// contains the literal SSH/SCP argv tokens it searches for.
    const ALLOW_BEGIN: &str = "nixling-ssh-allowlist begin";
    const ALLOW_END: &str = "nixling-ssh-allowlist end";

    /// Construct the quoted argv tokens (`"ssh"`, `"scp"`) without embedding the
    /// bare literal in this file, so the scanner is robust even if the
    /// test-module skip ever regresses.
    fn forbidden_tokens() -> [String; 2] {
        let ssh: String = ['s', 's', 'h'].iter().collect();
        let scp: String = ['s', 'c', 'p'].iter().collect();
        [format!("\"{ssh}\""), format!("\"{scp}\"")]
    }

    /// Return the 1-based line numbers that launch an SSH/SCP client outside an
    /// allowlist region. `#[cfg(test)] mod` blocks are skipped wholesale (test
    /// fixtures legitimately mention SSH); only column-0 `}` closes such a
    /// block, matching rustfmt's indentation of nested items.
    fn scan_ssh_argv_violations(src: &str) -> Vec<usize> {
        let [ssh_tok, scp_tok] = forbidden_tokens();
        let lines: Vec<&str> = src.lines().collect();
        let mut violations = Vec::new();
        let mut allow_depth: usize = 0;
        let mut in_test_mod = false;
        let mut i = 0;
        while i < lines.len() {
            let line = lines[i];
            let trimmed = line.trim();
            if !in_test_mod && trimmed == "#[cfg(test)]" {
                let next_is_mod = lines[i + 1..]
                    .iter()
                    .find(|candidate| !candidate.trim().is_empty())
                    .map(|candidate| candidate.trim_start().starts_with("mod "))
                    .unwrap_or(false);
                if next_is_mod {
                    in_test_mod = true;
                }
            }
            if in_test_mod {
                if line == "}" {
                    in_test_mod = false;
                }
                i += 1;
                continue;
            }
            if trimmed.contains(ALLOW_BEGIN) {
                allow_depth += 1;
                i += 1;
                continue;
            }
            if trimmed.contains(ALLOW_END) {
                allow_depth = allow_depth.saturating_sub(1);
                i += 1;
                continue;
            }
            if allow_depth == 0 && (line.contains(&ssh_tok) || line.contains(&scp_tok)) {
                violations.push(i + 1);
            }
            i += 1;
        }
        violations
    }

    #[test]
    fn crate_source_launches_ssh_only_from_allowlisted_sites() {
        let src = include_str!("lib.rs");
        let violations = scan_ssh_argv_violations(src);
        assert!(
            violations.is_empty(),
            "found SSH/SCP argv tokens outside the allowlist at lines {violations:?}; \
             wrap legitimate convenience/compat sites in nixling-ssh-allowlist markers"
        );
    }

    #[test]
    fn gate_flags_illicit_ssh_and_passes_allowlisted_and_test_blocks() {
        let [ssh_tok, _] = forbidden_tokens();
        // Illicit: a bare SSH argv in production code must be flagged.
        let illicit = format!("fn run() {{\n    let argv = vec![{ssh_tok}.to_owned()];\n}}\n");
        assert_eq!(scan_ssh_argv_violations(&illicit), vec![2]);

        // Sanctioned: the same call inside allowlist markers must pass.
        let sanctioned = format!(
            "fn run() {{\n    // {ALLOW_BEGIN}: x\n    let argv = vec![{ssh_tok}.to_owned()];\n    // {ALLOW_END}\n}}\n"
        );
        assert!(scan_ssh_argv_violations(&sanctioned).is_empty());

        // Test fixtures: an SSH token inside a `#[cfg(test)] mod` is skipped.
        let in_test = format!("#[cfg(test)]\nmod t {{\n    fn f() {{ let _ = {ssh_tok}; }}\n}}\n");
        assert!(scan_ssh_argv_violations(&in_test).is_empty());
    }

    /// Serialize PATH mutation across this module's runtime tests.
    static PATH_LOCK: Mutex<()> = Mutex::new(());

    /// Prepend a sentinel `bin/` (holding `ssh`/`scp` scripts that touch a
    /// marker file when invoked) to PATH for the guard's lifetime. Prepending
    /// keeps every other PATH-resolved tool reachable for concurrent tests.
    struct SshTrapGuard {
        old_path: Option<OsString>,
        marker: PathBuf,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl SshTrapGuard {
        fn install(dir: &std::path::Path) -> Self {
            let lock = PATH_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            let bin = dir.join("bin");
            std::fs::create_dir_all(&bin).expect("create trap bin");
            let marker = dir.join("ssh-spawned.marker");
            for tool in ["ssh", "scp"] {
                let script = bin.join(tool);
                let mut f = std::fs::File::create(&script).expect("create trap script");
                writeln!(f, "#!/bin/sh\necho spawned > {}\nexit 0", marker.display())
                    .expect("write trap script");
                let mut perms = std::fs::metadata(&script).expect("stat").permissions();
                std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
                std::fs::set_permissions(&script, perms).expect("chmod trap script");
            }
            let old_path = std::env::var_os("PATH");
            let mut entries = vec![bin];
            if let Some(existing) = &old_path {
                entries.extend(std::env::split_paths(existing));
            }
            let joined = std::env::join_paths(entries).expect("join PATH");
            std::env::set_var("PATH", joined);
            Self {
                old_path,
                marker,
                _lock: lock,
            }
        }

        fn ssh_was_spawned(&self) -> bool {
            self.marker.exists()
        }
    }

    impl Drop for SshTrapGuard {
        fn drop(&mut self) {
            match &self.old_path {
                Some(value) => std::env::set_var("PATH", value),
                None => std::env::remove_var("PATH"),
            }
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::current_dir()
            .expect("cwd")
            .join("target")
            .join(format!("ssh-gate-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch");
        dir
    }

    #[test]
    fn guest_control_config_path_never_spawns_ssh() {
        let dir = scratch("config-no-spawn");
        let trap = SshTrapGuard::install(&dir);

        // The connection branch with a missing socket must not spawn SSH.
        let context = Context {
            manifest_path: PathBuf::from("/dev/null"),
            bundle_path: PathBuf::from("/dev/null"),
            public_socket: dir.join("absent-public.sock"),
            broker_socket: PathBuf::from("/dev/null"),
            state_root: None,
            host_runtime_path: PathBuf::from("/dev/null"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: PathBuf::from("/dev/null"),
            metrics_url: "http://127.0.0.1:1/metrics".to_owned(),
        };
        let outcome = read_guest_config_via_socket(&context, "work")
            .expect("missing socket collapses to Unavailable");
        assert!(matches!(outcome, GuestConfigReadOutcome::Unavailable));

        // The success/staging branch must stage from received bytes, no SSH.
        let payload = b"{ services.foo.enable = true; }\n";
        let reply = serde_json::to_vec(&serde_json::json!({
            "type": "readGuestConfigResponse",
            "contentBase64": nixling_core::base64_codec::encode(payload),
        }))
        .expect("serialize reply");
        let staging = dir.join("staged.nix");
        let staged = finish_config_sync_from_reply(&reply, &staging, false)
            .expect("staging succeeds for a valid reply");
        assert_eq!(staged.bytes, payload.len());
        assert!(staging.exists());

        assert!(
            !trap.ssh_was_spawned(),
            "the guest-control config path must never spawn an SSH client"
        );

        drop(trap);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Write a minimal manifest declaring `vm` so `require_known_vm`
    /// passes and `cmd_config_sync` proceeds to transport selection.
    fn write_known_vm_manifest(path: &std::path::Path, vm: &str) {
        let manifest = serde_json::json!({
            (vm): {
                "name": vm,
                "env": "dev",
                "graphics": false,
                "tpm": false,
                "audio": false,
                "audioService": format!("nixling-{vm}-audio.service"),
                "usbipYubikey": false,
                "staticIp": null,
                "isNetVm": false,
                "stateDir": format!("/var/lib/nixling/vms/{vm}"),
                "bridge": "nl-dev",
                "sshUser": "alice"
            }
        });
        std::fs::write(
            path,
            serde_json::to_vec(&manifest).expect("serialize manifest"),
        )
        .expect("write manifest");
    }

    /// Write a bundle whose processes DAG declares `vm` but with NO
    /// `GuestControlHealth` node, modelling an old/partial generation that
    /// predates the guest-control transport. `vm_uses_guest_control`
    /// resolves false, so `cmd_config_sync` must fail closed.
    fn write_old_generation_bundle(bundle_path: &std::path::Path, vm: &str) {
        let base_dir = bundle_path.parent().expect("bundle parent");
        std::fs::create_dir_all(base_dir).expect("create bundle dir");
        let unique = bundle_path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("bundle file name");
        let processes_path = base_dir.join(format!("{unique}.processes.json"));
        let processes = nixling_core::processes::ProcessesJson {
            schema_version: "v2".to_owned(),
            vms: vec![nixling_core::processes::VmProcessDag {
                vm: vm.to_owned(),
                // No GuestControlHealth node: the VM is a known but
                // pre-guest-control generation.
                nodes: Vec::new(),
                edges: Vec::new(),
                invariants: nixling_core::processes::VmProcessInvariants {
                    swtpm_pre_start_flush: false,
                    per_vm_audit_pipeline: false,
                    usbip_gating: false,
                    tpm_ownership_migration_without_running_vm_mutation: false,
                },
            }],
        };
        std::fs::write(
            &processes_path,
            serde_json::to_vec(&processes).expect("serialize processes"),
        )
        .expect("write processes.json");
        let bundle = serde_json::json!({
            "bundleVersion": 4,
            "schemaVersion": "v2",
            "publicManifestPath": base_dir.join(format!("{unique}.vms.json")).to_string_lossy(),
            "hostPath": base_dir.join(format!("{unique}.host.json")).to_string_lossy(),
            "processesPath": processes_path.to_string_lossy(),
            "privilegesPath": base_dir.join(format!("{unique}.privileges.json")).to_string_lossy(),
            "closures": [],
            "minijailProfiles": [],
            "generation": { "generator": "test", "sourceRevision": null, "generatedAt": null },
        });
        std::fs::write(
            bundle_path,
            serde_json::to_vec(&bundle).expect("serialize bundle"),
        )
        .expect("write bundle.json");
    }

    #[test]
    fn config_sync_old_generation_fails_closed_without_socket_or_ssh() {
        // Binding fail-closed invariant: `config sync` against a known VM
        // whose bundle lacks the guest-control transport (an old or
        // partial generation) must reject with exit 70 +
        // `guest-control-unavailable-old-generation`, WITHOUT contacting
        // public.sock, WITHOUT staging/publishing, and WITHOUT taking the
        // SSH argv path. This is not live behaviour — it is the
        // hermetic guarantee that an unsupported generation can never
        // silently fall back to an SSH transport or a partial write.
        let dir = scratch("config-old-generation");
        let trap = SshTrapGuard::install(&dir);

        let vm = "oldgenvm";
        let manifest_path = dir.join("manifest.json");
        let bundle_path = dir.join("bundle.json");
        write_known_vm_manifest(&manifest_path, vm);
        write_old_generation_bundle(&bundle_path, vm);

        let context = Context {
            manifest_path,
            bundle_path,
            // A deliberately ABSENT socket: if a regression let the command
            // fall through to the transport, `read_guest_config_via_socket`
            // would surface `guest-control-transport-unavailable` (Unavailable
            // outcome) instead of the old-generation slug. The kind therefore
            // discriminates whether ANY public.sock request was attempted.
            public_socket: dir.join("absent-public.sock"),
            broker_socket: PathBuf::from("/dev/null"),
            state_root: None,
            host_runtime_path: PathBuf::from("/dev/null"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: PathBuf::from("/dev/null"),
            metrics_url: "http://127.0.0.1:1/metrics".to_owned(),
        };

        let args = super::ConfigSyncArgs {
            vm: vm.to_owned(),
            guest_path: DEFAULT_GUEST_CONFIG_PATH.to_owned(),
            host: None,
            user: None,
            key: None,
            known_hosts: None,
            dry_run: false,
            json: true,
        };

        let err = cmd_config_sync(&context, &args)
            .expect_err("old-generation config sync must fail closed");
        assert_eq!(
            err.exit_code, 70,
            "old generation maps to EXIT_GUEST_CONTROL_CONFIG"
        );
        let rendered = err.rendered_stderr.unwrap_or_default();
        assert!(
            rendered.contains("guest-control-unavailable-old-generation"),
            "old generation surfaces its fail-closed slug"
        );
        // Proves no public.sock request was sent: a transport attempt would
        // have produced the transport-unavailable slug against the absent
        // socket, not the old-generation slug.
        assert!(
            !rendered.contains("guest-control-transport-unavailable"),
            "the command must not reach the public.sock transport on an old generation"
        );
        // No SSH/SCP client may be spawned on any config-sync path.
        assert!(
            !trap.ssh_was_spawned(),
            "old-generation fail-closed must not spawn an SSH client"
        );
        // Nothing may be staged or published on the fail-closed path.
        assert!(
            !config_staging_path(vm).exists(),
            "old-generation fail-closed must not stage guest bytes"
        );

        drop(trap);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
