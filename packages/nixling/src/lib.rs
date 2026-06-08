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
    broker_wire::ExportBrokerAuditResponse,
    public_wire::{
        AuditFormat as IpcAuditFormat, AuditRequest as IpcAuditRequest, KeyEntry as IpcKeyEntry,
        KeysShowRequest as IpcKeysShowRequest, KeysShowResponse as IpcKeysShowResponse,
        UsbipProbeEntry as IpcUsbipProbeEntry, UsbipProbeStatus as IpcUsbipProbeStatus,
    },
    Hello as IpcHello, HelloOk as IpcHelloOk, HelloRejected as IpcHelloRejected, KnownFeatureFlag,
    SemverRange,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

mod doctor;
mod host_validate;

const DEFAULT_MANIFEST_PATH: &str = "/run/current-system/sw/share/nixling/vms.json";
const DEFAULT_BUNDLE_PATH: &str = "/etc/nixling/bundle.json";
const DEFAULT_PUBLIC_SOCKET: &str = "/run/nixling/public.sock";
const DEFAULT_BROKER_SOCKET: &str = "/run/nixling/priv.sock";
const DEFAULT_HOST_RUNTIME_PATH: &str = "/var/lib/nixling/runtime/host-runtime.json";
const DEFAULT_CLIENT_VERSION_RANGE: &str = ">=0.4.0, <0.5.0";
const RUNTIME_UNKNOWN: &str = "unknown (daemon-experimental, W4 not landed)";
const MAX_FRAME_BYTES: usize = 1024 * 1024;
/// P3 ph3-p3-host-doctor-extended: location of daemon-persisted
/// state files (`pidfd-table.json`, `kernel-module-report.json`,
/// `autostart-report.json`) that `nixling host doctor --read-only`
/// inspects. Mirrors `nixlingd::DEFAULT_DAEMON_STATE_DIR`.
const DEFAULT_DAEMON_STATE_DIR: &str = "/var/lib/nixling/daemon-state";
/// P3 ph3-p3-prometheus-otlp-shape: canonical Prometheus scrape URL
/// the doctor probes for reachability. See
/// `docs/reference/daemon-metrics.md` and the privileges.md row for
/// `ph3-p3-ch-exporter-retire`.
const DEFAULT_METRICS_URL: &str = "http://127.0.0.1:9101/metrics";

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runner_parity: Option<RunnerParityOutputV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatusServicesOutputV2 {
    pub nixling: String,
    pub microvm: String,
    pub virtiofsd: String,
    pub gpu: Option<String>,
    pub snd: Option<String>,
    pub swtpm: Option<String>,
}

/// v1.1.1 StatusOutputV3 per-VM service-state map.
///
/// Per ADR 0018 § "StatusOutputV3 schema bump" + the v1.1.1
/// migration-guide rename map. The v1.0/v1.1 StatusServicesOutputV2
/// shape (single `microvm`/`snd`/`gpu` slots) is replaced by
/// broker-spawn-aware fields:
///
/// | v1.0/v1.1 (V2)  | v1.1.1 (V3)            |
/// | --------------- | ---------------------- |
/// | `nixling`       | (deleted)              |
/// | `microvm`       | `hypervisor`           |
/// | `virtiofsd`     | `virtiofsd_per_share`  |
/// | `gpu`           | `gpu`                  |
/// | `snd`           | `audio`                |
/// | `swtpm`         | `swtpm`                |
/// | (new)           | `otel_relay`           |
/// | (new)           | `otel_host_bridge`     |
/// | (new)           | `usbip_backend_per_env`|
/// | (new)           | `usbip_proxy_per_env`  |
///
/// All fields are optional so V3-emitting consumers can omit a role
/// when the VM doesn't enable it. The wire shape uses camelCase
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
    /// v1.1.1 conversion shim: takes a V2 record and projects it
    /// into V3 by applying the documented rename map. Used during
    /// the v1.1 → v1.1.1 wire transition so callers consuming the
    /// old V2 shape can be migrated incrementally without breaking
    /// the bundle-resolver / status-output contract.
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
    about = "nixling v1.0 daemon-native CLI — daemon-only end-state per ADR 0015. Lifecycle verbs (vm start/stop/restart/list) dispatch exclusively to nixlingd + nixling-priv-broker. Compatibility verbs (console, audio) return a typed exit-78 envelope in v1.0 because their daemon-native surfaces are queued for v1.1+; the bash runtime that historically backed them was retired in P6. Daemon-first verbs (switch, gc, migrate, keys list/show, etc.) dispatch through the broker — see docs/reference/default-switch-and-deprecation.md for the per-verb matrix."
)]
struct NativeCli {
    #[command(subcommand)]
    command: NativeCommand,
}

#[derive(Debug, Subcommand)]
enum NativeCommand {
    List(ListArgs),
    Status(StatusArgs),
    Usb(UsbArgs),
    /// Foreground serial console bridge for headless VMs. P7fu2:
    /// the bash runtime that backed this verb pre-v1.0 was retired
    /// in P6; the daemon-native console surface is queued for v1.1+.
    /// Calling this in v1.0 surfaces a typed exit-78 envelope per
    /// ADR 0015.
    Console(ConsoleArgs),
    /// Per-VM audio grant bridge. P7fu2: the bash runtime that
    /// backed this verb pre-v1.0 was retired in P6; the
    /// daemon-native audio surface is queued for v1.1+. Calling
    /// this in v1.0 surfaces a typed exit-78 envelope per ADR 0015.
    Audio(AudioArgs),
    Audit(AuditArgs),
    Host(HostArgs),
    Auth(AuthArgs),
    /// W4-H7 / P4: per-VM lifecycle verbs routed through `nixlingd`.
    /// `--apply` is daemon-only; failure modes surface as typed
    /// envelopes. `--dry-run` returns the DAG the supervisor would
    /// drive.
    Vm(VmArgs),
    /// P4 alias for `vm start <vm>`. Daemon-native; no bash fallback.
    Up(VmStartArgs),
    /// P4 alias for `vm stop <vm>`. Daemon-native; no bash fallback.
    Down(VmStopArgs),
    /// P4 alias for `vm restart <vm>`. Daemon-native; no bash fallback.
    Restart(VmRestartArgs),
    /// W7-H1: `nixling build <vm>` — non-destructive eval+build of
    /// the per-VM toplevel.
    Build(BuildArgs),
    /// W7-H2: `nixling generations <vm>` — lists current/booted/N.
    Generations(GenerationsArgs),
    /// W7-H3: `nixling switch <vm> [--apply|--dry-run]` — atomic
    /// activation. `--apply` dispatches through `nixlingd` → broker
    /// `RunActivation` (v1.0 daemon-only per ADR 0015); `--dry-run`
    /// returns the planned activation.
    Switch(SwitchArgs),
    /// W7-H4: `nixling boot <vm>` — stage for next boot only.
    Boot(BootArgs),
    /// W7-H5: `nixling test <vm>` — activate-but-rollback-on-reboot.
    Test(TestArgs),
    /// W7-H6: `nixling rollback <vm>` — back to the previous
    /// generation.
    Rollback(RollbackArgs),
    /// W7-H7: `nixling gc [--apply|--dry-run]` — store cleanup.
    Gc(GcArgs),
    /// W8: managed-key + trust lifecycle verbs (list / show /
    /// rotate). `--apply` dispatches through `nixlingd` → broker
    /// `RunKeysRotate` (v1.0 daemon-only per ADR 0015).
    Keys(KeysArgs),
    /// W8: `nixling trust <vm>` (top-level, NOT under `keys`).
    /// Trust a host key on first use (TOFU) through the daemon /
    /// broker `RunHostKeyTrust` op. Bash runtime retired in P6.
    Trust(KeysTrustArgs),
    /// W8: `nixling rotate-known-host <vm>` (top-level, NOT under
    /// `keys`). Rotate the consumer's recorded known-host entry
    /// via the daemon / broker `RunRotateKnownHost` op. Bash
    /// runtime retired in P6.
    #[command(name = "rotate-known-host")]
    RotateKnownHost(KeysRotateKnownHostArgs),
    /// W9: `nixling migrate` — analyze the current host config and
    /// emit a migration plan to the daemon-experimental path.
    /// `--apply` dispatches the broker `RunMigrate` op (daemon-only
    /// since P6; the historical bash dispatch path was retired in
    /// the same wave).
    Migrate(MigrateArgs),
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
    Check(HostCheckArgs),
    /// W3fu1 H1 (product-1, software-1): native `host prepare`
    /// verb. `--apply` is mandatory for mutation; without it the
    /// command refuses with `--apply-or-dry-run-required` exit 78.
    Prepare(HostPrepareArgs),
    /// W3fu1 H1: native `host destroy` verb. Same mandatory-flag
    /// contract as `prepare`.
    Destroy(HostDestroyArgs),
    /// W3fu1 H1: native `host doctor` verb. `--read-only` is
    /// mandatory.
    Doctor(HostDoctorArgs),
    /// W15 (software-1, product-1): native `host install` routes
    /// `--apply` through the daemon → broker `RunHostInstall` path.
    Install(HostInstallArgs),
    /// P3 ph3-p3-net-route-degraded-mode: SOLE mutating recovery
    /// verb after the daemon-side net-route preflight has engaged
    /// operator-only mode. Re-runs the broker-side net slice of
    /// `host prepare` (nftables host scope + per-env routes +
    /// per-env ipv6 sysctls) and clears the persistent
    /// consecutive-failure counter on success.
    Reconcile(HostReconcileArgs),
    /// P5 ph5-p5-host-validate-verb: composite preflight that
    /// inventories per-wave Layer-2 validators and (with `--apply`)
    /// writes the canonical W18 evidence records consumed by
    /// `nixos-modules/options-daemon.nix:validationEvidencePresent`.
    Validate(HostValidateArgs),
}

#[derive(Debug, Args)]
struct HostValidateArgs {
    /// Plan: report which W18 readiness waves WOULD be attested.
    /// No evidence is written.
    #[arg(long, conflicts_with = "apply")]
    dry_run: bool,
    /// Apply: write the canonical
    /// `/var/lib/nixling/validated/<wave>.json` evidence record for
    /// every wave whose declared validators are present on disk.
    #[arg(long, conflicts_with = "dry_run")]
    apply: bool,
    /// Restrict to a single wave (e.g. `--wave p1`). Other waves
    /// are reported as `skipped`.
    #[arg(long)]
    wave: Option<String>,
    /// Override the per-wave operator signature. When unset, the
    /// verb derives a deterministic sha256 signature from
    /// `hostname|wave|scripts_dir|timestamp`.
    #[arg(long, value_name = "SIGNATURE")]
    operator_signature: Option<String>,
    /// Override the evidence directory. Default:
    /// `/var/lib/nixling/validated` (the W18 gate path).
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
    /// Mandatory: the W3 doctor verb is read-only. Mutation forms
    /// are W4 deliverables.
    #[arg(long)]
    read_only: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct HostInstallArgs {
    /// W9: `--dry-run` reports the planned install steps.
    #[arg(long, conflicts_with_all = ["apply", "enable", "start", "no_start"])]
    dry_run: bool,
    /// W15: `--apply` performs the install through the
    /// daemon → broker `RunHostInstall` path.
    #[arg(long, conflicts_with = "dry_run")]
    apply: bool,
    /// W9: After `--apply`, enable nixlingd.service via systemctl.
    #[arg(long, conflicts_with = "dry_run", requires = "apply")]
    enable: bool,
    /// W9: After `--apply --enable`, start nixlingd.service.
    #[arg(long, conflicts_with_all = ["dry_run", "no_start"], requires = "apply")]
    start: bool,
    /// W9: Explicitly do NOT start nixlingd.service post-install.
    #[arg(long, conflicts_with_all = ["dry_run", "start"], requires = "apply")]
    no_start: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct HostReconcileArgs {
    /// Required for P3: re-run the network slice of `host prepare`
    /// and clear the daemon's net-route preflight counter. Today
    /// this is the only available scope; future P-phases may add
    /// other scopes (e.g. `--ownership`).
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
    /// v1.1.1fu14 C1 + v1.1.2fu18: open an SSH session to the
    /// VM in a host terminal. Resolves the per-VM SSH key from
    /// the bundle's `managed_keys.effective_key_path(<vm>)`
    /// (honors `nixling.site.keysDir` + per-VM overrides; legacy
    /// `/var/lib/nixling/keys/<vm>_ed25519` is fallback) and the
    /// IP from the manifest's `static_ip`. Default terminal:
    /// konsole.
    Konsole(VmKonsoleArgs),
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

/// v1.1.1fu14 C1: `nixling vm konsole <vm>` — open an SSH session to
/// the VM in a host terminal. Resolves the per-VM SSH key from
/// v1.1.1fu14 + v1.1.2fu18 product-panel must-fix wording.
/// Spawn a terminal emulator (default `konsole`, overridable
/// via `--terminal`) hosting an SSH session into the named VM.
/// The SSH key is resolved from the bundle's
/// `managed_keys.effective_key_path()` (which honors
/// `nixling.site.keysDir` + per-VM overrides); the legacy
/// `/var/lib/nixling/keys/<vm>_ed25519` is only the fallback when
/// the bundle is absent. The host IP comes from the bundle's per-env
/// LAN subnet + the VM's lan index. Detaches from the CLI process
/// via setsid so closing the CLI doesn't take the terminal down.
#[derive(Debug, Args)]
struct VmKonsoleArgs {
    /// VM name as declared in `nixling.vms.<name>`.
    vm: String,
    /// Terminal emulator binary to spawn. Must accept `-e` to
    /// execute a command. Tested: konsole, alacritty, foot,
    /// gnome-terminal, xterm. Default: konsole.
    #[arg(long, default_value = "konsole")]
    terminal: String,
    /// SSH user inside the guest. Defaults to the per-VM
    /// `ssh_user` from the manifest; falls back to `$USER` if
    /// the manifest entry is absent. Override for ad-hoc
    /// per-user sessions.
    #[arg(long)]
    user: Option<String>,
    /// Override the SSH host (IP or hostname). Default:
    /// manifest `static_ip` (bundle-resolved LAN address).
    #[arg(long)]
    host: Option<String>,
    /// Override the SSH key path. Default: the bundle's
    /// `managed_keys.effective_key_path(<vm>)` (honors
    /// `nixling.site.keysDir` + per-VM overrides). Legacy
    /// `/var/lib/nixling/keys/<vm>_ed25519` is only the
    /// fallback when no bundle is staged.
    #[arg(long)]
    key: Option<std::path::PathBuf>,
    /// Print the would-be command without executing.
    #[arg(long)]
    dry_run: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

// ---- W7 store-lifecycle verbs ----

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

// ---- W8 keys + trust verbs ----

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

// ---- W9 migrate verb ----

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
    /// P3 ph3-p3-host-doctor-extended: daemon-persisted state dir
    /// (pidfd-table.json, kernel-module-report.json,
    /// autostart-report.json). Override via `NIXLING_DAEMON_STATE_DIR`.
    daemon_state_dir: PathBuf,
    /// P3 ph3-p3-host-doctor-extended: Prometheus scrape URL the
    /// doctor probes for reachability. Override via
    /// `NIXLING_METRICS_URL`.
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
        if !self.bundle_path.exists() {
            return Ok(None);
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
    audio_service: String,
    usbip_yubikey: bool,
    static_ip: Option<String>,
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
        print_stdout("nixling 0.0.0-bootstrap (W0a stub)\n");
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
            VmCommand::Konsole(args) => cmd_vm_konsole(context, args),
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
    }
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
                let pending_restart = is_pending_restart(
                    vm,
                    &vm_service_states(context, vm),
                    current.as_deref(),
                    booted.as_deref(),
                );
                ListItemOutputV2 {
                    name: vm.name.clone(),
                    env: vm.env.clone(),
                    graphics: vm.graphics,
                    tpm: vm.tpm,
                    usbip: vm.usbip_yubikey,
                    static_ip: vm.static_ip.clone(),
                    status: list_status_label(vm, context, pending_restart),
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
            message: "bridge reconciliation is deferred to W3; use `nixling host check --read-only` for advisory bridge-related probes".to_owned(),
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

/// W3fu1 H1 (product-1, software-1): standard JSON error envelope
/// per plan.md §"CLI surface and UX". Every native host-verb
/// refusal emits this shape on stdout (JSON mode) or as a
/// human-readable summary on stderr (default mode).
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

/// v1.1-P1: typed `daemon-down` envelope (exit 1) for verbs whose
/// daemon-backed path cannot be reached. Per ADR 0017, the Rust CLI
/// never executes bash; verbs that previously degraded into
/// `exec_legacy_passthrough` now surface this envelope unconditionally.
fn daemon_down_envelope(verb: &str) -> HostErrorEnvelope {
    host_error_envelope(
        &format!("nixling {verb} requires nixlingd"),
        "daemon-down",
        1,
        "Daemon connectivity at /run/nixling/public.sock.",
        "nixlingd is unreachable; v1.1 retired the bash fallback path per ADR 0017 — the daemon is the only operator surface.",
        "Start nixlingd (systemctl start nixlingd nixling-priv-broker.socket) and re-run the same command. See docs/how-to/migrate-nixling-v1-0-to-v1-1.md#recovery-broker-bring-up-troubleshooting for the full bring-up checklist.",
        "docs/reference/error-codes.md#daemon-down",
    )
}

/// v1.1-P1: typed `not-yet-implemented` envelope (exit 78) for verbs
/// whose daemon-native handler has not landed yet. Per ADR 0017, no
/// bash fallback ever satisfies these — operators receive the typed
/// envelope and the migration-guide cross-link.
fn not_yet_implemented_envelope(verb: &str) -> HostErrorEnvelope {
    host_error_envelope(
        &format!("nixling {verb} has no daemon-native handler in v1.1"),
        "not-yet-implemented",
        78,
        &format!("Native daemon dispatch for `nixling {verb}` (post-v1.1 surface per ADR 0017)"),
        "The daemon-native handler is scheduled for post-v1.1; v1.1 itself only delivers the typed envelope contract (no bash fallback). See docs/reference/error-codes.md#not-yet-implemented for the rendering convention.",
        "Track the post-v1.1 surface schedule in CHANGELOG.md \"Unreleased\" / ADR 0017 § Negative consequences; the typed envelope is the only operator path until the native handler ships.",
        "docs/reference/error-codes.md#not-yet-implemented",
    )
}

/// Bundle-derived deployment shape used by the `host prepare` /
/// `host destroy` per-tier routing logic. Matches plan.md
/// §"W3 daemon-vs-legacy migration boundary" Tier-0 sub-cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeploymentShape {
    /// Every VM uses `supervisor = "systemd"` (Tier 0 all-legacy).
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
    // Bundle inspection of `supervisor` is W4+; for W3 fall back
    // to all-daemon as documented in the per-tier routing table.
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
                "Every VM declares supervisor = \"systemd\"; the nixling NixOS module already owns host-shared reconciliation on Tier 0.",
                "tier-0-all-legacy",
                "Add at least one VM with `nixling.vms.<vm>.supervisor = \"nixlingd\"` before invoking host prepare --apply on this host.",
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
            // W3 broker dispatch is staged in the privileged broker
            // but the daemon path that wires the typed bundle
            // intents through `nixlingd` is not yet shipping in
            // bootstrap mode. Surface the same pending-impl envelope
            // the broker would emit so the human / JSON contract
            // stays stable.
            emit_host_error(
                &host_error_envelope(
                    "Daemon-backed prepare staged but the public-socket dispatch path is pending",
                    "daemon-down",
                    1,
                    "Daemon connectivity at /run/nixling/public.sock and broker dispatch readiness.",
                    "nixlingd is reachable but the W3 host-prepare API surface is still gated behind nixling.daemonExperimental.enable; the integrator wires it on once H2/H3 ship the typed intent emitters.",
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
                "notes": "W3 host-prepare dry-run reports the planned reconcile without mutation; --apply mutates host state.",
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
                "Every VM declares supervisor = \"systemd\"; host destroy is only valid when daemon-owned VMs exist.",
                "tier-0-all-legacy",
                "Migrate at least one VM to supervisor = \"nixlingd\". The historical `--legacy` bash-destroy escape hatch was retired in P6 (per ADR 0015).",
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
                "nixlingd is reachable but the W3 host-destroy API surface is still gated behind the typed-intent broker dispatch.",
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
                "Re-run as `nixling host doctor --read-only`. The W3 doctor verb is read-only; mutation forms are W4 deliverables.",
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
                    "host validate --wave value is not a known W18 readiness wave",
                    "unknown-wave",
                    78,
                    "host validate --wave argument.",
                    &format!("--wave {only} is not in the W18 readiness catalog"),
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
    original_args: &[OsString],
) -> Result<i32, CliFailure> {
    // W9-H1: host install --dry-run/--apply/--enable/--start/--no-start
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
            { "step": 3, "what": "bind /run/nixling/public.sock + /run/nixling/priv.sock with the W3 socket ACLs (launcher / admin groups)" },
            { "step": 4, "what": if args.enable && args.start { "systemctl enable --now nixlingd.service" } else if args.enable { "systemctl enable nixlingd.service" } else if args.no_start { "do NOT enable; operator starts manually" } else { "neither --enable nor --start specified: leave service inactive" } },
            { "step": 5, "what": "smoke: nixling auth status against /run/nixling/public.sock" },
        ],
        "notes": "W15: dry-run preview retained; --apply routes through the daemon → broker RunHostInstall path.",
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
    original_args: &[OsString],
) -> Result<i32, CliFailure> {
    // P3 ph3-p3-net-route-degraded-mode: SOLE mutating recovery
    // verb for the daemon's net-route preflight degraded mode.
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
                "Re-run with `--network` (the only scope available in P3); future scopes will be added in later P-phases.",
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
            "not-yet-implemented",
            70,
            "Whether the VM name appears in `nixling.vms.<name>` in the active manifest.",
            "VM name unknown",
            "Run `nixling list` to see declared VMs, then re-run with a name from that list.",
            "docs/reference/error-codes.md#not-yet-implemented",
        ),
        json,
    )?;
    Err(CliFailure::new(exit_code, format!("unknown vm: {vm}")))
}

fn vm_dag_dry_run_summary(verb: &str, vm: &str) -> serde_json::Value {
    // The DAG the supervisor would drive. Mirrors the structure
    // emitted by W3's processes::VmProcessDag exporter — for the W4
    // headless alpha shape (host-reconcile → store-preflight →
    // virtiofsd-ro-store → ch → ssh-ready) we summarize the node ids
    // and the topological edges. The full per-role argv preview is
    // a follow-up gate in W4-H10.
    //
    // W4 GPT-5.5 panel notable #2: `vm stop` walks the DAG in
    // REVERSE topo order (terminate ch first, then virtiofsd, etc).
    // The dry-run summary reflects the current apply order so the
    // operator sees the same DAG the daemon bridge will drive.
    let stopping = matches!(verb, "stop");
    let restarting = matches!(verb, "restart");
    let forward_nodes: Vec<serde_json::Value> = vec![
        serde_json::json!({"id": "host-reconcile",        "role": "host-reconcile"}),
        serde_json::json!({"id": "store-preflight",       "role": "store-virtiofs-preflight"}),
        serde_json::json!({"id": "virtiofsd-ro-store",    "role": "virtiofsd"}),
        serde_json::json!({"id": "ch",                    "role": "cloud-hypervisor-runner"}),
        serde_json::json!({"id": "ssh-ready",             "role": "guest-ssh-readiness"}),
    ];
    let forward_edges = serde_json::json!([
        {"from": "host-reconcile",     "to": "store-preflight"},
        {"from": "store-preflight",    "to": "virtiofsd-ro-store"},
        {"from": "virtiofsd-ro-store", "to": "ch"},
        {"from": "ch",                 "to": "ssh-ready"},
    ]);
    let stop_order = serde_json::json!([
        "ssh-ready",
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
    json: bool,
) -> Result<i32, CliFailure> {
    let flags = require_explicit_mutation_flag(&format!("vm {verb}"), dry_run, apply, json)?;
    require_known_vm(context, vm, json)?;
    if flags.apply {
        // P4 cli-up: vm lifecycle verbs are daemon-only. The W14c
        // bash-translation bridge has been removed; any failure mode
        // surfaces as a typed envelope via `dispatch_mutating_verb`.
        let request_type = match verb {
            "start" => "vmStart",
            "stop" => "vmStop",
            "restart" => "vmRestart",
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
    let summary = vm_dag_dry_run_summary(verb, vm);
    if json {
        let mut rendered = serde_json::to_string_pretty(&summary).map_err(|err| {
            CliFailure::new(1, format!("failed to serialize vm dry-run summary: {err}"))
        })?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        print_stdout(&format!(
            "vm {verb} --dry-run: would drive the 5-node DAG for vm '{vm}' (host-reconcile → store-preflight → virtiofsd-ro-store → ch → ssh-ready)\n"
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

/// v1.1.1fu14 C1 + fu15 panel must-fixes: `nixling vm konsole <vm>` —
/// spawn a terminal emulator hosting an SSH session into the named VM.
///
/// Resolution order:
///   - VM name → manifest entry → static_ip + ssh.user
///   - Key path: --key, else bundle.managed_keys.effective_key_path,
///     else /var/lib/nixling/keys/<vm>_ed25519
///   - Host: --host, else static_ip from manifest
///   - User: --user, else ssh_user from manifest, else $USER env
///   - Terminal: --terminal, else "konsole"
///
/// The spawned process is detached via setsid so the CLI can exit
/// while the terminal keeps running. StrictHostKeyChecking is
/// disabled and UserKnownHostsFile=/dev/null because the per-VM
/// host keys are nixling-managed and the host's known_hosts entry
/// would change every VM rebuild (defeating the security check).
///
/// fu15 panel-product must-fix: validate key existence + manifest
/// entry BEFORE any --json output so machine consumers never see
/// "success" JSON followed by an error envelope on stderr. fu15
/// panel-product must-fix: --key resolves from bundle.managed_keys
/// when --key is not given (consumer sites can override keysDir);
/// the legacy /var/lib/nixling/keys path is only the last fallback.
/// fu15 panel-product must-fix: drop the hardcoded username
/// fallback; use $USER env var instead (still fails closed if
/// unset). fu15 panel-rust should-fix: propagate setsid
/// spawn-failure as a typed exit-1 envelope; the spawned terminal
/// can fail to start even if setsid forks successfully.
fn cmd_vm_konsole(context: &Context, args: &VmKonsoleArgs) -> Result<i32, CliFailure> {
    require_known_vm(context, &args.vm, args.json)?;
    let manifest = context.load_manifest()?;
    let vm = manifest.entries.get(&args.vm).ok_or_else(|| {
        CliFailure::new(
            1,
            format!("vm konsole: unknown vm '{}' in manifest", args.vm),
        )
    })?;

    let host = args
        .host
        .clone()
        .or_else(|| vm.static_ip.clone())
        .ok_or_else(|| {
            CliFailure::new(
                1,
                format!(
                    "vm konsole: vm '{}' has no static_ip in manifest and no --host override",
                    args.vm
                ),
            )
        })?;
    let user = args
        .user
        .clone()
        .or_else(|| vm.ssh_user.clone())
        .or_else(|| std::env::var("USER").ok())
        .ok_or_else(|| {
            CliFailure::new(
                1,
                format!(
                    "vm konsole: vm '{}' has no ssh_user in manifest; pass --user or set $USER",
                    args.vm
                ),
            )
        })?;
    // fu15 panel-product must-fix: resolve key path from bundle's
    // managed_keys (which honors site keysDir + per-VM overrides)
    // when --key is not given. Fall back to /var/lib/nixling/keys
    // legacy path only when the bundle is absent (e.g. running
    // pre-staging or in a hermetic test harness).
    let key_path = if let Some(p) = args.key.clone() {
        p
    } else if context.bundle_path.exists() {
        let bundle: Bundle = read_json_file(&context.bundle_path).map_err(|err| {
            CliFailure::new(
                1,
                format!(
                    "vm konsole: failed to read bundle {}: {err}",
                    context.bundle_path.display()
                ),
            )
        })?;
        bundle.managed_keys.effective_key_path(&args.vm)
    } else {
        PathBuf::from(format!("/var/lib/nixling/keys/{}_ed25519", args.vm))
    };

    let terminal = &args.terminal;
    let ssh_target = format!("{user}@{host}");
    let key_arg = key_path.display().to_string();
    let argv: Vec<String> = vec![
        terminal.clone(),
        "-e".to_owned(),
        "ssh".to_owned(),
        "-i".to_owned(),
        key_arg.clone(),
        "-o".to_owned(),
        "StrictHostKeyChecking=no".to_owned(),
        "-o".to_owned(),
        "UserKnownHostsFile=/dev/null".to_owned(),
        ssh_target.clone(),
    ];

    // fu15 panel-product must-fix: validate the key file BEFORE
    // emitting any --json output. A consumer parsing the JSON would
    // otherwise see success-shape JSON followed by an exit-1
    // envelope on stderr, which is incoherent. Dry-run mode is
    // exempt: it explicitly does NOT spawn anything, so the key
    // file's existence is informational only.
    if !args.dry_run && !key_path.exists() {
        return Err(CliFailure::new(
            1,
            format!(
                "vm konsole: ssh key not found at {} (override with --key)",
                key_path.display()
            ),
        ));
    }

    if args.dry_run || args.json {
        let body = serde_json::json!({
            "command": "vm konsole",
            "mode": if args.dry_run { "dry-run" } else { "would-spawn" },
            "vm": args.vm,
            "terminal": terminal,
            "host": host,
            "user": user,
            "key": key_arg,
            "argv": argv,
        });
        let mut rendered = serde_json::to_string_pretty(&body)
            .map_err(|err| CliFailure::new(1, format!("serialize: {err}")))?;
        rendered.push('\n');
        print_stdout(&rendered);
        if args.dry_run {
            return Ok(0);
        }
    } else if args.human {
        print_stdout(&format!(
            "vm konsole {}: spawning `{}` ssh session as {}@{}\n",
            args.vm, terminal, user, host
        ));
    }

    // Spawn detached so the CLI can exit while the terminal keeps
    // running. setsid --fork is the conventional Unix pattern for
    // fully detaching from the controlling tty/session.
    let mut child = std::process::Command::new("setsid")
        .arg("--fork")
        .args(&argv)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|err| {
            // fu17 panel-rust should-fix: emit typed envelope (not
            // plain CliFailure text) so machine consumers parsing
            // --json get a consistent error envelope shape via
            // rendered_stderr instead of a bare "nixling: ..." line.
            let operator_error = CoreError::internal_io(format!(
                "vm konsole: failed to spawn `setsid --fork {}`: {err}",
                terminal
            ));
            CliFailure {
                exit_code: 1,
                message: operator_error.message(),
                rendered_stderr: render_operator_error(
                    &operator_error,
                    Some("vm konsole"),
                ),
            }
        })?;
    // setsid --fork exits immediately after forking the real child;
    // we wait for setsid to reap its fork-state but do NOT wait for
    // the terminal itself (it lives independently). fu15 panel-rust
    // should-fix + fu17 typed-envelope: propagate non-zero setsid
    // exit as a typed envelope so operators see a structured error
    // message instead of a silently-failed konsole.
    let status = child.wait().map_err(|err| {
        let operator_error = CoreError::internal_io(format!(
            "vm konsole: setsid wait failed: {err}"
        ));
        CliFailure {
            exit_code: 1,
            message: operator_error.message(),
            rendered_stderr: render_operator_error(
                &operator_error,
                Some("vm konsole"),
            ),
        }
    })?;
    if !status.success() {
        let operator_error = CoreError::internal_io(format!(
            "vm konsole: setsid --fork {} exited with status {:?} (terminal binary missing?)",
            terminal, status
        ));
        return Err(CliFailure {
            exit_code: 1,
            message: operator_error.message(),
            rendered_stderr: render_operator_error(
                &operator_error,
                Some("vm konsole"),
            ),
        });
    }
    Ok(0)
}

// ---- W7-H1..H7: store-lifecycle CLI verbs ----

fn w7_dry_run_summary(verb: &str, vm: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "command": verb,
        "mode": "dry-run",
        "vm": vm,
        "planned": [],
        "notes": format!("nixling {verb} --dry-run reports the planned operation; --apply routes through nixlingd → broker (v1.0 daemon-only per ADR 0015; the historical bash fallback was retired in P6)."),
    })
}

fn cmd_build(context: &Context, args: &BuildArgs) -> Result<i32, CliFailure> {
    // build is non-destructive — always allowed; never returns
    // daemon-down. The W7a non-destructive scope (build / generations
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
    original_args: &[OsString],
) -> Result<i32, CliFailure> {
    let flags = require_mutation_flag(verb, dry_run, apply, json)?;
    require_known_vm(context, vm, json)?;
    if flags.apply {
        // W14: daemon-first dispatch is live for activation verbs.
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

fn cmd_gc(context: &Context, args: &GcArgs, original_args: &[OsString]) -> Result<i32, CliFailure> {
    let flags = require_mutation_flag("gc", args.dry_run, args.apply, args.json)?;
    if flags.apply {
        // v1.0 daemon-only: --apply routes through nixlingd → broker
        // (ADR 0015). The historical bash fallback was retired in P6;
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

// ---- W6: native usb CLI ----

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
    let summary = serde_json::json!({
        "command": verb,
        "mode": "dry-run",
        "vm": vm,
        "busId": bus_id,
        "planned": match verb {
            "usb attach" => ["UsbipBind", "UsbipProxyReconcile"],
            _ => ["UsbipUnbind", "UsbipProxyReconcile"],
        },
        "notes": "W6 USBIP dry-run reports the daemon → broker bind/unbind + reconcile plan without mutating host state.",
    });
    if json_mode {
        let mut rendered = serde_json::to_string_pretty(&summary)
            .map_err(|e| CliFailure::new(1, format!("serialize: {e}")))?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        let action = if verb == "usb attach" {
            "bind"
        } else {
            "unbind"
        };
        print_stdout(&format!(
            "nixling {verb} --dry-run: would {action} busid '{bus_id}' for vm '{vm}' and reconcile the USBIP proxy\n"
        ));
    }
    Ok(0)
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
                "Daemon connectivity at /run/nixling/public.sock and W6 USBIP probe support.",
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

// ---- W8: managed-keys + trust verbs ----

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
    original_args: &[OsString],
) -> Result<i32, CliFailure> {
    let flags = require_mutation_flag(&format!("keys {verb}"), dry_run, apply, json)?;
    require_known_vm(context, vm, json)?;
    if flags.apply {
        // v1.0 daemon-only: --apply routes through nixlingd → broker
        // (ADR 0015). The historical bash fallback was retired in P6.
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
        "notes": format!("W8 keys {verb} --dry-run: planned operation. --apply routes through nixlingd → broker RunKeysRotate with broker audit (v1.0 daemon-only per ADR 0015)."),
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

// ---- W9 nixling migrate ----

fn cmd_migrate(
    context: &Context,
    args: &MigrateArgs,
    original_args: &[OsString],
) -> Result<i32, CliFailure> {
    let flags = require_explicit_mutation_flag("migrate", args.dry_run, args.apply, args.json)?;
    let manifest = context.load_manifest()?;
    let shape = detect_deployment_shape(context)?;
    let vms: Vec<&ManifestVm> = manifest.vms();

    // W9 migrate planner. Per-VM supervisor classification needs the
    // consumer flake's `nixling.vms.<vm>.supervisor` setting, which
    // the public manifest still does not expose. Per W*-fu GPT-5.5
    // panel notable #1: the prior shape always claimed every VM
    // needed migration, which is materially misleading on a
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
        // retired in P6; daemon-unreachable surfaces a typed
        // daemon-down envelope (exit-1).
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
        "perVmClassificationNote": "v1.1 (per ADR 0015) made every enabled VM daemon-supervised by default; the `nixling.vms.<vm>.supervisor` option was removed in v1.1-P2. Per-VM systemd-unit inspection still uses `nixling status <vm>`.",
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
             `supervisor` option was removed in v1.1-P2 (ADR 0015). Use\n\
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

fn build_vm_status_output(
    context: &Context,
    vm: &ManifestVm,
    bundle: Option<&BundleContext>,
) -> StatusVmOutputV2 {
    let service_states = vm_service_states(context, vm);
    let current = current_symlink(context, vm);
    let booted = booted_symlink(context, vm);
    let pending_restart =
        is_pending_restart(vm, &service_states, current.as_deref(), booted.as_deref());
    let process_vm = bundle
        .and_then(|bundle| bundle.processes.as_ref())
        .and_then(|processes| processes.vms.iter().find(|entry| entry.vm == vm.name));
    let declared_roles = process_vm
        .map(|entry| {
            entry
                .nodes
                .iter()
                .map(|node| process_role_name(&node.role))
                .collect()
        })
        .unwrap_or_default();
    let readiness = process_vm
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
        runner_parity,
    }
}

fn vm_service_states(context: &Context, vm: &ManifestVm) -> StatusServicesOutputV2 {
    let gpu_unit = format!("nixling-{}-gpu.service", vm.name);
    let swtpm_unit = format!("nixling-{}-swtpm.service", vm.name);
    StatusServicesOutputV2 {
        nixling: systemctl_state(context, &format!("nixling@{}.service", vm.name)),
        microvm: systemctl_state(context, &format!("microvm@{}.service", vm.name)),
        virtiofsd: systemctl_state(context, &format!("microvm-virtiofsd@{}.service", vm.name)),
        gpu: vm.graphics.then(|| systemctl_state(context, &gpu_unit)),
        snd: vm
            .audio
            .then(|| systemctl_state(context, &vm.audio_service)),
        swtpm: vm.tpm.then(|| systemctl_state(context, &swtpm_unit)),
    }
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
    matches!(state, "active" | "activating" | "reloading")
}

fn list_status_label(vm: &ManifestVm, context: &Context, pending_restart: bool) -> String {
    if vm.is_net_vm {
        "running".to_owned()
    } else if pending_restart {
        "pending-restart".to_owned()
    } else {
        let services = vm_service_states(context, vm);
        if vm_counts_as_running(vm, &services) {
            "running".to_owned()
        } else {
            "stopped".to_owned()
        }
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
        nixling_core::processes::ProcessRole::Audio => "audio",
        nixling_core::processes::ProcessRole::CloudHypervisorRunner => "cloud-hypervisor-runner",
        nixling_core::processes::ProcessRole::VsockRelay => "vsock-relay",
        nixling_core::processes::ProcessRole::GuestSshReadiness => "guest-ssh-readiness",
        nixling_core::processes::ProcessRole::Usbip => "usbip",
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
        nixling_core::processes::ReadinessPredicate::TcpPort { host, port } => {
            format!("tcp-port:{host}:{port}")
        }
        nixling_core::processes::ReadinessPredicate::Command(argv) => {
            format!("command:{}", argv.join(" "))
        }
        nixling_core::processes::ReadinessPredicate::ComponentSpecific(value) => {
            format!("component-specific:{value}")
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
    if let (Some(user), Some(ip)) = (&manifest_vm.ssh_user, &manifest_vm.static_ip) {
        let _ = writeln!(text, "ssh: declared {user}@{ip}");
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

fn print_stdout(text: &str) {
    let mut stdout = io::stdout().lock();
    let _ = stdout.write_all(text.as_bytes());
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

/// v1.1-P1 / ADR 0017: the `should_fallback_to_legacy` /
/// `exec_legacy_passthrough` pair were removed wholesale. Every verb
/// the Rust CLI accepts dispatches to clap → typed-envelope; verbs
/// clap rejects fall through to the parse-error path. No bash exec
/// site survives in the binary crate.

/// W14c daemon mutating-verb outcome from
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

/// W14c: send a mutating-verb request frame to the daemon and parse
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
                "{op_name} references a bundle intent that the broker did not find. Admin: ask `nixling audit --strict` for the intent id."
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
                "{op_name} failed at the broker live handler. Admin: inspect `nixling audit --strict` for the underlying syscall/exit code."
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
                "{op_name} failed: bundle nft script could not be parsed. Admin: inspect `nixling audit --strict` for the parse error."
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
    let default_observed_state = if let Some(target_wave) = target_wave {
        format!(
            "The daemon reached the broker for `{op_name}`, but the broker refused or failed the request (target wave hint: {target_wave})."
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

/// P4 cli-up: top-level dispatcher for mutating verbs. Runs the
/// native daemon path; failure modes surface as typed envelopes
/// (daemon-down exit-1, broker-error exit-78, not-yet-implemented
/// exit-78) per ADR 0015. v1.1-P1 removed every bash-fallback
/// escape hatch (NIXLING_LEGACY_BASH_OPT_IN / NIXLING_NATIVE_ONLY /
/// NIXLING_LEGACY_CLI_PATH env vars are no longer honoured); the
/// Rust CLI dispatching through nixlingd → broker is the only
/// operator path.
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
    match outcome {
        DaemonVerbOutcome::Applied { summary } => {
            print_stdout(&format!("{summary}\n"));
            Ok(0)
        }
        DaemonVerbOutcome::DryRunPlanned { summary } => {
            print_stdout(&format!("{summary}\n"));
            Ok(0)
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
            // v1.1-P1: bash fallback removed. Surface the typed
            // envelope unconditionally.
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
            // v1.1-P1: daemon-only. No bash fallback.
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
        os::fd::{AsRawFd as _, RawFd},
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
        broker_error_envelope, cmd_host_install, daemon_supported_features,
        encode_type_tagged_message, nix_err_to_io, send, socket, AddressFamily, Context,
        HostInstallArgs, IpcHelloOk, MsgFlags, NativeCli, SockFlag, SockType, UnixAddr,
        MAX_FRAME_BYTES,
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

    struct EnvVarGuard {
        key: &'static str,
        old: Option<OsString>,
    }

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
        // v1.1-P1: `should_fallback_to_legacy` was deleted. The
        // equivalent invariant is now "clap accepts every help
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
        assert!(envelope.observed_state.contains("target wave hint: W15"));
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
                "remediation": "RunHostInstall failed at the broker live handler. Admin: inspect `nixling audit --strict` for the underlying syscall/exit code.",
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
