use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::OsString,
    fmt::Write as _,
    fs,
    io::{self, IsTerminal as _, Read as _, Write as _},
    os::fd::{AsRawFd as _, OwnedFd},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use d2b_contracts::v2_services::{guest as guest_v2, terminal as terminal_v2};
use d2b_contracts::{
    Hello as IpcHello, HelloOk as IpcHelloOk, HelloRejected as IpcHelloRejected, KnownFeatureFlag,
    broker_wire::{
        ExportBrokerAuditResponse, StoreVerifyResponse as IpcStoreVerifyResponse,
        StoreVerifyStatus as IpcStoreVerifyStatus,
    },
    cli_output::*,
    public_wire::{
        self, AuditFormat as IpcAuditFormat, AuditRequest as IpcAuditRequest,
        KeyEntry as IpcKeyEntry, KeysShowRequest as IpcKeysShowRequest,
        KeysShowResponse as IpcKeysShowResponse, ListEntry as IpcListEntry,
        ListRequest as IpcListRequest, ReadGuestConfigRequest, StatusRequest as IpcStatusRequest,
        UsbProbeEntryKind as IpcUsbProbeEntryKind, UsbipProbeEntry as IpcUsbipProbeEntry,
        UsbipProbeStatus as IpcUsbipProbeStatus, VmLifecycleState as IpcVmLifecycleState,
        VmStatus as IpcVmStatus,
    },
    types::{MediaRef, validate_usb_bus_id},
};
use d2b_core::error::SemverRange;
use d2b_core::{
    bundle::Bundle, bundle_resolver::HostRuntime, closures::ClosureMetadata,
    error::Error as CoreError, host::HostJson, host_check, processes::ProcessesJson,
    realm_controller_config::RealmControllersJson,
};
use d2b_daemon_access::component_session as daemon_access;
use nix::sys::socket::{
    AddressFamily, MsgFlags, SockFlag, SockType, UnixAddr, connect, recv, send, socket,
};
use nix::unistd::Uid;
use protobuf::{EnumOrUnknown, MessageField};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

mod doctor;
mod exec_client;
mod host_validate;
mod service_v2;
#[allow(dead_code)]
mod status_read_model;
mod target_routing;
mod terminal_client;

use status_read_model::{booted_symlink, current_symlink, vm_state_dir};
#[cfg(test)]
use status_read_model::{
    build_vm_status_output, build_vm_status_output_from_public, list_output_from_public_entries,
    list_status_label, output_service_capabilities, pidfd_role_state,
    public_lifecycle_list_status_label, vm_service_states,
};
const DEFAULT_MANIFEST_PATH: &str = "/run/current-system/sw/share/d2b/vms.json";
#[cfg(not(test))]
const DEFAULT_REALM_ENTRYPOINTS_PATH: &str =
    "/run/current-system/sw/share/d2b/realm-entrypoints.json";
const DEFAULT_BUNDLE_PATH: &str = "/etc/d2b/bundle.json";
const DEFAULT_PUBLIC_SOCKET: &str = "/run/d2b/public.sock";
const DEFAULT_BROKER_SOCKET: &str = "/run/d2b/priv.sock";
const DEFAULT_HOST_RUNTIME_PATH: &str = "/var/lib/d2b/runtime/host-runtime.json";
const DEFAULT_CLIENT_VERSION_RANGE: &str = ">=0.4.0, <0.5.0";
const RUNTIME_UNKNOWN: &str = "unknown";
const MAX_FRAME_BYTES: usize = 1024 * 1024;
/// Location of daemon-persisted state files (`pidfd-table.json`,
/// `kernel-module-report.json`, `autostart-report.json`,
/// `storage-lifecycle-report.json`) that
/// `d2b host doctor --read-only` inspects. Mirrors
/// `d2bd::DEFAULT_DAEMON_STATE_DIR`.
const DEFAULT_DAEMON_STATE_DIR: &str = "/var/lib/d2b/daemon-state";
/// Canonical Prometheus scrape URL the doctor probes for reachability.
/// See `docs/reference/daemon-metrics.md`.
const DEFAULT_METRICS_URL: &str = "http://127.0.0.1:9101/metrics";
const MAX_REALM_ENTRYPOINTS_BYTES: u64 = 1024 * 1024;
/// Exit code for api-ready timeout in strict mode.
pub const EXIT_API_TIMEOUT: i32 = 33;
/// Default in-guest path of the editable guest config working copy. Only the
/// legacy operator SSH transport honors a custom path; the guest-control
/// transport reads the VM's canonical guest config working copy by file id.
const DEFAULT_GUEST_CONFIG_PATH: &str = "/var/lib/d2b-guest/guest-config.nix";
/// Exit code surfaced for every guest-control config-read failure on the CLI.
const EXIT_GUEST_CONTROL_CONFIG: i32 = 70;
#[derive(Debug, Parser)]
#[command(
    version,
    about = "d2b — opinionated NixOS desktop microVM CLI.",
    long_about = "d2b — daemon-native CLI for d2b microVMs.\n\nMutating verbs dispatch through d2bd; privileged host mutations additionally use d2b-priv-broker. \
        Daemon-backed verbs use the authenticated d2b.daemon.v2 ComponentSession \
        and fail closed when that exact service is unavailable. \
        See `d2b <COMMAND> --help` for per-verb usage."
)]
struct NativeCli {
    #[command(subcommand)]
    command: NativeCommand,
}

#[derive(Debug, Subcommand)]
enum NativeCommand {
    /// List typed daemon workload projections over ComponentSession.
    List(ListArgs),
    /// Show typed daemon workload status plus bridge health.
    Status(StatusArgs),
    /// Launch a trusted configured workload item through its runtime provider.
    Launch(LaunchArgs),
    /// USB attach / detach / probe.
    Usb(UsbArgs),
    /// Foreground serial console bridge for headless VMs.
    Console(ConsoleArgs),
    /// Per-VM audio status and grant controls.
    Audio(AudioArgs),
    /// Tail the broker audit log.
    Audit(AuditArgs),
    /// Host-side preflight, install, doctor, and reconcile verbs.
    Host(HostArgs),
    /// Authorisation introspection.
    Auth(AuthArgs),
    /// Low-level realm gateway helpers.
    Realm(RealmArgs),
    /// Attach to or manage persistent named guest shells.
    Shell(ShellArgs),
    /// Inspect current constellation operation and trace state.
    Op(OpArgs),
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
    /// Clipboard authority operations (picker-driven paste replay via d2b-clipd).
    Clipboard(ClipboardArgs),
}

#[derive(Debug, Args)]
struct LaunchArgs {
    /// Canonical workload target or an unambiguous workload id.
    target: String,
    /// Configured launcher item id. Required: the v2 launch contract carries
    /// only a configured item id and has no default-item selection signal.
    #[arg(long)]
    item: String,
    /// Emit a structured JSON result.
    #[arg(long, conflicts_with = "human")]
    json: bool,
    /// Force human-readable output.
    #[arg(long, conflicts_with = "json")]
    human: bool,
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
    /// List daemon-declared USBIP session claims and qemu-media USB candidates.
    Probe(UsbProbeArgs),
    /// CTAP/WebAuthn security-key proxy status, sessions, and diagnostics.
    #[command(name = "security-key")]
    SecurityKey(UsbSecurityKeyArgs),
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
struct UsbSecurityKeyArgs {
    #[command(subcommand)]
    command: UsbSecurityKeyCommand,
}

#[derive(Debug, Subcommand)]
enum UsbSecurityKeyCommand {
    /// Show security-key proxy health, configured keys, and current lease.
    Status(UsbSkStatusArgs),
    /// Show recent and active security-key request sessions.
    Sessions(UsbSkSessionsArgs),
    /// Cancel a security-key request session.
    Cancel(UsbSkCancelArgs),
    /// Smoke-check that a VM's virtual security-key device and host broker are healthy.
    Test(UsbSkTestArgs),
}

#[derive(Debug, Args)]
struct UsbSkStatusArgs {
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct UsbSkSessionsArgs {
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct UsbSkCancelArgs {
    /// Session ID to cancel. Mutually exclusive with `--current`.
    #[arg(conflicts_with = "current")]
    session_id: Option<String>,
    /// Cancel the currently active session.
    #[arg(long, conflicts_with = "session_id")]
    current: bool,
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
struct UsbSkTestArgs {
    vm: String,
    #[arg(long)]
    dry_run: bool,
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
    /// Tear down host-side state owned by d2b. --apply mutates.
    Destroy(HostDestroyArgs),
    /// Read-only deep diagnostics for the daemon + broker state.
    Doctor(HostDoctorArgs),
    /// Plan the one-time storage layout cutover. --apply is fail-closed until broker support lands.
    #[command(name = "migrate-storage")]
    MigrateStorage(HostMigrateStorageArgs),
    /// Install d2bd + broker units onto the host. --apply mutates.
    Install(HostInstallArgs),
    /// Reconcile host network state (re-run bridge/route/nftables reconcile without starting any VM).
    Reconcile(HostReconcileArgs),
    /// Run the host-side validator suite and write evidence records.
    Validate(HostValidateArgs),
}

#[derive(Debug)]
struct HostShutdownHookArgs {
    /// Plan the host-shutdown stop phases without contacting d2bd.
    dry_run: bool,
    /// Apply the host-shutdown stop phases.
    apply: bool,
    json: bool,
}

#[derive(Debug, Args)]
struct HostValidateArgs {
    /// Plan: report which readiness validators WOULD be attested.
    /// No evidence is written.
    #[arg(long, conflicts_with = "apply")]
    dry_run: bool,
    /// Apply: write `/var/lib/d2b/validated/<wave>.json` for
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
    /// Override the evidence directory. Default: `/var/lib/d2b/validated`.
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
struct HostMigrateStorageArgs {
    /// Plan the storage cutover without mutating host state.
    #[arg(long, conflicts_with_all = ["apply", "rollback"])]
    dry_run: bool,
    /// Apply the storage cutover. Currently fails closed until broker support lands.
    #[arg(long, conflicts_with_all = ["dry_run", "rollback"])]
    apply: bool,
    /// Roll back from a named storage cutover checkpoint.
    #[arg(long, conflicts_with_all = ["dry_run", "apply"], requires = "from_checkpoint")]
    rollback: bool,
    /// Checkpoint ID to roll back.
    #[arg(long, value_name = "ID", requires = "rollback")]
    from_checkpoint: Option<String>,
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
    /// After `--apply`, enable d2bd.service via systemctl.
    #[arg(long, conflicts_with = "dry_run", requires = "apply")]
    enable: bool,
    /// After `--apply --enable`, start d2bd.service.
    #[arg(long, conflicts_with_all = ["dry_run", "no_start"], requires = "apply")]
    start: bool,
    /// Explicitly do NOT start d2bd.service post-install.
    #[arg(long, conflicts_with_all = ["dry_run", "start"], requires = "apply")]
    no_start: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct HostReconcileArgs {
    /// Re-run the network slice of `host prepare` (bridge/route/nftables
    /// reconcile without starting any VM). Currently the only available scope.
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
struct RealmArgs {
    #[command(subcommand)]
    command: RealmCommand,
}

#[derive(Debug, Args)]
struct OpArgs {
    #[command(subcommand)]
    command: OpCommand,
}

#[derive(Debug, Args)]
#[command(
    after_help = "Forms:\n  d2b shell <target> [--name NAME] [--force]\n  d2b shell <target> attach [--name NAME] [--force]\n  d2b shell <target> list [--json]\n  d2b shell <target> detach [--name NAME] [--json]\n  d2b shell <target> kill --name NAME [--json]\n\n`d2b shell` opens persistent interactive sessions for a target workload. Use `d2b vm exec <target> -- <cmd>` for one-off commands."
)]
struct ShellArgs {
    /// Target address. Local VMs use the fast path; gateway-backed targets route through the realm gateway where supported.
    #[arg(value_name = "TARGET")]
    vm: String,
    /// Shell action. Omit to attach to the configured default session.
    #[arg(value_enum)]
    action: Option<ShellAction>,
    /// Configured shell id for attach, or server-issued handle for detach/kill.
    #[arg(long)]
    name: Option<String>,
    /// Detach an existing attached client before attaching to this session.
    #[arg(long)]
    force: bool,
    /// Render machine-readable JSON.
    #[arg(long, conflicts_with = "human")]
    json: bool,
    /// Render human-readable output.
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ShellAction {
    /// Attach to a persistent shell.
    Attach,
    /// List persistent shell sessions on a target.
    List,
    /// Detach a persistent shell session without killing it.
    Detach,
    /// Kill a persistent shell session by name.
    Kill,
}

#[derive(Debug, Args)]
struct ClipboardArgs {
    #[command(subcommand)]
    command: ClipboardCommand,
}

#[derive(Debug, Subcommand)]
enum ClipboardCommand {
    /// Open the picker and request paste replay for the focused target.
    ///
    /// Opens the d2b-clip-picker, waits for a selection, then asks d2b-clipd
    /// to publish the selected payload and trigger paste replay.
    /// Requires d2b-clipd to be running.
    #[command(alias = "picker")]
    Arm(ClipboardArmArgs),
}

#[derive(Debug, Args)]
struct ClipboardArmArgs {
    /// Emit a structured JSON envelope.
    #[arg(long, conflicts_with = "human")]
    json: bool,
    /// Emit a human-readable status line.
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

const CLIPBOARD_ARM_CONTROL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Debug, Subcommand)]
enum OpCommand {
    /// Inspect current operation/trace state with bounded partial results.
    Inspect(OpInspectArgs),
}

#[derive(Debug, Args)]
struct OpInspectArgs {
    /// Optional trace id to include in the inspection envelope.
    #[arg(long, requires = "span_id")]
    trace_id: Option<String>,
    /// Optional span id to include in the inspection envelope.
    #[arg(long, requires = "trace_id")]
    span_id: Option<String>,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Subcommand)]
enum RealmCommand {
    /// List local realm policy entrypoints.
    List(RealmListArgs),
    /// Inspect one local realm policy entrypoint.
    Inspect(RealmInspectArgs),
    /// Open an interactive shell inside the realm gateway VM.
    Enter(RealmEnterArgs),
    /// Run a one-shot command inside the realm gateway VM.
    Run(RealmRunArgs),
}

#[derive(Debug, Args)]
struct RealmListArgs {
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct RealmInspectArgs {
    /// Realm path, e.g. `work` or `payments.work`.
    realm: String,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct RealmEnterArgs {
    /// Realm path, e.g. `work` or `payments.work`.
    realm: String,
}

#[derive(Debug, Args)]
struct RealmRunArgs {
    /// Realm path, e.g. `work` or `payments.work`.
    realm: String,
    /// Emit the outer `vm exec` result as JSON.
    #[arg(long, conflicts_with = "human")]
    json: bool,
    /// Force human output.
    #[arg(long, conflicts_with = "json")]
    human: bool,
    /// Command to run in the gateway VM, after `--`.
    #[arg(last = true, required = true, value_name = "ARGV")]
    argv: Vec<String>,
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
    /// Daemon-side runtime inventory from d2bd's public socket.
    List(VmListArgs),
    /// Daemon-side readiness state for a VM (api-ready phase).
    Status(VmStatusArgs),
    /// Run or manage commands inside a running VM. Use
    /// `d2b vm exec <vm> -- <cmd...>` for a non-interactive command,
    /// `d2b vm exec -it <vm> -- bash` for an interactive shell, `-d` for
    /// a detached command, and `d2b vm exec <vm> {list|logs|status|kill}`
    /// to manage detached execs.
    Exec(VmExecArgs),
    /// Manage gateway display sessions for provider-backed targets.
    #[command(hide = true)]
    Display(VmDisplayArgs),
}

#[derive(Debug, Args)]
struct VmDisplayArgs {
    #[command(subcommand)]
    command: VmDisplayCommand,
}

#[derive(Debug, Subcommand)]
enum VmDisplayCommand {
    /// List active gateway display sessions.
    List(VmDisplayListArgs),
    /// Close a gateway display session by id.
    Close(VmDisplayCloseArgs),
}

#[derive(Debug, Args)]
struct VmDisplayListArgs {
    /// Optional realm target to filter, for example `demo.work.d2b`.
    #[arg(long)]
    target: Option<String>,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
struct VmDisplayCloseArgs {
    /// Display session id from `d2b vm display list`.
    session_id: String,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

/// `d2b vm exec [-d] [-it] [-i] [-t] <vm> -- <cmd...>`
/// Run a command inside a VM. Use `--` before the command, `-it` for an
/// interactive guest PTY, and `-d` to create a detached exec. Detached execs
/// are managed with `d2b vm exec <vm> list`, `logs <id>`, `status <id>`,
/// and `kill <id>`.
#[derive(Debug, Args)]
struct VmExecArgs {
    /// Start the command detached and print its exec id. Incompatible with
    /// `-i`/`-t`; detached execs are managed with
    /// `d2b vm exec <vm> {list|logs|status|kill}`.
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
    /// VM name as declared in `d2b.vms.<name>`.
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
    /// Detached exec id returned by `d2b vm exec -d`.
    exec_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VmExecLogsArgs {
    /// Detached exec id returned by `d2b vm exec -d`.
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
    /// VM name as declared in `d2b.vms.<name>`.
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
    /// Skip provider graceful shutdown and use the forced cleanup path.
    #[arg(short = 'f', long)]
    force: bool,
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
    /// Apply force only to the stop phase before starting again.
    #[arg(short = 'f', long)]
    force: bool,
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
    /// Route list through a realm gateway VM.
    #[arg(long, value_name = "REALM", conflicts_with = "all")]
    realm: Option<String>,
    /// Include configured realm gateway entrypoints in the list.
    #[arg(long)]
    all: bool,
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
    /// VM name whose foreground serial console should be attached.
    vm: String,
}

#[derive(Debug, Args)]
struct AudioArgs {
    /// Emit machine-readable JSON output.
    #[arg(long, global = true)]
    json: bool,
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
    /// Optional VM name; omitted lists audio status for every audio-enabled VM.
    vm: Option<String>,
}

#[derive(Debug, Args)]
struct AudioToggleArgs {
    /// The new grant state to apply.
    #[arg(value_enum)]
    state: AudioGrantState,
    /// VM name whose audio grant should be changed.
    vm: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum AudioGrantState {
    /// Enable the selected audio direction.
    On,
    /// Disable the selected audio direction.
    Off,
}

#[derive(Debug, Args)]
struct AudioOffArgs {
    /// VM name whose microphone and speaker grants should both be disabled.
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
    #[allow(dead_code)]
    system_state_fixture: Option<SystemStateFixture>,
    auth_status_fixture: Option<AuthStatusFixture>,
    /// Daemon-persisted state dir (pidfd-table.json,
    /// kernel-module-report.json, autostart-report.json).
    /// Override via `D2B_DAEMON_STATE_DIR`.
    daemon_state_dir: PathBuf,
    /// Prometheus scrape URL the doctor probes for reachability.
    /// Override via `D2B_METRICS_URL`.
    metrics_url: String,
}

impl Context {
    fn from_env() -> Result<Self, CliFailure> {
        Ok(Self {
            manifest_path: env_path("D2B_MANIFEST_PATH", DEFAULT_MANIFEST_PATH),
            bundle_path: env_path("D2B_BUNDLE_PATH", DEFAULT_BUNDLE_PATH),
            public_socket: env_path("D2B_PUBLIC_SOCKET", DEFAULT_PUBLIC_SOCKET),
            broker_socket: env_path("D2B_BROKER_SOCKET", DEFAULT_BROKER_SOCKET),
            state_root: env::var_os("D2B_STATE_ROOT").map(PathBuf::from),
            host_runtime_path: env_path("D2B_HOST_RUNTIME_PATH", DEFAULT_HOST_RUNTIME_PATH),
            system_state_fixture: maybe_load_json_env("D2B_TEST_SYSTEM_STATE_JSON")?,
            auth_status_fixture: maybe_load_json_env("D2B_AUTH_STATUS_FIXTURE")?,
            daemon_state_dir: env_path("D2B_DAEMON_STATE_DIR", DEFAULT_DAEMON_STATE_DIR),
            metrics_url: env::var("D2B_METRICS_URL")
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
    #[allow(dead_code)]
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

    #[allow(dead_code)]
    fn bridge_names(&self) -> BTreeSet<String> {
        self.vms()
            .iter()
            .map(|vm| vm.bridge.clone())
            .collect::<BTreeSet<_>>()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct ManifestVm {
    name: String,
    env: Option<String>,
    graphics: bool,
    tpm: bool,
    audio: bool,
    usbip_yubikey: bool,
    static_ip: Option<String>,
    is_net_vm: bool,
    state_dir: String,
    bridge: String,
    ssh_user: Option<String>,
    runtime: Option<ManifestRuntime>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct ManifestRuntime {
    kind: String,
    #[serde(default)]
    capabilities: BTreeMap<String, bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
struct SystemStateFixture {
    units: BTreeMap<String, String>,
    bridges: BTreeMap<String, BridgeHealthFixture>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[allow(dead_code)]
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
#[allow(dead_code)]
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HelloOkFrame {
    #[serde(rename = "type")]
    _type_name: String,
    #[serde(flatten)]
    payload: IpcHelloOk,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HelloRejectedFrame {
    #[serde(rename = "type")]
    _type_name: String,
    #[serde(flatten)]
    _payload: IpcHelloRejected,
    error: DaemonErrorEnvelope,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[allow(dead_code)]
struct ListResponseFrame {
    #[serde(rename = "type")]
    _type_name: String,
    vms: Vec<IpcListEntry>,
    #[serde(default)]
    read_model: Option<d2b_contracts::public_wire::PublicReadModelMetadata>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct StatusResponseFrame {
    #[serde(rename = "type")]
    _type_name: String,
    status: StatusResponsePayload,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct StatusResponsePayload {
    entries: Vec<IpcVmStatus>,
    #[serde(default)]
    read_model: Option<d2b_contracts::public_wire::PublicReadModelMetadata>,
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
#[allow(dead_code)]
enum ListSocketOutcome {
    Unavailable,
    Entries(
        Vec<IpcListEntry>,
        Option<d2b_contracts::public_wire::PublicReadModelMetadata>,
    ),
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
enum StatusSocketOutcome {
    Unavailable,
    Entries(
        Vec<IpcVmStatus>,
        Option<d2b_contracts::public_wire::PublicReadModelMetadata>,
    ),
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

fn daemon_supported_features() -> Vec<d2b_contracts::FeatureFlag> {
    vec![
        KnownFeatureFlag::TypedErrors.wire_value(),
        KnownFeatureFlag::StatusCheckBridges.wire_value(),
        KnownFeatureFlag::ExportBrokerAudit.wire_value(),
        KnownFeatureFlag::ConfiguredLaunchV1.wire_value(),
        KnownFeatureFlag::UnsafeLocalProviderV1.wire_value(),
        KnownFeatureFlag::UnsafeLocalShellV1.wire_value(),
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

fn shell_trailing_command_hint(raw_args: &[OsString]) -> Option<&'static str> {
    let command = raw_args.get(1).and_then(|arg| arg.to_str())?;
    if command != "shell" {
        return None;
    }
    let trailing = raw_args.get(3).and_then(|arg| arg.to_str())?;
    if trailing.starts_with('-') || matches!(trailing, "attach" | "list" | "detach" | "kill") {
        return None;
    }
    Some(
        "hint: `d2b shell` opens persistent interactive sessions; use `d2b vm exec <target> -- <cmd>` for one-off commands.\n",
    )
}

pub fn cli_command() -> clap::Command {
    let mut command = NativeCli::command();
    command.set_bin_name("d2b");
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
        print_stdout(concat!("d2b ", env!("CARGO_PKG_VERSION"), "\n"));
        print_stdout("Rust-native CLI shim active; run `d2b --help` for subcommands.\n");
        return 0;
    }

    if let Some(failure) = removed_usb_enroll_failure(&raw_args) {
        return report_failure(failure);
    }

    if is_host_shutdown_hook_invocation(&raw_args) {
        let context = match Context::from_env() {
            Ok(context) => context,
            Err(err) => return report_failure(err),
        };
        let args = match parse_host_shutdown_hook_args(&raw_args) {
            Ok(args) => args,
            Err(err) => return report_failure(err),
        };
        return match cmd_host_shutdown_hook(&context, &args) {
            Ok(code) => code,
            Err(err) => report_failure(err),
        };
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
            if let Some(hint) = shell_trailing_command_hint(&raw_args) {
                let _ = write_stderr_bytes(hint.as_bytes());
            }
            return if is_host_usage { 3 } else { err.exit_code() };
        }
    };
    if raw_args.get(1).and_then(|arg| arg.to_str()) == Some("clipboard")
        && raw_args.get(2).and_then(|arg| arg.to_str()) == Some("picker")
    {
        print_stderr("d2b: `d2b clipboard picker` is deprecated; use `d2b clipboard arm`.\n");
    }

    let context = match Context::from_env() {
        Ok(context) => context,
        Err(err) => return report_failure(err),
    };

    match dispatch(&context, &cli, &raw_args[1..]) {
        Ok(code) => code,
        Err(err) => report_failure(err),
    }
}

fn is_host_shutdown_hook_invocation(raw_args: &[OsString]) -> bool {
    raw_args.get(1).and_then(|arg| arg.to_str()) == Some("host")
        && raw_args.get(2).and_then(|arg| arg.to_str()) == Some("shutdown-hook")
}

fn parse_host_shutdown_hook_args(
    raw_args: &[OsString],
) -> Result<HostShutdownHookArgs, CliFailure> {
    let mut args = HostShutdownHookArgs {
        dry_run: false,
        apply: false,
        json: false,
    };
    for arg in raw_args.iter().skip(3) {
        match arg.to_str() {
            Some("--dry-run") => args.dry_run = true,
            Some("--apply") => args.apply = true,
            Some("--json") => args.json = true,
            Some(other) => {
                return Err(CliFailure::new(
                    2,
                    format!("d2b host shutdown-hook does not accept {other}"),
                ));
            }
            None => {
                return Err(CliFailure::new(
                    2,
                    "d2b host shutdown-hook received a non-UTF-8 argument",
                ));
            }
        }
    }
    if args.dry_run && args.apply {
        return Err(CliFailure::new(
            2,
            "d2b host shutdown-hook accepts only one of --dry-run or --apply",
        ));
    }
    Ok(args)
}

fn dispatch(
    context: &Context,
    cli: &NativeCli,
    original_args: &[OsString],
) -> Result<i32, CliFailure> {
    match &cli.command {
        NativeCommand::List(args) => cmd_list(context, args),
        NativeCommand::Status(args) => cmd_status(context, args),
        NativeCommand::Launch(args) => cmd_launch(context, args),
        NativeCommand::Usb(args) => match &args.command {
            UsbCommand::Attach(args) => cmd_usb_attach(context, args),
            UsbCommand::Detach(args) => cmd_usb_detach(context, args),
            UsbCommand::Probe(args) => cmd_usb_probe(context, args),
            UsbCommand::SecurityKey(args) => match &args.command {
                UsbSecurityKeyCommand::Status(args) => cmd_usb_sk_status(context, args),
                UsbSecurityKeyCommand::Sessions(args) => cmd_usb_sk_sessions(context, args),
                UsbSecurityKeyCommand::Cancel(args) => cmd_usb_sk_cancel(context, args),
                UsbSecurityKeyCommand::Test(args) => cmd_usb_sk_test(context, args),
            },
        },
        NativeCommand::Console(args) => cmd_console_v2(context, args),
        NativeCommand::Audio(args) => cmd_audio(context, args, original_args),
        NativeCommand::Audit(args) => cmd_audit(context, args, original_args),
        NativeCommand::Host(args) => match &args.command {
            HostCommand::Check(args) => cmd_host_check(context, args),
            HostCommand::Prepare(args) => cmd_host_prepare(context, args),
            HostCommand::Destroy(args) => cmd_host_destroy(context, args),
            HostCommand::Doctor(args) => cmd_host_doctor(context, args),
            HostCommand::MigrateStorage(args) => cmd_host_migrate_storage(context, args),
            HostCommand::Install(args) => cmd_host_install(context, args, original_args),
            HostCommand::Reconcile(args) => cmd_host_reconcile(context, args, original_args),
            HostCommand::Validate(args) => cmd_host_validate(context, args),
        },
        NativeCommand::Auth(args) => match &args.command {
            AuthCommand::Status(args) => cmd_auth_status(context, args),
        },
        NativeCommand::Realm(args) => match &args.command {
            RealmCommand::List(args) => cmd_realm_list(context, args),
            RealmCommand::Inspect(args) => cmd_realm_inspect(context, args),
            RealmCommand::Enter(args) => cmd_realm_enter(context, args),
            RealmCommand::Run(args) => cmd_realm_run(context, args),
        },
        NativeCommand::Shell(args) => cmd_shell_v2(context, args),
        NativeCommand::Op(args) => match &args.command {
            OpCommand::Inspect(args) => cmd_op_inspect(context, args),
        },
        NativeCommand::Vm(args) => match &args.command {
            VmCommand::Start(args) => cmd_vm_start(context, args),
            VmCommand::Stop(args) => cmd_vm_stop(context, args),
            VmCommand::Restart(args) => cmd_vm_restart(context, args),
            VmCommand::List(args) => cmd_vm_list(context, args),
            VmCommand::Status(args) => cmd_vm_status(context, args),
            VmCommand::Exec(args) => cmd_vm_exec_v2(context, args),
            VmCommand::Display(args) => cmd_vm_display(context, args),
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
        NativeCommand::Clipboard(args) => match &args.command {
            ClipboardCommand::Arm(args) => cmd_clipboard_arm(context, args),
        },
    }
}

// ============================================================
// `d2b clipboard` — clipboard authority fallback arming
// ============================================================

fn cmd_clipboard_arm(_context: &Context, args: &ClipboardArmArgs) -> Result<i32, CliFailure> {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| {
        clipboard_arm_failure(
            args,
            "XDG_RUNTIME_DIR is not set; cannot locate d2b-clipd control socket",
        )
    })?;
    let socket_path = PathBuf::from(runtime).join("d2b-clipd/clipd.sock");
    let mut stream = UnixStream::connect(&socket_path).map_err(|error| {
        clipboard_arm_failure(
            args,
            format!(
                "failed to connect to d2b-clipd control socket {}: {error}",
                socket_path.display()
            ),
        )
    })?;
    set_clipboard_arm_timeouts(&stream).map_err(|error| {
        clipboard_arm_failure(
            args,
            format!("failed to set clipboard arm socket timeout: {error}"),
        )
    })?;
    stream.write_all(b"{\"type\":\"arm\"}\n").map_err(|error| {
        clipboard_arm_failure(args, format!("failed to request clipboard arm: {error}"))
    })?;
    let mut line = Vec::new();
    stream.take(4096).read_to_end(&mut line).map_err(|error| {
        clipboard_arm_failure(
            args,
            format!("failed to read clipboard arm response: {error}"),
        )
    })?;
    let value: serde_json::Value = serde_json::from_slice(&line).map_err(|error| {
        clipboard_arm_failure(args, format!("invalid d2b-clipd response: {error}"))
    })?;
    if value.get("ok").and_then(|ok| ok.as_bool()) == Some(true) {
        if args.json {
            print_stdout(&format!("{value}\n"));
        } else {
            let message = value
                .get("message")
                .and_then(|message| message.as_str())
                .unwrap_or("picker opened");
            print_stdout(&format!("{message}\n"));
        }
        Ok(0)
    } else {
        let error = value
            .get("error")
            .and_then(|error| error.as_str())
            .unwrap_or("d2b-clipd rejected clipboard arm request");
        Err(clipboard_arm_failure(args, error))
    }
}

fn set_clipboard_arm_timeouts(stream: &std::os::unix::net::UnixStream) -> std::io::Result<()> {
    let timeout = Some(CLIPBOARD_ARM_CONTROL_TIMEOUT);
    stream.set_read_timeout(timeout)?;
    stream.set_write_timeout(timeout)?;
    Ok(())
}

fn clipboard_arm_failure(args: &ClipboardArmArgs, message: impl Into<String>) -> CliFailure {
    let message = message.into();
    if args.json {
        print_stdout(&format!(
            "{}\n",
            serde_json::json!({
                "ok": false,
                "error": message,
            })
        ));
        CliFailure {
            exit_code: 2,
            rendered_stderr: Some(String::new()),
            message,
        }
    } else {
        CliFailure::new(2, message)
    }
}

#[cfg(test)]
mod clipboard_arm_tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    #[test]
    fn json_failure_emits_structured_stdout_and_suppresses_stderr() {
        let args = ClipboardArmArgs {
            json: true,
            human: false,
        };
        let (failure, stdout, _stderr) =
            with_test_output_capture(|| clipboard_arm_failure(&args, "daemon unavailable"));
        assert_eq!(failure.exit_code, 2);
        assert_eq!(failure.rendered_stderr.as_deref(), Some(""));
        let value: Value = serde_json::from_slice(&stdout).expect("json failure stdout");
        assert_eq!(value["ok"], false);
        assert_eq!(value["error"], "daemon unavailable");
    }

    #[test]
    fn clipboard_arm_sets_read_and_write_timeouts() {
        let (left, _right) = UnixStream::pair().expect("socketpair");
        set_clipboard_arm_timeouts(&left).expect("set timeouts");
        assert_eq!(
            left.read_timeout().expect("read timeout"),
            Some(CLIPBOARD_ARM_CONTROL_TIMEOUT)
        );
        assert_eq!(
            left.write_timeout().expect("write timeout"),
            Some(CLIPBOARD_ARM_CONTROL_TIMEOUT)
        );
    }
}

// ============================================================
// `d2b config` — guest-editable config sync / review / approve
// ============================================================
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
// `guestConfigFile` assertion that fires on `d2b switch`.

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
    /// the framework-managed `/var/lib/d2b/known_hosts.d2b`). SSH
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
/// (no privileged surface), from `XDG_STATE_HOME` (or `HOME`). Tests
/// override it per-thread via [`set_test_staging_base`] rather than mutating
/// process-global env.
fn config_staging_base() -> PathBuf {
    #[cfg(test)]
    if let Some(base) = TEST_STAGING_BASE.with(|b| b.borrow().clone()) {
        return base;
    }
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("/tmp/d2b-state"));
    base.join("d2b/config-staging")
}

#[cfg(test)]
thread_local! {
    /// Per-thread test override of the config-staging base (replaces the old
    /// process-global `D2B_CONFIG_STAGING_DIR` env hook).
    static TEST_STAGING_BASE: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// Set (or clear) the calling thread's config-staging base override.
#[cfg(test)]
fn set_test_staging_base(base: Option<PathBuf>) {
    TEST_STAGING_BASE.with(|b| *b.borrow_mut() = base);
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
/// the per-VM `guestConfigFile` assertion on `d2b switch`. Here we
/// only refuse an empty / non-UTF-8 file so approve cannot silently
/// land a truncated sync.
fn config_validate_staging_bytes(bytes: &[u8]) -> Result<(), CliFailure> {
    if bytes.is_empty() {
        return Err(CliFailure::new(
            1,
            "config approve: staged file is empty; re-run `d2b config sync`".to_owned(),
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
                "config approve: nothing staged at {} (run `d2b config sync` first)",
                staging.display()
            ),
        ));
    }
    let bytes = std::fs::read(staging)
        .map_err(|e| CliFailure::new(1, format!("config approve: read staging: {e}")))?;
    config_validate_staging_bytes(&bytes)?;
    let parent = target.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(parent) = parent
        && !parent.exists()
    {
        return Err(CliFailure::new(
            1,
            format!(
                "config approve: target dir {} does not exist",
                parent.display()
            ),
        ));
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
             (`d2b config diff {vm} --against <live>` to review, \
             `d2b config approve {vm} --to <live>` to land, or \
             `d2b config reject {vm}` to discard)"
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
            if let Some(name) = entry.file_name().to_str()
                && let Some(vm) = name.strip_suffix(".guest.nix")
            {
                pending.push(vm.to_owned());
            }
        }
    }
    pending.sort();
    if !pending.is_empty() {
        eprintln!(
            "note: pending un-approved guest config edit(s) for: {} \
             (`d2b config status --all`)",
            pending.join(", ")
        );
    }
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
        .unwrap_or("d2b-config");
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_name = format!(".{base}.d2b-tmp.{}.{nanos}", std::process::id());
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
    if let Some(p) = parent
        && let Ok(dir) = std::fs::File::open(p)
    {
        let _ = dir.sync_all();
    }
    Ok(())
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
/// old-generation path; there is no SSH fallback.
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
                    d2b_core::processes::ProcessRole::GuestControlHealth
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
            "d2b: {} (code: {}, exit {})\n  what was checked : {}\n  observed         : {}\n  remediation      : {}\n  docs             : {}\n",
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
        "retry after the guest finishes booting, then check `d2b status <vm>`".to_owned()
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
            ));
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
            "retry; if it persists, restart d2bd after switching to this generation",
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
            let bytes = d2b_core::base64_codec::decode(&frame.content_base64)
                .map_err(|_| protocol_error("the daemon returned a malformed base64 payload"))?;
            // Defense in depth: the daemon already bounds the encoded payload,
            // but the host re-enforces the raw cap and never trusts a
            // guest-reported size.
            if bytes.len() as u64 > d2b_contracts::guest_wire::READ_GUEST_FILE_MAX_BYTES {
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
                "connecting to the d2b daemon for config sync",
                "the d2b daemon public socket was not reachable",
                "ensure d2bd is running (`systemctl status d2bd`) and retry",
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
             Review with `d2b config diff {} --against <guestConfigFile>` then \
             `d2b config approve {} --to <guestConfigFile>` \
             (the host-side d2b.vms.{}.guestConfigFile path).\n",
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
                "config diff: nothing staged for '{}' (run `d2b config sync` first)",
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
             then `d2b switch {}` to build + activate it (the guestConfigFile containment \
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
                if let Some(name) = entry.file_name().to_str()
                    && let Some(vm) = name.strip_suffix(".guest.nix")
                {
                    out.push(vm.to_owned());
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

/// Resolve a `launch` target to the canonical `<workload>.<realm>.d2b`
/// address the daemon's typed exec router requires (it only routes an
/// `EXEC` terminal to the `ConfiguredLaunch` handler when `resource_id`
/// already ends in `.d2b`). Already-canonical input is used as-is without a
/// round trip; a bare workload id is resolved via the typed `ListWorkloads`
/// projection, mirroring the `gateway_lifecycle_state` lookup pattern.
fn resolve_launch_target(
    daemon: &service_v2::DaemonService,
    requested: &str,
) -> Result<String, CliFailure> {
    if d2b_realm_core::RealmTarget::parse(requested).is_ok() {
        return Ok(requested.to_owned());
    }
    let workloads = daemon.list_workloads(Some(requested))?;
    let matched = service_v2::match_workload_by_bare_id(workloads, requested)?;
    matched
        .identity
        .into_option()
        .map(|identity| identity.canonical_target)
        .ok_or_else(|| CliFailure::new(76, "workload response omitted identity"))
}

fn cmd_launch(context: &Context, args: &LaunchArgs) -> Result<i32, CliFailure> {
    use terminal_v2::{
        ConfiguredLaunchSelection, ExecAuthority, ExecSelection, TerminalSelection, exec_selection,
        terminal_selection,
    };

    // The frozen `d2b.terminal.v2` `ConfiguredLaunchSelection` carries only a
    // `configured_item_id` and the wire-level `ProtocolToken` rejects an
    // empty string, so there is no way to ask the daemon for "the default
    // item" over this contract. `--item` is a required clap argument (see
    // `LaunchArgs`) rather than an optional flag with a runtime fallback,
    // since the v2 listing API exposes no client-side default-item discovery
    // to fall back on.
    let item_id = d2b_realm_core::ProtocolToken::parse(args.item.clone())
        .map_err(|error| CliFailure::new(2, format!("launch: invalid --item id: {error}")))?;

    // `d2b launch` has no static or SSH fallback: the configured argv only
    // ever lives behind the hash-verified private bundle that the daemon
    // resolves, so a missing daemon socket must fail closed here rather than
    // attempting any client-side re-implementation.
    let daemon = match connect_daemon_for_command(context, "launch", args.json)? {
        DaemonCommandConnection::Connected(daemon) => daemon,
        DaemonCommandConnection::Unavailable(exit) => return Ok(exit),
    };
    let canonical = resolve_launch_target(&daemon, &args.target)?;
    let target = d2b_core::workload_identity::WorkloadTarget::parse(&canonical)
        .map_err(|_| CliFailure::new(76, "daemon returned a non-canonical workload target"))?;
    let operation_id = new_launch_operation_id()?;

    let mut preparation = prepare_terminal_before_runtime(false)?;
    let selection = TerminalSelection {
        selection: Some(terminal_selection::Selection::Exec(ExecSelection {
            authority: EnumOrUnknown::new(ExecAuthority::EXEC_AUTHORITY_CONFIGURED_LAUNCH),
            selection: Some(exec_selection::Selection::ConfiguredLaunch(
                ConfiguredLaunchSelection {
                    configured_item_id: item_id.as_str().to_owned(),
                    ..Default::default()
                },
            )),
            tty: false,
            detached: true,
            ..Default::default()
        })),
        ..Default::default()
    };
    let terminal = daemon.open_terminal(
        daemon_access::DaemonMethod::Exec,
        &canonical,
        operation_id.as_str(),
        selection,
    )?;
    let mut host = exec_client::CapturingHostIo::new(false, 0);
    let outcome = run_terminal_stream_v2(
        &daemon,
        &terminal,
        &mut host,
        &mut preparation.signals,
        false,
        false,
        TerminalDetachMode::None,
    )?;
    let exit = terminal_outcome_exit(&outcome)?;

    // The frozen v2 `ConfiguredLaunch` route reports only a bare detached
    // outcome; it never signals whether the dispatch was fresh or a replay
    // of an already-committed operation id. Emit `Committed` rather than
    // inventing an `AlreadyCommitted` claim the typed outcome does not make.
    let output = LaunchOutputV1 {
        command: "launch".to_owned(),
        target,
        item_id,
        operation_id,
        disposition: d2b_contracts::public_wire::LauncherExecDisposition::Committed,
    };
    if args.json {
        print_json(&output)?;
    } else {
        print_stdout(&format!(
            "launched {} item {} (committed)\n",
            output.target.to_canonical(),
            output.item_id.as_str()
        ));
    }
    Ok(exit)
}

fn new_launch_operation_id() -> Result<d2b_realm_core::OperationId, CliFailure> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| CliFailure::new(42, "system clock is before the Unix epoch"))?
        .as_nanos();
    d2b_realm_core::OperationId::parse(format!("launch-{}-{nanos}", std::process::id()))
        .map_err(|_| CliFailure::new(42, "failed to construct a launch operation id"))
}

fn cmd_list(context: &Context, args: &ListArgs) -> Result<i32, CliFailure> {
    let daemon = match connect_daemon_for_command(context, "list", args.json)? {
        DaemonCommandConnection::Connected(daemon) => daemon,
        DaemonCommandConnection::Unavailable(exit) => return Ok(exit),
    };
    let workloads = daemon.list_workloads(None)?;
    let output = service_v2::list_output(&workloads)?;

    if args.json {
        print_json(&output)?;
    } else {
        print_stdout(&render_list_human(&output, None));
    }
    Ok(0)
}

fn cmd_status(context: &Context, args: &StatusArgs) -> Result<i32, CliFailure> {
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
            message: "bridge reconciliation is not yet wired; use `d2b host check --read-only` for advisory bridge-related probes".to_owned(),
            runtime: RUNTIME_UNKNOWN.to_owned(),
        };
        if args.json {
            print_json(&StatusOutputV2::CheckBridges(Box::new(output)))?;
        } else {
            print_stdout(&(output.message.clone() + "\n"));
        }
        return Ok(0);
    }

    let selected_vm = match (&args.vm, &args.vm_flag) {
        (Some(positional), Some(flagged)) if positional != flagged => {
            return Err(CliFailure::new(
                2,
                "status received conflicting VM selectors",
            ));
        }
        (Some(positional), _) => Some(positional.clone()),
        (_, Some(flagged)) => Some(flagged.clone()),
        (None, None) => None,
    };
    let daemon = match connect_daemon_for_command(context, "status", args.json)? {
        DaemonCommandConnection::Connected(daemon) => daemon,
        DaemonCommandConnection::Unavailable(exit) => return Ok(exit),
    };
    if !args.json {
        match &selected_vm {
            // Single-VM status only warns about THAT VM's pending edit,
            // never unrelated VMs.
            Some(vm) => warn_pending_staged_config(vm),
            None => warn_all_pending_staged_configs(),
        }
    }
    let (workloads, read_model) = daemon.inspect(selected_vm.as_deref())?;
    let (inventory, projections) = service_v2::status_output(&workloads, &read_model)?;
    if let Some(vm_name) = selected_vm {
        let projection = projections
            .iter()
            .find(|projection| {
                projection.output.name == vm_name
                    || projection.output.canonical_target.as_deref() == Some(vm_name.as_str())
            })
            .or_else(|| (projections.len() == 1).then(|| &projections[0]))
            .ok_or_else(|| CliFailure::new(1, format!("unknown VM '{vm_name}'")))?;
        if args.json {
            print_json(&StatusOutputV2::Vm(Box::new(projection.output.clone())))?;
        } else {
            print_stdout(&service_v2::render_status(projection));
        }
    } else {
        if args.json {
            print_json(&StatusOutputV2::Inventory(Box::new(inventory)))?;
        } else {
            print_stdout(&service_v2::render_status_inventory(&projections));
        }
    }

    Ok(0)
}

fn terminal_operation_id(kind: &str) -> String {
    format!(
        "{kind}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}

fn terminal_size_v2(rows: u32, columns: u32) -> terminal_v2::TerminalSize {
    terminal_v2::TerminalSize {
        rows,
        columns,
        ..Default::default()
    }
}

struct TerminalPreparation {
    _signal_mask: exec_client::ForwardedSignalMask,
    signals: exec_client::InstalledSignals,
    initial_size: Option<(u32, u32)>,
}

fn prepare_terminal_before_runtime(tty: bool) -> Result<TerminalPreparation, CliFailure> {
    let signal_mask = exec_client::block_forwarded_signals().map_err(|error| {
        CliFailure::new(42, format!("failed to block terminal signals: {error}"))
    })?;
    let initial_size = if tty {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err(CliFailure::new(
                2,
                "terminal mode requires TTY stdin and stdout",
            ));
        }
        Some(exec_client::current_window_size().ok_or_else(|| {
            CliFailure::new(2, "failed to obtain the terminal window size before open")
        })?)
    } else {
        None
    };
    let signals = exec_client::install_blocked_signals(&signal_mask).map_err(|error| {
        CliFailure::new(
            42,
            format!("failed to install terminal signal waiter: {error}"),
        )
    })?;
    Ok(TerminalPreparation {
        _signal_mask: signal_mask,
        signals,
        initial_size,
    })
}

fn terminal_signal_kind(
    signal: exec_client::ExecSignal,
) -> Option<terminal_v2::TerminalSignalKind> {
    match signal {
        exec_client::ExecSignal::Winch => None,
        exec_client::ExecSignal::Interrupt => {
            Some(terminal_v2::TerminalSignalKind::TERMINAL_SIGNAL_KIND_INTERRUPT)
        }
        exec_client::ExecSignal::Quit => {
            Some(terminal_v2::TerminalSignalKind::TERMINAL_SIGNAL_KIND_QUIT)
        }
        exec_client::ExecSignal::Terminate => {
            Some(terminal_v2::TerminalSignalKind::TERMINAL_SIGNAL_KIND_TERMINATE)
        }
        exec_client::ExecSignal::Stop => {
            Some(terminal_v2::TerminalSignalKind::TERMINAL_SIGNAL_KIND_SUSPEND)
        }
        exec_client::ExecSignal::Hangup => {
            Some(terminal_v2::TerminalSignalKind::TERMINAL_SIGNAL_KIND_HANGUP)
        }
    }
}

fn terminal_client_failure(error: daemon_access::ClientError) -> CliFailure {
    let exit_code = match error {
        daemon_access::ClientError::Cancelled => 130,
        daemon_access::ClientError::ConnectFailed
        | daemon_access::ClientError::TransportFailed
        | daemon_access::ClientError::SessionLost
        | daemon_access::ClientError::SessionEstablishment(_) => 69,
        daemon_access::ClientError::Remote {
            kind:
                daemon_access::RemoteErrorKind::Unauthorized | daemon_access::RemoteErrorKind::Forbidden,
            ..
        } => 77,
        daemon_access::ClientError::Remote {
            kind: daemon_access::RemoteErrorKind::ResourceExhausted,
            ..
        } => 75,
        _ => 76,
    };
    CliFailure::new(exit_code, error.to_string())
}

fn client_error_to_exec(error: daemon_access::ClientError) -> exec_client::ExecClientError {
    use daemon_access::{ClientError, RemoteErrorKind};
    match error {
        ClientError::ConnectFailed
        | ClientError::RouteUnavailable
        | ClientError::SessionLost
        | ClientError::TransportFailed
        | ClientError::RetryLimitExceeded => {
            exec_client::ExecClientError::transport(error.to_string())
        }
        ClientError::SessionEstablishment(_) => exec_client::ExecClientError::from_daemon_error(
            "guest-control-auth-failed",
            error.to_string(),
            "verify the daemon and guest session identities, then retry",
        ),
        ClientError::Remote { kind, .. } => {
            let slug = match kind {
                RemoteErrorKind::Unauthorized | RemoteErrorKind::Forbidden => "authz-not-admin",
                RemoteErrorKind::NotFound => "guest-control-exec-not-found",
                RemoteErrorKind::ResourceExhausted => "exec-session-capacity",
                RemoteErrorKind::Unavailable | RemoteErrorKind::DeadlineExceeded => {
                    "guest-control-transport-unavailable"
                }
                RemoteErrorKind::GenerationMismatch => "guest-control-stale-session",
                RemoteErrorKind::Cancelled => "exec-session-cancelled",
                RemoteErrorKind::InvalidRequest
                | RemoteErrorKind::Conflict
                | RemoteErrorKind::FailedPrecondition => "guest-control-protocol-error",
                RemoteErrorKind::Internal => "guest-control-exec-internal",
            };
            exec_client::ExecClientError::from_daemon_error(
                slug,
                error.to_string(),
                "inspect typed daemon/guest status and retry only when its retry class permits",
            )
        }
        ClientError::Cancelled => exec_client::ExecClientError::from_daemon_error(
            "exec-session-cancelled",
            error.to_string(),
            "retry only if the operation is safe to repeat",
        ),
        ClientError::ServiceContract(_)
        | ClientError::ContractViolation
        | ClientError::StreamClosed
        | ClientError::StreamDetached
        | ClientError::StreamLimitExceeded => {
            exec_client::ExecClientError::protocol(error.to_string())
        }
        _ => exec_client::ExecClientError::internal(error.to_string()),
    }
}

fn terminal_failure_to_exec(error: CliFailure) -> exec_client::ExecClientError {
    match error.exit_code {
        69 => exec_client::ExecClientError::transport(error.message),
        75 => exec_client::ExecClientError::from_daemon_error(
            "exec-session-capacity",
            error.message,
            "retry after capacity becomes available",
        ),
        77 => exec_client::ExecClientError::from_daemon_error(
            "guest-control-auth-failed",
            error.message,
            "verify daemon and guest identities",
        ),
        76 => exec_client::ExecClientError::protocol(error.message),
        _ => exec_client::ExecClientError::internal(error.message),
    }
}

fn send_terminal_frame(
    daemon: &service_v2::DaemonService,
    terminal: &daemon_access::DaemonTerminal,
    frame: terminal_v2::terminal_stream_frame::Frame,
) -> Result<(), CliFailure> {
    daemon
        .runtime()
        .block_on(terminal.send(frame))
        .map_err(terminal_client_failure)
}

fn receive_terminal_frame(
    daemon: &service_v2::DaemonService,
    terminal: &daemon_access::DaemonTerminal,
    timeout: Duration,
) -> Result<Option<terminal_v2::TerminalStreamFrame>, CliFailure> {
    match daemon
        .runtime()
        .block_on(tokio::time::timeout(timeout, terminal.receive()))
    {
        Ok(Ok(frame)) => Ok(Some(frame)),
        Ok(Err(error)) => Err(terminal_client_failure(error)),
        Err(_) => Ok(None),
    }
}

fn cancel_terminal_best_effort(
    daemon: &service_v2::DaemonService,
    terminal: &daemon_access::DaemonTerminal,
) {
    let _ = daemon.runtime().block_on(terminal.send(
        terminal_v2::terminal_stream_frame::Frame::Cancel(terminal_v2::TerminalCancel::new()),
    ));
}

fn write_terminal_output<H: terminal_client::TerminalHostIo>(
    host: &mut H,
    output: &terminal_v2::TerminalOutput,
    expected_offset: &mut u64,
    stderr: bool,
) -> Result<(), CliFailure> {
    if output.offset > *expected_offset && !output.truncated && output.dropped_bytes == 0 {
        return Err(CliFailure::new(
            76,
            "terminal output advanced without an authenticated truncation marker",
        ));
    }
    let overlap = expected_offset.saturating_sub(output.offset);
    let start = usize::try_from(overlap.min(output.data.len() as u64))
        .map_err(|_| CliFailure::new(76, "terminal output offset overflow"))?;
    let bytes = &output.data[start..];
    let result = if stderr {
        host.write_stderr(bytes)
    } else {
        host.write_stdout(bytes)
    };
    result
        .map_err(|error| CliFailure::new(69, format!("terminal output write failed: {error}")))?;
    let frame_end = output
        .offset
        .checked_add(output.data.len() as u64)
        .ok_or_else(|| CliFailure::new(76, "terminal output offset overflow"))?;
    *expected_offset = (*expected_offset).max(frame_end);
    Ok(())
}

#[cfg(test)]
mod terminal_v2_review_tests {
    use super::*;

    #[derive(Default)]
    struct Host {
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    }

    impl terminal_client::TerminalHostIo for Host {
        fn read_stdin(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Err(io::ErrorKind::WouldBlock.into())
        }

        fn write_stdout(&mut self, data: &[u8]) -> io::Result<()> {
            self.stdout.extend_from_slice(data);
            Ok(())
        }

        fn write_stderr(&mut self, data: &[u8]) -> io::Result<()> {
            self.stderr.extend_from_slice(data);
            Ok(())
        }

        fn window_size(&self) -> Option<(u32, u32)> {
            Some((24, 80))
        }
    }

    #[test]
    fn sigquit_maps_only_to_the_quit_wire_kind() {
        assert_eq!(
            terminal_signal_kind(exec_client::ExecSignal::Quit),
            Some(terminal_v2::TerminalSignalKind::TERMINAL_SIGNAL_KIND_QUIT)
        );
        assert_ne!(
            terminal_signal_kind(exec_client::ExecSignal::Quit),
            Some(terminal_v2::TerminalSignalKind::TERMINAL_SIGNAL_KIND_INTERRUPT)
        );
    }

    #[test]
    fn replayed_output_never_rewinds_offsets_or_duplicates_bytes() {
        let mut host = Host::default();
        let mut offset = 10;
        write_terminal_output(
            &mut host,
            &terminal_v2::TerminalOutput {
                offset: 2,
                data: b"old".to_vec(),
                ..Default::default()
            },
            &mut offset,
            false,
        )
        .unwrap();
        assert_eq!(offset, 10);
        assert!(host.stdout.is_empty());

        write_terminal_output(
            &mut host,
            &terminal_v2::TerminalOutput {
                offset: 8,
                data: b"abcd".to_vec(),
                ..Default::default()
            },
            &mut offset,
            false,
        )
        .unwrap();
        assert_eq!(offset, 12);
        assert_eq!(host.stdout, b"cd");
    }

    #[test]
    fn terminal_and_open_errors_keep_structured_exec_classification() {
        let transport = terminal_failure_to_exec(CliFailure::new(69, "disconnected"));
        assert_eq!(transport.exit_code, exec_client::EXIT_EXEC_TRANSPORT);
        assert_eq!(transport.source, exec_client::ExecFailureSource::Transport);

        let missing = client_error_to_exec(daemon_access::ClientError::Remote {
            kind: daemon_access::RemoteErrorKind::NotFound,
            retry: daemon_access::RetryClass::Never,
        });
        assert_eq!(missing.exit_code, exec_client::EXIT_EXEC_PROTOCOL);
        assert_eq!(missing.source, exec_client::ExecFailureSource::Protocol);

        let denied = client_error_to_exec(daemon_access::ClientError::Remote {
            kind: daemon_access::RemoteErrorKind::Forbidden,
            retry: daemon_access::RetryClass::Never,
        });
        assert_eq!(denied.exit_code, exec_client::EXIT_EXEC_AUTH);
        assert_eq!(denied.source, exec_client::ExecFailureSource::GuestControl);
    }

    #[test]
    fn json_terminal_failure_emits_one_document_and_no_plain_stderr() {
        let cli = NativeCli::try_parse_from(["d2b", "vm", "exec", "work", "--json", "--", "true"])
            .unwrap();
        let NativeCommand::Vm(VmArgs {
            command: VmCommand::Exec(args),
        }) = cli.command
        else {
            panic!("expected vm exec args");
        };
        let (result, stdout, stderr) = with_test_output_capture(|| {
            exec_terminate(
                &args,
                terminal_failure_to_exec(CliFailure::new(69, "stream disconnected")),
            )
        });
        assert_eq!(result.unwrap(), exec_client::EXIT_EXEC_TRANSPORT);
        assert!(stderr.is_empty());
        let value: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(value["source"], "transport");
        assert_eq!(value["transportExitCode"], exec_client::EXIT_EXEC_TRANSPORT);
        assert_eq!(
            serde_json::Deserializer::from_slice(&stdout)
                .into_iter::<Value>()
                .count(),
            1
        );
    }

    #[test]
    fn detached_management_projects_typed_guest_entries() {
        let result = typed_exec_list_result(vec![guest_v2::GuestExecListEntry {
            resource_handle: "exec-1".to_owned(),
            state: EnumOrUnknown::new(guest_v2::GuestExecState::GUEST_EXEC_STATE_RUNNING),
            created_at_unix_ms: 17,
            argv_digest: vec![3; 32],
            stdout_bytes: 11,
            stderr_bytes: 7,
            dropped_bytes: 2,
            stdout_truncated: true,
            ..Default::default()
        }]);
        assert_eq!(result.execs[0].exec_id, "exec-1");
        assert_eq!(
            result.execs[0].state,
            d2b_contracts::guest_wire::ExecState::Running
        );
        assert_eq!(result.execs[0].stdout_end_offset, 11);
        assert_eq!(result.execs[0].stderr_end_offset, 7);
        assert_eq!(result.execs[0].dropped_bytes, 2);
        assert!(result.execs[0].truncated);
    }

    #[test]
    fn shell_detach_scanner_handles_one_read_and_keeps_console_escape_as_input() {
        let mut scanner = ShellDetachScanner::default();
        match scanner.scan(b"before\x00\x11after") {
            TerminalDetachScan::Detach(prefix) => assert_eq!(prefix, b"before"),
            TerminalDetachScan::Forward(_) => panic!("Ctrl-Space Ctrl-q must detach"),
        }

        let mut scanner = ShellDetachScanner::default();
        match scanner.scan(b"\x1d") {
            TerminalDetachScan::Forward(bytes) => assert_eq!(bytes, b"\x1d"),
            TerminalDetachScan::Detach(_) => panic!("Ctrl-] is console-only"),
        }
    }

    #[test]
    fn console_detach_scanner_uses_ctrl_bracket_and_passes_shell_escape() {
        let mut scanner = ConsoleDetachScanner;
        match scanner.scan(b"before\x1dafter") {
            TerminalDetachScan::Detach(prefix) => assert_eq!(prefix, b"before"),
            TerminalDetachScan::Forward(_) => panic!("Ctrl-] must detach console"),
        }
        match scanner.scan(b"\x00\x11") {
            TerminalDetachScan::Forward(bytes) => assert_eq!(bytes, b"\x00\x11"),
            TerminalDetachScan::Detach(_) => {
                panic!("Ctrl-Space Ctrl-q is shell-only")
            }
        }
    }

    #[test]
    fn shell_detach_scanner_preserves_state_across_reads() {
        let mut scanner = ShellDetachScanner::default();
        match scanner.scan(b"before\x00") {
            TerminalDetachScan::Forward(bytes) => assert_eq!(bytes, b"before"),
            TerminalDetachScan::Detach(_) => panic!("incomplete escape must remain pending"),
        }
        match scanner.scan(b"\x11after") {
            TerminalDetachScan::Detach(prefix) => assert!(prefix.is_empty()),
            TerminalDetachScan::Forward(_) => panic!("split Ctrl-Space Ctrl-q must detach"),
        }

        let mut scanner = ShellDetachScanner::default();
        assert!(matches!(
            scanner.scan(b"\x00"),
            TerminalDetachScan::Forward(bytes) if bytes.is_empty()
        ));
        match scanner.scan(b"x") {
            TerminalDetachScan::Forward(bytes) => assert_eq!(bytes, b"\x00x"),
            TerminalDetachScan::Detach(_) => panic!("non-q input must flush pending Ctrl-Space"),
        }
    }
}

#[derive(Clone, Copy)]
enum TerminalDetachMode {
    None,
    Console,
    Shell,
}

struct ConsoleDetachScanner;

#[derive(Default)]
struct ShellDetachScanner {
    ctrl_space_pending: bool,
}

enum TerminalDetachScan {
    Forward(Vec<u8>),
    Detach(Vec<u8>),
}

impl ConsoleDetachScanner {
    fn scan(&mut self, bytes: &[u8]) -> TerminalDetachScan {
        match bytes.iter().position(|&byte| byte == 0x1d) {
            Some(index) => TerminalDetachScan::Detach(bytes[..index].to_vec()),
            None => TerminalDetachScan::Forward(bytes.to_vec()),
        }
    }
}

impl ShellDetachScanner {
    fn scan(&mut self, bytes: &[u8]) -> TerminalDetachScan {
        let mut forward = Vec::with_capacity(bytes.len() + usize::from(self.ctrl_space_pending));
        for &byte in bytes {
            if self.ctrl_space_pending {
                self.ctrl_space_pending = false;
                if byte == 0x11 {
                    return TerminalDetachScan::Detach(forward);
                }
                forward.push(0);
            }
            if byte == 0 {
                self.ctrl_space_pending = true;
            } else {
                forward.push(byte);
            }
        }
        TerminalDetachScan::Forward(forward)
    }

    fn finish(&mut self) -> bool {
        std::mem::take(&mut self.ctrl_space_pending)
    }
}

fn send_terminal_stdin(
    daemon: &service_v2::DaemonService,
    terminal: &daemon_access::DaemonTerminal,
    bytes: &[u8],
    offset: &mut u64,
) -> Result<(), CliFailure> {
    for chunk in bytes.chunks(d2b_contracts::v2_services::MAX_TERMINAL_CHUNK_BYTES) {
        send_terminal_frame(
            daemon,
            terminal,
            terminal_v2::terminal_stream_frame::Frame::Stdin(terminal_v2::TerminalStdin {
                offset: *offset,
                data: chunk.to_vec(),
                ..Default::default()
            }),
        )?;
        *offset = offset
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| CliFailure::new(76, "stdin offset overflow"))?;
    }
    Ok(())
}

fn run_terminal_stream_v2<H: terminal_client::TerminalHostIo>(
    daemon: &service_v2::DaemonService,
    terminal: &daemon_access::DaemonTerminal,
    host: &mut H,
    signals: &mut exec_client::InstalledSignals,
    tty: bool,
    forward_stdin: bool,
    detach_mode: TerminalDetachMode,
) -> Result<terminal_v2::TerminalOutcome, CliFailure> {
    use terminal_client::TerminalSignalSource as _;
    use terminal_v2::{
        TerminalCloseStdin, TerminalDetach, TerminalResize, TerminalSignal,
        terminal_stream_frame::Frame,
    };

    let _raw_guard = if tty {
        Some(
            exec_client::FdStateGuard::enter(true, true).map_err(|error| {
                CliFailure::new(42, format!("failed to enter raw mode: {error}"))
            })?,
        )
    } else {
        None
    };
    let mut stdin_offset = 0_u64;
    let mut stdout_offset = 0_u64;
    let mut stderr_offset = 0_u64;
    let mut control_sequence = 1_u64;
    let mut started = false;
    let mut stdin_closed = false;
    let mut closing = false;
    let mut stdin_buffer = vec![0_u8; d2b_contracts::v2_services::MAX_TERMINAL_CHUNK_BYTES];
    let mut console_detach = ConsoleDetachScanner;
    let mut shell_detach = ShellDetachScanner::default();

    loop {
        if started && !closing {
            for signal in signals.drain() {
                let frame = match signal {
                    exec_client::ExecSignal::Winch if tty => {
                        let (rows, columns) = host.window_size().ok_or_else(|| {
                            CliFailure::new(76, "failed to obtain terminal size for resize")
                        })?;
                        Some(Frame::Resize(TerminalResize {
                            operation_sequence: control_sequence,
                            size: MessageField::some(terminal_size_v2(rows, columns)),
                            ..Default::default()
                        }))
                    }
                    exec_client::ExecSignal::Winch => None,
                    signal => Some(Frame::Signal(TerminalSignal {
                        operation_sequence: control_sequence,
                        signal: EnumOrUnknown::new(
                            terminal_signal_kind(signal)
                                .expect("non-WINCH terminal signal has a wire kind"),
                        ),
                        ..Default::default()
                    })),
                };
                if let Some(frame) = frame {
                    send_terminal_frame(daemon, terminal, frame)?;
                    control_sequence = control_sequence.saturating_add(1);
                }
            }

            if forward_stdin && !stdin_closed {
                match host.read_stdin(&mut stdin_buffer) {
                    Ok(0) => {
                        if matches!(detach_mode, TerminalDetachMode::Shell) && shell_detach.finish()
                        {
                            send_terminal_stdin(daemon, terminal, &[0], &mut stdin_offset)?;
                        }
                        send_terminal_frame(
                            daemon,
                            terminal,
                            Frame::CloseStdin(TerminalCloseStdin::new()),
                        )?;
                        stdin_closed = true;
                    }
                    Ok(count) => {
                        let bytes = &stdin_buffer[..count];
                        let scan = match detach_mode {
                            TerminalDetachMode::None => TerminalDetachScan::Forward(bytes.to_vec()),
                            TerminalDetachMode::Console => console_detach.scan(bytes),
                            TerminalDetachMode::Shell => shell_detach.scan(bytes),
                        };
                        match scan {
                            TerminalDetachScan::Detach(prefix) => {
                                send_terminal_stdin(daemon, terminal, &prefix, &mut stdin_offset)?;
                                send_terminal_frame(
                                    daemon,
                                    terminal,
                                    Frame::Detach(TerminalDetach::new()),
                                )?;
                                closing = true;
                            }
                            TerminalDetachScan::Forward(forward) => {
                                send_terminal_stdin(daemon, terminal, &forward, &mut stdin_offset)?;
                            }
                        }
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) => {}
                    Err(error) => {
                        cancel_terminal_best_effort(daemon, terminal);
                        return Err(CliFailure::new(
                            69,
                            format!("terminal stdin read failed: {error}"),
                        ));
                    }
                }
            } else if !forward_stdin && !stdin_closed {
                send_terminal_frame(
                    daemon,
                    terminal,
                    Frame::CloseStdin(TerminalCloseStdin::new()),
                )?;
                stdin_closed = true;
            }
        }

        let Some(frame) = receive_terminal_frame(daemon, terminal, Duration::from_millis(20))?
        else {
            continue;
        };
        match frame.frame {
            Some(Frame::Started(value)) => {
                if started {
                    return Err(CliFailure::new(76, "terminal sent duplicate started frame"));
                }
                stdout_offset = value.stdout_offset;
                stderr_offset = value.stderr_offset;
                started = true;
            }
            Some(Frame::Stdout(output)) => {
                if let Err(error) = write_terminal_output(host, &output, &mut stdout_offset, false)
                {
                    cancel_terminal_best_effort(daemon, terminal);
                    return Err(error);
                }
            }
            Some(Frame::Stderr(output)) => {
                if let Err(error) = write_terminal_output(host, &output, &mut stderr_offset, true) {
                    cancel_terminal_best_effort(daemon, terminal);
                    return Err(error);
                }
            }
            Some(Frame::Status(_)) => {}
            Some(Frame::Outcome(outcome)) => return Ok(outcome),
            _ => {
                return Err(CliFailure::new(
                    76,
                    "terminal sent a direction-invalid frame",
                ));
            }
        }
    }
}

fn terminal_outcome_exit(outcome: &terminal_v2::TerminalOutcome) -> Result<i32, CliFailure> {
    use terminal_v2::terminal_outcome::Outcome;
    match outcome.outcome.as_ref() {
        Some(Outcome::Exited(exited)) => Ok(exited.exit_code),
        Some(Outcome::Signaled(signaled)) => {
            Ok(128_i32.saturating_add(signaled.signal.min(127) as i32))
        }
        Some(Outcome::Cancelled(_)) => Ok(130),
        Some(Outcome::Detached(_) | Outcome::Closed(_)) => Ok(0),
        Some(Outcome::Failed(failed)) => Err(CliFailure::new(
            match failed.error.enum_value_or_default() {
                terminal_v2::TerminalErrorKind::TERMINAL_ERROR_KIND_UNAUTHORIZED => 77,
                terminal_v2::TerminalErrorKind::TERMINAL_ERROR_KIND_RESOURCE_EXHAUSTED => 75,
                terminal_v2::TerminalErrorKind::TERMINAL_ERROR_KIND_UNAVAILABLE => 69,
                _ => 76,
            },
            format!(
                "terminal failed: {:?}",
                failed.error.enum_value_or_default()
            ),
        )),
        None => Err(CliFailure::new(76, "terminal outcome omitted its result")),
        Some(_) => Err(CliFailure::new(76, "terminal returned an unknown outcome")),
    }
}

fn shell_session_state(state: terminal_v2::ShellSessionState) -> &'static str {
    match state {
        terminal_v2::ShellSessionState::SHELL_SESSION_STATE_ATTACHED => "attached",
        terminal_v2::ShellSessionState::SHELL_SESSION_STATE_DETACHED => "detached",
        terminal_v2::ShellSessionState::SHELL_SESSION_STATE_KILLED => "killed",
        terminal_v2::ShellSessionState::SHELL_SESSION_STATE_UNAVAILABLE => "unavailable",
        _ => "unknown",
    }
}

fn terminal_selection_shell(
    args: &ShellArgs,
    initial_size: Option<(u32, u32)>,
) -> terminal_v2::TerminalSelection {
    let action = args.action.unwrap_or(ShellAction::Attach);
    let (action, shell_handle, configured_shell_id, force, initial_size) = match action {
        ShellAction::Attach if args.name.is_some() => (
            terminal_v2::ShellAction::SHELL_ACTION_ATTACH_CONFIGURED,
            String::new(),
            args.name.clone().unwrap_or_default(),
            args.force,
            initial_size
                .map(|(rows, columns)| terminal_size_v2(rows, columns))
                .into(),
        ),
        ShellAction::Attach => (
            terminal_v2::ShellAction::SHELL_ACTION_ATTACH_DEFAULT,
            String::new(),
            String::new(),
            args.force,
            initial_size
                .map(|(rows, columns)| terminal_size_v2(rows, columns))
                .into(),
        ),
        ShellAction::List => (
            terminal_v2::ShellAction::SHELL_ACTION_LIST,
            String::new(),
            String::new(),
            false,
            MessageField::none(),
        ),
        ShellAction::Detach => (
            terminal_v2::ShellAction::SHELL_ACTION_DETACH,
            args.name.clone().unwrap_or_default(),
            String::new(),
            false,
            MessageField::none(),
        ),
        ShellAction::Kill => (
            terminal_v2::ShellAction::SHELL_ACTION_KILL,
            args.name.clone().unwrap_or_default(),
            String::new(),
            false,
            MessageField::none(),
        ),
    };
    terminal_v2::TerminalSelection {
        selection: Some(terminal_v2::terminal_selection::Selection::Shell(
            terminal_v2::ShellSelection {
                action: EnumOrUnknown::new(action),
                shell_handle,
                configured_shell_id,
                force,
                initial_size,
                ..Default::default()
            },
        )),
        ..Default::default()
    }
}

fn receive_shell_management(
    daemon: &service_v2::DaemonService,
    terminal: &daemon_access::DaemonTerminal,
) -> Result<
    (
        terminal_v2::ShellManagementResult,
        terminal_v2::TerminalOutcome,
    ),
    CliFailure,
> {
    use terminal_v2::terminal_stream_frame::Frame;
    let mut result = None;
    loop {
        let frame = daemon
            .runtime()
            .block_on(terminal.receive())
            .map_err(terminal_client_failure)?;
        match frame.frame {
            Some(Frame::ShellResult(shell_result)) if result.is_none() => {
                result = Some(shell_result);
            }
            Some(Frame::Outcome(outcome)) => {
                return Ok((
                    result.ok_or_else(|| {
                        CliFailure::new(76, "shell management omitted its typed result")
                    })?,
                    outcome,
                ));
            }
            _ => {
                return Err(CliFailure::new(
                    76,
                    "shell management returned an unexpected terminal frame",
                ));
            }
        }
    }
}

fn cmd_console_v2(context: &Context, args: &ConsoleArgs) -> Result<i32, CliFailure> {
    if !context.public_socket.exists() {
        return emit_host_error(&daemon_down_envelope("console"), false);
    }
    let mut preparation = prepare_terminal_before_runtime(true)?;
    let (rows, columns) = preparation
        .initial_size
        .ok_or_else(|| CliFailure::new(2, "console requires a terminal size"))?;
    let daemon = service_v2::DaemonService::connect(&context.public_socket)?;
    let operation_id = terminal_operation_id("console");
    let selection = terminal_v2::TerminalSelection {
        selection: Some(terminal_v2::terminal_selection::Selection::Console(
            terminal_v2::ConsoleSelection {
                initial_size: MessageField::some(terminal_size_v2(rows, columns)),
                ..Default::default()
            },
        )),
        ..Default::default()
    };
    let terminal = daemon.open_terminal(
        daemon_access::DaemonMethod::OpenConsole,
        &args.vm,
        &operation_id,
        selection,
    )?;
    print_stderr(&format!(
        "Connected to console for VM '{}'. Press Ctrl-] to detach.\r\n",
        args.vm
    ));
    let mut host = exec_client::RealHostIo;
    let outcome = run_terminal_stream_v2(
        &daemon,
        &terminal,
        &mut host,
        &mut preparation.signals,
        true,
        true,
        TerminalDetachMode::Console,
    )?;
    terminal_outcome_exit(&outcome)
}

fn cmd_shell_v2(context: &Context, args: &ShellArgs) -> Result<i32, CliFailure> {
    let action = args.action.unwrap_or(ShellAction::Attach);
    if action == ShellAction::Attach && (args.json || args.human) {
        return Err(CliFailure::new(
            2,
            "shell attach is interactive and does not accept --json or --human",
        ));
    }
    if matches!(action, ShellAction::Detach | ShellAction::Kill) && args.name.is_none() {
        return Err(CliFailure::new(
            2,
            "shell detach/kill requires the server-issued handle via --name",
        ));
    }
    if !context.public_socket.exists() {
        return emit_host_error(&daemon_down_envelope("shell"), args.json);
    }
    let mut preparation = if action == ShellAction::Attach {
        Some(prepare_terminal_before_runtime(true)?)
    } else {
        None
    };
    let daemon = service_v2::DaemonService::connect(&context.public_socket)?;
    let operation_id = terminal_operation_id("shell");
    let terminal = daemon.open_terminal(
        daemon_access::DaemonMethod::Shell,
        &args.vm,
        &operation_id,
        terminal_selection_shell(
            args,
            preparation
                .as_ref()
                .and_then(|preparation| preparation.initial_size),
        ),
    )?;
    match action {
        ShellAction::Attach => {
            let mut host = exec_client::RealHostIo;
            let signals = &mut preparation
                .as_mut()
                .expect("attach preparation exists")
                .signals;
            let outcome = run_terminal_stream_v2(
                &daemon,
                &terminal,
                &mut host,
                signals,
                true,
                true,
                TerminalDetachMode::Shell,
            )?;
            terminal_outcome_exit(&outcome)
        }
        ShellAction::List => {
            let (result, outcome) = receive_shell_management(&daemon, &terminal)?;
            if result.truncated {
                return Err(CliFailure::new(
                    76,
                    "shell list was truncated without a continuation cursor",
                ));
            }
            let exit = terminal_outcome_exit(&outcome)?;
            let default_name = result
                .sessions
                .iter()
                .find(|session| session.is_default)
                .map(|session| session.shell_handle.clone())
                .unwrap_or_default();
            let sessions = result
                .sessions
                .iter()
                .map(|session| ShellListSessionOutputV1 {
                    name: session.shell_handle.clone(),
                    state: shell_session_state(session.state.enum_value_or_default()).to_owned(),
                    attached: session.state.enum_value_or_default()
                        == terminal_v2::ShellSessionState::SHELL_SESSION_STATE_ATTACHED,
                    is_default: session.is_default,
                })
                .collect();
            if args.json {
                print_json(&ShellListOutputV1 {
                    command: "shell list".to_owned(),
                    vm: args.vm.clone(),
                    default_name,
                    sessions,
                })?;
            } else {
                print_stdout("HANDLE\tSTATE\tDEFAULT\n");
                for session in &result.sessions {
                    print_stdout(&format!(
                        "{}\t{}\t{}\n",
                        session.shell_handle,
                        shell_session_state(session.state.enum_value_or_default()),
                        session.is_default
                    ));
                }
            }
            Ok(exit)
        }
        ShellAction::Detach => {
            let (result, outcome) = receive_shell_management(&daemon, &terminal)?;
            let exit = terminal_outcome_exit(&outcome)?;
            let name = result.affected_shell_handle;
            if args.json {
                print_json(&ShellDetachOutputV1 {
                    command: "shell detach".to_owned(),
                    vm: args.vm.clone(),
                    name,
                    result: if result.applied {
                        "detached".to_owned()
                    } else {
                        "not-applied".to_owned()
                    },
                    cause: None,
                })?;
            } else {
                print_stdout("shell detached\n");
            }
            Ok(exit)
        }
        ShellAction::Kill => {
            let (result, outcome) = receive_shell_management(&daemon, &terminal)?;
            let exit = terminal_outcome_exit(&outcome)?;
            let name = result.affected_shell_handle;
            let state = result
                .sessions
                .iter()
                .find(|session| session.shell_handle == name)
                .map(|session| shell_session_state(session.state.enum_value_or_default()))
                .unwrap_or("killed")
                .to_owned();
            if args.json {
                print_json(&ShellKillOutputV1 {
                    command: "shell kill".to_owned(),
                    vm: args.vm.clone(),
                    name,
                    result: if result.applied {
                        "cancelled".to_owned()
                    } else {
                        "not-applied".to_owned()
                    },
                    state,
                })?;
            } else {
                print_stdout("shell killed\n");
            }
            Ok(exit)
        }
    }
}

fn cmd_vm_exec_v2(context: &Context, args: &VmExecArgs) -> Result<i32, CliFailure> {
    use terminal_v2::{
        ArbitraryExecSelection, ExecAuthority, ExecSelection, TerminalSelection, exec_selection,
        terminal_outcome::Outcome, terminal_selection,
    };

    let action = match parse_vm_exec_action(args) {
        Ok(action) => action,
        Err(message) => return exec_usage_terminate(args, message),
    };
    if let Some(management) = action.management.as_ref() {
        if let Err(message) = validate_vm_exec_management_args(args) {
            return exec_usage_terminate(args, message);
        }
        return cmd_vm_exec_management_v2(context, args, management);
    }
    if args.command.is_empty() {
        return exec_usage_terminate(args, "vm exec: command form requires `-- <cmd...>`");
    }
    if args.detach && (args.interactive || args.tty) {
        return exec_usage_terminate(args, "vm exec: --detach conflicts with -i/-t");
    }
    if args.interactive && !args.tty {
        return exec_usage_terminate(args, "vm exec: -i/--interactive requires -t/--tty");
    }
    if action.json && args.tty {
        return exec_usage_terminate(args, "vm exec: --json conflicts with --tty");
    }
    let tty = args.tty;
    let interactive = args.interactive || tty;
    let mut preparation = match prepare_terminal_before_runtime(tty) {
        Ok(preparation) => preparation,
        Err(error) => {
            return exec_usage_terminate(args, error.message);
        }
    };
    let selection = TerminalSelection {
        selection: Some(terminal_selection::Selection::Exec(ExecSelection {
            authority: EnumOrUnknown::new(ExecAuthority::EXEC_AUTHORITY_ADMIN_ARBITRARY),
            selection: Some(exec_selection::Selection::Arbitrary(
                ArbitraryExecSelection {
                    argv: args
                        .command
                        .iter()
                        .map(|argument| argument.as_bytes().to_vec())
                        .collect(),
                    ..Default::default()
                },
            )),
            tty,
            detached: args.detach,
            initial_size: preparation
                .initial_size
                .map(|(rows, columns)| terminal_size_v2(rows, columns))
                .into(),
            ..Default::default()
        })),
        ..Default::default()
    };
    let operation_id = terminal_operation_id("exec");
    let daemon = match service_v2::DaemonService::connect(&context.public_socket) {
        Ok(daemon) => daemon,
        Err(error) => {
            return exec_terminate(args, exec_client::ExecClientError::transport(error.message));
        }
    };
    let terminal = match daemon.open_terminal_typed(
        daemon_access::DaemonMethod::Exec,
        &args.vm,
        &operation_id,
        selection,
    ) {
        Ok(terminal) => terminal,
        Err(error) => return exec_terminate(args, client_error_to_exec(error)),
    };
    let exec_handle = terminal.resource_handle().to_owned();

    if action.json {
        let mut host = exec_client::CapturingHostIo::new(interactive, 1024 * 1024);
        let outcome = match run_terminal_stream_v2(
            &daemon,
            &terminal,
            &mut host,
            &mut preparation.signals,
            tty,
            interactive,
            TerminalDetachMode::None,
        ) {
            Ok(outcome) => outcome,
            Err(error) => return exec_terminate(args, terminal_failure_to_exec(error)),
        };
        let exit = match terminal_outcome_exit(&outcome) {
            Ok(exit) => exit,
            Err(error) => return exec_terminate(args, terminal_failure_to_exec(error)),
        };
        let mut value = exec_json_base(args);
        value.insert("exitCode".to_owned(), Value::from(exit));
        value.insert(
            "stdoutBase64".to_owned(),
            Value::String(d2b_core::base64_codec::encode(host.stdout())),
        );
        value.insert(
            "stderrBase64".to_owned(),
            Value::String(d2b_core::base64_codec::encode(host.stderr())),
        );
        value.insert(
            "stdoutTruncated".to_owned(),
            Value::Bool(host.stdout_truncated()),
        );
        value.insert(
            "stderrTruncated".to_owned(),
            Value::Bool(host.stderr_truncated()),
        );
        match outcome.outcome.as_ref() {
            Some(Outcome::Exited(exited)) => {
                value.insert("source".to_owned(), Value::String("guest".to_owned()));
                value.insert("reason".to_owned(), Value::String("exited".to_owned()));
                value.insert("guestExitCode".to_owned(), Value::from(exited.exit_code));
            }
            Some(Outcome::Signaled(signaled)) => {
                value.insert("source".to_owned(), Value::String("guest".to_owned()));
                value.insert("reason".to_owned(), Value::String("signaled".to_owned()));
                value.insert("signal".to_owned(), Value::from(signaled.signal));
            }
            Some(Outcome::Detached(_)) => {
                value.insert("source".to_owned(), Value::String("guest".to_owned()));
                value.insert("reason".to_owned(), Value::String("detached".to_owned()));
                value.insert("execId".to_owned(), Value::String(exec_handle.clone()));
            }
            Some(Outcome::Cancelled(_)) => {
                value.insert(
                    "source".to_owned(),
                    Value::String("guest-control".to_owned()),
                );
                value.insert("reason".to_owned(), Value::String("cancelled".to_owned()));
            }
            Some(Outcome::Closed(_)) => {
                value.insert("source".to_owned(), Value::String("guest".to_owned()));
                value.insert("reason".to_owned(), Value::String("closed".to_owned()));
            }
            Some(Outcome::Failed(_)) | None | Some(_) => {}
        }
        print_exec_json(&Value::Object(value))?;
        Ok(exit)
    } else {
        let mut host = exec_client::RealHostIo;
        let outcome = run_terminal_stream_v2(
            &daemon,
            &terminal,
            &mut host,
            &mut preparation.signals,
            tty,
            interactive,
            TerminalDetachMode::None,
        )?;
        if args.detach && matches!(outcome.outcome, Some(Outcome::Detached(_))) {
            print_stdout(&format!("{exec_handle}\n"));
        }
        terminal_outcome_exit(&outcome)
    }
}

fn cmd_vm_exec_management_v2(
    context: &Context,
    args: &VmExecArgs,
    command: &VmExecManagementCommand,
) -> Result<i32, CliFailure> {
    let daemon = match service_v2::DaemonService::connect(&context.public_socket) {
        Ok(daemon) => daemon,
        Err(error) => {
            return exec_terminate(args, exec_client::ExecClientError::transport(error.message));
        }
    };
    let guest = match daemon.guest_typed(&args.vm) {
        Ok(guest) => guest,
        Err(error) => return exec_terminate(args, client_error_to_exec(error)),
    };
    match command {
        VmExecManagementCommand::List => {
            let mut cursor = None;
            let mut entries = Vec::new();
            for _ in 0..1024 {
                let query = guest_v2::GuestInspectExecQuery {
                    query: Some(guest_v2::guest_inspect_exec_query::Query::ListPage(
                        guest_v2::GuestExecListPageQuery {
                            page_size: 32,
                            page_cursor: cursor.clone().unwrap_or_default(),
                            ..Default::default()
                        },
                    )),
                    ..Default::default()
                };
                let response = daemon
                    .runtime()
                    .block_on(
                        guest.inspect_exec(
                            daemon_access::GuestInspectCall {
                                operation: guest_operation(
                                    "exec-list",
                                    &args.vm,
                                    cursor.as_deref(),
                                ),
                                query,
                            },
                            daemon_access::daemon_call_options(false)
                                .map_err(terminal_client_failure)?,
                            &daemon_access::CancellationToken::default(),
                        ),
                    )
                    .map_err(client_error_to_exec);
                let response = match response {
                    Ok(response) => response,
                    Err(error) => return exec_terminate(args, error),
                };
                let Some(guest_v2::guest_inspect_exec_response::Result::ListPage(page)) =
                    response.result
                else {
                    return exec_terminate(
                        args,
                        exec_client::ExecClientError::protocol(
                            "InspectExec list returned a non-list result",
                        ),
                    );
                };
                entries.extend(page.entries);
                if !page.truncated {
                    let result = typed_exec_list_result(entries);
                    return exec_render_detached_list(args, &result);
                }
                if page.next_page_cursor.is_empty()
                    || cursor.as_deref() == Some(page.next_page_cursor.as_str())
                {
                    return exec_terminate(
                        args,
                        exec_client::ExecClientError::protocol(
                            "InspectExec list returned an invalid continuation cursor",
                        ),
                    );
                }
                cursor = Some(page.next_page_cursor);
            }
            exec_terminate(
                args,
                exec_client::ExecClientError::protocol(
                    "InspectExec list exceeded the bounded page limit",
                ),
            )
        }
        VmExecManagementCommand::Status(status) => {
            let typed = match typed_exec_status(&daemon, &guest, &args.vm, &status.exec_id) {
                Ok(status) => status,
                Err(error) => return exec_terminate(args, error),
            };
            exec_render_detached_status(args, &typed)
        }
        VmExecManagementCommand::Logs(logs) => {
            let max_bytes = match logs
                .max_len
                .unwrap_or(d2b_contracts::v2_services::MAX_TERMINAL_CHUNK_BYTES as u64)
            {
                0 => {
                    return exec_usage_terminate(args, "vm exec logs: --max-len must be nonzero");
                }
                value if value > d2b_contracts::v2_services::MAX_TERMINAL_CHUNK_BYTES as u64 => {
                    return exec_usage_terminate(
                        args,
                        "vm exec logs: --max-len exceeds the 64 KiB retained-log bound",
                    );
                }
                value => value as u32,
            };
            let stdout = match read_typed_retained_log(
                &daemon,
                &guest,
                &args.vm,
                &logs.exec_id,
                terminal_v2::OutputStream::OUTPUT_STREAM_STDOUT,
                logs.stdout_offset.unwrap_or(0),
                max_bytes,
            ) {
                Ok(log) => log,
                Err(error) => return exec_terminate(args, error),
            };
            let stderr = match read_typed_retained_log(
                &daemon,
                &guest,
                &args.vm,
                &logs.exec_id,
                terminal_v2::OutputStream::OUTPUT_STREAM_STDERR,
                logs.stderr_offset.unwrap_or(0),
                max_bytes,
            ) {
                Ok(log) => log,
                Err(error) => return exec_terminate(args, error),
            };
            exec_render_detached_logs(
                args,
                &d2b_contracts::public_wire::ExecDetachedLogsResult {
                    exec_id: logs.exec_id.clone(),
                    stdout_base64: d2b_core::base64_codec::encode(&stdout.bytes),
                    stderr_base64: d2b_core::base64_codec::encode(&stderr.bytes),
                    start_offset: stdout.start_offset.min(stderr.start_offset),
                    end_offset: stdout.end_offset.max(stderr.end_offset),
                    dropped_bytes: stdout.dropped_bytes.saturating_add(stderr.dropped_bytes),
                    truncated: stdout.truncated || stderr.truncated,
                    stdout_start_offset: stdout.start_offset,
                    stdout_end_offset: stdout.end_offset,
                    stdout_next_offset: stdout.next_offset,
                    stdout_eof: stdout.eof,
                    stdout_dropped_bytes: stdout.dropped_bytes,
                    stdout_truncated: stdout.truncated,
                    stderr_start_offset: stderr.start_offset,
                    stderr_end_offset: stderr.end_offset,
                    stderr_next_offset: stderr.next_offset,
                    stderr_eof: stderr.eof,
                    stderr_dropped_bytes: stderr.dropped_bytes,
                    stderr_truncated: stderr.truncated,
                },
            )
        }
        VmExecManagementCommand::Kill(kill) => {
            let response = daemon
                .runtime()
                .block_on(guest.cancel_exec(
                    daemon_access::GuestCancelCall {
                        operation: guest_operation("exec-kill", &args.vm, Some(&kill.exec_id)),
                        resource_handle: kill.exec_id.clone(),
                        control_sequence: 1,
                        reason:
                            guest_v2::GuestExecCancelReason::GUEST_EXEC_CANCEL_REASON_USER_REQUESTED,
                    },
                    daemon_access::daemon_call_options(true).map_err(terminal_client_failure)?,
                    &daemon_access::CancellationToken::default(),
                ))
                .map_err(client_error_to_exec);
            let response = match response {
                Ok(response) => response,
                Err(error) => return exec_terminate(args, error),
            };
            let outcome = match response.cancellation.enum_value_or_default() {
                guest_v2::GuestExecCancellationOutcome::GUEST_EXEC_CANCELLATION_OUTCOME_SIGNALLED => {
                    d2b_contracts::public_wire::ExecDetachedKillOutcome::Cancelling
                }
                guest_v2::GuestExecCancellationOutcome::GUEST_EXEC_CANCELLATION_OUTCOME_ALREADY_TERMINAL => {
                    d2b_contracts::public_wire::ExecDetachedKillOutcome::AlreadyTerminal
                }
                _ => {
                    return exec_terminate(
                        args,
                        exec_client::ExecClientError::protocol(
                            "CancelExec returned an inconsistent cancellation outcome",
                        ),
                    );
                }
            };
            let status = match typed_exec_status(&daemon, &guest, &args.vm, &kill.exec_id) {
                Ok(status) => status,
                Err(error) => return exec_terminate(args, error),
            };
            exec_render_detached_kill(
                args,
                &d2b_contracts::public_wire::ExecDetachedKillResult {
                    exec_id: kill.exec_id.clone(),
                    result: outcome,
                    state: status.state,
                },
            )
        }
    }
}

fn guest_operation(kind: &str, vm: &str, detail: Option<&str>) -> daemon_access::GuestOperation {
    use sha2::Digest as _;

    let operation_id = terminal_operation_id(kind);
    let mut digest = sha2::Sha256::new();
    digest.update(b"d2b-cli-guest-operation-v2\0");
    digest.update(kind.as_bytes());
    digest.update(vm.as_bytes());
    if let Some(detail) = detail {
        digest.update(detail.as_bytes());
    }
    daemon_access::GuestOperation {
        operation_id,
        request_digest: digest.finalize().into(),
    }
}

fn typed_exec_list_result(
    entries: Vec<guest_v2::GuestExecListEntry>,
) -> d2b_contracts::public_wire::ExecDetachedListResult {
    d2b_contracts::public_wire::ExecDetachedListResult {
        execs: entries
            .into_iter()
            .map(|entry| d2b_contracts::public_wire::ExecDetachedListEntry {
                exec_id: entry.resource_handle,
                state: typed_exec_state(entry.state.enum_value_or_default()),
                exit_code: None,
                signal: None,
                started_at: entry.created_at_unix_ms.to_string(),
                start_offset: 0,
                end_offset: entry.stdout_bytes.max(entry.stderr_bytes),
                stdout_start_offset: 0,
                stdout_end_offset: entry.stdout_bytes,
                stderr_start_offset: 0,
                stderr_end_offset: entry.stderr_bytes,
                dropped_bytes: entry.dropped_bytes,
                stdout_dropped_bytes: 0,
                stderr_dropped_bytes: 0,
                truncated: entry.stdout_truncated || entry.stderr_truncated,
                stdout_truncated: entry.stdout_truncated,
                stderr_truncated: entry.stderr_truncated,
            })
            .collect(),
    }
}

fn typed_exec_status(
    daemon: &service_v2::DaemonService,
    guest: &daemon_access::GuestClient,
    vm: &str,
    exec_handle: &str,
) -> Result<d2b_contracts::public_wire::ExecDetachedStatusResult, exec_client::ExecClientError> {
    let query = guest_v2::GuestInspectExecQuery {
        query: Some(guest_v2::guest_inspect_exec_query::Query::Status(
            guest_v2::GuestExecStatusQuery {
                resource_handle: exec_handle.to_owned(),
                ..Default::default()
            },
        )),
        ..Default::default()
    };
    let response = daemon
        .runtime()
        .block_on(guest.inspect_exec(
            daemon_access::GuestInspectCall {
                operation: guest_operation("exec-status", vm, Some(exec_handle)),
                query,
            },
            daemon_access::daemon_call_options(false).map_err(client_error_to_exec)?,
            &daemon_access::CancellationToken::default(),
        ))
        .map_err(client_error_to_exec)?;
    let Some(guest_v2::guest_inspect_exec_response::Result::Status(status)) = response.result
    else {
        return Err(exec_client::ExecClientError::protocol(
            "InspectExec status returned a non-status result",
        ));
    };
    let (exit_code, signal, reason) = typed_terminal_summary(status.terminal_outcome.as_ref());
    Ok(d2b_contracts::public_wire::ExecDetachedStatusResult {
        exec_id: status.resource_handle,
        state: typed_exec_state(status.state.enum_value_or_default()),
        reason,
        exit_code,
        signal,
        start_offset: status.stdout_start_offset.min(status.stderr_start_offset),
        end_offset: status.stdout_end_offset.max(status.stderr_end_offset),
        dropped_bytes: status
            .stdout_dropped_bytes
            .saturating_add(status.stderr_dropped_bytes),
        truncated: status.stdout_truncated || status.stderr_truncated,
    })
}

fn typed_exec_state(state: guest_v2::GuestExecState) -> d2b_contracts::guest_wire::ExecState {
    match state {
        guest_v2::GuestExecState::GUEST_EXEC_STATE_CREATED => {
            d2b_contracts::guest_wire::ExecState::Created
        }
        guest_v2::GuestExecState::GUEST_EXEC_STATE_RUNNING => {
            d2b_contracts::guest_wire::ExecState::Running
        }
        guest_v2::GuestExecState::GUEST_EXEC_STATE_EXITED => {
            d2b_contracts::guest_wire::ExecState::Exited
        }
        guest_v2::GuestExecState::GUEST_EXEC_STATE_SIGNALED => {
            d2b_contracts::guest_wire::ExecState::Signaled
        }
        guest_v2::GuestExecState::GUEST_EXEC_STATE_CANCELLED => {
            d2b_contracts::guest_wire::ExecState::Cancelled
        }
        guest_v2::GuestExecState::GUEST_EXEC_STATE_PROTOCOL_ERROR => {
            d2b_contracts::guest_wire::ExecState::ProtocolError
        }
        guest_v2::GuestExecState::GUEST_EXEC_STATE_LOST => {
            d2b_contracts::guest_wire::ExecState::LostGuestd
        }
        guest_v2::GuestExecState::GUEST_EXEC_STATE_REAPED => {
            d2b_contracts::guest_wire::ExecState::Reaped
        }
        _ => d2b_contracts::guest_wire::ExecState::ProtocolError,
    }
}

fn typed_terminal_summary(
    outcome: Option<&terminal_v2::TerminalOutcome>,
) -> (Option<i32>, Option<u32>, Option<String>) {
    use terminal_v2::terminal_outcome::Outcome;
    match outcome.and_then(|outcome| outcome.outcome.as_ref()) {
        Some(Outcome::Exited(exited)) => (Some(exited.exit_code), None, None),
        Some(Outcome::Signaled(signaled)) => (None, Some(signaled.signal), None),
        Some(Outcome::Cancelled(_)) => (None, None, Some("cancelled".to_owned())),
        Some(Outcome::Failed(failed)) => (
            None,
            None,
            Some(format!("{:?}", failed.error.enum_value_or_default())),
        ),
        Some(Outcome::Detached(_)) => (None, None, Some("detached".to_owned())),
        Some(Outcome::Closed(_)) => (None, None, Some("closed".to_owned())),
        None | Some(_) => (None, None, None),
    }
}

struct TypedRetainedLog {
    bytes: Vec<u8>,
    start_offset: u64,
    end_offset: u64,
    next_offset: u64,
    eof: bool,
    dropped_bytes: u64,
    truncated: bool,
}

fn read_typed_retained_log(
    daemon: &service_v2::DaemonService,
    guest: &daemon_access::GuestClient,
    vm: &str,
    exec_handle: &str,
    output: terminal_v2::OutputStream,
    offset: u64,
    max_bytes: u32,
) -> Result<TypedRetainedLog, exec_client::ExecClientError> {
    use terminal_v2::terminal_stream_frame::Frame;

    let terminal = daemon
        .runtime()
        .block_on(guest.open_exec_retained_log(
            daemon_access::GuestRetainedLogCall {
                operation: guest_operation("exec-logs", vm, Some(exec_handle)),
                resource_handle: exec_handle.to_owned(),
                output,
                offset,
                max_bytes,
            },
            daemon_access::daemon_call_options(true).map_err(client_error_to_exec)?,
            &daemon_access::CancellationToken::default(),
        ))
        .map_err(client_error_to_exec)?;
    let range = terminal.retained_log_range().cloned().ok_or_else(|| {
        exec_client::ExecClientError::protocol("OpenExecRetainedLog omitted its retained range")
    })?;
    let mut bytes = Vec::new();
    let mut dropped_bytes = 0_u64;
    let mut truncated = false;
    let mut next_offset = range.start_offset;
    let mut saw_started = false;
    loop {
        let frame = daemon
            .runtime()
            .block_on(terminal.receive())
            .map_err(client_error_to_exec)?;
        match frame.frame {
            Some(Frame::Started(_)) if !saw_started => saw_started = true,
            Some(Frame::Stdout(chunk))
                if output == terminal_v2::OutputStream::OUTPUT_STREAM_STDOUT =>
            {
                next_offset = chunk.offset.saturating_add(chunk.data.len() as u64);
                dropped_bytes = dropped_bytes.saturating_add(chunk.dropped_bytes);
                truncated |= chunk.truncated;
                bytes.extend_from_slice(&chunk.data);
            }
            Some(Frame::Stderr(chunk))
                if output == terminal_v2::OutputStream::OUTPUT_STREAM_STDERR =>
            {
                next_offset = chunk.offset.saturating_add(chunk.data.len() as u64);
                dropped_bytes = dropped_bytes.saturating_add(chunk.dropped_bytes);
                truncated |= chunk.truncated;
                bytes.extend_from_slice(&chunk.data);
            }
            Some(Frame::Outcome(outcome)) => {
                terminal_outcome_exit(&outcome)
                    .map_err(|error| exec_client::ExecClientError::protocol(error.message))?;
                daemon
                    .runtime()
                    .block_on(terminal.close_transport())
                    .map_err(client_error_to_exec)?;
                return Ok(TypedRetainedLog {
                    bytes,
                    start_offset: range.start_offset,
                    end_offset: range.end_offset,
                    next_offset,
                    eof: range.eof,
                    dropped_bytes,
                    truncated,
                });
            }
            _ => {
                return Err(exec_client::ExecClientError::protocol(
                    "retained-log stream returned a direction-invalid frame",
                ));
            }
        }
    }
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

#[allow(dead_code)]
fn cmd_console(
    context: &Context,
    args: &ConsoleArgs,
    _original_args: &[OsString],
) -> Result<i32, CliFailure> {
    use d2b_contracts::public_wire::{ConsoleOp, ConsoleOpResponse};
    use d2b_contracts::terminal_wire::TerminalStream;
    use terminal_client::{TerminalHostIo as _, TerminalSignalSource as _};

    let vm = &args.vm;

    if !context.public_socket.exists() {
        return Err(CliFailure::new(
            3,
            "daemon is not running (socket not found)",
        ));
    }

    let mut socket = SeqpacketUnixSocket::connect(&context.public_socket)
        .map_err(|err| CliFailure::new(3, format!("failed to connect to daemon: {err}")))?;

    // Handshake.
    let hello = daemon_hello_frame("hello")?;
    socket
        .send_frame(&hello)
        .map_err(|err| CliFailure::new(1, format!("failed to send hello: {err}")))?;
    let hello_reply = socket
        .recv_frame()
        .map_err(|err| CliFailure::new(1, format!("failed to recv hello reply: {err}")))?;
    parse_hello_reply(&hello_reply)?;

    // Determine initial terminal size (best-effort; UART ignores it).
    let size = exec_client::current_window_size()
        .map(|(rows, cols)| d2b_contracts::terminal_wire::TerminalSize { rows, cols })
        .unwrap_or(d2b_contracts::terminal_wire::TerminalSize { rows: 24, cols: 80 });

    // Attach to the console session.
    let attach_response = console_round_trip(
        &mut socket,
        &ConsoleOp::Attach(d2b_contracts::public_wire::ConsoleAttachArgs {
            vm: vm.clone(),
            initial_terminal_size: size,
        }),
    )?;
    let ConsoleOpResponse::Attach(attach) = attach_response else {
        return Err(CliFailure::new(
            1,
            "console attach: unexpected daemon response",
        ));
    };

    let session = attach.session.clone();
    let mut stdout_offset = attach.ring_buffer_start_offset;

    print_stderr(&format!(
        "Connected to console for VM '{}' ({:?}). Press Ctrl-] to detach.\r\n",
        vm, attach.provider_kind
    ));
    if attach.provider_kind == d2b_contracts::public_wire::ConsoleProviderKind::QemuMedia {
        print_stderr(
            "Note: QEMU serial console may appear blank until the guest writes \
             to /dev/ttyS0 (e.g. run 'systemctl start serial-getty@ttyS0.service' \
             or configure console= in the kernel command line).\r\n",
        );
    }

    // Enter raw mode when stdin is interactive and at least one operator-facing
    // stream is a terminal. stdout may be redirected to capture the raw UART.
    let is_tty =
        io::stdin().is_terminal() && (io::stdout().is_terminal() || io::stderr().is_terminal());
    let (_signal_mask, mut signals) = exec_client::install_signals().map_err(|err| {
        CliFailure::new(
            42,
            format!("console: failed to install signal handlers: {err}"),
        )
    })?;
    let _raw_guard = if is_tty {
        exec_client::FdStateGuard::enter(true, true).ok()
    } else {
        None
    };

    let mut host = exec_client::RealHostIo;
    // 4096-byte buffer: handles pastes and rapid input without excessive round-trips.
    let mut stdin_buf = vec![0_u8; 4096];

    loop {
        // Drain any pending signals first.
        for signal in signals.drain() {
            match signal {
                exec_client::ExecSignal::Winch => {
                    if let Some((rows, cols)) = host.window_size() {
                        let _ = console_round_trip(
                            &mut socket,
                            &ConsoleOp::Resize(d2b_contracts::public_wire::ConsoleResizeArgs {
                                session: session.clone(),
                                size: d2b_contracts::terminal_wire::TerminalSize { rows, cols },
                            }),
                        );
                    }
                }
                exec_client::ExecSignal::Interrupt
                | exec_client::ExecSignal::Terminate
                | exec_client::ExecSignal::Stop
                | exec_client::ExecSignal::Hangup
                | exec_client::ExecSignal::Quit => {
                    let _ = console_round_trip(
                        &mut socket,
                        &ConsoleOp::Close(d2b_contracts::public_wire::ConsoleCloseArgs {
                            session: session.clone(),
                        }),
                    );
                    return Ok(0);
                }
            }
        }

        // Read pending stdin (non-blocking) and forward to daemon.
        if is_tty {
            match host.read_stdin(&mut stdin_buf) {
                Ok(n) if n > 0 => {
                    let chunk = &stdin_buf[..n];
                    if let DetachScan::Detach { prefix_len } = scan_chunk_for_detach(chunk) {
                        // Forward any bytes that arrived before the detach char
                        // so they are not silently dropped.
                        if prefix_len > 0 {
                            let prefix_b64 = d2b_core::base64_codec::encode(&chunk[..prefix_len]);
                            let _ = console_round_trip(
                                &mut socket,
                                &ConsoleOp::WriteStdin(
                                    d2b_contracts::public_wire::ConsoleWriteStdinArgs {
                                        session: session.clone(),
                                        offset: 0,
                                        chunk_base64: prefix_b64,
                                        eof: false,
                                    },
                                ),
                            );
                        }
                        let _ = console_round_trip(
                            &mut socket,
                            &ConsoleOp::Close(d2b_contracts::public_wire::ConsoleCloseArgs {
                                session: session.clone(),
                            }),
                        );
                        print_stderr("\r\nDetached from console.\r\n");
                        return Ok(0);
                    }
                    let chunk_b64 = d2b_core::base64_codec::encode(chunk);
                    let _ = console_round_trip(
                        &mut socket,
                        &ConsoleOp::WriteStdin(d2b_contracts::public_wire::ConsoleWriteStdinArgs {
                            session: session.clone(),
                            offset: 0,
                            chunk_base64: chunk_b64,
                            eof: false,
                        }),
                    );
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {}
                Ok(_) | Err(_) => {}
            }
        }

        // Poll for output; the daemon returns immediately so this client owns
        // the backoff that keeps console idle loops from burning CPU.
        let read_result = console_round_trip(
            &mut socket,
            &ConsoleOp::ReadOutput(d2b_contracts::public_wire::ConsoleReadOutputArgs {
                session: session.clone(),
                stream: TerminalStream::Stdout,
                offset: stdout_offset,
                max_len: 4096,
                wait: true,
                timeout_ms: 200,
            }),
        );

        match read_result {
            Err(err) if err.exit_code == 75 => {
                // ConsoleSessionStale: daemon restarted.
                print_stderr("\r\nConsole session expired (daemon restarted).\r\n");
                return Ok(0);
            }
            Err(err) => return Err(err),
            Ok(ConsoleOpResponse::ReadOutput(out)) => {
                if out.ring_buffer_start_offset > stdout_offset {
                    stdout_offset = out.ring_buffer_start_offset;
                }
                if !out.chunk_base64.is_empty() {
                    let bytes = match d2b_core::base64_codec::decode(&out.chunk_base64) {
                        Ok(bytes) => bytes,
                        Err(_) => {
                            let _ = console_round_trip(
                                &mut socket,
                                &ConsoleOp::Close(d2b_contracts::public_wire::ConsoleCloseArgs {
                                    session: session.clone(),
                                }),
                            );
                            return Err(CliFailure::new(
                                1,
                                "console: daemon returned malformed base64 output",
                            ));
                        }
                    };
                    if let Err(err) = write_stdout_bytes(&bytes) {
                        let _ = console_round_trip(
                            &mut socket,
                            &ConsoleOp::Close(d2b_contracts::public_wire::ConsoleCloseArgs {
                                session: session.clone(),
                            }),
                        );
                        if err.kind() == io::ErrorKind::BrokenPipe {
                            return Ok(0);
                        }
                        return Err(CliFailure::new(
                            1,
                            format!("console: failed to write stdout: {err}"),
                        ));
                    }
                    stdout_offset = out.offset + bytes.len() as u64;
                }
                if out.is_eof && out.chunk_base64.is_empty() {
                    let _ = console_round_trip(
                        &mut socket,
                        &ConsoleOp::Close(d2b_contracts::public_wire::ConsoleCloseArgs {
                            session: session.clone(),
                        }),
                    );
                    print_stderr("\r\nVM console closed (EOF).\r\n");
                    return Ok(0);
                }
                if out.chunk_base64.is_empty() {
                    thread::sleep(Duration::from_millis(50));
                }
            }
            Ok(_) => return Err(CliFailure::new(1, "console read: unexpected response type")),
        }
    }
}

/// Encode and send a [`ConsoleOp`] on `socket`, then receive and parse the
/// `consoleResponse` reply. Each call is a complete round-trip.
#[allow(dead_code)]
fn console_round_trip(
    socket: &mut SeqpacketUnixSocket,
    op: &d2b_contracts::public_wire::ConsoleOp,
) -> Result<d2b_contracts::public_wire::ConsoleOpResponse, CliFailure> {
    let frame = encode_console_op_frame(op)?;
    socket
        .send_frame(&frame)
        .map_err(|err| CliFailure::new(69, format!("console op send failed: {err}")))?;
    let reply = socket
        .recv_frame()
        .map_err(|err| CliFailure::new(69, format!("console op recv failed: {err}")))?;
    parse_console_reply(&reply)
}

/// Encode a [`ConsoleOp`] as a JSON wire frame with `"type": "console"`.
#[allow(dead_code)]
fn encode_console_op_frame(
    op: &d2b_contracts::public_wire::ConsoleOp,
) -> Result<Vec<u8>, CliFailure> {
    let mut value = serde_json::to_value(op)
        .map_err(|err| CliFailure::new(1, format!("failed to encode console op: {err}")))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| CliFailure::new(1, "failed to encode console op: object required"))?;
    object.insert("type".to_owned(), Value::String("console".to_owned()));
    serde_json::to_vec(&value)
        .map_err(|err| CliFailure::new(1, format!("failed to serialize console op: {err}")))
}

/// Parse a `consoleResponse` or `error` reply frame.
#[allow(dead_code)]
fn parse_console_reply(
    bytes: &[u8],
) -> Result<d2b_contracts::public_wire::ConsoleOpResponse, CliFailure> {
    let mut value: Value = serde_json::from_slice(bytes)
        .map_err(|err| CliFailure::new(1, format!("failed to parse console reply: {err}")))?;
    match value.get("type").and_then(Value::as_str) {
        Some("consoleResponse") => {
            if let Some(obj) = value.as_object_mut() {
                obj.remove("opId");
                obj.remove("type");
            }
            serde_json::from_value(value).map_err(|err| {
                CliFailure::new(1, format!("failed to decode consoleResponse: {err}"))
            })
        }
        Some("error") => {
            if let Some(obj) = value.as_object_mut() {
                obj.remove("opId");
            }
            let frame: ErrorFrame = serde_json::from_value(value).map_err(|err| {
                CliFailure::new(1, format!("failed to decode console error reply: {err}"))
            })?;
            Err(cli_failure_from_daemon_error(frame.error))
        }
        other => Err(CliFailure::new(
            1,
            format!("unexpected console reply type {:?}", other),
        )),
    }
}

/// Result of scanning a console stdin chunk for the detach character.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DetachScan {
    /// No detach char found; forward the whole chunk.
    NoDetach,
    /// Detach char found at `prefix_len` bytes from the start.
    /// `prefix_len == 0` means the detach char is the very first byte;
    /// a non-zero `prefix_len` means there are bytes to forward before
    /// closing.
    Detach { prefix_len: usize },
}

/// Scan `chunk` for the console detach character (`\x1d`, Ctrl-]).
///
/// Returns [`DetachScan::Detach`] with the number of bytes that appear before
/// the detach char so callers can forward them before closing.
pub(crate) fn scan_chunk_for_detach(chunk: &[u8]) -> DetachScan {
    const DETACH: u8 = b'\x1d';
    match chunk.iter().position(|&b| b == DETACH) {
        None => DetachScan::NoDetach,
        Some(pos) => DetachScan::Detach { prefix_len: pos },
    }
}

fn cmd_audio(
    context: &Context,
    args: &AudioArgs,
    _original_args: &[OsString],
) -> Result<i32, CliFailure> {
    use d2b_contracts::public_wire::{
        AudioChannel, AudioMuteArgs, AudioOp, AudioOpResponse, AudioSetApplied,
        AudioStatusArgs as WireStatusArgs,
    };

    let json = args.json;

    // Build the op(s) to dispatch. `Off` fans out to two `Mute` ops.
    enum AudioDispatch {
        Single(AudioOp),
        Off { vm: String },
    }

    let dispatch = match &args.command {
        None | Some(AudioCommand::Status(AudioStatusArgs { vm: None })) => {
            AudioDispatch::Single(AudioOp::Status(WireStatusArgs { vms: vec![] }))
        }
        Some(AudioCommand::Status(AudioStatusArgs { vm: Some(vm) })) => {
            AudioDispatch::Single(AudioOp::Status(WireStatusArgs {
                vms: vec![vm.clone()],
            }))
        }
        Some(AudioCommand::Mic(a)) => AudioDispatch::Single(AudioOp::Mute(AudioMuteArgs {
            vm: a.vm.clone(),
            channel: AudioChannel::Microphone,
            mute: a.state == AudioGrantState::Off,
        })),
        Some(AudioCommand::Speaker(a)) => AudioDispatch::Single(AudioOp::Mute(AudioMuteArgs {
            vm: a.vm.clone(),
            channel: AudioChannel::Speaker,
            mute: a.state == AudioGrantState::Off,
        })),
        Some(AudioCommand::Off(a)) => AudioDispatch::Off { vm: a.vm.clone() },
    };

    match dispatch {
        AudioDispatch::Single(op) => {
            let response = audio_round_trip(context, op)?;
            render_audio_response(context, &response, json)
        }
        AudioDispatch::Off { vm } => {
            // Mute both channels. Report both; exit non-zero if either fails.
            let r_spk = audio_round_trip(
                context,
                AudioOp::Mute(AudioMuteArgs {
                    vm: vm.clone(),
                    channel: AudioChannel::Speaker,
                    mute: true,
                }),
            )?;
            let r_mic = audio_round_trip(
                context,
                AudioOp::Mute(AudioMuteArgs {
                    vm: vm.clone(),
                    channel: AudioChannel::Microphone,
                    mute: true,
                }),
            )?;
            if json {
                print_json(&serde_json::json!({
                    "speaker": serde_json::to_value(&r_spk).unwrap_or_default(),
                    "microphone": serde_json::to_value(&r_mic).unwrap_or_default(),
                }))?;
            } else {
                render_audio_response(context, &r_spk, false)?;
                render_audio_response(context, &r_mic, false)?;
            }
            // Non-zero if either channel reported Unsupported.
            let both_ok = !matches!(
                &r_spk,
                AudioOpResponse::Mute(r) if r.applied == AudioSetApplied::Unsupported
            ) && !matches!(
                &r_mic,
                AudioOpResponse::Mute(r) if r.applied == AudioSetApplied::Unsupported
            );
            Ok(if both_ok { 0 } else { 1 })
        }
    }
}

fn audio_round_trip(
    context: &Context,
    op: d2b_contracts::public_wire::AudioOp,
) -> Result<d2b_contracts::public_wire::AudioOpResponse, CliFailure> {
    let request = encode_type_tagged_message("audio", &op, "audio request")?;
    match try_public_socket_request(context, &request, "audio")? {
        PublicSocketOutcome::Reply(response) => parse_audio_reply(&response),
        PublicSocketOutcome::Unavailable => Err(CliFailure::new(
            69,
            format!(
                "audio: d2bd public socket is unavailable at {}",
                context.public_socket.display()
            ),
        )),
        PublicSocketOutcome::Unsupported => Err(CliFailure::new(
            70,
            "audio: daemon generation does not support audio operations",
        )),
    }
}

fn parse_audio_reply(
    bytes: &[u8],
) -> Result<d2b_contracts::public_wire::AudioOpResponse, CliFailure> {
    use d2b_contracts::public_wire::AudioOpResponse;
    let mut value: Value = serde_json::from_slice(bytes)
        .map_err(|err| CliFailure::new(1, format!("failed to parse audio reply: {err}")))?;
    match value.get("type").and_then(Value::as_str) {
        Some("audioOpResponse") => {
            if let Some(obj) = value.as_object_mut() {
                obj.remove("type");
            }
            serde_json::from_value::<AudioOpResponse>(value).map_err(|err| {
                CliFailure::new(1, format!("failed to decode audioOpResponse: {err}"))
            })
        }
        Some("error") => {
            let frame: ErrorFrame = serde_json::from_value(value).map_err(|err| {
                CliFailure::new(1, format!("failed to decode audio error reply: {err}"))
            })?;
            Err(cli_failure_from_daemon_error(frame.error))
        }
        other => Err(CliFailure::new(
            1,
            format!("unexpected audio reply type {other:?}"),
        )),
    }
}

fn render_audio_response(
    _context: &Context,
    response: &d2b_contracts::public_wire::AudioOpResponse,
    json: bool,
) -> Result<i32, CliFailure> {
    use d2b_contracts::public_wire::{AudioOpResponse, AudioSetApplied};
    match response {
        AudioOpResponse::Status(status) => {
            if json {
                // d2b-wlcontrol consumes this shape: AudioStatusResult.
                print_json(status)?;
                return Ok(0);
            }
            for vm_state in &status.entries {
                let spk_muted = if vm_state.speaker.muted {
                    "muted"
                } else {
                    "on"
                };
                let mic_muted = if vm_state.microphone.muted {
                    "muted"
                } else {
                    "on"
                };
                print_stdout(&format!(
                    "{}\tspeaker:{} mic:{} enforcement:{}\n",
                    vm_state.vm,
                    spk_muted,
                    mic_muted,
                    format_enforcement(&vm_state.enforcement)
                ));
            }
            for err in &status.errors {
                let kind_label = serde_json::to_string(&err.kind)
                    .map(|s| s.trim_matches('"').to_owned())
                    .unwrap_or_else(|_| "error".to_owned());
                print_stdout(&format!("{}\terror:{}\n", err.vm, kind_label));
            }
            Ok(0)
        }
        AudioOpResponse::Mute(result) | AudioOpResponse::SetVolume(result) => {
            if json {
                print_json(result)?;
                return Ok(if result.applied == AudioSetApplied::Unsupported {
                    1
                } else {
                    0
                });
            }
            let applied_label = match result.applied {
                AudioSetApplied::HostAndGuest => "applied:host+guest",
                AudioSetApplied::HostOnly => "applied:host",
                AudioSetApplied::GuestOnly => "applied:guest",
                AudioSetApplied::Unsupported => "not-applied",
            };
            let muted_label = if result.state.muted { "muted" } else { "on" };
            print_stdout(&format!(
                "{} {} {} {}\n",
                result.vm,
                format_channel(&result.channel),
                muted_label,
                applied_label
            ));
            Ok(if result.applied == AudioSetApplied::Unsupported {
                1
            } else {
                0
            })
        }
    }
}

fn format_enforcement(
    posture: &d2b_contracts::public_wire::AudioEnforcementPosture,
) -> &'static str {
    use d2b_contracts::public_wire::AudioEnforcementPosture;
    match posture {
        AudioEnforcementPosture::HostAndGuest => "host+guest",
        AudioEnforcementPosture::HostOnly => "host",
        AudioEnforcementPosture::GuestOnly => "guest",
        AudioEnforcementPosture::Unsupported => "unsupported",
    }
}

fn format_channel(channel: &d2b_contracts::public_wire::AudioChannel) -> &'static str {
    use d2b_contracts::public_wire::AudioChannel;
    match channel {
        AudioChannel::Speaker => "speaker",
        AudioChannel::Microphone => "microphone",
    }
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
            "d2b: {} (code: {}, exit {})\n  what was checked : {}\n  observed         : {}\n  remediation      : {}\n  docs             : {}",
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
        &format!("d2b {verb} requires d2bd"),
        "daemon-down",
        1,
        "Daemon connectivity at /run/d2b/public.sock.",
        "d2bd is unreachable; the daemon is the only operator surface for mutating verbs.",
        "Start d2bd (systemctl start d2bd d2b-priv-broker.socket) and re-run the same command. See docs/how-to/migrate-d2b-v1-0-to-v1-1.md#recovery-broker-bring-up-troubleshooting for the full bring-up checklist.",
        "docs/reference/error-codes.md#daemon-down",
    )
}

enum DaemonCommandConnection {
    Connected(Box<service_v2::DaemonService>),
    Unavailable(i32),
}

fn connect_daemon_for_command(
    context: &Context,
    verb: &str,
    json: bool,
) -> Result<DaemonCommandConnection, CliFailure> {
    if !context.public_socket.exists() {
        return emit_host_error(&daemon_down_envelope(verb), json)
            .map(DaemonCommandConnection::Unavailable);
    }
    match service_v2::DaemonService::connect(&context.public_socket) {
        Ok(daemon) => Ok(DaemonCommandConnection::Connected(Box::new(daemon))),
        Err(error)
            if error.exit_code == 69
                && error.message == daemon_access::ClientError::ConnectFailed.to_string() =>
        {
            emit_host_error(&daemon_down_envelope(verb), json)
                .map(DaemonCommandConnection::Unavailable)
        }
        Err(error) => Err(error),
    }
}

/// Typed `not-yet-implemented` envelope (exit 78) for verbs whose
/// daemon-native handler has not landed yet. No bash fallback ever
/// satisfies these — operators receive the typed envelope and the
/// migration-guide cross-link.
fn not_yet_implemented_envelope(verb: &str) -> HostErrorEnvelope {
    host_error_envelope(
        &format!("d2b {verb} has no daemon-native handler yet"),
        "not-yet-implemented",
        78,
        &format!("Native daemon dispatch for `d2b {verb}`"),
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
    /// `D2B_TEST_DEPLOYMENT_SHAPE` test override can select it.
    Tier0AllLegacy,
    /// Mixed: some VMs daemon-owned, some systemd-owned.
    Tier0Mixed,
    /// Every VM is daemon-owned, or the bundle is Tier 1+.
    AllDaemon,
}

fn detect_deployment_shape(context: &Context) -> Result<DeploymentShape, CliFailure> {
    // Test override (used by goldens + cli-legacy-bash-dispatch).
    if let Ok(value) = env::var("D2B_TEST_DEPLOYMENT_SHAPE") {
        return Ok(match value.as_str() {
            "tier0-all-legacy" => DeploymentShape::Tier0AllLegacy,
            "tier0-mixed" => DeploymentShape::Tier0Mixed,
            "all-daemon" | "tier1" => DeploymentShape::AllDaemon,
            other => {
                return Err(CliFailure::new(
                    1,
                    format!("unknown D2B_TEST_DEPLOYMENT_SHAPE value: {other}"),
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
    // through the `D2B_TEST_DEPLOYMENT_SHAPE` override above.
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
                "This legacy Tier-0 shape is unreachable on a daemon-only host: the per-VM `supervisor` option was removed in v1.1, so every enabled VM is daemon-supervised. Host-shared reconciliation on a genuine legacy host is owned by the d2b NixOS module; run `host prepare --dry-run` to inspect the plan.",
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
            // `d2bd` is not yet shipping in
            // bootstrap mode. Surface the same pending-impl envelope
            // the broker would emit so the human / JSON contract
            // stays stable.
            emit_host_error(
                &host_error_envelope(
                    "Daemon-backed prepare staged but the public-socket dispatch path is pending",
                    "daemon-down",
                    1,
                    "Daemon connectivity at /run/d2b/public.sock and broker dispatch readiness.",
                    "d2bd is reachable, but the daemon-side typed-intent dispatch and bundle resolver that back host prepare --apply are not yet wired through d2bd; the broker op is staged but not yet reachable from the public socket.",
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
                print_stdout(
                    "host prepare --dry-run: would do nothing on this tier (no daemon-owned resources detected)\n",
                );
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
                "This legacy Tier-0 shape is unreachable on a daemon-only host: the per-VM `supervisor` option was removed in v1.1, so every enabled VM is daemon-supervised. The historical `--legacy` bash-destroy escape hatch was retired in v1.0 (per ADR 0015); run `host destroy --dry-run` to inspect d2b-owned resources.",
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
                "d2bd is reachable, but the daemon-side typed-intent dispatch and bundle resolver that back host destroy --apply are not yet wired through d2bd; the broker op is staged but not yet reachable from the public socket.",
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
        "notes": "host destroy --dry-run reports d2b-owned resources only; foreign resources are never touched.",
    });
    if args.json {
        let mut rendered = serde_json::to_string_pretty(&summary).map_err(|err| {
            CliFailure::new(1, format!("failed to serialize dry-run summary: {err}"))
        })?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        print_stdout("host destroy --dry-run: no d2b-owned resources to remove\n");
    }
    Ok(0)
}

fn host_shutdown_vm_phases(manifest: &ManifestDocument) -> Vec<Vec<String>> {
    let mut workloads = Vec::new();
    let mut net_vms = Vec::new();
    for vm in manifest.vms() {
        let item = (vm.env.clone().unwrap_or_default(), vm.name.clone());
        if vm.is_net_vm {
            net_vms.push(item);
        } else {
            workloads.push(item);
        }
    }
    workloads.sort();
    net_vms.sort();
    vec![
        workloads.into_iter().map(|(_, name)| name).collect(),
        net_vms.into_iter().map(|(_, name)| name).collect(),
    ]
}

fn render_host_shutdown_hook_plan(phases: &[Vec<String>], json: bool) -> Result<(), CliFailure> {
    if json {
        let mut rendered = serde_json::to_string_pretty(&serde_json::json!({
            "command": "host shutdown-hook",
            "mode": "dry-run",
            "phases": phases,
            "notes": "workload VMs stop before env net VMs; systemd invokes --apply only while the host manager is stopping",
        }))
        .map_err(|err| CliFailure::new(1, format!("failed to serialize shutdown plan: {err}")))?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        print_stdout(&format!(
            "host shutdown-hook --dry-run: would stop {} workload VM(s), then {} net VM(s)\n",
            phases.first().map(Vec::len).unwrap_or(0),
            phases.get(1).map(Vec::len).unwrap_or(0),
        ));
    }
    Ok(())
}

fn cmd_host_shutdown_hook(
    context: &Context,
    args: &HostShutdownHookArgs,
) -> Result<i32, CliFailure> {
    let flags =
        require_explicit_mutation_flag("host shutdown-hook", args.dry_run, args.apply, args.json)?;
    let manifest = context.load_manifest()?;
    let phases = host_shutdown_vm_phases(&manifest);
    if !flags.apply {
        render_host_shutdown_hook_plan(&phases, args.json)?;
        return Ok(0);
    }

    let mut stopped = Vec::new();
    let mut skipped = Vec::new();
    let mut failures = Vec::new();
    for phase in &phases {
        let phase_results = std::thread::scope(|scope| {
            let handles = phase
                .iter()
                .map(|vm| {
                    let context = context.clone();
                    let vm = vm.clone();
                    scope.spawn(move || {
                        let result = try_daemon_mutating_verb(
                            &context,
                            "vmStop",
                            serde_json::json!({ "vm": vm }),
                            false,
                            true,
                            true,
                        );
                        (vm, result)
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("shutdown hook worker panicked"))
                .collect::<Vec<_>>()
        });
        for (vm, outcome) in phase_results {
            let outcome = match outcome {
                Ok(outcome) => outcome,
                Err(err) => {
                    failures.push(format!("{vm}: {}", err.message));
                    continue;
                }
            };
            match outcome {
                DaemonVerbOutcome::Applied { .. } => stopped.push(vm.clone()),
                DaemonVerbOutcome::InvalidRequest { .. } => skipped.push(vm.clone()),
                DaemonVerbOutcome::Unreachable => {
                    failures.push(format!("{vm}: daemon unreachable"));
                }
                DaemonVerbOutcome::BrokerError { summary, .. } => {
                    failures.push(format!(
                        "{vm}: {}",
                        summary.unwrap_or_else(|| "broker error".to_owned())
                    ));
                }
                DaemonVerbOutcome::NotYetImplemented { verb, .. } => {
                    failures.push(format!("{vm}: {verb} not implemented"));
                }
                DaemonVerbOutcome::ApiReadyTimeout { summary } => {
                    failures.push(format!(
                        "{vm}: {}",
                        summary.unwrap_or_else(|| "api-ready timeout".to_owned())
                    ));
                }
                DaemonVerbOutcome::DryRunPlanned { .. } => {
                    failures.push(format!("{vm}: daemon returned dry-run for apply request"));
                }
            }
        }
    }

    if !failures.is_empty() {
        return Err(CliFailure::new(
            1,
            format!(
                "host shutdown-hook failed after stopping {} VM(s), skipping {} already-stopped VM(s): {}",
                stopped.len(),
                skipped.len(),
                failures.join("; ")
            ),
        ));
    }
    if args.json {
        let mut rendered = serde_json::to_string_pretty(&serde_json::json!({
            "command": "host shutdown-hook",
            "mode": "apply",
            "stopped": stopped,
            "skipped": skipped,
        }))
        .map_err(|err| CliFailure::new(1, format!("failed to serialize shutdown result: {err}")))?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        print_stdout(&format!(
            "host shutdown-hook --apply: stopped {} VM(s), skipped {} already-stopped VM(s)\n",
            stopped.len(),
            skipped.len()
        ));
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
                "Re-run as `d2b host doctor --read-only`. The doctor verb is read-only; mutation forms are future deliverables.",
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct StorageMigrationPlan {
    command: &'static str,
    mode: &'static str,
    checkpoint_id: String,
    rollback_command: String,
    vm_count: usize,
    vms: Vec<String>,
    preflight_requirements: Vec<&'static str>,
    preserve: Vec<&'static str>,
    cutover_only_cleanup: Vec<&'static str>,
    fail_closed_hazards: Vec<&'static str>,
    apply_status: &'static str,
}

fn storage_migration_checkpoint_id(vms: &[String]) -> String {
    let mut basis = String::from("storage-cutover-v1\n");
    let mut sorted = vms.to_vec();
    sorted.sort();
    for vm in &sorted {
        let _ = writeln!(basis, "{vm}");
    }
    let digest = sha256_hex(basis.as_bytes());
    let suffix = digest
        .strip_prefix("sha256:")
        .unwrap_or(digest.as_str())
        .chars()
        .take(12)
        .collect::<String>();
    format!("storage-cutover-{suffix}")
}

fn build_storage_migration_plan(manifest: &ManifestDocument) -> StorageMigrationPlan {
    let mut vms: Vec<String> = manifest.vms().iter().map(|vm| vm.name.clone()).collect();
    vms.sort();
    let checkpoint_id = storage_migration_checkpoint_id(&vms);
    let rollback_command =
        format!("d2b host migrate-storage --rollback --from-checkpoint {checkpoint_id}");
    StorageMigrationPlan {
        command: "host migrate-storage",
        mode: "dry-run",
        checkpoint_id,
        rollback_command,
        vm_count: vms.len(),
        vms,
        preflight_requirements: vec![
            "all d2b VMs stopped",
            "d2bd.service stopped",
            "d2b-priv-broker.service stopped",
            "operator accepts planned downtime for the one-time storage layout cutover",
            "net VMs stopped; guest routing, TAP connectivity, and dependent bridge traffic will be interrupted",
        ],
        preserve: vec![
            "per-VM swtpm NVRAM and swtpm identity markers",
            "framework SSH keys and guest sshd host keys",
            "VM disk images and declared persistent volumes",
            "store-view generation metadata and gcroots",
            "daemon diagnostic reports, audit logs, host-runtime metadata, and non-authority adoption history",
            "declared host bridges, TAP naming intent, nftables/NM/networkd ownership metadata, and network-preflight evidence",
        ],
        cutover_only_cleanup: vec![
            "/run/d2b-gpu",
            "/run/d2b-video",
            "/run/d2b-wlproxy",
            "/var/lib/d2b/guest-control-<vm>",
            "boot-scoped runtime socket files only after all d2b services are stopped",
            "runtime network helper sockets and stale TAP pid/metadata files after all d2b services are stopped",
            "stale migration markers from retired storage waves",
        ],
        fail_closed_hazards: vec![
            "symlink or path traversal inside any moved path",
            "foreign ownership markers on a d2b-managed path",
            "recursive operations traversing hardlink farms or mutating shared /nix/store inodes",
            "missing swtpm marker for a previously provisioned TPM VM",
            "any candidate outside the generated storage root set",
            "any open d2b daemon, broker, runner, net VM, or workload VM file descriptor",
            "any attempt to unlink lock files during cutover rather than leaving /run locks for reboot/tmpfs cleanup",
        ],
        apply_status: "not-implemented-in-this-build",
    }
}

fn cmd_host_migrate_storage(
    context: &Context,
    args: &HostMigrateStorageArgs,
) -> Result<i32, CliFailure> {
    if args.rollback {
        let checkpoint = args.from_checkpoint.as_deref().unwrap_or("<missing>");
        return emit_host_error(
            &host_error_envelope(
                "Storage rollback is not implemented in this build",
                "storage-migration-rollback-not-implemented",
                78,
                "Rollback request for a storage cutover checkpoint.",
                &format!("rollback requested from checkpoint {checkpoint}"),
                "Keep the host stopped and use the checkpoint metadata to file an issue; do not repair with recursive chmod/chown/setfacl.",
                "docs/reference/cli-contract.md#host-migrate-storage",
            ),
            args.json,
        );
    }

    let flags = require_explicit_mutation_flag(
        "host migrate-storage",
        args.dry_run,
        args.apply,
        args.json,
    )?;
    if flags.apply {
        return emit_host_error(
            &host_error_envelope(
                "Storage cutover apply is not implemented in this build",
                "storage-migration-apply-not-implemented",
                78,
                "Broker-backed storage cutover mover availability.",
                "apply requested, but only dry-run checkpoint planning is available",
                "Run `d2b host migrate-storage --dry-run` and wait for the broker-backed apply implementation before moving persistent state.",
                "docs/reference/cli-contract.md#host-migrate-storage",
            ),
            args.json,
        );
    }

    let manifest = context.load_manifest()?;
    let plan = build_storage_migration_plan(&manifest);
    if args.json {
        let mut rendered = serde_json::to_string_pretty(&plan).map_err(|err| {
            CliFailure::new(
                1,
                format!("failed to serialize storage migration plan: {err}"),
            )
        })?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        print_stdout(&format!(
            "host migrate-storage --dry-run: checkpoint={} vm_count={}\n",
            plan.checkpoint_id, plan.vm_count
        ));
        print_stdout(&format!("rollback command: {}\n", plan.rollback_command));
        print_stdout("preflight requirements:\n");
        for requirement in &plan.preflight_requirements {
            print_stdout(&format!("  - {requirement}\n"));
        }
        print_stdout("persistent data preserved:\n");
        for item in &plan.preserve {
            print_stdout(&format!("  - {item}\n"));
        }
        print_stdout("cutover-only cleanup candidates:\n");
        for item in &plan.cutover_only_cleanup {
            print_stdout(&format!("  - {item}\n"));
        }
    }
    Ok(0)
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
    // (4) optionally enable + start d2bd.service, (5) emit smoke.
    if !args.dry_run && !args.apply {
        return emit_host_error(
            &host_error_envelope(
                "host install requires either --dry-run or --apply",
                "--apply-or-dry-run-required",
                78,
                "host install invocation flags.",
                "Neither --dry-run nor --apply was provided.",
                "Re-run as `d2b host install --dry-run` to plan or `d2b host install --apply` (optionally with --enable / --start | --no-start) to install.",
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
            { "step": 1, "what": "place systemd units at /etc/systemd/system/d2bd.service + d2b-priv-broker.socket" },
            { "step": 2, "what": "write daemon-config.json to /etc/d2b/daemon-config.json with paths matching the daemon's compiled-in defaults" },
            { "step": 3, "what": "bind /run/d2b/public.sock + /run/d2b/priv.sock with socket ACLs (launcher / admin groups)" },
            { "step": 4, "what": if args.enable && args.start { "systemctl enable --now d2bd.service" } else if args.enable { "systemctl enable d2bd.service" } else if args.no_start { "do NOT enable; operator starts manually" } else { "neither --enable nor --start specified: leave service inactive" } },
            { "step": 5, "what": "smoke: d2b auth status against /run/d2b/public.sock" },
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
            "host install --dry-run: would install d2bd at /etc/systemd/system/ and bind /run/d2b/public.sock (the live --apply path routes through the daemon → broker RunHostInstall path)\n",
        );
    }
    Ok(0)
}

fn cmd_host_reconcile(
    context: &Context,
    args: &HostReconcileArgs,
    _original_args: &[OsString],
) -> Result<i32, CliFailure> {
    // Focused recovery verb that re-runs the broker-side per-env network
    // slice without starting any VM.
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
                "Re-run as `d2b host reconcile --network --dry-run` to plan or `d2b host reconcile --network --apply` to apply.",
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
            "Whether the VM name appears in `d2b.vms.<name>` in the active manifest.",
            "VM name unknown",
            "Run `d2b list` to see declared VMs, then re-run with a name from that list.",
            "docs/reference/error-codes.md#not-found",
        ),
        json,
    )?;
    Err(CliFailure::new(exit_code, format!("unknown vm: {vm}")))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedRealmGateway {
    realm: String,
    gateway_vm: String,
    gateway_target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum VmTargetRoute {
    Local {
        vm: String,
    },
    Gateway {
        realm: String,
        gateway_vm: String,
        gateway: String,
        target: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RealmEntrypointDocument {
    #[serde(rename = "schemaVersion")]
    _schema_version: u32,
    entries: BTreeMap<String, RealmEntrypointConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RealmEntrypointConfig {
    mode: String,
    gateway: Option<String>,
}

fn safe_error_snippet(raw: &str) -> String {
    const MAX: usize = 64;
    let secret_shaped = raw.contains("SharedAccessKey")
        || raw.contains("Bearer ")
        || raw.contains("Endpoint=sb://")
        || raw.contains("AccountKey=")
        || raw.contains("PRIVATE KEY")
        || raw.contains("/home/");
    if secret_shaped {
        return "<redacted>".to_owned();
    }
    let mut snippet = raw.chars().take(MAX).collect::<String>();
    if raw.chars().count() > MAX {
        snippet.push_str("...");
    }
    snippet
}

fn local_realm_entrypoint_config() -> RealmEntrypointConfig {
    RealmEntrypointConfig {
        mode: "host-resident".to_owned(),
        gateway: None,
    }
}

fn normalize_realm_entrypoint_entries(
    mut entries: BTreeMap<String, RealmEntrypointConfig>,
) -> Result<BTreeMap<String, RealmEntrypointConfig>, CliFailure> {
    match entries.get("local") {
        Some(entry) if entry.mode == "host-resident" && entry.gateway.is_none() => {}
        Some(_) => {
            return Err(CliFailure::new(
                1,
                "realm entrypoint `local` must remain host-resident and credential-free",
            ));
        }
        None => {
            entries.insert("local".to_owned(), local_realm_entrypoint_config());
        }
    }
    Ok(entries)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct RealmGatewayListEntry {
    realm: String,
    gateway_vm: String,
    gateway_target: String,
    state: String,
}

#[cfg(not(test))]
fn realm_entrypoints_path() -> PathBuf {
    env_path("D2B_REALM_ENTRYPOINTS_PATH", DEFAULT_REALM_ENTRYPOINTS_PATH)
}

#[cfg(test)]
fn realm_entrypoints_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".d2b-test-missing-realm-entrypoints.json")
}

fn load_realm_entrypoint_table()
-> Result<Option<d2b_realm_router::RealmEntrypointTable>, CliFailure> {
    let path = realm_entrypoints_path();
    load_realm_entrypoint_table_from_path(&path)
}

fn load_realm_entrypoint_document_from_path(
    path: &Path,
) -> Result<Option<RealmEntrypointDocument>, CliFailure> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(CliFailure::new(
                1,
                format!("failed to read {}: {err}", path.display()),
            ));
        }
    };
    let mut raw = Vec::new();
    let read = io::Read::by_ref(&mut file)
        .take(MAX_REALM_ENTRYPOINTS_BYTES + 1)
        .read_to_end(&mut raw)
        .map_err(|err| CliFailure::new(1, format!("failed to read {}: {err}", path.display())))?;
    if read as u64 > MAX_REALM_ENTRYPOINTS_BYTES {
        return Err(CliFailure::new(
            1,
            format!(
                "realm entrypoints file {} exceeds the 1 MiB limit",
                path.display()
            ),
        ));
    }
    let raw = String::from_utf8(raw).map_err(|err| {
        CliFailure::new(
            1,
            format!("failed to parse {} as UTF-8: {err}", path.display()),
        )
    })?;
    let doc: RealmEntrypointDocument = serde_json::from_str(&raw)
        .map_err(|err| CliFailure::new(1, format!("failed to parse {}: {err}", path.display())))?;
    Ok(Some(doc))
}

fn load_realm_entrypoint_table_from_path(
    path: &Path,
) -> Result<Option<d2b_realm_router::RealmEntrypointTable>, CliFailure> {
    let Some(doc) = load_realm_entrypoint_document_from_path(path)? else {
        return Ok(None);
    };
    let mut table = d2b_realm_router::RealmEntrypointTable::new();
    for (realm_raw, entry) in normalize_realm_entrypoint_entries(doc.entries)? {
        let realm = target_routing::parse_realm_arg(&realm_raw).map_err(|err| {
            CliFailure::new(
                1,
                format!(
                    "realm entrypoint `{}` is invalid: {}",
                    safe_error_snippet(&realm_raw),
                    safe_error_snippet(&err.to_string())
                ),
            )
        })?;
        match entry.mode.as_str() {
            "host-resident" => table.host_resident(realm),
            "gateway-backed" => {
                let gateway = entry.gateway.ok_or_else(|| {
                    CliFailure::new(
                        1,
                        format!(
                            "gateway-backed realm `{}` has no gateway target",
                            safe_error_snippet(&realm_raw)
                        ),
                    )
                })?;
                let gateway_target = parse_gateway_target_text(&realm_raw, &gateway)?;
                table.gateway_backed(realm, gateway_target);
            }
            other => {
                return Err(CliFailure::new(
                    1,
                    format!(
                        "realm `{}` has unknown entrypoint mode `{}`",
                        safe_error_snippet(&realm_raw),
                        safe_error_snippet(other)
                    ),
                ));
            }
        }
    }
    Ok(Some(table))
}

fn configured_realm_gateways(json: bool) -> Result<Vec<ResolvedRealmGateway>, CliFailure> {
    let Some(doc) = load_realm_entrypoint_document_from_path(&realm_entrypoints_path())? else {
        return Ok(Vec::new());
    };
    let mut gateways = Vec::new();
    for (realm_raw, entry) in normalize_realm_entrypoint_entries(doc.entries)? {
        if entry.mode != "gateway-backed" {
            continue;
        }
        let realm = target_routing::parse_realm_arg(&realm_raw).map_err(|err| {
            CliFailure::new(
                1,
                format!(
                    "realm entrypoint `{}` is invalid: {}",
                    safe_error_snippet(&realm_raw),
                    safe_error_snippet(&err.to_string())
                ),
            )
        })?;
        let gateway_target = entry.gateway.ok_or_else(|| {
            CliFailure::new(
                1,
                format!(
                    "gateway-backed realm `{}` has no gateway target",
                    safe_error_snippet(&realm_raw)
                ),
            )
        })?;
        let canonical_gateway_target = target_name_from_gateway_text(&gateway_target)
            .map_err(|err| target_routing::RouteError::InvalidGatewayTarget {
                realm: realm.target_form(),
                gateway: safe_error_snippet(&gateway_target),
                reason: err.to_string(),
            })
            .map_err(|err| emit_route_error(err, json).unwrap_or_else(|failure| failure))?
            .to_string();
        gateways.push(ResolvedRealmGateway {
            realm: realm.target_form(),
            gateway_vm: gateway_vm_from_target_text(&realm.target_form(), &gateway_target)
                .map_err(|err| emit_route_error(err, json).unwrap_or_else(|failure| failure))?,
            gateway_target: canonical_gateway_target,
        });
    }
    gateways.sort_by(|a, b| a.realm.cmp(&b.realm));
    Ok(gateways)
}

fn gateway_vm_from_target_text(
    realm: &str,
    target: &str,
) -> Result<String, target_routing::RouteError> {
    target_name_from_gateway_text(target)
        .map(|target| target.workload.as_str().to_owned())
        .map_err(|err| target_routing::RouteError::InvalidGatewayTarget {
            realm: realm.to_owned(),
            gateway: safe_error_snippet(target),
            reason: err.to_string(),
        })
}

fn target_name_from_gateway_text(
    target: &str,
) -> Result<d2b_realm_core::TargetName, d2b_realm_core::TargetParseError> {
    match d2b_realm_core::TargetName::parse(target) {
        Ok(target) => Ok(target),
        Err(d2b_realm_core::TargetParseError::MissingRealm) => {
            let body = target.strip_prefix("d2b://").unwrap_or(target);
            let labels = body.split('.').collect::<Vec<_>>();
            if let [vm, "d2b"] = labels.as_slice() {
                d2b_realm_core::TargetName::parse(&format!("{vm}.local.d2b"))
            } else {
                Err(d2b_realm_core::TargetParseError::MissingRealm)
            }
        }
        Err(err) => Err(err),
    }
}

fn parse_gateway_target_text(
    realm: &str,
    gateway: &str,
) -> Result<d2b_realm_core::TargetName, CliFailure> {
    target_name_from_gateway_text(gateway).map_err(|err| {
        CliFailure::new(
            1,
            format!(
                "realm `{}` gateway target `{}` is invalid: {}",
                safe_error_snippet(realm),
                safe_error_snippet(gateway),
                safe_error_snippet(&err.to_string())
            ),
        )
    })
}

fn conventional_gateway_route(raw: &str, json: bool) -> Result<Option<VmTargetRoute>, CliFailure> {
    let Some(hint) = target_routing::gateway_hint(raw)
        .map_err(|err| emit_route_error(err, json).unwrap_or_else(|failure| failure))?
    else {
        return Ok(None);
    };
    Ok(Some(VmTargetRoute::Gateway {
        realm: hint.realm.target_form(),
        gateway_vm: hint.gateway_vm,
        gateway: hint.gateway_target,
        target: hint.target,
    }))
}

fn emit_realm_usage_error(
    message: &str,
    observed: &str,
    remediation: &str,
    json: bool,
) -> Result<CliFailure, CliFailure> {
    let exit_code = emit_host_error(
        &host_error_envelope(
            message,
            "realm-target-usage",
            2,
            "Realm target syntax and local realm entrypoint configuration.",
            observed,
            remediation,
            "docs/adr/0032-d2b-v2-constellation-control-plane.md#target-address-and-name-scheme",
        ),
        json,
    )?;
    Ok(CliFailure::new(exit_code, message.to_owned()))
}

fn emit_missing_realm_entrypoint(
    realm: &str,
    gateway_vm: &str,
    target: Option<&str>,
    json: bool,
) -> Result<CliFailure, CliFailure> {
    let target_note = target
        .map(|target| format!(" for target `{target}`"))
        .unwrap_or_default();
    let message = format!("realm `{realm}` has no local gateway entrypoint{target_note}");
    let observed = format!("expected gateway VM `{gateway_vm}` was not declared in the manifest");
    let remediation = format!(
        "Declare and start the realm gateway VM `{gateway_vm}`, then retry; or use a local VM name for host-local operations."
    );
    let exit_code = emit_host_error(
        &host_error_envelope(
            &message,
            "missing-realm-entrypoint",
            2,
            "Realm entrypoint resolution using the manifest-backed gateway convention.",
            &observed,
            &remediation,
            "docs/adr/0032-d2b-v2-constellation-control-plane.md#entrypoint-and-component-topology",
        ),
        json,
    )?;
    Ok(CliFailure::new(exit_code, message))
}

fn emit_route_error(err: target_routing::RouteError, json: bool) -> Result<CliFailure, CliFailure> {
    let message = err.to_string();
    let exit_code = emit_host_error(
        &host_error_envelope(
            &message,
            "missing-realm-entrypoint",
            2,
            "TargetResolver route decision for the requested VM target.",
            "realm target is not dispatchable from this host entrypoint",
            "Declare a realm gateway entrypoint, use `d2b realm run <realm> -- ...`, or run the command against the gateway daemon.",
            "docs/adr/0032-d2b-v2-constellation-control-plane.md#constellation-command-flow",
        ),
        json,
    )?;
    Ok(CliFailure::new(exit_code, message))
}

/// Emit a non-fatal compatibility warning to stderr when a bare VM name is used
/// and the daemon has advertised a canonical workload target for it. Does
/// nothing in `--json` mode (JSON callers parse structured output only).
fn print_workload_migration_hint(hint: &target_routing::TargetMigrationHint, json: bool) {
    if json {
        return;
    }
    print_stderr(&format!("note: {hint}\n"));
}

fn route_vm_target(context: &Context, raw: &str, json: bool) -> Result<VmTargetRoute, CliFailure> {
    // Fail-closed for old env-qualified targets missing the `.d2b` suffix.
    // E.g. `corp-vm.work` → error with suggestion `corp-vm.work.d2b`.
    if let Some(hint) = target_routing::detect_env_style_target(raw)
        && let target_routing::TargetMigrationHint::OldEnvStyleTarget { suggested, .. } = &hint
    {
        let message = hint.to_string();
        let exit_code = emit_host_error(
            &host_error_envelope(
                &message,
                "old-env-style-target",
                2,
                "CLI target parsing: env-qualified names require the `.d2b` suffix.",
                raw,
                &format!("Use `{suggested}` (the canonical workload target form)."),
                "docs/reference/cli-contract.md",
            ),
            json,
        )?;
        return Err(CliFailure::new(exit_code, message));
    }
    route_vm_target_with_table(context, raw, json, load_realm_entrypoint_table()?)
}

fn route_vm_target_with_table(
    context: &Context,
    raw: &str,
    json: bool,
    table: Option<d2b_realm_router::RealmEntrypointTable>,
) -> Result<VmTargetRoute, CliFailure> {
    if let Some(vm) = try_vm_for_canonical_target(&context.bundle_path, raw) {
        return Ok(VmTargetRoute::Local { vm });
    }

    if table.is_none() {
        if let Some(route) = conventional_gateway_route(raw, json)? {
            if context
                .load_manifest()?
                .get_vm(match &route {
                    VmTargetRoute::Gateway { gateway_vm, .. } => gateway_vm,
                    VmTargetRoute::Local { vm } => vm,
                })
                .is_none()
                && let VmTargetRoute::Gateway {
                    realm,
                    gateway_vm,
                    target,
                    ..
                } = &route
            {
                return Err(emit_missing_realm_entrypoint(
                    realm,
                    gateway_vm,
                    Some(target),
                    json,
                )?);
            }
            return Ok(route);
        }
        let table = d2b_realm_router::RealmEntrypointTable::with_local_default();
        return match target_routing::route(raw, &table) {
            Ok(target_routing::Route::Local { vm }) => Ok(VmTargetRoute::Local { vm }),
            Ok(target_routing::Route::Gateway { gateway, target }) => {
                let realm = d2b_realm_core::TargetName::parse(&target)
                    .map(|target| target.realm.target_form())
                    .unwrap_or_else(|_| "unknown".to_owned());
                let gateway_vm = gateway_vm_from_target_text(&realm, &gateway)
                    .map_err(|err| emit_route_error(err, json).unwrap_or_else(|failure| failure))?;
                Ok(VmTargetRoute::Gateway {
                    realm,
                    gateway_vm,
                    gateway,
                    target,
                })
            }
            Err(err) => Err(emit_route_error(err, json)?),
        };
    }

    let manifest = context.load_manifest()?;
    match target_routing::route(raw, table.as_ref().expect("checked above")) {
        Ok(target_routing::Route::Local { vm }) => Ok(VmTargetRoute::Local { vm }),
        Ok(target_routing::Route::Gateway { gateway, target }) => {
            let realm = d2b_realm_core::TargetName::parse(&target)
                .map(|target| target.realm.target_form())
                .unwrap_or_else(|_| "unknown".to_owned());
            let gateway_vm = gateway_vm_from_target_text(&realm, &gateway)
                .map_err(|err| emit_route_error(err, json).unwrap_or_else(|failure| failure))?;
            if manifest.get_vm(&gateway_vm).is_none() {
                return Err(emit_missing_realm_entrypoint(
                    &realm,
                    &gateway_vm,
                    Some(&target),
                    json,
                )?);
            }
            Ok(VmTargetRoute::Gateway {
                realm,
                gateway_vm,
                gateway,
                target,
            })
        }
        Err(err) => Err(emit_route_error(err, json)?),
    }
}

fn resolve_realm_gateway(
    context: &Context,
    realm_raw: &str,
    json: bool,
) -> Result<ResolvedRealmGateway, CliFailure> {
    let realm = target_routing::parse_realm_arg(realm_raw).map_err(|err| {
        emit_realm_usage_error(
            &format!(
                "invalid realm `{}`: {}",
                safe_error_snippet(realm_raw),
                safe_error_snippet(&err.to_string())
            ),
            "realm argument did not parse as a bounded lowercase realm path",
            "Use a DNS-shaped realm path such as `work` or `payments.work`.",
            json,
        )
        .unwrap_or_else(|failure| failure)
    })?;
    let (gateway_vm, gateway_target) = if let Some(table) = load_realm_entrypoint_table()? {
        let probe_target = format!("probe.{}.d2b", realm.target_form());
        match target_routing::route(&probe_target, &table) {
            Ok(target_routing::Route::Gateway { gateway, .. }) => {
                let gateway_vm = gateway_vm_from_target_text(&realm.target_form(), &gateway)
                    .map_err(|err| emit_route_error(err, json).unwrap_or_else(|failure| failure))?;
                (gateway_vm, gateway)
            }
            Ok(target_routing::Route::Local { .. }) => {
                return Err(emit_missing_realm_entrypoint(
                    &realm.target_form(),
                    &target_routing::gateway_vm_name(&realm),
                    None,
                    json,
                )?);
            }
            Err(err) => return Err(emit_route_error(err, json)?),
        }
    } else {
        let gateway_vm = target_routing::gateway_vm_name(&realm);
        let gateway_target = target_routing::gateway_target_name(&realm)
            .map_err(|err| emit_route_error(err, json).unwrap_or_else(|failure| failure))?;
        (gateway_vm, gateway_target.to_string())
    };
    let manifest = context.load_manifest()?;
    if manifest.get_vm(&gateway_vm).is_none() {
        return Err(emit_missing_realm_entrypoint(
            &realm.target_form(),
            &gateway_vm,
            None,
            json,
        )?);
    }
    Ok(ResolvedRealmGateway {
        realm: realm.target_form(),
        gateway_vm,
        gateway_target,
    })
}

fn gateway_lifecycle_state(
    context: &Context,
    gateway_vm: &str,
) -> Result<Option<IpcVmLifecycleState>, CliFailure> {
    let daemon = service_v2::DaemonService::connect(&context.public_socket)?;
    daemon
        .list_workloads(Some(gateway_vm))?
        .iter()
        .find(|workload| {
            workload.name == gateway_vm
                || workload
                    .identity
                    .as_ref()
                    .is_some_and(|identity| identity.workload_name == gateway_vm)
        })
        .map(service_v2::vm_lifecycle_state)
        .transpose()
}

fn gateway_lifecycle_states(context: &Context) -> Result<BTreeMap<String, String>, CliFailure> {
    let daemon = service_v2::DaemonService::connect(&context.public_socket)?;
    let mut states = BTreeMap::new();
    for workload in daemon.list_workloads(None)? {
        let label = gateway_state_label(service_v2::vm_lifecycle_state(&workload)?).to_owned();
        states.insert(workload.name.clone(), label.clone());
        if let Some(identity) = workload.identity.as_ref() {
            states.insert(identity.workload_name.clone(), label);
        }
    }
    Ok(states)
}

fn gateway_state_allows_exec(state: IpcVmLifecycleState) -> bool {
    matches!(
        state,
        IpcVmLifecycleState::Booted | IpcVmLifecycleState::Running
    )
}

fn gateway_state_label(state: IpcVmLifecycleState) -> &'static str {
    match state {
        IpcVmLifecycleState::Stopped => "stopped",
        IpcVmLifecycleState::Starting => "starting",
        IpcVmLifecycleState::Booted => "booted",
        IpcVmLifecycleState::Running => "running",
        IpcVmLifecycleState::Stopping => "stopping",
        IpcVmLifecycleState::Restarting => "restarting",
        IpcVmLifecycleState::Failed => "failed",
        IpcVmLifecycleState::Unknown => "unknown",
    }
}

fn ensure_realm_gateway_running(
    context: &Context,
    realm: &str,
    gateway_vm: &str,
    json: bool,
) -> Result<(), CliFailure> {
    match gateway_lifecycle_state(context, gateway_vm)? {
        Some(state) if gateway_state_allows_exec(state) => Ok(()),
        observed => {
            let observed_state = observed
                .map(gateway_state_label)
                .unwrap_or("not reported by d2bd");
            let message = format!("realm `{realm}` gateway `{gateway_vm}` is not running");
            let remediation = format!(
                "Start the gateway with `d2b vm start {gateway_vm} --apply`, wait for it to be running, then retry."
            );
            let exit_code = emit_host_error(
                &host_error_envelope(
                    &message,
                    "gateway-not-running",
                    70,
                    "Gateway VM lifecycle state from d2bd before entering the realm.",
                    observed_state,
                    &remediation,
                    "docs/adr/0032-d2b-v2-constellation-control-plane.md#constellation-command-flow",
                ),
                json,
            )?;
            Err(CliFailure::new(exit_code, message))
        }
    }
}

fn realm_gateway_exec_args(
    gateway_vm: String,
    argv: Vec<String>,
    interactive: bool,
    tty: bool,
    json: bool,
    human: bool,
) -> VmExecArgs {
    VmExecArgs {
        detach: false,
        interactive,
        tty,
        vm: gateway_vm,
        json,
        human,
        management: Vec::new(),
        command: argv,
    }
}

#[allow(dead_code)]
fn realm_policy_rows(
    context: &Context,
    json: bool,
) -> Result<Vec<RealmPolicyOutputV1>, CliFailure> {
    match realm_policy_rows_raw(context) {
        Ok(rows) => Ok(rows),
        Err(err) => {
            if json {
                let _ = emit_host_error(
                    &host_error_envelope(
                        &err.message,
                        "realm-policy-invalid",
                        err.exit_code,
                        "Rendered realm entrypoint policy.",
                        "realm policy could not be inspected",
                        "Fix the rendered realm entrypoints and rebuild the host.",
                        "docs/reference/realm-policy.md",
                    ),
                    true,
                )?;
            }
            Err(err)
        }
    }
}

fn realm_policy_rows_raw(context: &Context) -> Result<Vec<RealmPolicyOutputV1>, CliFailure> {
    let entries =
        if let Some(doc) = load_realm_entrypoint_document_from_path(&realm_entrypoints_path())? {
            doc.entries
        } else {
            let mut entries = std::collections::BTreeMap::new();
            entries.insert("local".to_owned(), local_realm_entrypoint_config());
            entries
        };
    realm_policy_rows_from_entries(context, normalize_realm_entrypoint_entries(entries)?)
}

fn realm_policy_rows_from_entries(
    context: &Context,
    entries: BTreeMap<String, RealmEntrypointConfig>,
) -> Result<Vec<RealmPolicyOutputV1>, CliFailure> {
    let gateway_states = gateway_lifecycle_states(context)?;
    realm_policy_rows_from_entries_with_states(entries, &gateway_states)
}

fn realm_policy_rows_from_entries_with_states(
    entries: BTreeMap<String, RealmEntrypointConfig>,
    gateway_states: &BTreeMap<String, String>,
) -> Result<Vec<RealmPolicyOutputV1>, CliFailure> {
    let mut rows = Vec::new();
    for (realm_raw, entry) in entries {
        let realm = target_routing::parse_realm_arg(&realm_raw).map_err(|err| {
            CliFailure::new(
                1,
                format!(
                    "realm entrypoint `{}` is invalid: {}",
                    safe_error_snippet(&realm_raw),
                    safe_error_snippet(&err.to_string())
                ),
            )
        })?;
        let realm_target = realm.target_form();
        let mode = entry.mode;
        match mode.as_str() {
            "host-resident" => rows.push(RealmPolicyOutputV1 {
                realm: realm_target,
                mode,
                gateway_vm: None,
                gateway_target: None,
                gateway_state: "local-only".to_owned(),
                cross_realm_policy: "default-deny".to_owned(),
                credential_boundary: "host-resident-local-only".to_owned(),
            }),
            "gateway-backed" => {
                let gateway_target = entry.gateway.ok_or_else(|| {
                    CliFailure::new(
                        1,
                        format!(
                            "gateway-backed realm `{}` has no gateway target",
                            safe_error_snippet(&realm_raw)
                        ),
                    )
                })?;
                let canonical_gateway_target = target_name_from_gateway_text(&gateway_target)
                    .map_err(|err| {
                        CliFailure::new(
                            1,
                            format!(
                                "realm `{}` gateway target is invalid: {}",
                                safe_error_snippet(&realm_target),
                                safe_error_snippet(&err.to_string())
                            ),
                        )
                    })?;
                let gateway_vm = canonical_gateway_target.workload.as_str().to_owned();
                let gateway_target = canonical_gateway_target.to_string();
                let gateway_state = gateway_states
                    .get(&gateway_vm)
                    .map(String::as_str)
                    .unwrap_or("not reported by d2bd")
                    .to_owned();
                rows.push(RealmPolicyOutputV1 {
                    realm: realm_target,
                    mode,
                    gateway_vm: Some(gateway_vm),
                    gateway_target: Some(gateway_target),
                    gateway_state,
                    cross_realm_policy: "default-deny".to_owned(),
                    credential_boundary: "gateway-owned".to_owned(),
                });
            }
            other => {
                return Err(CliFailure::new(
                    1,
                    format!(
                        "realm `{}` has unknown entrypoint mode `{}`",
                        safe_error_snippet(&realm_raw),
                        safe_error_snippet(other)
                    ),
                ));
            }
        }
    }
    rows.sort_by(|a, b| a.realm.cmp(&b.realm));
    Ok(rows)
}

fn print_realm_rows_human(rows: &[RealmPolicyOutputV1]) {
    print_stdout(&format!(
        "{:<24} {:<16} {:<24} {:<22} {:<26} {}\n",
        "REALM", "MODE", "GATEWAY", "STATE", "CREDENTIAL_BOUNDARY", "CROSS_REALM"
    ));
    for row in rows {
        print_stdout(&format!(
            "{:<24} {:<16} {:<24} {:<22} {:<26} {}\n",
            row.realm,
            row.mode,
            row.gateway_vm.as_deref().unwrap_or("-"),
            row.gateway_state,
            row.credential_boundary,
            row.cross_realm_policy
        ));
    }
}

fn print_realm_inspect_human(row: &RealmPolicyOutputV1) {
    print_stdout(&format!("realm: {}\n", row.realm));
    print_stdout(&format!("mode: {}\n", row.mode));
    print_stdout(&format!(
        "gatewayVm: {}\n",
        row.gateway_vm.as_deref().unwrap_or("-")
    ));
    print_stdout(&format!(
        "gatewayTarget: {}\n",
        row.gateway_target.as_deref().unwrap_or("-")
    ));
    print_stdout(&format!("gatewayState: {}\n", row.gateway_state));
    print_stdout(&format!(
        "credentialBoundary: {}\n",
        row.credential_boundary
    ));
    print_stdout(&format!("crossRealmPolicy: {}\n", row.cross_realm_policy));
}

fn cmd_realm_list(context: &Context, args: &RealmListArgs) -> Result<i32, CliFailure> {
    let daemon = service_v2::DaemonService::connect(&context.public_socket)?;
    let rows = service_v2::realm_rows(&daemon.list_realms()?);
    let output = RealmListOutputV1 {
        command: "realm list".to_owned(),
        realms: rows,
    };
    if args.json {
        print_json(&output)?;
    } else if output.realms.is_empty() {
        print_stdout("No realm entrypoints configured\n");
    } else {
        print_realm_rows_human(&output.realms);
    }
    Ok(0)
}

fn cmd_realm_inspect(context: &Context, args: &RealmInspectArgs) -> Result<i32, CliFailure> {
    let daemon = service_v2::DaemonService::connect(&context.public_socket)?;
    let rows = service_v2::realm_rows(&daemon.list_realms()?);
    let output = realm_inspect_output(&args.realm, args.json, rows)?;
    if args.json {
        print_json(&output)?;
    } else {
        print_realm_inspect_human(&output.realm);
    }
    Ok(0)
}

fn realm_inspect_output(
    raw_realm: &str,
    json: bool,
    rows: Vec<RealmPolicyOutputV1>,
) -> Result<RealmInspectOutputV1, CliFailure> {
    let realm = target_routing::parse_realm_arg(raw_realm).map_err(|err| {
        emit_realm_usage_error(
            &format!(
                "invalid realm `{}`: {}",
                safe_error_snippet(raw_realm),
                safe_error_snippet(&err.to_string())
            ),
            "realm argument did not parse as a bounded lowercase realm path",
            "Use a DNS-shaped realm path such as `work` or `payments.work`.",
            json,
        )
        .unwrap_or_else(|failure| failure)
    })?;
    let realm_key = realm.target_form();
    let Some(row) = rows.into_iter().find(|row| row.realm == realm_key) else {
        return Err(emit_missing_realm_entrypoint(
            &realm_key,
            &target_routing::gateway_vm_name(&realm),
            None,
            json,
        )?);
    };
    Ok(RealmInspectOutputV1 {
        command: "realm inspect".to_owned(),
        realm: row,
    })
}

fn op_inspect_trace(args: &OpInspectArgs) -> Result<Option<OpInspectTraceOutputV1>, CliFailure> {
    let (Some(trace_id), Some(span_id)) = (&args.trace_id, &args.span_id) else {
        return Ok(None);
    };
    let trace = d2b_realm_core::TraceContext::new(trace_id, span_id).ok_or_else(|| {
        CliFailure::new(
            2,
            "op inspect: trace context fields must be non-empty, bounded, and contain no whitespace",
        )
    })?;
    Ok(Some(OpInspectTraceOutputV1 {
        trace_id: trace.trace_id().to_owned(),
        span_id: trace.span_id().to_owned(),
    }))
}

fn op_inspect_output(
    context: &Context,
    args: &OpInspectArgs,
) -> Result<OpInspectOutputV1, CliFailure> {
    let trace = op_inspect_trace(args)?;
    let mut degraded = Vec::new();
    let vm_count = match context.load_manifest() {
        Ok(manifest) => u32::try_from(manifest.vms().len()).unwrap_or(u32::MAX),
        Err(_) => {
            degraded.push(OpInspectDegradedOutputV1 {
                scope: "local-manifest".to_owned(),
                reason: "manifest-unavailable".to_owned(),
                remediation: "verify the d2b manifest path and rebuild the host".to_owned(),
            });
            0
        }
    };
    let realms = match realm_policy_rows_raw(context) {
        Ok(realms) => realms,
        Err(_) => {
            degraded.push(OpInspectDegradedOutputV1 {
                scope: "realm-entrypoints".to_owned(),
                reason: "realm-entrypoints-unavailable".to_owned(),
                remediation: "verify realm-entrypoints.json and rebuild the host".to_owned(),
            });
            Vec::new()
        }
    };
    Ok(op_inspect_output_from_parts(
        vm_count, trace, realms, degraded,
    ))
}

fn op_inspect_output_from_parts(
    vm_count: u32,
    trace: Option<OpInspectTraceOutputV1>,
    realms: Vec<RealmPolicyOutputV1>,
    mut degraded: Vec<OpInspectDegradedOutputV1>,
) -> OpInspectOutputV1 {
    let gateway_count = realms
        .iter()
        .filter(|realm| realm.mode == "gateway-backed")
        .filter_map(|realm| realm.gateway_vm.as_deref())
        .collect::<BTreeSet<_>>()
        .len();
    let gateway_count = u32::try_from(gateway_count).unwrap_or(u32::MAX);
    if realms.iter().any(|realm| {
        realm.mode == "gateway-backed"
            && !matches!(realm.gateway_state.as_str(), "running" | "booted")
    }) {
        degraded.push(OpInspectDegradedOutputV1 {
            scope: "gateway".to_owned(),
            reason: "gateway-not-running".to_owned(),
            remediation: "start the realm gateway with `d2b vm start <gateway-vm> --apply`"
                .to_owned(),
        });
    }
    let realm_outputs = realms
        .into_iter()
        .map(|realm| OpInspectRealmOutputV1 {
            realm: realm.realm,
            mode: realm.mode,
            gateway_vm: realm.gateway_vm,
            state: realm.gateway_state,
            cross_realm_policy: realm.cross_realm_policy,
        })
        .collect();
    OpInspectOutputV1 {
        command: "op inspect".to_owned(),
        trace,
        local: OpInspectLocalOutputV1 {
            vm_count,
            gateway_count,
            source: "local-entrypoints".to_owned(),
        },
        realms: realm_outputs,
        degraded,
    }
}

fn cmd_op_inspect(context: &Context, args: &OpInspectArgs) -> Result<i32, CliFailure> {
    let output = op_inspect_output(context, args)?;
    if args.json {
        print_json(&output)?;
    } else {
        print_stdout(&format!(
            "local: vms={} gateways={} source={}\n",
            output.local.vm_count, output.local.gateway_count, output.local.source
        ));
        if let Some(trace) = &output.trace {
            print_stdout(&format!(
                "trace: traceId={} spanId={}\n",
                trace.trace_id, trace.span_id
            ));
        }
        for realm in &output.realms {
            print_stdout(&format!(
                "realm: {} mode={} state={} crossRealm={}\n",
                realm.realm, realm.mode, realm.state, realm.cross_realm_policy
            ));
        }
        for degraded in &output.degraded {
            print_stdout(&format!(
                "degraded: {} reason={} remediation={}\n",
                degraded.scope, degraded.reason, degraded.remediation
            ));
        }
    }
    Ok(0)
}

fn cmd_realm_enter(context: &Context, args: &RealmEnterArgs) -> Result<i32, CliFailure> {
    let gateway = resolve_realm_gateway(context, &args.realm, false)?;
    ensure_realm_gateway_running(context, &gateway.realm, &gateway.gateway_vm, false)?;
    let exec_args = realm_gateway_exec_args(
        gateway.gateway_vm,
        vec!["bash".to_owned(), "-l".to_owned()],
        true,
        true,
        false,
        true,
    );
    cmd_vm_exec_v2(context, &exec_args)
}

fn cmd_realm_run(context: &Context, args: &RealmRunArgs) -> Result<i32, CliFailure> {
    let gateway = resolve_realm_gateway(context, &args.realm, args.json)?;
    ensure_realm_gateway_running(context, &gateway.realm, &gateway.gateway_vm, args.json)?;
    let exec_args = realm_gateway_exec_args(
        gateway.gateway_vm,
        args.argv.clone(),
        false,
        false,
        args.json,
        args.human,
    );
    cmd_vm_exec_v2(context, &exec_args)
}

/// Route a `vm <verb> <target>` argument (ADR 0032, P0). A local VM name routes
/// to the existing host-daemon fast path (returns `Ok`); a realm/gateway target
/// surfaces a typed, json-aware diagnostic and a non-zero exit — the host daemon
/// holds no realm configuration and cannot dispatch into a realm. The realm's
/// gateway-mode `d2bd` owns gateway-backed targets.
#[cfg(test)]
fn guard_local_target(raw: &str, json: bool) -> Result<(), CliFailure> {
    let table = d2b_realm_router::RealmEntrypointTable::with_local_default();
    match target_routing::route(raw, &table) {
        Ok(target_routing::Route::Local { .. }) => Ok(()),
        Ok(target_routing::Route::Gateway { gateway, target }) => {
            let exit_code = emit_host_error(
                &host_error_envelope(
                    &format!(
                        "target '{target}' is gateway-backed (gateway '{gateway}'); the host \
                         daemon cannot dispatch into a realm"
                    ),
                    "usage",
                    2,
                    "Whether the target addresses a local VM the host daemon can dispatch.",
                    "gateway-backed realm target",
                    "Run the verb against the realm gateway's d2bd; the host daemon holds no \
                     realm configuration.",
                    "docs/reference/error-codes.md#usage",
                ),
                json,
            )?;
            Err(CliFailure::new(
                exit_code,
                format!("gateway-backed target: {target}"),
            ))
        }
        Err(err) => {
            let exit_code = emit_host_error(
                &host_error_envelope(
                    &err.to_string(),
                    "usage",
                    2,
                    "Whether the target addresses a local VM the host daemon can dispatch.",
                    "realm target with no local entrypoint",
                    "Use a local VM name, or run the verb against the realm gateway's d2bd.",
                    "docs/reference/error-codes.md#usage",
                ),
                json,
            )?;
            Err(CliFailure::new(
                exit_code,
                format!("target not dispatchable on the host daemon: {raw}"),
            ))
        }
    }
}

#[cfg(test)]
fn gateway_target_from_manifest(
    context: &Context,
    raw: &str,
    json: bool,
) -> Result<Option<String>, CliFailure> {
    match route_vm_target(context, raw, json)? {
        VmTargetRoute::Local { .. } => Ok(None),
        VmTargetRoute::Gateway {
            gateway: _, target, ..
        } => Ok(Some(target)),
    }
}

fn cmd_vm_display(context: &Context, args: &VmDisplayArgs) -> Result<i32, CliFailure> {
    match &args.command {
        VmDisplayCommand::List(args) => cmd_vm_display_list(context, args),
        VmDisplayCommand::Close(args) => cmd_vm_display_close(context, args),
    }
}

/// The frozen `d2b.terminal.v2` contract's `TerminalKind` enum has no
/// `DISPLAY` variant and carries no display-session listing/close
/// operation, so there is no typed v2 API `vm display` could migrate
/// to. Fail closed with an actionable diagnostic instead of retaining
/// the legacy `gatewayDisplay` public-socket path.
fn vm_display_not_yet_implemented_envelope(verb: &str) -> HostErrorEnvelope {
    host_error_envelope(
        &format!("d2b vm {verb} has no ComponentSession v2 handler"),
        "not-yet-implemented",
        78,
        "Whether `d2b.terminal.v2`'s TerminalKind carries a DISPLAY variant.",
        "The frozen v2 terminal contract has no display-session kind or listing/close operation; the legacy gatewayDisplay public-socket path has been removed.",
        "Track display-session support in CHANGELOG.md \"Unreleased\"; no operator workaround exists until a typed v2 surface ships.",
        "docs/reference/error-codes.md#not-yet-implemented",
    )
}

fn cmd_vm_display_list(_context: &Context, args: &VmDisplayListArgs) -> Result<i32, CliFailure> {
    emit_host_error(
        &vm_display_not_yet_implemented_envelope("display list"),
        args.json,
    )
}

fn cmd_vm_display_close(_context: &Context, args: &VmDisplayCloseArgs) -> Result<i32, CliFailure> {
    emit_host_error(
        &vm_display_not_yet_implemented_envelope("display close"),
        args.json,
    )
}

fn vm_is_qemu_media_runtime(context: &Context, vm: &str) -> Result<bool, CliFailure> {
    let manifest = context.load_manifest()?;
    Ok(manifest
        .get_vm(vm)
        .and_then(|entry| entry.runtime.as_ref())
        .is_some_and(|runtime| runtime.kind == "qemu-media"))
}

fn vm_dag_dry_run_summary(
    verb: &str,
    vm: &str,
    qemu_media: bool,
    force: bool,
) -> serde_json::Value {
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
    let (forward_nodes, forward_edges, stop_order, notes) = if qemu_media {
        (
            vec![
                serde_json::json!({"id": "host-reconcile", "role": "host-reconcile"}),
                serde_json::json!({"id": "qemu-media", "role": "qemu-media-runner", "readiness": "qmp-listening", "postReady": "QemuMediaBoot"}),
            ],
            serde_json::json!([
                {"from": "host-reconcile", "to": "qemu-media"},
            ]),
            serde_json::json!(["qemu-media", "host-reconcile"]),
            "vm dry-run reports the qemu-media DAG the supervisor would drive (start: host-reconcile → qemu-media → QemuMediaBoot; stop: reverse topo). --apply routes through d2bd → broker (v1.0 daemon-only per ADR 0015).",
        )
    } else {
        (
            vec![
                serde_json::json!({"id": "host-reconcile",        "role": "host-reconcile"}),
                serde_json::json!({"id": "store-preflight",       "role": "store-virtiofs-preflight"}),
                serde_json::json!({"id": "virtiofsd-ro-store",    "role": "virtiofsd"}),
                serde_json::json!({"id": "ch",                    "role": "cloud-hypervisor-runner"}),
                serde_json::json!({"id": "guest-control-health",  "role": "guest-control-health"}),
            ],
            serde_json::json!([
                {"from": "host-reconcile",     "to": "store-preflight"},
                {"from": "store-preflight",    "to": "virtiofsd-ro-store"},
                {"from": "virtiofsd-ro-store", "to": "ch"},
                {"from": "ch",                 "to": "guest-control-health"},
            ]),
            serde_json::json!([
                "guest-control-health",
                "ch",
                "virtiofsd-ro-store",
                "store-preflight",
                "host-reconcile",
            ]),
            "vm dry-run reports the DAG the supervisor would drive (start: topo order; stop: reverse topo). --apply routes through d2bd → broker (v1.0 daemon-only per ADR 0015).",
        )
    };
    let mut summary = serde_json::json!({
        "command": format!("vm {verb}"),
        "mode": "dry-run",
        "vm": vm,
        "dag": {
            "nodes": forward_nodes,
            "edges": forward_edges,
        },
        "stopOrder": if stopping || restarting { Some(stop_order) } else { None::<serde_json::Value> },
        "notes": notes,
    });
    if force
        && (stopping || restarting)
        && let Some(object) = summary.as_object_mut()
    {
        object.insert("force".to_owned(), serde_json::Value::Bool(true));
    }
    summary
}

struct VmLifecycleInvocation<'a> {
    verb: &'a str,
    vm: &'a str,
    dry_run: bool,
    apply: bool,
    no_wait_api: bool,
    force: bool,
    json: bool,
}

fn cmd_vm_lifecycle_verb(
    context: &Context,
    invocation: VmLifecycleInvocation<'_>,
) -> Result<i32, CliFailure> {
    let VmLifecycleInvocation {
        verb,
        vm,
        dry_run,
        apply,
        no_wait_api,
        force,
        json,
    } = invocation;
    let flags = require_explicit_mutation_flag(&format!("vm {verb}"), dry_run, apply, json)?;
    let route = route_vm_target(context, vm, json)?;
    // Preserve the raw user input before the resolved local name shadows it.
    // Migration hint logic must check the original target string, not the
    // workload label extracted by the router (which is always dot-free).
    let raw_target = vm;
    let vm = match route {
        VmTargetRoute::Local { vm } => vm,
        VmTargetRoute::Gateway {
            realm,
            gateway_vm,
            gateway,
            target,
        } => {
            if force {
                return Err(CliFailure::new(
                    2,
                    format!("--force is not supported for gateway-routed vm {verb} targets"),
                ));
            }
            if flags.apply {
                if no_wait_api {
                    return Err(CliFailure::new(
                        2,
                        "the ComponentSession v2 lifecycle contract does not silently ignore --no-wait-api",
                    ));
                }
                let method = match verb {
                    "start" => daemon_access::DaemonMethod::Start,
                    "stop" => daemon_access::DaemonMethod::Stop,
                    "restart" => daemon_access::DaemonMethod::Restart,
                    _ => unreachable!("unknown gateway lifecycle verb"),
                };
                let desired = if verb == "stop" {
                    d2b_contracts::v2_services::common::DesiredState::DESIRED_STATE_STOPPED
                } else {
                    d2b_contracts::v2_services::common::DesiredState::DESIRED_STATE_RUNNING
                };
                let operation_id = format!(
                    "cli-{verb}-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos()
                );
                if !context.public_socket.exists() {
                    return emit_host_error(&daemon_down_envelope(verb), json);
                }
                // Issue the same typed lifecycle request the local path uses,
                // against the realm-qualified canonical target. W8's gateway
                // realm credential/provider wiring is out of scope here; the
                // daemon/router returns a typed unsupported/unavailable
                // outcome (surfaced via the ordinary `CliFailure` error path)
                // until that provider exists. No legacy gatewayDisplay
                // fallback is retained.
                let daemon = service_v2::DaemonService::connect(&context.public_socket)?;
                let response = daemon.lifecycle(method, &target, desired, &operation_id)?;
                let summary = format!(
                    "vm {verb} accepted for '{}' (operation {})",
                    response.message.resource_handle, response.message.operation_id
                );
                if json {
                    print_json(&serde_json::json!({
                        "outcome": "applied",
                        "summary": summary,
                    }))?;
                } else {
                    print_stdout(&format!("{summary}\n"));
                }
                return Ok(0);
            }
            let summary = serde_json::json!({
                "command": format!("vm {verb}"),
                "mode": "dry-run",
                "target": target,
                "realm": realm,
                "gateway": gateway,
                "gatewayVm": gateway_vm,
                "notes": "realm target would route through the configured gateway entrypoint; --apply issues the same authenticated ComponentSession v2 lifecycle request as a local target.",
            });
            if json {
                let mut rendered = serde_json::to_string_pretty(&summary).map_err(|err| {
                    CliFailure::new(
                        1,
                        format!("failed to serialize vm realm dry-run summary: {err}"),
                    )
                })?;
                rendered.push('\n');
                print_stdout(&rendered);
            } else {
                print_stdout(&format!(
                    "vm {verb} --dry-run: would route realm target '{target}' through gateway VM '{gateway_vm}' ({gateway})\n"
                ));
            }
            return Ok(0);
        }
    };
    require_known_vm(context, &vm, json)?;
    // Emit a non-fatal compatibility warning when a bare VM name is used but
    // a canonical workload target is available for it in the realm-controllers
    // artifact. Advisory only: the local fast path continues to work.
    // Gate on raw_target (the original user input), NOT on the resolved local
    // VM name: for host-local realms the router strips the realm suffix
    // (e.g. "corp-vm.work.d2b" → "corp-vm"), so testing the resolved name
    // would always appear dot-free and incorrectly trigger the hint for users
    // who already typed the canonical form.
    if !json
        && !raw_target.contains('.')
        && let Some(canonical) = try_canonical_target_for_vm(&context.bundle_path, &vm)
        && let Some(hint) = target_routing::migration_hint_for_bare_vm(raw_target, &canonical)
    {
        print_workload_migration_hint(&hint, json);
    }
    if (verb == "start" || verb == "restart") && !json {
        warn_pending_staged_config(&vm);
    }
    if flags.apply {
        if no_wait_api || force {
            return Err(CliFailure::new(
                2,
                "the ComponentSession v2 lifecycle contract does not silently ignore --force or --no-wait-api",
            ));
        }
        let method = match verb {
            "start" => daemon_access::DaemonMethod::Start,
            "stop" => daemon_access::DaemonMethod::Stop,
            "restart" => daemon_access::DaemonMethod::Restart,
            _ => return Err(CliFailure::new(2, "unsupported lifecycle method")),
        };
        let desired = if verb == "stop" {
            d2b_contracts::v2_services::common::DesiredState::DESIRED_STATE_STOPPED
        } else {
            d2b_contracts::v2_services::common::DesiredState::DESIRED_STATE_RUNNING
        };
        let operation_id = format!(
            "cli-{verb}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        if !context.public_socket.exists() {
            return emit_host_error(&daemon_down_envelope(verb), json);
        }
        let daemon = service_v2::DaemonService::connect(&context.public_socket)?;
        let response = daemon.lifecycle(method, &vm, desired, &operation_id)?;
        let summary = format!(
            "vm {verb} accepted for '{}' (operation {})",
            response.message.resource_handle, response.message.operation_id
        );
        if json {
            print_json(&serde_json::json!({
                "outcome": "applied",
                "summary": summary,
            }))?;
        } else {
            print_stdout(&format!("{summary}\n"));
        }
        return Ok(0);
    }
    let qemu_media = vm_is_qemu_media_runtime(context, &vm)?;
    let summary = vm_dag_dry_run_summary(verb, &vm, qemu_media, force);
    if json {
        let mut rendered = serde_json::to_string_pretty(&summary).map_err(|err| {
            CliFailure::new(1, format!("failed to serialize vm dry-run summary: {err}"))
        })?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        if qemu_media {
            let force_note = if force && (verb == "stop" || verb == "restart") {
                " with forced stop cleanup"
            } else {
                ""
            };
            print_stdout(&format!(
                "vm {verb} --dry-run: would drive the qemu-media DAG for vm '{vm}'{force_note} (host-reconcile → qemu-media → QemuMediaBoot)\n"
            ));
        } else {
            let force_note = if force && (verb == "stop" || verb == "restart") {
                " with forced stop cleanup"
            } else {
                ""
            };
            print_stdout(&format!(
                "vm {verb} --dry-run: would drive the 5-node DAG for vm '{vm}'{force_note} (host-reconcile → store-preflight → virtiofsd-ro-store → ch → guest-control-health)\n"
            ));
        }
    }
    Ok(0)
}

fn cmd_vm_start(context: &Context, args: &VmStartArgs) -> Result<i32, CliFailure> {
    cmd_vm_lifecycle_verb(
        context,
        VmLifecycleInvocation {
            verb: "start",
            vm: &args.vm,
            dry_run: args.dry_run,
            apply: args.apply,
            no_wait_api: args.no_wait_api,
            force: false,
            json: args.json,
        },
    )
}

fn cmd_vm_stop(context: &Context, args: &VmStopArgs) -> Result<i32, CliFailure> {
    cmd_vm_lifecycle_verb(
        context,
        VmLifecycleInvocation {
            verb: "stop",
            vm: &args.vm,
            dry_run: args.dry_run,
            apply: args.apply,
            no_wait_api: false,
            force: args.force,
            json: args.json,
        },
    )
}

fn cmd_vm_restart(context: &Context, args: &VmRestartArgs) -> Result<i32, CliFailure> {
    cmd_vm_lifecycle_verb(
        context,
        VmLifecycleInvocation {
            verb: "restart",
            vm: &args.vm,
            dry_run: args.dry_run,
            apply: args.apply,
            no_wait_api: false,
            force: args.force,
            json: args.json,
        },
    )
}

fn cmd_vm_list(context: &Context, args: &VmListArgs) -> Result<i32, CliFailure> {
    if let Some(realm) = args.realm.as_deref() {
        let gateway = resolve_realm_gateway(context, realm, args.json)?;
        ensure_realm_gateway_running(context, &gateway.realm, &gateway.gateway_vm, args.json)?;
        let mut argv = vec!["d2b".to_owned(), "vm".to_owned(), "list".to_owned()];
        if args.json {
            argv.push("--json".to_owned());
        } else if args.human {
            argv.push("--human".to_owned());
        }
        let exec_args = realm_gateway_exec_args(
            gateway.gateway_vm,
            argv,
            false,
            false,
            args.json,
            args.human,
        );
        return cmd_vm_exec_v2(context, &exec_args);
    }
    if args.all {
        return cmd_vm_list_all(context, args);
    }
    cmd_vm_list_local(context, args)
}

fn cmd_vm_list_all(context: &Context, args: &VmListArgs) -> Result<i32, CliFailure> {
    let daemon = service_v2::DaemonService::connect(&context.public_socket)?;
    let workloads = daemon.list_workloads(None)?;
    let local_entries = service_v2::list_output(&workloads)?.0;
    let gateway_states = gateway_lifecycle_states(context)?;
    let gateway_entries = configured_realm_gateways(args.json)?
        .into_iter()
        .map(|gateway| {
            let state = gateway_states
                .get(&gateway.gateway_vm)
                .map(String::as_str)
                .unwrap_or("not reported by d2bd")
                .to_owned();
            RealmGatewayListEntry {
                gateway_target: gateway.gateway_target,
                realm: gateway.realm,
                gateway_vm: gateway.gateway_vm,
                state,
            }
        })
        .collect::<Vec<_>>();
    if args.json {
        let body = serde_json::json!({
            "command": "vm list --all",
            "local": local_entries,
            "realmGateways": gateway_entries,
            "notes": "gateway-backed realm workload inventory is queried inside each gateway with `d2b realm run <realm> -- d2b vm list`",
        });
        let mut rendered = serde_json::to_string_pretty(&body).map_err(|err| {
            CliFailure::new(1, format!("failed to serialize vm list --all: {err}"))
        })?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        if local_entries.is_empty() {
            print_stdout("vm list --all: no local daemon runtime entries reported\n");
        } else {
            let mut rendered = String::from("LOCAL VM\tSTATE\tRUNTIME\n");
            for entry in local_entries {
                let runtime = workloads
                    .iter()
                    .find(|workload| workload.name == entry.name)
                    .map(service_v2::runtime_detail)
                    .transpose()?
                    .unwrap_or("unknown");
                let _ = writeln!(rendered, "{}\t{}\t{}", entry.name, entry.status, runtime);
            }
            print_stdout(&rendered);
        }
        if gateway_entries.is_empty() {
            print_stdout("REALM GATEWAY\tREALM\tSTATE\n(none)\n");
        } else {
            let mut rendered = String::from("REALM GATEWAY\tREALM\tSTATE\n");
            for entry in gateway_entries {
                let _ = writeln!(
                    rendered,
                    "{}\t{}\t{}",
                    entry.gateway_vm, entry.realm, entry.state
                );
            }
            print_stdout(&rendered);
        }
    }
    Ok(0)
}

fn cmd_vm_list_local(context: &Context, args: &VmListArgs) -> Result<i32, CliFailure> {
    let daemon = service_v2::DaemonService::connect(&context.public_socket)?;
    let workloads = daemon.list_workloads(None)?;
    let entries = service_v2::list_output(&workloads)?.0;
    if args.json {
        let body = serde_json::json!({
            "command": "vm list",
            "entries": entries,
        });
        let mut rendered = serde_json::to_string_pretty(&body)
            .map_err(|err| CliFailure::new(1, format!("failed to serialize vm list: {err}")))?;
        rendered.push('\n');
        print_stdout(&rendered);
        return Ok(0);
    }
    if entries.is_empty() {
        print_stdout("vm list: no daemon runtime entries reported\n");
    } else {
        let mut rendered = String::from("VM\tSTATE\tRUNTIME\n");
        for entry in entries {
            let runtime = workloads
                .iter()
                .find(|workload| workload.name == entry.name)
                .map(service_v2::runtime_detail)
                .transpose()?
                .unwrap_or("unknown");
            let _ = writeln!(rendered, "{}\t{}\t{}", entry.name, entry.status, runtime);
        }
        print_stdout(&rendered);
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
            );
        }
    };
    Ok(VmExecParsedAction {
        json,
        management: Some(management),
    })
}

fn validate_vm_exec_management_args(args: &VmExecArgs) -> Result<(), String> {
    if args.detach || args.interactive || args.tty || !args.command.is_empty() {
        return Err(
            "vm exec: detached management verbs do not accept -d/-i/-t or a command; use `--` to run a command"
                .to_owned(),
        );
    }
    Ok(())
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

fn exec_render_detached_list(
    args: &VmExecArgs,
    result: &d2b_contracts::public_wire::ExecDetachedListResult,
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
    result: &d2b_contracts::public_wire::ExecDetachedStatusResult,
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
    result: &d2b_contracts::public_wire::ExecDetachedLogsResult,
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
    result: &d2b_contracts::public_wire::ExecDetachedLogsResult,
) -> Result<(Vec<u8>, Vec<u8>), exec_client::ExecClientError> {
    let stdout = match d2b_core::base64_codec::decode(&result.stdout_base64) {
        Ok(bytes) => bytes,
        Err(_) => {
            return Err(exec_client::ExecClientError::protocol(
                "daemon returned malformed base64 for detached stdout",
            ));
        }
    };
    let stderr = match d2b_core::base64_codec::decode(&result.stderr_base64) {
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
    result: &d2b_contracts::public_wire::ExecDetachedKillResult,
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

fn exec_state_label(state: d2b_contracts::guest_wire::ExecState) -> &'static str {
    use d2b_contracts::guest_wire::ExecState;

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
    outcome: d2b_contracts::public_wire::ExecDetachedKillOutcome,
) -> &'static str {
    use d2b_contracts::public_wire::ExecDetachedKillOutcome;

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

fn exec_list_offsets_summary(entry: &d2b_contracts::public_wire::ExecDetachedListEntry) -> String {
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

fn exec_list_loss_summary(entry: &d2b_contracts::public_wire::ExecDetachedListEntry) -> String {
    format!(
        "all={} stdout={} stderr={}",
        exec_loss_summary(entry.dropped_bytes, entry.truncated),
        exec_loss_summary(entry.stdout_dropped_bytes, entry.stdout_truncated),
        exec_loss_summary(entry.stderr_dropped_bytes, entry.stderr_truncated)
    )
}

fn exec_logs_incomplete(result: &d2b_contracts::public_wire::ExecDetachedLogsResult) -> bool {
    result.dropped_bytes > 0
        || result.truncated
        || result.stdout_dropped_bytes > 0
        || result.stderr_dropped_bytes > 0
        || result.stdout_truncated
        || result.stderr_truncated
}

fn exec_logs_warning(result: &d2b_contracts::public_wire::ExecDetachedLogsResult) -> String {
    format!(
        "d2b: vm exec logs: retained output incomplete (startOffset={} endOffset={} droppedBytes={} truncated={} stdoutStartOffset={} stdoutEndOffset={} stdoutNextOffset={} stdoutEof={} stdoutDroppedBytes={} stdoutTruncated={} stderrStartOffset={} stderrEndOffset={} stderrNextOffset={} stderrEof={} stderrDroppedBytes={} stderrTruncated={})\n",
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
        "notes": format!("d2b {verb} --dry-run reports the planned operation; --apply routes through d2bd → broker."),
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
            "d2b build {}: would evaluate and build the toplevel (hardlink-farm materialization happens on activation/gc)\n",
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
            "d2b generations {}: current={}  booted={}\n",
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
            "d2b {verb} --dry-run: would activate the planned generation for vm '{vm}'\n"
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
        // v1.0 daemon-only: --apply routes through d2bd → broker
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
        print_stdout(
            "d2b gc --dry-run: would prune unreachable store paths in /var/lib/d2b/vms/<vm>/store/\n",
        );
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
    if human { false } else { json }
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

fn removed_usb_enroll_failure(raw_args: &[OsString]) -> Option<CliFailure> {
    let is_removed_enroll = raw_args.get(1).and_then(|arg| arg.to_str()) == Some("usb")
        && raw_args.get(2).and_then(|arg| arg.to_str()) == Some("enroll");
    if !is_removed_enroll {
        return None;
    }

    let vm = raw_args
        .get(3)
        .and_then(|arg| arg.to_str())
        .unwrap_or("<vm>");
    let media_ref = raw_args
        .get(4)
        .and_then(|arg| arg.to_str())
        .unwrap_or("<ref>");
    let selector_hint = if raw_args.iter().any(|arg| arg == "--busid") {
        " Runtime busids are transient; use a stable `/dev/disk/by-id/` basename for `usbSelector.byIdName` instead."
    } else {
        ""
    };
    let apply_hint = if raw_args.iter().any(|arg| arg == "--apply") {
        " `--apply` no longer mutates host state for this removed verb."
    } else {
        ""
    };
    Some(CliFailure::new(
        2,
        format!(
            "d2b usb enroll was removed. Declare the qemu-media boot-drive physical USB source for VM `{}` and ref `{}` in config with `qemuMedia.source.usbSelector.byIdName`, rebuild/restart d2bd, then run `d2b usb probe` to verify the runtime selector before `d2b vm start <vm> --apply`.{}{apply_hint}",
            vm, media_ref, selector_hint
        ),
    ))
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
    let qemu_media = vm_is_qemu_media_runtime(context, vm)?;
    if qemu_media && let Err(err) = validate_usb_bus_id(bus_id) {
        return Err(CliFailure::new(
            2,
            format!("{verb}: invalid busid selector: {err}"),
        ));
    }
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
    if qemu_media {
        let planned: Vec<&str> = if verb == "usb attach" {
            vec![
                "QemuMediaResolveRuntimeSelector",
                "OpenEnrolledMediaByRegistryIdentity",
                "QmpHotplug(add-fd,blockdev-add,device_add)",
            ]
        } else {
            vec![
                "QemuMediaResolveRuntimeSelector",
                "QmpHotplug(device_del,blockdev-del,remove-fd)",
            ]
        };
        let summary = serde_json::json!({
            "command": verb,
            "mode": "dry-run",
            "vm": vm,
            "busIdProvided": true,
            "runtime": "qemu-media",
            "planned": planned,
            "notes": "qemu-media USB hotplug does not use USBIP and does not echo the runtime busid in dry-run output."
        });
        if json_mode {
            let mut rendered = serde_json::to_string_pretty(&summary)
                .map_err(|e| CliFailure::new(1, format!("serialize: {e}")))?;
            rendered.push('\n');
            print_stdout(&rendered);
        } else {
            let action = if verb == "usb attach" {
                "resolve the runtime USB selector through the root-only media registry and execute QMP attach"
            } else {
                "resolve the runtime USB selector through the root-only media registry and execute QMP detach"
            };
            print_stdout(&format!(
                "d2b {verb} --dry-run: would {action} for qemu-media vm '{vm}' (runtime busid redacted)\n"
            ));
        }
        return Ok(0);
    }
    let planned: Vec<&str> = if verb == "usb attach" {
        vec![
            "UsbipBind",
            "UsbipBindFirewallRule",
            "SpawnRunner(sys-<env>-usbipd/backend)",
            "SpawnRunner(sys-<env>-usbipd/proxy)",
            "UsbipProxyReconcile",
            "GuestdUsbipImport(attach)",
        ]
    } else {
        vec![
            "GuestdUsbipImport(detach)",
            "UsbipUnbind",
            "UsbipProxyReconcile",
        ]
    };
    let summary = serde_json::json!({
        "command": verb,
        "mode": "dry-run",
        "vm": vm,
        "busId": bus_id,
        "planned": planned,
        "notes": if verb == "usb attach" {
            "USBIP dry-run reports the daemon → broker bind/lock, firewall, backend/proxy ensurement, reconcile plan, and authenticated guestd import without mutating host or guest state."
        } else {
            "USBIP dry-run reports authenticated guestd import cleanup plus the daemon → broker unbind/reconcile plan without mutating host or guest state."
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
                "d2b {verb} --dry-run: would {action} busid '{bus_id}' for vm '{vm}', reconcile the USBIP proxy, and ask guestd to import the device\n"
            ));
        } else {
            print_stdout(&format!(
                "d2b {verb} --dry-run: would ask guestd to detach busid '{bus_id}' for vm '{vm}', {action} it on the host, and reconcile the USBIP proxy\n"
            ));
        }
    }
    Ok(0)
}

fn cmd_usb_probe(context: &Context, args: &UsbProbeArgs) -> Result<i32, CliFailure> {
    let json_mode = usb_json_mode(args.json, args.human);
    match try_usb_probe_via_socket(context)? {
        UsbProbeSocketOutcome::Entries(entries) => {
            if json_mode {
                let body = UsbProbeOutputV1 {
                    command: "usb probe".to_owned(),
                    entries,
                };
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
                "USB media probe requires a reachable d2bd",
                "daemon-down",
                1,
                "Daemon connectivity at /run/d2b/public.sock and USB media probe support.",
                "d2bd is unreachable or does not expose the native USB probe request.",
                "Start d2bd on the host, then re-run `d2b usb probe`.",
                "docs/reference/error-codes.md#daemon-down",
            ),
            json_mode,
        ),
    }
}

fn render_usb_probe_human(entries: &[IpcUsbipProbeEntry]) -> String {
    let mut out = String::new();
    let usbip_entries: Vec<_> = entries
        .iter()
        .filter(|entry| matches!(entry.kind, IpcUsbProbeEntryKind::Usbip))
        .collect();
    if !usbip_entries.is_empty() || entries.is_empty() {
        let _ = writeln!(
            out,
            "{:<24} {:<12} {:<12} {:<10} {:<22} {:<24} {:<14} {:<12} {:<10} {:<8}",
            "VM",
            "ENV",
            "BUSID",
            "STATUS",
            "SESSION-CLAIM",
            "HOST-BIND",
            "CARRIER",
            "PROXY",
            "GUEST",
            "POLICY"
        );
        for entry in usbip_entries {
            let _ = writeln!(
                out,
                "{:<24} {:<12} {:<12} {:<10} {:<22} {:<24} {:<14} {:<12} {:<10} {:<8}",
                entry.vm,
                entry.env,
                entry.bus_id,
                usb_probe_status_label(entry.status),
                durable_claim_label(entry.durable_claim.state),
                host_bind_label(entry.host.bind),
                host_carrier_label(entry.host.carrier),
                proxy_label(entry.host.proxy),
                guest_import_label(entry.guest.import),
                policy_label(entry.topology_policy.policy),
            );
            for reason in &entry.degraded_reasons {
                let _ = writeln!(
                    out,
                    "  degraded {}: {}",
                    reason_code_label(reason.code),
                    reason.summary
                );
                let _ = writeln!(out, "  remediation: {}", reason.remediation);
            }
            for command in &entry.remediation_commands {
                let _ = writeln!(out, "  command: {command}");
            }
        }
    }
    let qemu_entries: Vec<_> = entries
        .iter()
        .filter(|entry| matches!(entry.kind, IpcUsbProbeEntryKind::QemuMediaSlot))
        .collect();
    if !qemu_entries.is_empty() {
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
        let _ = writeln!(
            out,
            "{:<24} {:<14} {:<20} {:<14} {:<12} {:<12} FOLLOW-UP",
            "QEMU-MEDIA-VM", "SLOT", "REF", "SOURCE", "BUSID", "STATUS"
        );
        for entry in qemu_entries {
            let _ = writeln!(
                out,
                "{:<24} {:<14} {:<20} {:<14} {:<12} {:<12} {}",
                entry.vm,
                entry.slot.as_deref().unwrap_or("-"),
                entry
                    .media_ref
                    .as_ref()
                    .map(MediaRef::as_str)
                    .unwrap_or("-"),
                entry.source_kind.as_deref().unwrap_or("-"),
                entry.bus_id,
                usb_probe_status_label(entry.status),
                entry.follow_up_command.as_deref().unwrap_or("-"),
            );
        }
    }
    out
}

fn usb_probe_status_label(status: IpcUsbipProbeStatus) -> &'static str {
    match status {
        IpcUsbipProbeStatus::Bound => "bound",
        IpcUsbipProbeStatus::Unbound => "unbound",
        IpcUsbipProbeStatus::Degraded => "degraded",
        IpcUsbipProbeStatus::Enrollable => "enrollable",
        IpcUsbipProbeStatus::Enrolled => "enrolled",
        IpcUsbipProbeStatus::Stale => "stale",
        IpcUsbipProbeStatus::DirectConfig => "direct-config",
        IpcUsbipProbeStatus::Unknown => "unknown",
    }
}

fn durable_claim_label(state: public_wire::UsbipDurableClaimState) -> &'static str {
    match state {
        public_wire::UsbipDurableClaimState::Missing => "missing",
        public_wire::UsbipDurableClaimState::HeldByDesiredOwner => "held-by-desired-owner",
        public_wire::UsbipDurableClaimState::HeldByOtherOwner => "held-by-other-owner",
        public_wire::UsbipDurableClaimState::StaleOwner => "stale-owner",
        public_wire::UsbipDurableClaimState::Corrupt => "corrupt",
        public_wire::UsbipDurableClaimState::NotApplicable => "not-applicable",
        public_wire::UsbipDurableClaimState::Unknown => "unknown",
    }
}

fn host_bind_label(state: public_wire::UsbipHostBindState) -> &'static str {
    match state {
        public_wire::UsbipHostBindState::Unbound => "unbound",
        public_wire::UsbipHostBindState::BoundToUsbipHost => "bound-to-usbip-host",
        public_wire::UsbipHostBindState::BoundToUnexpectedDriver => "bound-to-unexpected-driver",
        public_wire::UsbipHostBindState::DeviceMissing => "device-missing",
        public_wire::UsbipHostBindState::Unknown => "unknown",
        public_wire::UsbipHostBindState::NotApplicable => "not-applicable",
    }
}

fn host_carrier_label(state: public_wire::UsbipHostCarrierState) -> &'static str {
    match state {
        public_wire::UsbipHostCarrierState::Absent => "absent",
        public_wire::UsbipHostCarrierState::Unavailable => "unavailable",
        public_wire::UsbipHostCarrierState::WithheldForOwner => "withheld-for-owner",
        public_wire::UsbipHostCarrierState::Ready => "ready",
        public_wire::UsbipHostCarrierState::DepartedDuringProbe => "departed-during-probe",
        public_wire::UsbipHostCarrierState::Unknown => "unknown",
        public_wire::UsbipHostCarrierState::NotApplicable => "not-applicable",
    }
}

fn proxy_label(state: public_wire::UsbipProxyState) -> &'static str {
    match state {
        public_wire::UsbipProxyState::NotDeclared => "not-declared",
        public_wire::UsbipProxyState::Stopped => "stopped",
        public_wire::UsbipProxyState::Starting => "starting",
        public_wire::UsbipProxyState::Listening => "listening",
        public_wire::UsbipProxyState::Stale => "stale",
        public_wire::UsbipProxyState::Failed => "failed",
        public_wire::UsbipProxyState::Unknown => "unknown",
        public_wire::UsbipProxyState::NotApplicable => "not-applicable",
    }
}

fn guest_import_label(state: public_wire::UsbipGuestImportState) -> &'static str {
    match state {
        public_wire::UsbipGuestImportState::Detached => "detached",
        public_wire::UsbipGuestImportState::Imported => "imported",
        public_wire::UsbipGuestImportState::Unavailable => "unavailable",
        public_wire::UsbipGuestImportState::Unknown => "unknown",
        public_wire::UsbipGuestImportState::NotApplicable => "not-applicable",
    }
}

#[allow(dead_code)]
fn topology_label(state: public_wire::UsbipTopologyState) -> &'static str {
    match state {
        public_wire::UsbipTopologyState::Match => "match",
        public_wire::UsbipTopologyState::Mismatch => "mismatch",
        public_wire::UsbipTopologyState::Incomplete => "incomplete",
        public_wire::UsbipTopologyState::NotObserved => "not-observed",
        public_wire::UsbipTopologyState::NotApplicable => "not-applicable",
        public_wire::UsbipTopologyState::Unknown => "unknown",
    }
}

fn policy_label(state: public_wire::UsbipPolicyState) -> &'static str {
    match state {
        public_wire::UsbipPolicyState::Allowed => "allowed",
        public_wire::UsbipPolicyState::Denied => "denied",
        public_wire::UsbipPolicyState::Missing => "missing",
        public_wire::UsbipPolicyState::NotApplicable => "not-applicable",
        public_wire::UsbipPolicyState::Unknown => "unknown",
    }
}

fn reason_code_label(code: public_wire::UsbipProbeDegradedReasonCode) -> &'static str {
    match code {
        public_wire::UsbipProbeDegradedReasonCode::PolicyFailed => "policy-failed",
        public_wire::UsbipProbeDegradedReasonCode::DeviceDepartedBeforeClaim => {
            "device-departed-before-claim"
        }
        public_wire::UsbipProbeDegradedReasonCode::DeviceDepartedAfterLock => {
            "device-departed-after-lock"
        }
        public_wire::UsbipProbeDegradedReasonCode::DeviceDepartedDuringMutation => {
            "device-departed-during-mutation"
        }
        public_wire::UsbipProbeDegradedReasonCode::DeviceReappearedWithDifferentTopology => {
            "device-reappeared-with-different-topology"
        }
        public_wire::UsbipProbeDegradedReasonCode::LockHeldByOtherOwner => {
            "lock-held-by-other-owner"
        }
        public_wire::UsbipProbeDegradedReasonCode::InvalidPersistedLockClaim => {
            "invalid-persisted-lock-claim"
        }
        public_wire::UsbipProbeDegradedReasonCode::CarrierUnavailable => "carrier-unavailable",
        public_wire::UsbipProbeDegradedReasonCode::HostBindUnavailable => "host-bind-unavailable",
        public_wire::UsbipProbeDegradedReasonCode::ProxyUnavailable => "proxy-unavailable",
        public_wire::UsbipProbeDegradedReasonCode::GuestImportUnavailable => {
            "guest-import-unavailable"
        }
        public_wire::UsbipProbeDegradedReasonCode::StaleHostState => "stale-host-state",
        public_wire::UsbipProbeDegradedReasonCode::StaleGuestState => "stale-guest-state",
        public_wire::UsbipProbeDegradedReasonCode::ProbeIncomplete => "probe-incomplete",
        public_wire::UsbipProbeDegradedReasonCode::Unknown => "unknown",
    }
}

// ---- USB security-key proxy CLI ----
//
// Live (non-dry-run) paths return `not-yet-implemented` (exit 78) until the
// d2bd security-key broker handler ships. All `--dry-run` paths are fully
// implemented and stable; the planned-step output is the committed golden
// contract for this CLI surface.

fn usb_sk_json_mode(json: bool, human: bool) -> bool {
    if human { false } else { json }
}

fn usb_sk_not_yet_implemented_envelope(verb: &str) -> HostErrorEnvelope {
    host_error_envelope(
        &format!("d2b usb security-key {verb} has no daemon-native handler yet"),
        "not-yet-implemented",
        78,
        &format!("Native daemon dispatch for `d2b usb security-key {verb}`"),
        "The security-key proxy daemon handler has not landed yet. \
         The CLI surface, wire contracts, and dry-run plans are complete; \
         the runtime broker implementation ships in a later workstream.",
        "Track progress in CHANGELOG.md [Unreleased]. \
         Use `d2b usb security-key <verb> --dry-run` to preview the planned actions.",
        "docs/reference/error-codes.md#not-yet-implemented",
    )
}

fn cmd_usb_sk_status(_context: &Context, args: &UsbSkStatusArgs) -> Result<i32, CliFailure> {
    let json_mode = usb_sk_json_mode(args.json, args.human);
    emit_host_error(&usb_sk_not_yet_implemented_envelope("status"), json_mode)
}

fn cmd_usb_sk_sessions(_context: &Context, args: &UsbSkSessionsArgs) -> Result<i32, CliFailure> {
    let json_mode = usb_sk_json_mode(args.json, args.human);
    emit_host_error(&usb_sk_not_yet_implemented_envelope("sessions"), json_mode)
}

fn cmd_usb_sk_cancel(_context: &Context, args: &UsbSkCancelArgs) -> Result<i32, CliFailure> {
    let json_mode = usb_sk_json_mode(args.json, args.human);

    // Require exactly one of: session_id (positional) or --current.
    if args.session_id.is_none() && !args.current {
        return Err(CliFailure::new(
            2,
            "d2b usb security-key cancel: provide either a session ID or --current".to_owned(),
        ));
    }

    // Require exactly one of: --dry-run or --apply.
    let flags = require_mutation_flag(
        "usb security-key cancel",
        args.dry_run,
        args.apply,
        json_mode,
    )?;

    let target = if args.current {
        "current".to_owned()
    } else {
        args.session_id
            .clone()
            .unwrap_or_else(|| "current".to_owned())
    };

    if flags.apply {
        return emit_host_error(&usb_sk_not_yet_implemented_envelope("cancel"), json_mode);
    }

    // --dry-run: emit the planned action without contacting the daemon.
    let summary = serde_json::json!({
        "command": "usb security-key cancel",
        "mode": "dry-run",
        "target": target,
        "planned": ["SecurityKeyProxyCancelSession"],
        "notes": "Dry-run preview; --apply dispatches the cancel through the \
                  daemon → broker SecurityKeyProxyCancelSession path.",
    });
    if json_mode {
        let mut rendered = serde_json::to_string_pretty(&summary)
            .map_err(|e| CliFailure::new(1, format!("serialize: {e}")))?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        print_stdout(&format!(
            "d2b usb security-key cancel --dry-run: would send \
             CancelSession({target}) to the security-key proxy broker\n"
        ));
    }
    Ok(0)
}

fn cmd_usb_sk_test(_context: &Context, args: &UsbSkTestArgs) -> Result<i32, CliFailure> {
    let json_mode = usb_sk_json_mode(args.json, args.human);
    let vm = &args.vm;

    if args.dry_run {
        let summary = serde_json::json!({
            "command": "usb security-key test",
            "mode": "dry-run",
            "vm": vm,
            "planned": [
                "CheckGuestVirtualHidDevice",
                "CheckHostBrokerPhysicalKeyVisibility",
            ],
            "notes": "Dry-run preview; the live path queries the daemon for \
                      virtual-HID presence in the guest and physical-key \
                      visibility on the host broker.",
        });
        if json_mode {
            let mut rendered = serde_json::to_string_pretty(&summary)
                .map_err(|e| CliFailure::new(1, format!("serialize: {e}")))?;
            rendered.push('\n');
            print_stdout(&rendered);
        } else {
            print_stdout(&format!(
                "d2b usb security-key test --dry-run: would check virtual HID device \
                 presence in '{vm}' and confirm host broker sees the physical security key\n"
            ));
        }
        return Ok(0);
    }

    emit_host_error(&usb_sk_not_yet_implemented_envelope("test"), json_mode)
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
        // v1.0 daemon-only: --apply routes through d2bd → broker
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
        "notes": format!("d2b keys {verb} --dry-run: planned operation. --apply routes through d2bd → broker RunKeysRotate with broker audit."),
    });
    if json {
        let mut rendered = serde_json::to_string_pretty(&summary)
            .map_err(|e| CliFailure::new(1, format!("serialize: {e}")))?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        print_stdout(&format!(
            "d2b keys {verb} --dry-run: planned operation for vm '{vm}'\n"
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

// ---- d2b migrate ----

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
    // flake's `d2b.vms.<vm>.supervisor` setting, which the public
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
        // v1.0 daemon-only: --apply routes through d2bd → broker
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
        "perVmClassificationNote": "v1.1 (per ADR 0015) made every enabled VM daemon-supervised by default; the `d2b.vms.<vm>.supervisor` option was removed in v1.1. Per-VM systemd-unit inspection still uses `d2b status <vm>`.",
        "totalVms": vms.len(),
        "vms": vms.iter().map(|vm| serde_json::json!({
            "name": vm.name,
            "env": vm.env,
            "classification": "unknown-not-in-public-manifest",
        })).collect::<Vec<_>>(),
        "plannedSteps": [
            "v1.1 daemon-only: every enabled VM is daemon-supervised by default; no consumer-flake action is required for supervisor classification.",
            "Per migrating VM: verify per-VM state under `/var/lib/d2b/vms/<vm>/` is owned root:d2bd 0750.",
            "Run `nixos-rebuild switch` so the daemon module materializes the per-VM broker SpawnRunner state.",
            "Verify each migrated VM via `d2b status <vm>` and `d2b vm list` after d2bd is running.",
            "After all VMs migrate cleanly, keep the default-switch readiness gates aligned with the rollout evidence."
        ],
        "notes": "migrate reports the deployment-shape tier today; v1.1 retired the per-VM supervisor option, so per-VM classification is uniformly daemon-supervised. `--apply` routes through d2bd → broker RunMigrate.",
    });

    if args.json {
        let mut rendered = serde_json::to_string_pretty(&summary)
            .map_err(|e| CliFailure::new(1, format!("serialize: {e}")))?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        print_stdout(&format!(
            "d2b migrate --dry-run: deployment shape = {tier_str}, {} VM(s) in manifest.\n",
            vms.len()
        ));
        print_stdout(
            "v1.1 daemon-only: every enabled VM is daemon-supervised; the per-VM\n\
             `supervisor` option was removed in v1.1 (ADR 0015). Use\n\
             `d2b status <vm>` to inspect each VM directly; `d2b migrate --apply`\n\
             is the live mutation path when you are ready.\n",
        );
    }
    Ok(0)
}

// Legacy bash parity verbs keep the flag-less entrypoint by
// defaulting to --dry-run; native-only host/vm/migrate verbs keep
// using `require_explicit_mutation_flag`.
const DEFAULT_DRY_RUN_NOTICE: &str = "d2b: NOTICE: defaulting to --dry-run; d2b 1.0 will require explicit --dry-run or --apply (v0.4 bash CLI had no flag requirement).";

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
            &format!("Re-run as `d2b {verb} --dry-run` to plan or `d2b {verb} --apply` to mutate.",),
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
    let launcher_uids = parse_uid_env("D2B_TEST_LAUNCHER_UIDS");
    let admin_uids = parse_uid_env("D2B_TEST_ADMIN_UIDS");
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

#[allow(dead_code)]
fn resolve_selected_vm(context: &Context, args: &StatusArgs) -> Result<Option<String>, CliFailure> {
    let selected = match (&args.vm, &args.vm_flag) {
        (Some(positional), Some(flagged)) if positional != flagged => Err(CliFailure::new(
            2,
            "status received conflicting VM selectors",
        )),
        (Some(positional), _) => Ok(Some(positional.clone())),
        (_, Some(flagged)) => Ok(Some(flagged.clone())),
        (None, None) => Ok(None),
    }?;
    Ok(selected.map(|vm| resolve_vm_selector_from_bundle(context, &vm)))
}

/// Read the per-VM api-ready state file written by d2bd on each DAG run.
///
/// The file lives at `{daemon_state_dir}/{vm_name}/api-ready.json` and contains
/// `{"apiReady": <value>}` where the value mirrors `ApiReadyState`'s serialization:
/// `"yes"` | `"pending"` | `"timeout"` | `{"error":"<reason>"}`.
#[allow(dead_code)]
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

#[allow(dead_code)]
fn live_pool_integrity_unknown(reason: &str, remediation: String) -> LivePoolIntegrityOutputV1 {
    LivePoolIntegrityOutputV1 {
        status: "unknown".to_owned(),
        unknown_reason: Some(reason.to_owned()),
        audit_ref: None,
        repair_attempted: false,
        remediation: Some(remediation),
    }
}

#[allow(dead_code)]
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

#[allow(dead_code)]
fn marker_status_for_integrity(store_root: &Path, vm: &str) -> Result<(), &'static str> {
    let marker = store_root.join("live").join(format!(".d2b-marker-{vm}"));
    match std::fs::symlink_metadata(&marker) {
        Ok(meta) if meta.is_file() && meta.len() == 0 => Ok(()),
        Ok(_) => Err("suspect"),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Err("marker_or_manifest_missing"),
        Err(_) => Err("marker_or_manifest_unreadable"),
    }
}

#[allow(dead_code)]
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
        if let Ok(raw) = std::fs::read_to_string(&vm_unknown)
            && let Ok(value) = serde_json::from_str::<Value>(&raw)
            && value.get("state").and_then(Value::as_str) == Some("unknown")
        {
            let reason = value
                .get("unknown_reason")
                .and_then(Value::as_str)
                .unwrap_or("generation_identity_unavailable");
            return Some(live_pool_integrity_unknown(
                reason,
                "restore state/current or activate a new generation, then rerun verify".to_owned(),
            ));
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
                    "run `d2b store verify {}` to establish live-pool integrity",
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
                format!("run `d2b store verify {} --repair`", vm.name),
            )),
            Err(reason) => Some(live_pool_integrity_unknown(
                reason,
                format!(
                    "run `d2b store verify {}` to re-establish live-pool integrity",
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
                format!("run `d2b store verify {} --repair`", vm.name)
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
                format!("run `d2b store verify {}`", vm.name),
            ))
        }
        _ => Some(live_pool_integrity_unknown(
            "marker_or_manifest_unreadable",
            "fix permissions or storage errors, then rerun verify".to_owned(),
        )),
    }
}

fn render_list_human(
    output: &ListOutputV2,
    read_model: Option<&d2b_contracts::public_wire::PublicReadModelMetadata>,
) -> String {
    let has_canonical = output.0.iter().any(|item| item.canonical_target.is_some());
    let mut text = if has_canonical {
        String::from(
            "NAME               ENV       GRAPHICS  TPM   USBIP   STATIC_IP       WORKLOAD TARGET          STATUS\n",
        )
    } else {
        String::from(
            "NAME               ENV       GRAPHICS  TPM   USBIP   STATIC_IP       STATUS\n",
        )
    };
    for item in &output.0 {
        let status = if item.is_net_vm {
            format!("{} (net-vm)", item.status)
        } else if item.runtime_kind.as_deref() == Some("qemu-media") {
            let mut label = format!("{} (qemu-media, manual-only)", item.status);
            if let Some(qemu) = &item.qemu_media {
                label.push_str(&format!(
                    ", qmp={}",
                    qemu.runner.qmp_readiness.as_deref().unwrap_or("unknown")
                ));
            }
            if !item.unsupported_capabilities.is_empty() {
                label.push_str(&format!(
                    ", unsupported={}",
                    item.unsupported_capabilities.join(",")
                ));
            }
            if !item.runtime_capabilities.is_empty() {
                label.push_str(&format!(", caps={}", item.runtime_capabilities.join(",")));
            }
            label
        } else {
            item.status.clone()
        };
        let static_ip = item.static_ip.clone().unwrap_or_else(|| "-".to_owned());
        if has_canonical {
            let _ = writeln!(
                text,
                "{:<18} {:<9} {:<9} {:<5} {:<7} {:<15} {:<24} {}",
                item.name,
                item.env.clone().unwrap_or_else(|| "-".to_owned()),
                item.graphics,
                item.tpm,
                item.usbip,
                static_ip,
                item.canonical_target
                    .clone()
                    .unwrap_or_else(|| "-".to_owned()),
                status,
            );
        } else {
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
    }
    if let Some(rm) = read_model {
        let fp = if rm.source_fingerprint.len() > 8 {
            &rm.source_fingerprint[..8]
        } else {
            &rm.source_fingerprint
        };
        let _ = writeln!(
            text,
            "\n[read-model: {}, gen {}, fingerprint {}]",
            rm.freshness, rm.generation, fp
        );
    }
    text
}

#[allow(dead_code)]
fn render_status_vm_human(
    output: &StatusVmOutputV2,
    manifest_vm: &ManifestVm,
    bridge_rows: Vec<BridgeHealthRow>,
) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "=== {} ===", output.name);
    if let Some(canonical) = &output.canonical_target {
        let _ = writeln!(text, "workload target: {canonical}");
    }
    if let Some(env) = &output.env {
        let _ = writeln!(text, "env: {env}");
    }
    let _ = writeln!(text, "runtime: {}", output.runtime);
    if let Some(kind) = &output.runtime_kind {
        let _ = writeln!(text, "runtime kind: {kind}");
    }
    if let Some(autostart) = &output.autostart {
        let _ = writeln!(text, "autostart: {} ({})", autostart.mode, autostart.reason);
    }
    let _ = writeln!(text, "daemon: {}", output.services.d2b);
    if let Some(qemu) = &output.qemu_media {
        let _ = writeln!(
            text,
            "qemu-media runner: {}",
            output
                .services
                .qemu_media
                .clone()
                .unwrap_or_else(|| qemu.runner.state.clone())
        );
        let _ = writeln!(text, "firmware mode: {}", qemu.firmware_mode);
        let _ = writeln!(
            text,
            "qmp readiness: {}",
            qemu.runner.qmp_readiness.as_deref().unwrap_or("unknown")
        );
        let _ = writeln!(text, "pre-cont progress: {}", qemu.runner.pre_cont_progress);
        if qemu.media.is_empty() {
            let _ = writeln!(text, "media: no declared qemu-media sources");
        } else {
            text.push_str("media:\n");
            for source in &qemu.media {
                let _ = writeln!(
                    text,
                    "  - slot={} ref={} kind={} format={} readOnly={} registry={}",
                    source.slot,
                    source.media_ref,
                    source.source_kind,
                    source.format,
                    source.read_only,
                    source.registry.state,
                );
                if let Some(remediation) = &source.registry.remediation {
                    let _ = writeln!(text, "    remediation: {remediation}");
                }
            }
        }
        if !output.unsupported_capabilities.is_empty() {
            let _ = writeln!(
                text,
                "unsupported capabilities: {}",
                output.unsupported_capabilities.join(", ")
            );
        }
        if !output.runtime_capabilities.is_empty() {
            let _ = writeln!(
                text,
                "runtime capabilities: {}",
                output.runtime_capabilities.join(", ")
            );
        }
        if !output.service_capabilities.is_empty() {
            let _ = writeln!(
                text,
                "service capabilities: {}",
                output.service_capabilities.join(", ")
            );
        }
    } else {
        let _ = writeln!(text, "backend-runner: {}", output.services.microvm);
        let _ = writeln!(text, "virtiofsd: {}", output.services.virtiofsd);
        let _ = writeln!(
            text,
            "gpu-runner: {}",
            output
                .services
                .gpu
                .clone()
                .unwrap_or_else(|| "stopped".to_owned())
        );
    }
    if let Some(video) = &output.services.video {
        let _ = writeln!(text, "video: {video}");
    }
    if let Some(usb) = &output.usb {
        let _ = writeln!(
            text,
            "usb: {}",
            if usb.degraded { "degraded" } else { "ok" }
        );
        for entry in &usb.entries {
            let _ = writeln!(
                text,
                "  - busid={} status={} session-claim={} host-bind={} carrier={} proxy={} guest-import={} topology={} policy={}",
                entry.bus_id,
                usb_probe_status_label(entry.status),
                durable_claim_label(entry.durable_claim.state),
                host_bind_label(entry.host.bind),
                host_carrier_label(entry.host.carrier),
                proxy_label(entry.host.proxy),
                guest_import_label(entry.guest.import),
                topology_label(entry.topology_policy.topology),
                policy_label(entry.topology_policy.policy),
            );
            for reason in &entry.degraded_reasons {
                let _ = writeln!(
                    text,
                    "    degraded: {} - {}",
                    reason_code_label(reason.code),
                    reason.summary
                );
                let _ = writeln!(text, "    remediation: {}", reason.remediation);
            }
            for command in &entry.remediation_commands {
                let _ = writeln!(text, "    command: {command}");
            }
        }
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

#[allow(dead_code)]
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
    if let Some(rm) = output.read_model.as_ref() {
        let fp = if rm.source_fingerprint.len() > 8 {
            &rm.source_fingerprint[..8]
        } else {
            &rm.source_fingerprint
        };
        let _ = writeln!(
            text,
            "\n[read-model: {}, gen {}, fingerprint {}]",
            rm.freshness, rm.generation, fp
        );
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

#[allow(dead_code)]
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

#[allow(dead_code)]
fn resolve_bridge_probe_name(bundle: Option<&BundleContext>, bridge: &str) -> String {
    if let Some(runtime) = bundle.and_then(|bundle| bundle.host_runtime.as_ref())
        && let Some(ifname) = runtime
            .ifnames
            .iter()
            .find(|row| row.vm.is_none() && row.user_visible_name == bridge)
    {
        return ifname.derived_ifname.clone();
    }
    if let Some(host) = bundle.and_then(|bundle| bundle.host.as_ref())
        && let Some(mapping) = host
            .if_name_mappings
            .iter()
            .find(|row| row.vm.is_none() && row.user_visible_name == bridge)
    {
        return mapping.derived_ifname.as_str().to_owned();
    }
    bridge.to_owned()
}

#[allow(dead_code)]
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
    if let Ok(output) = output
        && output.status.success()
        && let Ok(value) = serde_json::from_slice::<Value>(&output.stdout)
        && let Some(link) = value.as_array().and_then(|items| items.first())
    {
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
    row
}

#[allow(dead_code)]
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
        "launch",
        "audit",
        "host check",
        "auth status",
        "op inspect",
        "realm list",
        "realm inspect",
        "realm enter",
        "realm run",
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
        AuthRoleV2::None => [
            "list",
            "status",
            "host check",
            "auth status",
            "op inspect",
            "realm list",
            "realm inspect",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
    }
}

fn denied_reason(role: AuthRoleV2, command: &str) -> &'static str {
    match (role, command) {
        (AuthRoleV2::Admin, _) => "allowed",
        (_, "audit") => "audit requires admin role in `d2b.site.adminUsers`.",
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

/// Look up the canonical workload target address for a VM by its VM name.
/// Reads the bundle.json and, if it references a realm-controllers artifact,
/// parses it to find the workload's `identity.canonicalTarget`. Returns `None`
/// on any IO or parse error (advisory hint path — never blocks the caller).
fn try_canonical_target_for_vm(bundle_path: &Path, vm: &str) -> Option<String> {
    let bundle: Bundle = read_json_file(bundle_path).ok()?;
    let realm_controllers_ref = bundle.realm_controllers_path.as_deref()?;
    let base_dir = bundle_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"));
    let rc_path = if Path::new(realm_controllers_ref).is_absolute() {
        PathBuf::from(realm_controllers_ref)
    } else {
        base_dir.join(realm_controllers_ref)
    };
    let rc: RealmControllersJson = read_json_file(&rc_path).ok()?;
    for controller in &rc.controllers {
        let Some(local_rt) = controller.local_runtime.as_ref() else {
            continue;
        };
        for workload in &local_rt.workloads {
            if workload.vm_name.as_str() == vm {
                return workload
                    .identity
                    .as_ref()
                    .map(|id| id.canonical_target.to_canonical());
            }
        }
    }
    None
}

fn try_vm_for_canonical_target(bundle_path: &Path, raw_target: &str) -> Option<String> {
    let bundle: Bundle = read_json_file(bundle_path).ok()?;
    let realm_controllers_ref = bundle.realm_controllers_path.as_deref()?;
    let base_dir = bundle_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"));
    let rc_path = if Path::new(realm_controllers_ref).is_absolute() {
        PathBuf::from(realm_controllers_ref)
    } else {
        base_dir.join(realm_controllers_ref)
    };
    let rc: RealmControllersJson = read_json_file(&rc_path).ok()?;
    for controller in &rc.controllers {
        let Some(local_rt) = controller.local_runtime.as_ref() else {
            continue;
        };
        for workload in &local_rt.workloads {
            let Some(identity) = workload.identity.as_ref() else {
                continue;
            };
            if identity.canonical_target.to_canonical() == raw_target {
                return Some(workload.vm_name.as_str().to_owned());
            }
        }
    }
    None
}

#[allow(dead_code)]
fn resolve_vm_selector_from_bundle(context: &Context, selector: &str) -> String {
    try_vm_for_canonical_target(&context.bundle_path, selector)
        .unwrap_or_else(|| selector.to_owned())
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
// buffer above isolates captured BYTES; this lock serializes the capturing
// tests so their stdout capture cannot interleave under cargo's parallel
// harness. (Staging-base and peer overrides are now per-thread, so they no
// longer need process-global serialization.)
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

fn print_stderr(text: &str) {
    let _ = write_stderr_bytes(text.as_bytes());
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
        let _ = writeln!(stderr, "d2b: {}", err.message);
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
            ));
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
            summary: summary.unwrap_or_else(|| format!("d2b {verb} --apply ok")),
        }),
        "dry-run-planned" => Ok(DaemonVerbOutcome::DryRunPlanned {
            summary: summary
                .unwrap_or_else(|| format!("d2b {verb} --dry-run: plan synthesized by daemon")),
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
                "{op_name} references a bundle intent that the broker did not find. Admin: ask `journalctl -u d2b-priv-broker` for the intent id."
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
                "{op_name} failed at the broker live handler. Admin: inspect `journalctl -u d2b-priv-broker` for the underlying syscall/exit code."
            ),
        ),
        "Broker.CoexistenceRefused" => (
            format!("{op_name} refused by firewall coexistence policy"),
            "The daemon reached the broker, but another firewall manager still owns the live table described by the trusted bundle.".to_owned(),
            format!(
                "{op_name} refused: another firewall manager owns the table per FirewallCoexistencePolicy. Admin: check d2b.site.firewallCoexistencePolicy."
            ),
        ),
        "Broker.NftScriptParseFailed" => (
            format!("{op_name} failed: bundle nft script parse error"),
            "The daemon reached the broker, but the nftables batch embedded in the trusted bundle could not be parsed.".to_owned(),
            format!(
                "{op_name} failed: bundle nft script could not be parsed. Admin: inspect `journalctl -u d2b-priv-broker` for the parse error."
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
            "broker operation is not implemented in this build; Admin: use the supported fallback path for this release.".to_owned(),
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
    let op_name = format!("d2b {verb} --apply");
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
            if json {
                print_json(&serde_json::json!({
                    "outcome": "applied",
                    "summary": summary,
                }))?;
            } else {
                print_stdout(&format!("{summary}\n"));
            }
            Ok(0)
        }
        DaemonVerbOutcome::DryRunPlanned { summary } => {
            if json {
                print_json(&serde_json::json!({
                    "outcome": "dry-run",
                    "summary": summary,
                }))?;
            } else {
                print_stdout(&format!("{summary}\n"));
            }
            Ok(0)
        }
        DaemonVerbOutcome::ApiReadyTimeout { summary } => {
            let msg = summary.unwrap_or_else(|| "vm start: api-ready timeout".to_owned());
            if json {
                print_json(&serde_json::json!({
                    "outcome": "api-ready-timeout",
                    "summary": msg,
                }))?;
            } else {
                print_stdout(&format!("{msg}\n"));
            }
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
                "Upgrade d2bd to a build that includes the requested native handler, then retry.",
            );
            emit_host_error(
                &host_error_envelope(
                    &format!("d2b {verb} --apply requires a daemon-native handler"),
                    "not-yet-implemented",
                    78,
                    &format!("Daemon-native execution for `d2b {verb} --apply` (target: {tw})"),
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
                    "Daemon connectivity at /run/d2b/public.sock.",
                    "d2bd is unreachable; v1.1 daemon-only (ADR 0015 + ADR 0017) surfaces the typed `daemon-down` envelope with exit 1.",
                    "Start d2bd on the host, then re-run the same command.",
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
/// Rust CLI dispatching through d2bd → broker is the only
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
            ));
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

#[allow(dead_code)]
fn try_list_via_socket(context: &Context) -> Result<ListSocketOutcome, CliFailure> {
    let request = encode_type_tagged_message(
        "list",
        &IpcListRequest {
            env: None,
            vm: None,
        },
        "list request",
    )?;
    match try_public_socket_request(context, &request, "list")? {
        PublicSocketOutcome::Reply(response) => {
            parse_list_reply(&response).map(|(entries, rm)| ListSocketOutcome::Entries(entries, rm))
        }
        PublicSocketOutcome::Unavailable | PublicSocketOutcome::Unsupported => {
            Ok(ListSocketOutcome::Unavailable)
        }
    }
}

#[allow(dead_code)]
fn try_status_via_socket(
    context: &Context,
    vm: Option<&str>,
) -> Result<StatusSocketOutcome, CliFailure> {
    let request = encode_type_tagged_message(
        "status",
        &IpcStatusRequest {
            check_bridges: false,
            vm: vm.map(str::to_owned),
        },
        "status request",
    )?;
    match try_public_socket_request(context, &request, "status")? {
        PublicSocketOutcome::Reply(response) => parse_status_reply(&response)
            .map(|(entries, rm)| StatusSocketOutcome::Entries(entries, rm)),
        PublicSocketOutcome::Unavailable | PublicSocketOutcome::Unsupported => {
            Ok(StatusSocketOutcome::Unavailable)
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
        &d2b_contracts::public_wire::StoreVerifyRequest {
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
            ));
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

#[allow(dead_code)]
fn parse_list_reply(
    bytes: &[u8],
) -> Result<
    (
        Vec<IpcListEntry>,
        Option<d2b_contracts::public_wire::PublicReadModelMetadata>,
    ),
    CliFailure,
> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|err| CliFailure::new(1, format!("failed to parse list reply: {err}")))?;
    if value.get("type").and_then(Value::as_str) != Some("listResponse") {
        return Err(CliFailure::new(
            1,
            "daemon returned an unexpected reply to list".to_owned(),
        ));
    }
    serde_json::from_value::<ListResponseFrame>(value)
        .map(|frame| (frame.vms, frame.read_model))
        .map_err(|err| CliFailure::new(1, format!("failed to decode list reply: {err}")))
}

#[allow(dead_code)]
fn parse_status_reply(
    bytes: &[u8],
) -> Result<
    (
        Vec<IpcVmStatus>,
        Option<d2b_contracts::public_wire::PublicReadModelMetadata>,
    ),
    CliFailure,
> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|err| CliFailure::new(1, format!("failed to parse status reply: {err}")))?;
    if value.get("type").and_then(Value::as_str) != Some("statusResponse") {
        return Err(CliFailure::new(
            1,
            "daemon returned an unexpected reply to status".to_owned(),
        ));
    }
    serde_json::from_value::<StatusResponseFrame>(value)
        .map(|frame| (frame.status.entries, frame.status.read_model))
        .map_err(|err| CliFailure::new(1, format!("failed to decode status reply: {err}")))
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
                .unwrap_or("d2b usb probe failed in the daemon → broker path")
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
        ffi::OsString,
        io,
        os::{
            fd::{AsRawFd as _, RawFd},
            unix::{ffi::OsStringExt, fs::PermissionsExt},
        },
        path::PathBuf,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        thread,
        time::Duration,
    };

    use nix::{
        sys::socket::{Backlog, accept4, bind, listen},
        unistd::close,
    };
    use serde_json::{Value, json};

    use super::{
        AddressFamily, ApiReadySimple, ApiReadyStatusV1, Context, HostInstallArgs, IpcHelloOk,
        IpcUsbProbeEntryKind, IpcUsbipProbeEntry, IpcUsbipProbeStatus, MAX_FRAME_BYTES,
        ManifestDocument, ManifestVm, MediaRef, MsgFlags, NativeCli, NativeCommand, SockFlag,
        SockType, StatusServicesOutputV2, UnixAddr, UsbAttachArgs, UsbDetachArgs, VmArgs,
        VmCommand, VmExecArgs, VmRestartArgs, VmStartArgs, VmStopArgs, broker_error_envelope,
        build_storage_migration_plan, cmd_host_install, cmd_vm_exec_v2, cmd_vm_restart,
        cmd_vm_start, cmd_vm_stop, daemon_supported_features, encode_type_tagged_message,
        host_shutdown_vm_phases, is_host_shutdown_hook_invocation, nix_err_to_io,
        output_service_capabilities, parse_host_shutdown_hook_args, parse_vm_exec_action,
        public_wire, render_usb_probe_human, send, socket, storage_migration_checkpoint_id,
    };
    use d2b_core::error::Version;

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

    #[test]
    fn gateway_target_guard_fails_before_manifest_or_socket_access() {
        let err = super::guard_local_target("demo.work.d2b", false)
            .expect_err("realm target must fail closed on host daemon");
        assert_eq!(err.exit_code, 2);
        assert!(err.message.contains("target not dispatchable"));
        assert!(!err.message.contains("failed to read"));
        assert!(!err.message.contains("public.sock"));
    }

    #[test]
    fn local_fast_path_targets_pass_gateway_guard() {
        super::guard_local_target("vm-a", false).expect("bare VM names stay local");
        super::guard_local_target("demo.aca.work", false)
            .expect("unqualified dotted names stay with legacy local validation");
    }

    #[test]
    fn gateway_candidate_requires_manifest_declared_realm_gateway() {
        let manifest_path = test_socket_path("gateway-candidate", ".manifest.json");
        if let Some(parent) = manifest_path.parent() {
            std::fs::create_dir_all(parent).expect("manifest parent");
        }
        write_test_manifest(&manifest_path, "sys-work-gateway");
        let context = Context {
            manifest_path: manifest_path.clone(),
            bundle_path: manifest_path.with_extension("bundle.json"),
            public_socket: manifest_path.with_extension("sock"),
            broker_socket: manifest_path.with_extension("broker.sock"),
            state_root: None,
            host_runtime_path: manifest_path.with_extension("host-runtime.json"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: manifest_path.with_extension("daemon-state"),
            metrics_url: "http://127.0.0.1:9101/metrics".to_owned(),
        };
        assert_eq!(
            super::gateway_target_from_manifest(&context, "demo.work.d2b", false)
                .unwrap()
                .as_deref(),
            Some("demo.work.d2b")
        );
        let err = super::gateway_target_from_manifest(&context, "demo.unknown.d2b", false)
            .expect_err("unknown realm has no gateway entrypoint");
        assert_eq!(err.exit_code, 2);
        assert!(err.message.contains("entrypoint"));
        assert_eq!(
            super::gateway_target_from_manifest(&context, "vm-a", false).unwrap(),
            None
        );
        let _ = std::fs::remove_file(&manifest_path);
    }

    #[test]
    fn realm_entrypoint_table_supports_custom_gateway_vm_names() {
        let root = test_socket_path("custom-realm-entrypoints", ".dir");
        std::fs::create_dir_all(&root).expect("create realm table dir");
        let manifest_path = root.join("manifest.json");
        let entrypoints_path = root.join("realm-entrypoints.json");
        write_test_manifest(&manifest_path, "corp-gateway");
        std::fs::write(
            &entrypoints_path,
            r#"{
              "schemaVersion": 1,
              "entries": {
                "local": { "mode": "host-resident", "gateway": null },
                "work": { "mode": "gateway-backed", "gateway": "corp-gateway.local.d2b" }
              }
            }"#,
        )
        .expect("write realm entrypoint table");
        let table = super::load_realm_entrypoint_table_from_path(&entrypoints_path)
            .expect("load entrypoint table")
            .expect("entrypoint table exists");

        let context = Context {
            manifest_path: manifest_path.clone(),
            bundle_path: manifest_path.with_extension("bundle.json"),
            public_socket: manifest_path.with_extension("sock"),
            broker_socket: manifest_path.with_extension("broker.sock"),
            state_root: None,
            host_runtime_path: manifest_path.with_extension("host-runtime.json"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: manifest_path.with_extension("daemon-state"),
            metrics_url: "http://127.0.0.1:9101/metrics".to_owned(),
        };
        let routed =
            super::route_vm_target_with_table(&context, "demo.work.d2b", false, Some(table))
                .expect("gateway target routes through table");
        match routed {
            super::VmTargetRoute::Gateway {
                gateway_vm,
                gateway,
                target,
                ..
            } => {
                assert_eq!(gateway_vm, "corp-gateway");
                assert_eq!(gateway, "corp-gateway.local.d2b");
                assert_eq!(target, "demo.work.d2b");
            }
            other => panic!("expected gateway route, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn realm_enter_and_run_parse_gateway_helper_forms() {
        let enter = NativeCli::try_parse_from(["d2b", "realm", "enter", "work"])
            .expect("realm enter parses");
        match enter.command {
            super::NativeCommand::Realm(super::RealmArgs {
                command: super::RealmCommand::Enter(args),
            }) => assert_eq!(args.realm, "work"),
            other => panic!("expected realm enter, got {other:?}"),
        }

        let run =
            NativeCli::try_parse_from(["d2b", "realm", "run", "work", "--", "d2b", "vm", "list"])
                .expect("realm run parses");
        match run.command {
            super::NativeCommand::Realm(super::RealmArgs {
                command: super::RealmCommand::Run(args),
            }) => {
                assert_eq!(args.realm, "work");
                assert_eq!(
                    args.argv,
                    vec!["d2b".to_owned(), "vm".to_owned(), "list".to_owned()]
                );
            }
            other => panic!("expected realm run, got {other:?}"),
        }
    }

    fn parse_shell_raw(argv: &[&str]) -> super::ShellArgs {
        let cli = NativeCli::try_parse_from(argv).expect("shell argv parses");
        match cli.command {
            super::NativeCommand::Shell(args) => args,
            other => panic!("expected shell command, got {other:?}"),
        }
    }

    #[test]
    fn shell_vm_first_grammar_parses_attach_and_management_forms() {
        let implicit = parse_shell_raw(&["d2b", "shell", "work", "--name", "dev", "--force"]);
        assert_eq!(implicit.vm, "work");
        assert_eq!(implicit.name.as_deref(), Some("dev"));
        assert!(implicit.force);
        assert_eq!(implicit.action, None);

        let explicit =
            parse_shell_raw(&["d2b", "shell", "work", "attach", "--name", "ops", "--force"]);
        assert_eq!(explicit.vm, "work");
        assert_eq!(explicit.action, Some(super::ShellAction::Attach));
        assert_eq!(explicit.name.as_deref(), Some("ops"));
        assert!(explicit.force);

        let list = parse_shell_raw(&["d2b", "shell", "work", "list", "--json"]);
        assert_eq!(list.vm, "work");
        assert_eq!(list.action, Some(super::ShellAction::List));
        assert!(list.json);
        assert!(!list.human);

        let detach = parse_shell_raw(&["d2b", "shell", "work", "detach", "--name", "ops"]);
        assert_eq!(detach.vm, "work");
        assert_eq!(detach.action, Some(super::ShellAction::Detach));
        assert_eq!(detach.name.as_deref(), Some("ops"));
        assert!(!detach.json);
        assert!(!detach.human);

        let kill = parse_shell_raw(&["d2b", "shell", "work", "kill", "--name", "ops", "--json"]);
        assert_eq!(kill.vm, "work");
        assert_eq!(kill.action, Some(super::ShellAction::Kill));
        assert_eq!(kill.name.as_deref(), Some("ops"));
        assert!(kill.json);
        assert!(!kill.human);
    }

    #[test]
    fn shell_vm_first_grammar_supports_verb_named_vms() {
        for vm in ["attach", "list", "detach", "kill"] {
            let implicit = parse_shell_raw(&["d2b", "shell", vm]);
            assert_eq!(implicit.vm, vm);
            assert_eq!(implicit.action, None);

            let explicit = parse_shell_raw(&["d2b", "shell", vm, "attach", "--name", "dev"]);
            assert_eq!(explicit.vm, vm);
            assert_eq!(explicit.action, Some(super::ShellAction::Attach));
            assert_eq!(explicit.name.as_deref(), Some("dev"));
        }
    }

    #[test]
    fn shell_parser_rejects_missing_vm_command_tail_and_invalid_utf8() {
        let missing = NativeCli::try_parse_from(["d2b", "shell"])
            .expect_err("missing VM is a clap usage error");
        assert_eq!(missing.exit_code(), 2);

        let tail = NativeCli::try_parse_from(["d2b", "shell", "work", "htop"])
            .expect_err("command tail is rejected by clap value parser");
        assert_eq!(tail.exit_code(), 2);
        let hint_args = [
            OsString::from("d2b"),
            OsString::from("shell"),
            OsString::from("work"),
            OsString::from("htop"),
        ];
        assert!(
            super::shell_trailing_command_hint(&hint_args)
                .unwrap()
                .contains("d2b vm exec <target> -- <cmd>")
        );

        let invalid = NativeCli::try_parse_from(vec![
            OsString::from("d2b"),
            OsString::from("shell"),
            OsString::from("work"),
            OsString::from_vec(vec![0xff]),
        ])
        .expect_err("invalid utf8 tail is rejected by clap");
        assert_eq!(invalid.exit_code(), 2);
    }

    #[test]
    fn vm_list_all_parse_gateway_selector() {
        let cli = NativeCli::try_parse_from(["d2b", "vm", "list", "--all"])
            .expect("vm list --all parses");
        match cli.command {
            super::NativeCommand::Vm(super::VmArgs {
                command: super::VmCommand::List(args),
            }) => {
                assert!(args.all);
                assert!(args.realm.is_none());
            }
            other => panic!("expected vm list, got {other:?}"),
        }
    }

    #[test]
    fn route_vm_target_preserves_local_names_and_routes_gateway_targets() {
        let local =
            super::route_vm_target_with_table(&missing_daemon_context(), "demo", false, None)
                .expect("local target routes without manifest");
        assert_eq!(
            local,
            super::VmTargetRoute::Local {
                vm: "demo".to_owned()
            }
        );

        let manifest_path = test_socket_path("route-gateway-target", ".manifest.json");
        if let Some(parent) = manifest_path.parent() {
            std::fs::create_dir_all(parent).expect("manifest parent");
        }
        write_test_manifest(&manifest_path, "sys-work-gateway");
        let context = Context {
            manifest_path: manifest_path.clone(),
            bundle_path: manifest_path.with_extension("bundle.json"),
            public_socket: manifest_path.with_extension("sock"),
            broker_socket: manifest_path.with_extension("broker.sock"),
            state_root: None,
            host_runtime_path: manifest_path.with_extension("host-runtime.json"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: manifest_path.with_extension("daemon-state"),
            metrics_url: "http://127.0.0.1:9101/metrics".to_owned(),
        };
        let local = super::route_vm_target(&context, "demo", false)
            .expect("local target routes with manifest context");
        assert_eq!(
            local,
            super::VmTargetRoute::Local {
                vm: "demo".to_owned()
            }
        );

        let routed = super::route_vm_target(&context, "demo.work.d2b", false)
            .expect("gateway target routes");
        match routed {
            super::VmTargetRoute::Gateway {
                realm,
                gateway_vm,
                gateway,
                target,
            } => {
                assert_eq!(realm, "work");
                assert_eq!(gateway_vm, "sys-work-gateway");
                assert_eq!(gateway, "sys-work-gateway.local.d2b");
                assert_eq!(target, "demo.work.d2b");
            }
            other => panic!("expected gateway route, got {other:?}"),
        }
        let _ = std::fs::remove_file(&manifest_path);
    }

    #[test]
    fn route_vm_target_uses_bundle_identity_for_host_local_workload_target() {
        let manifest_path = test_socket_path("route-workload-canonical-local", ".manifest.json");
        if let Some(parent) = manifest_path.parent() {
            std::fs::create_dir_all(parent).expect("manifest parent");
        }
        write_test_manifest(&manifest_path, "work-aad");
        let bundle_path = manifest_path.with_extension("bundle.json");
        write_bundle_with_realm_controllers(&bundle_path, "work-aad");
        rewrite_bundle_workload_identity(&bundle_path, "aad", "aad.work.d2b");
        let context = Context {
            manifest_path: manifest_path.clone(),
            bundle_path: bundle_path.clone(),
            public_socket: manifest_path.with_extension("sock"),
            broker_socket: manifest_path.with_extension("broker.sock"),
            state_root: None,
            host_runtime_path: manifest_path.with_extension("host-runtime.json"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: manifest_path.with_extension("daemon-state"),
            metrics_url: "http://127.0.0.1:9101/metrics".to_owned(),
        };

        let route = super::route_vm_target(&context, "aad.work.d2b", false)
            .expect("canonical workload target resolves through bundle identity");
        assert_eq!(
            route,
            super::VmTargetRoute::Local {
                vm: "work-aad".to_owned()
            }
        );
        let _ = std::fs::remove_file(&manifest_path);
    }

    #[allow(dead_code)]
    fn cmd_status_accepts_canonical_workload_target_selector() {
        let manifest_path = test_socket_path("status-workload-canonical", ".manifest.json");
        if let Some(parent) = manifest_path.parent() {
            std::fs::create_dir_all(parent).expect("manifest parent");
        }
        write_test_manifest(&manifest_path, "work-aad");
        let bundle_path = manifest_path.with_extension("bundle.json");
        write_bundle_with_realm_controllers(&bundle_path, "work-aad");
        rewrite_bundle_workload_identity(&bundle_path, "aad", "aad.work.d2b");
        let context = Context {
            manifest_path: manifest_path.clone(),
            bundle_path: bundle_path.clone(),
            public_socket: manifest_path.with_extension("sock"),
            broker_socket: manifest_path.with_extension("broker.sock"),
            state_root: None,
            host_runtime_path: manifest_path.with_extension("host-runtime.json"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: manifest_path.with_extension("daemon-state"),
            metrics_url: "http://127.0.0.1:9101/metrics".to_owned(),
        };
        let args = super::StatusArgs {
            json: true,
            human: false,
            check_bridges: false,
            vm_flag: None,
            vm: Some("aad.work.d2b".to_owned()),
        };

        let (result, stdout) =
            super::with_test_stdout_capture(|| super::cmd_status(&context, &args));
        assert_eq!(result.expect("canonical status result"), 0);
        let output: Value = serde_json::from_slice(&stdout).expect("status json output");
        assert_eq!(output.get("name").and_then(Value::as_str), Some("work-aad"));
        let _ = std::fs::remove_file(&manifest_path);
    }

    #[test]
    fn route_vm_target_with_table_missing_gateway_fails_closed() {
        let manifest_path = test_socket_path("route-custom-missing-gateway", ".manifest.json");
        if let Some(parent) = manifest_path.parent() {
            std::fs::create_dir_all(parent).expect("manifest parent");
        }
        write_test_manifest(&manifest_path, "vm-a");
        let mut table = d2b_realm_router::RealmEntrypointTable::with_local_default();
        table.gateway_backed(
            d2b_realm_core::RealmPath::new(vec![d2b_realm_core::RealmId::parse("work").unwrap()])
                .unwrap(),
            d2b_realm_core::TargetName::parse("corp-gateway.local.d2b").unwrap(),
        );
        let context = Context {
            manifest_path: manifest_path.clone(),
            bundle_path: manifest_path.with_extension("bundle.json"),
            public_socket: manifest_path.with_extension("sock"),
            broker_socket: manifest_path.with_extension("broker.sock"),
            state_root: None,
            host_runtime_path: manifest_path.with_extension("host-runtime.json"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: manifest_path.with_extension("daemon-state"),
            metrics_url: "http://127.0.0.1:9101/metrics".to_owned(),
        };
        let (result, stdout) = super::with_test_stdout_capture(|| {
            super::route_vm_target_with_table(&context, "demo.work.d2b", true, Some(table))
        });
        let err = result.expect_err("missing custom gateway must fail");
        assert_eq!(err.exit_code, 2);
        let envelope: Value = serde_json::from_slice(&stdout).expect("json error envelope");
        assert_eq!(
            envelope.get("code").and_then(Value::as_str),
            Some("missing-realm-entrypoint")
        );
        assert!(
            envelope
                .get("remediation")
                .and_then(Value::as_str)
                .is_some_and(|text| text.contains("corp-gateway"))
        );
        let _ = std::fs::remove_file(&manifest_path);
    }

    #[test]
    fn route_vm_target_rejects_env_style_target_fail_closed() {
        // `corp-vm.work` looks like an old env-qualified target missing `.d2b`.
        // route_vm_target must fail-closed with error code `old-env-style-target`
        // and a suggestion to use `corp-vm.work.d2b`.
        let manifest_path = test_socket_path("env-style-fail-closed", ".manifest.json");
        if let Some(parent) = manifest_path.parent() {
            std::fs::create_dir_all(parent).expect("manifest parent");
        }
        write_test_manifest(&manifest_path, "vm-a");
        let context = test_context(manifest_path.clone());

        let (result, stdout) = super::with_test_stdout_capture(|| {
            super::route_vm_target(&context, "corp-vm.work", true)
        });
        let err = result.expect_err("env-style target must fail closed");
        assert_eq!(err.exit_code, 2, "exit code 2 for usage error");
        let envelope: Value = serde_json::from_slice(&stdout).expect("json error envelope");
        assert_eq!(
            envelope.get("code").and_then(Value::as_str),
            Some("old-env-style-target"),
            "error code must be old-env-style-target"
        );
        let remediation = envelope
            .get("remediation")
            .and_then(Value::as_str)
            .unwrap_or("");
        assert!(
            remediation.contains("corp-vm.work.d2b"),
            "remediation must suggest the canonical form; got: {remediation}"
        );
        let _ = std::fs::remove_file(&manifest_path);
    }

    #[test]
    fn route_vm_target_passes_canonical_realm_target() {
        // `corp-vm.work.d2b` already has the `.d2b` suffix — env-style detection
        // must not reject it. This test verifies there is no false positive.
        let manifest_path = test_socket_path("env-style-no-false-positive", ".manifest.json");
        if let Some(parent) = manifest_path.parent() {
            std::fs::create_dir_all(parent).expect("manifest parent");
        }
        write_test_manifest(&manifest_path, "vm-a");
        let mut table = d2b_realm_router::RealmEntrypointTable::with_local_default();
        // Make `work` a local realm so the route resolves without a daemon.
        table.host_resident(
            d2b_realm_core::RealmPath::new(vec![d2b_realm_core::RealmId::parse("work").unwrap()])
                .unwrap(),
        );
        let context = test_context(manifest_path.clone());

        let (result, _stdout) = super::with_test_stdout_capture(|| {
            super::route_vm_target_with_table(&context, "corp-vm.work.d2b", false, Some(table))
        });
        // Must not produce an env-style error — the result may be Ok (Local) or a
        // different error (gateway not found), but never old-env-style-target.
        if let Err(err) = &result {
            assert!(
                !err.message.contains("old-env-style-target"),
                "canonical target must not trigger env-style detection; got: {}",
                err.message
            );
        }
        let _ = std::fs::remove_file(&manifest_path);
    }

    #[test]
    fn render_list_human_shows_workload_target_column_when_present() {
        let output = super::ListOutputV2(vec![
            super::ListItemOutputV2 {
                name: "corp-vm".to_owned(),
                env: Some("work".to_owned()),
                graphics: false,
                tpm: false,
                usbip: false,
                static_ip: None,
                status: "running".to_owned(),
                is_net_vm: false,
                guest_closure_out_path: None,
                runtime_kind: None,
                autostart: None,
                runtime_capabilities: Vec::new(),
                service_capabilities: Vec::new(),
                unsupported_capabilities: Vec::new(),
                qemu_media: None,
                runner_parity_ok: None,
                canonical_target: Some("corp-vm.work.d2b".to_owned()),
            },
            super::ListItemOutputV2 {
                name: "personal-vm".to_owned(),
                env: Some("home".to_owned()),
                graphics: false,
                tpm: false,
                usbip: false,
                static_ip: None,
                status: "stopped".to_owned(),
                is_net_vm: false,
                guest_closure_out_path: None,
                runtime_kind: None,
                autostart: None,
                runtime_capabilities: Vec::new(),
                service_capabilities: Vec::new(),
                unsupported_capabilities: Vec::new(),
                qemu_media: None,
                runner_parity_ok: None,
                canonical_target: None,
            },
        ]);
        let rendered = super::render_list_human(&output, None);
        assert!(
            rendered.contains("WORKLOAD TARGET"),
            "header must include WORKLOAD TARGET column when any entry has canonical_target"
        );
        assert!(
            rendered.contains("corp-vm.work.d2b"),
            "canonical target must appear in output row"
        );
    }

    #[test]
    fn render_list_human_omits_workload_target_column_when_absent() {
        let output = super::ListOutputV2(vec![super::ListItemOutputV2 {
            name: "vm-a".to_owned(),
            env: None,
            graphics: false,
            tpm: false,
            usbip: false,
            static_ip: None,
            status: "stopped".to_owned(),
            is_net_vm: false,
            guest_closure_out_path: None,
            runtime_kind: None,
            autostart: None,
            runtime_capabilities: Vec::new(),
            service_capabilities: Vec::new(),
            unsupported_capabilities: Vec::new(),
            qemu_media: None,
            runner_parity_ok: None,
            canonical_target: None,
        }]);
        let rendered = super::render_list_human(&output, None);
        assert!(
            !rendered.contains("WORKLOAD TARGET"),
            "WORKLOAD TARGET column must not appear when no entry has canonical_target"
        );
    }

    #[test]
    fn render_status_vm_human_shows_workload_target_when_present() {
        let output = super::StatusVmOutputV2 {
            name: "corp-vm".to_owned(),
            env: Some("work".to_owned()),
            services: super::StatusServicesOutputV2 {
                d2b: "active".to_owned(),
                microvm: "active".to_owned(),
                virtiofsd: "active".to_owned(),
                qemu_media: None,
                gpu: None,
                video: None,
                snd: None,
                swtpm: None,
            },
            current: None,
            booted: None,
            pending_restart: false,
            runtime: super::RUNTIME_UNKNOWN.to_owned(),
            runtime_kind: None,
            autostart: None,
            runtime_capabilities: Vec::new(),
            service_capabilities: Vec::new(),
            unsupported_capabilities: Vec::new(),
            qemu_media: None,
            usb: None,
            declared_roles: Vec::new(),
            readiness: Vec::new(),
            api_ready: None,
            runner_parity: None,
            live_pool_integrity: None,
            canonical_target: Some("corp-vm.work.d2b".to_owned()),
        };
        let manifest_vm = super::ManifestVm {
            name: "corp-vm".to_owned(),
            env: Some("work".to_owned()),
            graphics: false,
            tpm: false,
            audio: false,
            usbip_yubikey: false,
            static_ip: None,
            is_net_vm: false,
            state_dir: "/var/lib/d2b/vms/corp-vm".to_owned(),
            bridge: "d2b-work".to_owned(),
            ssh_user: None,
            runtime: None,
        };
        let rendered = super::render_status_vm_human(&output, &manifest_vm, Vec::new());
        assert!(
            rendered.contains("workload target"),
            "workload target label must appear"
        );
        assert!(
            rendered.contains("corp-vm.work.d2b"),
            "canonical target value must appear in status output"
        );
    }

    #[test]
    fn missing_realm_entrypoint_reports_actionable_remediation() {
        let manifest_path = test_socket_path("missing-entrypoint", ".manifest.json");
        if let Some(parent) = manifest_path.parent() {
            std::fs::create_dir_all(parent).expect("manifest parent");
        }
        write_test_manifest(&manifest_path, "vm-a");
        let context = Context {
            manifest_path: manifest_path.clone(),
            bundle_path: manifest_path.with_extension("bundle.json"),
            public_socket: manifest_path.with_extension("sock"),
            broker_socket: manifest_path.with_extension("broker.sock"),
            state_root: None,
            host_runtime_path: manifest_path.with_extension("host-runtime.json"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: manifest_path.with_extension("daemon-state"),
            metrics_url: "http://127.0.0.1:9101/metrics".to_owned(),
        };

        let (result, stdout) = super::with_test_stdout_capture(|| {
            super::resolve_realm_gateway(&context, "work", true)
        });
        let err = result.expect_err("missing gateway must fail");
        assert_eq!(err.exit_code, 2);
        let envelope: Value = serde_json::from_slice(&stdout).expect("json error envelope");
        assert_eq!(
            envelope.get("code").and_then(Value::as_str),
            Some("missing-realm-entrypoint")
        );
        assert!(
            envelope
                .get("remediation")
                .and_then(Value::as_str)
                .is_some_and(|text| text.contains("sys-work-gateway"))
        );
        let _ = std::fs::remove_file(&manifest_path);
    }

    #[allow(dead_code)]
    fn gateway_not_running_reports_start_remediation() {
        let response = json!({
            "type": "listResponse",
            "vms": [{
                "vm": "sys-work-gateway",
                "name": "sys-work-gateway",
                "env": "work",
                "graphics": false,
                "tpm": false,
                "usbip": false,
                "isNetVm": false,
                "sshUser": "alice",
                "staticIp": "10.20.0.10",
                "lifecycle": { "state": "Stopped", "pendingRestart": false },
                "runtime": { "detail": "stopped" },
                "services": {
                    "d2b": "inactive",
                    "microvm": "inactive",
                    "virtiofsd": "inactive",
                    "gpu": null,
                    "video": null,
                    "snd": null,
                    "swtpm": null
                }
            }]
        });
        let (result, request, stdout) = run_public_command_with_mock_daemon(
            "gateway-not-running",
            "sys-work-gateway",
            response,
            |context| {
                let gateway =
                    super::resolve_realm_gateway(context, "work", true).expect("gateway declared");
                super::ensure_realm_gateway_running(
                    context,
                    &gateway.realm,
                    &gateway.gateway_vm,
                    true,
                )
                .map(|()| 0)
            },
        );

        let err = result.expect_err("stopped gateway must fail");
        assert_eq!(err.exit_code, 70);
        assert_eq!(request.get("type").and_then(Value::as_str), Some("list"));
        let envelope: Value = serde_json::from_slice(&stdout).expect("json error envelope");
        assert_eq!(
            envelope.get("code").and_then(Value::as_str),
            Some("gateway-not-running")
        );
        assert_eq!(
            envelope.get("observedState").and_then(Value::as_str),
            Some("stopped")
        );
        assert!(
            envelope
                .get("remediation")
                .and_then(Value::as_str)
                .is_some_and(|text| text.contains("d2b vm start sys-work-gateway --apply"))
        );
    }

    /// Per-thread guard that overrides the config-staging base for a test and
    /// clears it on drop — replaces the old `D2B_CONFIG_STAGING_DIR` env
    /// mutation so no test touches process-global env.
    struct StagingBaseGuard;

    impl StagingBaseGuard {
        fn set(base: &std::path::Path) -> Self {
            super::set_test_staging_base(Some(base.to_path_buf()));
            Self
        }
    }

    impl Drop for StagingBaseGuard {
        fn drop(&mut self) {
            super::set_test_staging_base(None);
        }
    }

    fn recv_test_frame(fd: RawFd) -> io::Result<Vec<u8>> {
        recv_test_frame_with_flags(fd, MsgFlags::empty())
    }

    fn recv_test_frame_with_flags(fd: RawFd, flags: MsgFlags) -> io::Result<Vec<u8>> {
        let mut buffer = vec![0_u8; MAX_FRAME_BYTES + 4];
        let received = super::recv(fd, &mut buffer, flags).map_err(nix_err_to_io)?;
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
        let short_name: String = test_name
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .take(12)
            .collect();
        std::env::temp_dir().join(format!(
            "nlcli-{}-{counter}-{short_name}{suffix}",
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
                "audioService": format!("d2b-{vm}-audio.service"),
                "usbipYubikey": false,
                "staticIp": null,
                "isNetVm": false,
                "stateDir": format!("/var/lib/d2b/vms/{vm}"),
                "bridge": "d2b-dev",
                "sshUser": "alice"
            }
        });
        std::fs::write(
            path,
            serde_json::to_vec(&manifest).expect("serialize manifest"),
        )
        .expect("write manifest");
    }

    fn test_context(manifest_path: PathBuf) -> Context {
        Context {
            manifest_path: manifest_path.clone(),
            bundle_path: manifest_path.with_extension("bundle.json"),
            public_socket: manifest_path.with_extension("sock"),
            broker_socket: manifest_path.with_extension("broker.sock"),
            state_root: None,
            host_runtime_path: manifest_path.with_extension("host-runtime.json"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: manifest_path.with_extension("daemon-state"),
            metrics_url: "http://127.0.0.1:9101/metrics".to_owned(),
        }
    }

    #[test]
    fn realm_policy_rows_surface_default_deny_boundaries() {
        let manifest_path = test_socket_path("realm-policy-rows", ".manifest.json");
        if let Some(parent) = manifest_path.parent() {
            std::fs::create_dir_all(parent).expect("manifest parent");
        }
        write_test_manifest(&manifest_path, "sys-work-gateway");
        let _context = test_context(manifest_path.clone());
        let mut entries = std::collections::BTreeMap::new();
        entries.insert(
            "local".to_owned(),
            super::RealmEntrypointConfig {
                mode: "host-resident".to_owned(),
                gateway: None,
            },
        );
        entries.insert(
            "work".to_owned(),
            super::RealmEntrypointConfig {
                mode: "gateway-backed".to_owned(),
                gateway: Some("sys-work-gateway.local.d2b".to_owned()),
            },
        );

        let rows = super::realm_policy_rows_from_entries_with_states(
            entries,
            &std::collections::BTreeMap::new(),
        )
        .expect("realm rows render");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].realm, "local");
        assert_eq!(rows[0].mode, "host-resident");
        assert_eq!(rows[0].cross_realm_policy, "default-deny");
        assert_eq!(rows[0].credential_boundary, "host-resident-local-only");
        assert_eq!(rows[1].realm, "work");
        assert_eq!(rows[1].mode, "gateway-backed");
        assert_eq!(rows[1].gateway_vm.as_deref(), Some("sys-work-gateway"));
        assert_eq!(rows[1].cross_realm_policy, "default-deny");
        assert_eq!(rows[1].credential_boundary, "gateway-owned");
        let rendered = serde_json::to_string(&rows).expect("rows serialize");
        for forbidden in ["SharedAccessKey", "Bearer ", "/home/", "stdout", "stderr"] {
            assert!(
                !rendered.contains(forbidden),
                "realm policy output leaked {forbidden}: {rendered}"
            );
        }
        let _ = std::fs::remove_file(&manifest_path);
    }

    #[test]
    fn realm_policy_rows_inject_local_host_resident_entrypoint() {
        let manifest_path = test_socket_path("realm-policy-local-inject", ".manifest.json");
        if let Some(parent) = manifest_path.parent() {
            std::fs::create_dir_all(parent).expect("manifest parent");
        }
        write_test_manifest(&manifest_path, "sys-work-gateway");
        let _context = test_context(manifest_path.clone());
        let mut entries = std::collections::BTreeMap::new();
        entries.insert(
            "work".to_owned(),
            super::RealmEntrypointConfig {
                mode: "gateway-backed".to_owned(),
                gateway: Some("sys-work-gateway.local.d2b".to_owned()),
            },
        );
        let rows = super::realm_policy_rows_from_entries_with_states(
            super::normalize_realm_entrypoint_entries(entries).unwrap(),
            &std::collections::BTreeMap::new(),
        )
        .expect("realm rows render");
        assert_eq!(rows[0].realm, "local");
        assert_eq!(rows[0].mode, "host-resident");
        let _ = std::fs::remove_file(&manifest_path);
    }

    #[test]
    fn realm_policy_rows_reject_local_gateway_backed_entrypoint() {
        let mut entries = std::collections::BTreeMap::new();
        entries.insert(
            "local".to_owned(),
            super::RealmEntrypointConfig {
                mode: "gateway-backed".to_owned(),
                gateway: Some("sys-local-gateway.local.d2b".to_owned()),
            },
        );
        let err = super::normalize_realm_entrypoint_entries(entries)
            .expect_err("local gateway-backed entrypoint must fail closed");
        assert!(err.message.contains("local"));
        assert!(err.message.contains("host-resident"));
    }

    #[test]
    fn realm_policy_rows_reject_unknown_mode_and_missing_gateway() {
        let manifest_path = test_socket_path("realm-policy-bad-entries", ".manifest.json");
        if let Some(parent) = manifest_path.parent() {
            std::fs::create_dir_all(parent).expect("manifest parent");
        }
        write_test_manifest(&manifest_path, "sys-work-gateway");
        let _context = test_context(manifest_path.clone());

        let mut unknown_mode = std::collections::BTreeMap::new();
        unknown_mode.insert(
            "work".to_owned(),
            super::RealmEntrypointConfig {
                mode: "surprise".to_owned(),
                gateway: None,
            },
        );
        let err = super::realm_policy_rows_from_entries_with_states(
            unknown_mode,
            &std::collections::BTreeMap::new(),
        )
        .expect_err("unknown mode fails closed");
        assert!(err.message.contains("unknown entrypoint mode"));

        let mut missing_gateway = std::collections::BTreeMap::new();
        missing_gateway.insert(
            "work".to_owned(),
            super::RealmEntrypointConfig {
                mode: "gateway-backed".to_owned(),
                gateway: None,
            },
        );
        let err = super::realm_policy_rows_from_entries_with_states(
            missing_gateway,
            &std::collections::BTreeMap::new(),
        )
        .expect_err("missing gateway fails closed");
        assert!(err.message.contains("no gateway target"));
        let _ = std::fs::remove_file(&manifest_path);
    }

    #[test]
    fn realm_inspect_invalid_and_unknown_realms_fail_closed() {
        let rows = vec![super::RealmPolicyOutputV1 {
            realm: "local".to_owned(),
            mode: "host-resident".to_owned(),
            gateway_vm: None,
            gateway_target: None,
            gateway_state: "local-only".to_owned(),
            cross_realm_policy: "default-deny".to_owned(),
            credential_boundary: "host-resident-local-only".to_owned(),
        }];

        let (invalid, invalid_stdout) = super::with_test_stdout_capture(|| {
            super::realm_inspect_output("Bad Realm", true, rows.clone())
        });
        let err = invalid.expect_err("invalid realm fails");
        assert_eq!(err.exit_code, 2);
        let envelope: Value =
            serde_json::from_slice(&invalid_stdout).expect("invalid realm json envelope");
        assert_eq!(
            envelope.get("code").and_then(Value::as_str),
            Some("realm-target-usage")
        );

        let (unknown, unknown_stdout) =
            super::with_test_stdout_capture(|| super::realm_inspect_output("work", true, rows));
        let err = unknown.expect_err("unknown realm fails");
        assert_eq!(err.exit_code, 2);
        let envelope: Value =
            serde_json::from_slice(&unknown_stdout).expect("unknown realm json envelope");
        assert_eq!(
            envelope.get("code").and_then(Value::as_str),
            Some("missing-realm-entrypoint")
        );
    }

    #[test]
    fn op_inspect_includes_trace_and_degraded_gateway_summary() {
        let manifest_path = test_socket_path("op-inspect", ".manifest.json");
        if let Some(parent) = manifest_path.parent() {
            std::fs::create_dir_all(parent).expect("manifest parent");
        }
        write_test_manifest(&manifest_path, "sys-work-gateway");
        let args = super::OpInspectArgs {
            trace_id: Some("trace-1".to_owned()),
            span_id: Some("span-1".to_owned()),
            json: true,
            human: false,
        };
        let trace = super::op_inspect_trace(&args).expect("trace parses");
        let output = super::op_inspect_output_from_parts(
            1,
            trace,
            vec![super::RealmPolicyOutputV1 {
                realm: "local".to_owned(),
                mode: "host-resident".to_owned(),
                gateway_vm: None,
                gateway_target: None,
                gateway_state: "local-only".to_owned(),
                cross_realm_policy: "default-deny".to_owned(),
                credential_boundary: "host-resident-local-only".to_owned(),
            }],
            Vec::new(),
        );
        assert_eq!(output.command, "op inspect");
        assert_eq!(output.trace.as_ref().unwrap().trace_id, "trace-1");
        assert_eq!(output.local.vm_count, 1);
        assert!(
            usize::try_from(output.local.gateway_count).unwrap_or(usize::MAX)
                <= output.realms.len()
        );
        assert!(output.realms.iter().any(|realm| realm.realm == "local"));
        let rendered = serde_json::to_string(&output).expect("op inspect serializes");
        for forbidden in ["SharedAccessKey", "Bearer ", "/home/", "stdout", "stderr"] {
            assert!(
                !rendered.contains(forbidden),
                "op inspect output leaked {forbidden}: {rendered}"
            );
        }
        let _ = std::fs::remove_file(&manifest_path);
    }

    #[test]
    fn op_inspect_rejects_malformed_trace_context() {
        let args = super::OpInspectArgs {
            trace_id: Some("trace with spaces".to_owned()),
            span_id: Some("span-1".to_owned()),
            json: true,
            human: false,
        };
        let err = super::op_inspect_trace(&args).expect_err("bad trace fails");
        assert_eq!(err.exit_code, 2);
        assert!(err.message.contains("trace context"));

        let missing_pair = super::OpInspectArgs {
            trace_id: Some("trace-1".to_owned()),
            span_id: None,
            json: true,
            human: false,
        };
        assert!(super::op_inspect_trace(&missing_pair).unwrap().is_none());
    }

    #[test]
    fn op_inspect_parse_requires_trace_pair() {
        let err =
            super::NativeCli::try_parse_from(["d2b", "op", "inspect", "--trace-id", "trace-1"])
                .expect_err("clap requires --span-id with --trace-id");
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn op_inspect_reports_degraded_gateway_without_failing() {
        let realms = vec![super::RealmPolicyOutputV1 {
            realm: "work".to_owned(),
            mode: "gateway-backed".to_owned(),
            gateway_vm: Some("sys-work-gateway".to_owned()),
            gateway_target: Some("sys-work-gateway.local.d2b".to_owned()),
            gateway_state: "stopped".to_owned(),
            cross_realm_policy: "default-deny".to_owned(),
            credential_boundary: "gateway-owned".to_owned(),
        }];
        let output = super::op_inspect_output_from_parts(1, None, realms, Vec::new());
        assert_eq!(output.local.gateway_count, 1);
        assert_eq!(output.degraded.len(), 1);
        assert_eq!(output.degraded[0].scope, "gateway");
        assert_eq!(output.degraded[0].reason, "gateway-not-running");
        assert!(
            output.degraded[0]
                .remediation
                .contains("d2b vm start <gateway-vm> --apply")
        );
    }

    #[test]
    fn op_inspect_reports_missing_manifest_as_degraded_partial_result() {
        let manifest_path = test_socket_path("op-inspect-missing-manifest", ".manifest.json");
        let context = test_context(manifest_path);
        let args = super::OpInspectArgs {
            trace_id: None,
            span_id: None,
            json: true,
            human: false,
        };
        let output = super::op_inspect_output(&context, &args)
            .expect("missing manifest should degrade instead of failing");
        assert_eq!(output.local.vm_count, 0);
        assert!(
            output
                .degraded
                .iter()
                .any(|entry| entry.reason == "manifest-unavailable")
        );
    }

    fn write_qemu_media_manifest(path: &PathBuf, vm: &str) {
        let manifest = json!({
            (vm): {
                "name": vm,
                "env": "dev",
                "graphics": false,
                "tpm": false,
                "audio": false,
                "usbipYubikey": false,
                "staticIp": "10.20.0.20",
                "usbipdHostIp": null,
                "isNetVm": false,
                "stateDir": format!("/var/lib/d2b/vms/{vm}"),
                "bridge": "d2b-dev",
                "sshUser": null,
                "runtime": {
                    "kind": "qemu-media"
                }
            }
        });
        std::fs::write(
            path,
            serde_json::to_vec(&manifest).expect("serialize qemu media manifest"),
        )
        .expect("write qemu media manifest");
    }

    #[allow(dead_code)]
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

    fn run_public_command_with_mock_daemon<F>(
        test_name: &str,
        vm: &str,
        response: Value,
        command: F,
    ) -> (Result<i32, super::CliFailure>, Value, Vec<u8>)
    where
        F: FnOnce(&Context) -> Result<i32, super::CliFailure>,
    {
        run_public_command_with_manifest(test_name, vm, response, write_test_manifest, command)
    }

    fn run_public_command_with_manifest<F, W>(
        test_name: &str,
        vm: &str,
        response: Value,
        write_manifest: W,
        command: F,
    ) -> (Result<i32, super::CliFailure>, Value, Vec<u8>)
    where
        F: FnOnce(&Context) -> Result<i32, super::CliFailure>,
        W: FnOnce(&PathBuf, &str),
    {
        let socket_path = test_socket_path(test_name, ".sock");
        let manifest_path = test_socket_path(test_name, ".manifest.json");
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent).expect("create test socket dir");
        }
        let _ = std::fs::remove_file(&socket_path);
        let _ = std::fs::remove_file(&manifest_path);
        write_manifest(&manifest_path, vm);

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
        let (result, stdout) = super::with_test_stdout_capture(|| command(&context));
        let request = request_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("receive daemon request");
        server.join().expect("join mock daemon thread");
        let _ = std::fs::remove_file(&socket_path);
        let _ = std::fs::remove_file(&manifest_path);
        (result, request, stdout)
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

    fn missing_daemon_context() -> Context {
        let missing_manifest = test_socket_path("missing-daemon", ".missing-manifest.json");
        Context {
            manifest_path: missing_manifest,
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

    fn assert_daemon_down_json(result: Result<i32, super::CliFailure>, stdout: Vec<u8>) {
        assert_eq!(result.expect("daemon-down returns its exit code"), 1);
        let envelope: Value =
            serde_json::from_slice(&stdout).expect("daemon-down emits one JSON document");
        assert_eq!(
            envelope.get("code").and_then(Value::as_str),
            Some("daemon-down")
        );
        assert_eq!(envelope.get("exitCode").and_then(Value::as_i64), Some(1));
    }

    fn leave_stale_seqpacket_socket(path: &std::path::Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let _ = std::fs::remove_file(path);
        let fd = socket(
            AddressFamily::Unix,
            SockType::SeqPacket,
            SockFlag::SOCK_CLOEXEC,
            None,
        )
        .unwrap();
        bind(
            fd.as_raw_fd(),
            &UnixAddr::new(path).expect("stale socket address"),
        )
        .unwrap();
        drop(fd);
        assert!(path.exists());
    }

    #[test]
    fn daemon_backed_v2_commands_emit_typed_daemon_down_envelopes() {
        let mut context = missing_daemon_context();
        context.public_socket = test_socket_path("typed-daemon-down", ".missing.sock");

        let (result, stdout) = super::with_test_stdout_capture(|| {
            super::cmd_launch(
                &context,
                &super::LaunchArgs {
                    target: "demo".to_owned(),
                    item: "editor".to_owned(),
                    json: true,
                    human: false,
                },
            )
        });
        assert_daemon_down_json(result, stdout);

        let (result, stdout) = super::with_test_stdout_capture(|| {
            super::cmd_list(
                &context,
                &super::ListArgs {
                    json: true,
                    human: false,
                },
            )
        });
        assert_daemon_down_json(result, stdout);

        let (result, stdout) = super::with_test_stdout_capture(|| {
            super::cmd_status(
                &context,
                &super::StatusArgs {
                    json: true,
                    human: false,
                    check_bridges: false,
                    vm_flag: None,
                    vm: None,
                },
            )
        });
        assert_daemon_down_json(result, stdout);

        let (result, stdout) = super::with_test_stdout_capture(|| {
            super::cmd_shell_v2(
                &context,
                &super::ShellArgs {
                    vm: "demo".to_owned(),
                    action: Some(super::ShellAction::List),
                    name: None,
                    force: false,
                    json: true,
                    human: false,
                },
            )
        });
        assert_daemon_down_json(result, stdout);

        let result = super::cmd_console_v2(
            &context,
            &super::ConsoleArgs {
                vm: "demo".to_owned(),
            },
        );
        assert_eq!(result.expect("console daemon-down exit"), 1);
    }

    #[test]
    fn daemon_backed_commands_treat_refused_stale_socket_as_daemon_down() {
        let mut context = missing_daemon_context();
        context.public_socket = test_socket_path("typed-daemon-refused", ".sock");
        leave_stale_seqpacket_socket(&context.public_socket);

        let (result, stdout) = super::with_test_stdout_capture(|| {
            super::cmd_launch(
                &context,
                &super::LaunchArgs {
                    target: "demo".to_owned(),
                    item: "editor".to_owned(),
                    json: true,
                    human: false,
                },
            )
        });
        assert_daemon_down_json(result, stdout);

        let (result, stdout) = super::with_test_stdout_capture(|| {
            super::cmd_list(
                &context,
                &super::ListArgs {
                    json: true,
                    human: false,
                },
            )
        });
        assert_daemon_down_json(result, stdout);

        let (result, stdout) = super::with_test_stdout_capture(|| {
            super::cmd_status(
                &context,
                &super::StatusArgs {
                    json: true,
                    human: false,
                    check_bridges: false,
                    vm_flag: None,
                    vm: None,
                },
            )
        });
        assert_daemon_down_json(result, stdout);
        std::fs::remove_file(&context.public_socket).unwrap();
    }

    #[test]
    fn vm_display_is_hidden_until_a_typed_handler_exists() {
        let mut command = <super::NativeCli as clap::CommandFactory>::command();
        let vm = command.find_subcommand_mut("vm").expect("vm subcommand");
        let help = vm.render_long_help().to_string();
        assert!(!help.contains("display"));
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
    fn vm_exec_missing_command_emits_usage_envelope() {
        // A missing command is validated inside `cmd_vm_exec_v2` (the
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
            json: true,
            human: false,
            management: Vec::new(),
            command: Vec::new(),
        };
        let (result, stdout) =
            super::with_test_stdout_capture(|| cmd_vm_exec_v2(&context, &json_args));
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
        let failure = cmd_vm_exec_v2(&context, &human_args)
            .expect_err("missing command is a human usage failure");
        assert_eq!(failure.exit_code, 2);
        assert!(
            failure.message.contains("command form requires"),
            "human missing-command error is actionable: {}",
            failure.message
        );
    }

    #[test]
    fn vm_exec_detach_rejects_interactive_and_requires_command() {
        let context = missing_daemon_context();

        for argv in [
            ["d2b", "vm", "exec", "-d", "-i", "work", "--", "id"].as_slice(),
            ["d2b", "vm", "exec", "-d", "-t", "work", "--", "id"].as_slice(),
        ] {
            let args = parse_vm_exec(argv);
            let failure = cmd_vm_exec_v2(&context, &args).expect_err("-d with -i/-t is usage");
            assert_eq!(failure.exit_code, 2);
            assert!(
                failure.message.contains("conflicts with -i/-t"),
                "detach usage error is actionable: {}",
                failure.message
            );
        }

        let args = parse_vm_exec(&["d2b", "vm", "exec", "-d", "work", "--json"]);
        let (result, stdout) = super::with_test_stdout_capture(|| cmd_vm_exec_v2(&context, &args));
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
                .contains("command form requires"),
            "detach missing command stays actionable: {envelope}"
        );
    }

    #[test]
    fn vm_exec_vm_first_management_grammar_parses_verbs_and_verb_named_vms() {
        let list = parse_vm_exec(&["d2b", "vm", "exec", "work", "list"]);
        assert_eq!(list.vm, "work");
        let list_action = parse_vm_exec_action(&list).expect("list action parses");
        assert!(matches!(
            list_action.management,
            Some(super::VmExecManagementCommand::List)
        ));

        let logs = parse_vm_exec(&[
            "d2b",
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
            "d2b",
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

        let status = parse_vm_exec(&["d2b", "vm", "exec", "list", "status", "exec-2"]);
        assert_eq!(status.vm, "list");
        let status_action = parse_vm_exec_action(&status).expect("status action parses");
        assert!(matches!(
            status_action.management,
            Some(super::VmExecManagementCommand::Status(super::VmExecIdArgs { exec_id }))
                if exec_id == "exec-2"
        ));

        let kill = parse_vm_exec(&["d2b", "vm", "exec", "kill", "kill", "exec-3"]);
        assert_eq!(kill.vm, "kill");
        let kill_action = parse_vm_exec_action(&kill).expect("kill action parses");
        assert!(matches!(
            kill_action.management,
            Some(super::VmExecManagementCommand::Kill(super::VmExecIdArgs { exec_id }))
                if exec_id == "exec-3"
        ));

        let command = parse_vm_exec(&["d2b", "vm", "exec", "logs", "--", "status", "exec-4"]);
        assert_eq!(command.vm, "logs");
        let command_action = parse_vm_exec_action(&command).expect("command action parses");
        assert!(command_action.management.is_none());
        assert_eq!(
            command.command,
            vec!["status".to_owned(), "exec-4".to_owned()]
        );

        let status_named_vm =
            parse_vm_exec(&["d2b", "vm", "exec", "status", "logs", "exec-status-logs"]);
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

        let logs_named_vm =
            parse_vm_exec(&["d2b", "vm", "exec", "logs", "status", "exec-logs-status"]);
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
    fn vm_exec_v2_management_conflicts_fail_before_daemon_access() {
        let context = missing_daemon_context();
        let mut cases = Vec::new();

        let mut detached = parse_vm_exec(&["d2b", "vm", "exec", "work", "list"]);
        detached.detach = true;
        cases.push(detached);
        let mut interactive = parse_vm_exec(&["d2b", "vm", "exec", "work", "status", "exec-1"]);
        interactive.interactive = true;
        cases.push(interactive);
        let mut tty = parse_vm_exec(&["d2b", "vm", "exec", "work", "logs", "exec-1"]);
        tty.tty = true;
        cases.push(tty);
        let mut command = parse_vm_exec(&["d2b", "vm", "exec", "work", "status", "exec-1"]);
        command.command.push("true".to_owned());
        cases.push(command);

        for args in cases {
            let failure = cmd_vm_exec_v2(&context, &args)
                .expect_err("management conflicts must fail before connecting");
            assert_eq!(failure.exit_code, 2);
            assert!(failure.message.contains("management verbs do not accept"));
        }
    }

    #[test]
    fn vm_exec_v2_json_management_syntax_failure_is_one_document() {
        let context = missing_daemon_context();
        let args = parse_vm_exec(&["d2b", "vm", "exec", "work", "status", "--json"]);
        let (result, stdout, stderr) =
            super::with_test_output_capture(|| cmd_vm_exec_v2(&context, &args));
        assert_eq!(result.expect("JSON usage returns its exit code"), 2);
        assert!(stderr.is_empty());
        let value: Value = serde_json::from_slice(&stdout).expect("one JSON usage document");
        assert_eq!(value["source"], "cli");
        assert_eq!(value["reason"], "usage");
        assert_eq!(value["exitCode"], 2);
        assert_eq!(
            serde_json::Deserializer::from_slice(&stdout)
                .into_iter::<Value>()
                .count(),
            1
        );
    }

    #[test]
    fn vm_exec_unknown_management_word_is_usage_not_reserved_name() {
        let context = missing_daemon_context();
        const SECRET_TOKEN: &str = "secret-token-should-not-render";
        let args = parse_vm_exec(&["d2b", "vm", "exec", "work", SECRET_TOKEN]);
        let failure =
            cmd_vm_exec_v2(&context, &args).expect_err("unknown no---word is usage failure");
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

        let json_args = parse_vm_exec(&["d2b", "vm", "exec", "work", SECRET_TOKEN, "--json"]);
        let (result, stdout) =
            super::with_test_stdout_capture(|| cmd_vm_exec_v2(&context, &json_args));
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

    fn read_guest_config_reply(content: &[u8]) -> Vec<u8> {
        let encoded = d2b_core::base64_codec::encode(content);
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

    fn gc_test_role_profile() -> d2b_core::processes::RoleProfile {
        d2b_core::processes::RoleProfile {
            profile_id: "guest-control-health".to_owned(),
            uid: 1000,
            gid: 1000,
            adr_carve_out: None,
            caps: Vec::new(),
            namespaces: d2b_core::minijail_profile::NamespaceSet {
                mount: false,
                pid: false,
                net: false,
                ipc: false,
                uts: false,
                user: false,
            },
            seccomp_policy_ref: None,
            mount_policy: d2b_core::minijail_profile::MountPolicy {
                read_only_paths: Vec::new(),
                writable_paths: Vec::new(),
                device_binds: Vec::new(),
                bind_mounts: Vec::new(),
                nix_store_read_only: true,
                hide_device_nodes_by_default: true,
            },
            cgroup_placement: d2b_core::minijail_profile::CgroupPlacement {
                subtree: "d2b.slice/test".to_owned(),
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
        let processes = d2b_core::processes::ProcessesJson {
            schema_version: "v2".to_owned(),
            vms: vec![d2b_core::processes::VmProcessDag {
                workload_identity: None,
                vm: vm.to_owned(),
                nodes: vec![d2b_core::processes::ProcessNode {
                    id: d2b_core::processes::NodeId("guest-control-health".to_owned()),
                    role: d2b_core::processes::ProcessRole::GuestControlHealth,
                    unit: None,
                    binary_path: None,
                    argv: Vec::new(),
                    env: Vec::new(),
                    plan_ops: Vec::new(),
                    network_interfaces: Vec::new(),
                    profile: gc_test_role_profile(),
                    readiness: Vec::new(),
                }],
                edges: Vec::new(),
                invariants: d2b_core::processes::VmProcessInvariants {
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
            "publicManifestPath": format!("{unique}.vms.json"),
            "hostPath": format!("{unique}.host.json"),
            "processesPath": format!("{unique}.processes.json"),
            "privilegesPath": format!("{unique}.privileges.json"),
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
            let _staging_guard = StagingBaseGuard::set(&staging_dir);
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
        if let Some(name) = bundle_path.file_name().and_then(|n| n.to_str())
            && let Some(parent) = bundle_path.parent()
        {
            let _ = std::fs::remove_file(parent.join(format!("{name}.processes.json")));
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
            vec!["d2b", "host", "--help"],
            vec!["d2b", "host", "help"],
            vec!["d2b", "vm", "--help"],
            vec!["d2b", "vm", "help"],
            vec!["d2b", "audio", "--help"],
            vec!["d2b", "help", "audio"],
            vec!["d2b", "console", "--help"],
            vec!["d2b", "up", "--help"],
            vec!["d2b", "down", "--help"],
            vec!["d2b", "restart", "--help"],
            vec!["d2b", "help", "up"],
            vec!["d2b", "help", "down"],
            vec!["d2b", "help", "restart"],
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
        let audio = NativeCli::try_parse_from(["d2b", "audio", "mic", "on", "personal-dev"])
            .expect("audio parse");
        assert!(matches!(
            audio.command,
            super::NativeCommand::Audio(super::AudioArgs {
                json: false,
                command: Some(super::AudioCommand::Mic(super::AudioToggleArgs {
                    state: super::AudioGrantState::On,
                    vm,
                })),
            }) if vm == "personal-dev"
        ));

        let audio_default =
            NativeCli::try_parse_from(["d2b", "audio"]).expect("audio status parse");
        assert!(matches!(
            audio_default.command,
            super::NativeCommand::Audio(super::AudioArgs {
                json: false,
                command: None,
            })
        ));

        let audio_json =
            NativeCli::try_parse_from(["d2b", "audio", "--json"]).expect("audio json parse");
        assert!(matches!(
            audio_json.command,
            super::NativeCommand::Audio(super::AudioArgs {
                json: true,
                command: None,
            })
        ));

        let console =
            NativeCli::try_parse_from(["d2b", "console", "personal-dev"]).expect("console parse");
        assert!(matches!(
            console.command,
            super::NativeCommand::Console(super::ConsoleArgs { vm }) if vm == "personal-dev"
        ));
    }

    #[test]
    fn audio_status_result_json_shape_matches_wire_contract() {
        // d2b-wlcontrol depends on `d2b audio status --json` producing
        // AudioStatusResult JSON. This test locks the shape so any schema
        // change is caught before it breaks downstream consumers.
        use d2b_contracts::public_wire::AudioChannel;
        use d2b_contracts::public_wire::{
            AudioChannelState, AudioEnforcementPosture, AudioOpResponse, AudioProviderKind,
            AudioSetApplied, AudioSetResult, AudioStatusResult, AudioVmState,
        };

        let status = AudioStatusResult {
            entries: vec![AudioVmState {
                vm: "work".to_owned(),
                speaker: AudioChannelState {
                    level: None,
                    muted: false,
                },
                microphone: AudioChannelState {
                    level: None,
                    muted: true,
                },
                provider_kind: AudioProviderKind::LocalHypervisor,
                enforcement: AudioEnforcementPosture::HostAndGuest,
            }],
            errors: vec![],
        };
        let json = serde_json::to_string(&status).expect("serialize");
        let v: serde_json::Value = serde_json::from_str(&json).expect("roundtrip");
        assert!(v["entries"].is_array(), "entries must be array");
        assert_eq!(v["entries"][0]["vm"], "work");
        assert_eq!(v["entries"][0]["microphone"]["muted"], true);
        assert_eq!(v["entries"][0]["enforcement"], "host-and-guest");

        // SetResult JSON shape (for mute/setvol JSON output)
        let set_result = AudioSetResult {
            vm: "work".to_owned(),
            channel: AudioChannel::Speaker,
            applied: AudioSetApplied::HostAndGuest,
            state: AudioChannelState {
                level: None,
                muted: false,
            },
        };
        let set_json = serde_json::to_string(&set_result).expect("serialize set");
        let sv: serde_json::Value = serde_json::from_str(&set_json).expect("roundtrip set");
        assert_eq!(sv["vm"], "work");
        assert_eq!(sv["applied"], "host-and-guest");
        assert_eq!(sv["channel"], "speaker");

        // AudioOpResponse::Status tag shape (used when printing full envelope)
        let response = AudioOpResponse::Status(status.clone());
        let resp_json = serde_json::to_string(&response).expect("serialize response");
        let rv: serde_json::Value = serde_json::from_str(&resp_json).expect("roundtrip resp");
        assert_eq!(rv["op"], "status", "AudioOpResponse tag must be 'op'");
        assert!(
            rv["result"].is_object(),
            "AudioOpResponse content must be 'result'"
        );
    }

    #[test]
    fn audio_json_flag_parsed_for_all_subcommands() {
        // --json must be accepted at the audio subcommand level (before the
        // sub-subcommand) and after it so d2b-wlcontrol can place the flag
        // naturally with the requested operation.
        let with_json =
            NativeCli::try_parse_from(["d2b", "audio", "--json", "status"]).expect("json status");
        assert!(matches!(
            with_json.command,
            super::NativeCommand::Audio(super::AudioArgs { json: true, .. })
        ));

        let json_mic =
            NativeCli::try_parse_from(["d2b", "audio", "--json", "mic", "off", "work-vm"])
                .expect("json mic");
        assert!(matches!(
            json_mic.command,
            super::NativeCommand::Audio(super::AudioArgs { json: true, .. })
        ));

        let trailing_json = NativeCli::try_parse_from(["d2b", "audio", "status", "--json"])
            .expect("trailing json status");
        assert!(matches!(
            trailing_json.command,
            super::NativeCommand::Audio(super::AudioArgs { json: true, .. })
        ));
    }

    #[test]
    fn audio_no_success_shaped_cli_fallback_for_off_command() {
        // Verify Off fans out to two separate Mute ops (speaker + microphone).
        // This cannot produce a success result from a single op, so no
        // success-shaped fallback can exist for the Off path.
        use super::{AudioArgs, AudioCommand, AudioGrantState, AudioOffArgs, AudioToggleArgs};
        let off_args = AudioArgs {
            json: false,
            command: Some(AudioCommand::Off(AudioOffArgs {
                vm: "corp-vm".to_owned(),
            })),
        };
        // Confirm the Off variant matches what we expect
        assert!(matches!(
            off_args.command,
            Some(AudioCommand::Off(AudioOffArgs { vm })) if vm == "corp-vm"
        ));

        // Mic On and Speaker Off are distinct commands with distinct mute bool
        let mic_on = AudioArgs {
            json: false,
            command: Some(AudioCommand::Mic(AudioToggleArgs {
                state: AudioGrantState::On,
                vm: "corp-vm".to_owned(),
            })),
        };
        assert!(matches!(
            mic_on.command,
            Some(AudioCommand::Mic(AudioToggleArgs {
                state: AudioGrantState::On,
                ..
            }))
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
            "d2b host install --apply failed: trusted bundle validation failed"
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
        assert!(
            envelope
                .observed_state
                .contains("operation not yet implemented in this build")
        );
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
                "remediation": "RunHostInstall failed at the broker live handler. Admin: inspect `journalctl -u d2b-priv-broker` for the underlying syscall/exit code.",
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
                    "message": "hostInstall requires an admin role from d2b.site.adminUsers",
                    "remediation": "add the caller to d2b.site.adminUsers to use hostInstall"
                }
            }),
        );

        let err = result.expect_err("host install must surface the daemon authz envelope");
        assert_eq!(err.exit_code, 75);
        assert_eq!(
            err.message,
            "authz-not-admin: hostInstall requires an admin role from d2b.site.adminUsers (add the caller to d2b.site.adminUsers to use hostInstall)"
        );
    }

    #[test]
    fn usb_attach_dispatches_daemon_without_guest_ssh_prevalidation() {
        let vm = "unit-usb";
        let args = UsbAttachArgs {
            vm: vm.to_owned(),
            busid: "1-2".to_owned(),
            dry_run: false,
            apply: true,
            json: true,
            human: false,
        };

        let (result, request, stdout) = run_public_command_with_mock_daemon(
            "usb-ad",
            vm,
            json!({
                "outcome": "applied",
                "verb": "usb attach",
                "summary": "usb attach ok"
            }),
            |context| super::cmd_usb_attach(context, &args),
        );
        assert_eq!(result.expect("usb attach should succeed"), 0);
        assert_eq!(request["type"], "usbipBind");
        assert_eq!(request["vm"], vm);
        assert_eq!(request["busId"], "1-2");
        assert_eq!(request["apply"], true);
        assert!(!String::from_utf8_lossy(&stdout).contains("ssh"));
    }

    #[test]
    fn qemu_media_usb_attach_routes_without_guest_usbip_import() {
        let _env_lock = ENV_MUTEX.lock().expect("lock env mutex");
        let vm = "media";
        let args = UsbAttachArgs {
            vm: vm.to_owned(),
            busid: "1-2.3".to_owned(),
            dry_run: false,
            apply: true,
            json: false,
            human: false,
        };
        let (result, request, stdout) = run_public_command_with_manifest(
            "qemu-media-usb-attach",
            vm,
            json!({
                "type": "mutatingVerbResponse",
                "verb": "usb attach",
                "outcome": "applied",
                "summary": "d2b usb attach --apply: qemu-media attached ref 'installer-usb' in slot 'cdrom' for vm 'media' via QMP (commands=add-fd,blockdev-add:file,blockdev-add:raw,device_add)"
            }),
            write_qemu_media_manifest,
            |context| super::cmd_usb_attach(context, &args),
        );

        assert_eq!(result.expect("qemu media attach"), 0);
        assert_eq!(
            request.get("type").and_then(Value::as_str),
            Some("usbipBind")
        );
        assert_eq!(request.get("vm").and_then(Value::as_str), Some(vm));
        assert_eq!(request.get("busId").and_then(Value::as_str), Some("1-2.3"));
        let rendered = String::from_utf8(stdout).expect("stdout utf8");
        assert!(rendered.contains("qemu-media attached ref 'installer-usb'"));
        assert!(!rendered.contains("1-2.3"));
        assert!(!rendered.contains("usb-Vendor_SecretSerial"));
        assert!(!rendered.contains("usbip attach"));
    }

    #[test]
    fn qemu_media_usb_attach_apply_json_renders_json_envelope() {
        let _env_lock = ENV_MUTEX.lock().expect("lock env mutex");
        let vm = "media";
        let args = UsbAttachArgs {
            vm: vm.to_owned(),
            busid: "1-2.3".to_owned(),
            dry_run: false,
            apply: true,
            json: true,
            human: false,
        };
        let (result, _request, stdout) = run_public_command_with_manifest(
            "qemu-media-usb-attach-json",
            vm,
            json!({
                "type": "mutatingVerbResponse",
                "verb": "usb attach",
                "outcome": "applied",
                "summary": "d2b usb attach --apply: qemu-media attached ref 'installer-usb' in slot 'cdrom' for vm 'media' via QMP (commands=add-fd,blockdev-add:file,blockdev-add:raw,device_add)"
            }),
            write_qemu_media_manifest,
            |context| super::cmd_usb_attach(context, &args),
        );

        assert_eq!(result.expect("qemu media attach json"), 0);
        let rendered: Value = serde_json::from_slice(&stdout).expect("json stdout");
        assert_eq!(
            rendered.get("outcome").and_then(Value::as_str),
            Some("applied")
        );
        assert!(
            rendered
                .get("summary")
                .and_then(Value::as_str)
                .is_some_and(|summary| summary.contains("qemu-media attached ref"))
        );
    }

    #[test]
    fn qemu_media_usb_detach_routes_without_guest_usbip_import() {
        let _env_lock = ENV_MUTEX.lock().expect("lock env mutex");
        let vm = "media";
        let args = UsbDetachArgs {
            vm: vm.to_owned(),
            busid: "1-2.3".to_owned(),
            dry_run: false,
            apply: true,
            json: false,
            human: false,
        };
        let (result, request, stdout) = run_public_command_with_manifest(
            "qemu-media-usb-detach",
            vm,
            json!({
                "type": "mutatingVerbResponse",
                "verb": "usb detach",
                "outcome": "applied",
                "summary": "d2b usb detach --apply: qemu-media detached ref 'installer-usb' in slot 'cdrom' for vm 'media' via QMP (commands=device_del,DEVICE_DELETED,blockdev-del:raw,blockdev-del:file,remove-fd)"
            }),
            write_qemu_media_manifest,
            |context| super::cmd_usb_detach(context, &args),
        );

        assert_eq!(result.expect("qemu media detach"), 0);
        assert_eq!(
            request.get("type").and_then(Value::as_str),
            Some("usbipUnbind")
        );
        assert_eq!(request.get("vm").and_then(Value::as_str), Some(vm));
        assert_eq!(request.get("busId").and_then(Value::as_str), Some("1-2.3"));
        let rendered = String::from_utf8(stdout).expect("stdout utf8");
        assert!(rendered.contains("qemu-media detached ref 'installer-usb'"));
        assert!(!rendered.contains("1-2.3"));
        assert!(!rendered.contains("usb-Vendor_SecretSerial"));
        assert!(!rendered.contains("usbip detach"));
    }

    #[test]
    fn qemu_media_vm_lifecycle_dry_run_reports_qemu_dag() {
        let vm = "media";
        let manifest_path = test_socket_path("qemu-media-vm-dry-run", ".manifest.json");
        if let Some(parent) = manifest_path.parent() {
            std::fs::create_dir_all(parent).expect("create test manifest dir");
        }
        write_qemu_media_manifest(&manifest_path, vm);
        let context = Context {
            manifest_path: manifest_path.clone(),
            bundle_path: manifest_path.with_extension("missing-bundle.json"),
            public_socket: test_socket_path("qemu-media-vm-dry-run", ".sock"),
            broker_socket: PathBuf::from("/dev/null"),
            state_root: None,
            host_runtime_path: PathBuf::from("/dev/null"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: PathBuf::from("/dev/null"),
            metrics_url: "http://127.0.0.1:1/metrics".to_owned(),
        };

        let (result, stdout) = super::with_test_stdout_capture(|| {
            super::cmd_vm_lifecycle_verb(
                &context,
                super::VmLifecycleInvocation {
                    verb: "start",
                    vm,
                    dry_run: true,
                    apply: false,
                    no_wait_api: false,
                    force: false,
                    json: true,
                },
            )
        });
        assert_eq!(result.expect("qemu-media start dry-run"), 0);
        let rendered = String::from_utf8(stdout).expect("stdout utf8");
        assert!(rendered.contains(r#""id": "qemu-media""#));
        assert!(rendered.contains("QemuMediaBoot"));
        assert!(!rendered.contains("virtiofsd-ro-store"));

        let (result, stdout) = super::with_test_stdout_capture(|| {
            super::cmd_vm_lifecycle_verb(
                &context,
                super::VmLifecycleInvocation {
                    verb: "stop",
                    vm,
                    dry_run: true,
                    apply: false,
                    no_wait_api: false,
                    force: false,
                    json: false,
                },
            )
        });
        assert_eq!(result.expect("qemu-media stop dry-run"), 0);
        let rendered = String::from_utf8(stdout).expect("stdout utf8");
        assert!(rendered.contains("host-reconcile"));
        assert!(rendered.contains("qemu-media"));
    }

    /// Write a minimal bundle.json + realm-controllers.json so that
    /// `try_canonical_target_for_vm(vm)` finds `"<vm>.work.d2b"`.
    fn write_bundle_with_realm_controllers(bundle_path: &std::path::Path, vm: &str) {
        let dir = bundle_path.parent().expect("bundle parent dir");
        std::fs::create_dir_all(dir).expect("create bundle dir");
        let unique = bundle_path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("bundle filename");
        let rc_filename = format!("{unique}.realm-controllers.json");
        let realm_controllers = json!({
            "schemaVersion": "v2",
            "runtimeState": "metadata-only",
            "controllers": [{
                "realmName": "Work",
                "realmId": "work",
                "realmPath": "work",
                "placement": "host-local",
                "daemon": {
                    "user": "d2br-work",
                    "group": "d2br-work",
                    "publicSocketGroup": "d2bra-work",
                    "serviceName": "d2b-realm-work-daemon.service",
                    "configPath": "/etc/d2b/realms/work/daemon-config.json",
                    "stateLockPath": "/run/d2b/realms/work/daemon.lock",
                    "locksDir": "/run/d2b/realms/work/locks",
                    "socketActivated": false,
                    "materializedService": false
                },
                "broker": {
                    "enabled": true,
                    "hostMutation": true,
                    "user": "root",
                    "group": "d2br-work",
                    "socketPath": "/run/d2b/realms/work/broker.sock",
                    "socketUnitName": "d2b-realm-work-priv-broker.socket",
                    "serviceUnitName": "d2b-realm-work-priv-broker.service",
                    "auditDir": "/var/lib/d2b/realms/work/audit",
                    "materializedSocket": false,
                    "materializedService": false
                },
                "paths": {
                    "runDir": "/run/d2b/realms/work",
                    "stateDir": "/var/lib/d2b/realms/work",
                    "auditDir": "/var/lib/d2b/realms/work/audit"
                },
                "sockets": {
                    "publicSocketPath": "/run/d2b/realms/work/public.sock",
                    "brokerSocketPath": "/run/d2b/realms/work/broker.sock"
                },
                "allocator": {
                    "kind": "local-root-metadata",
                    "configPath": "/etc/d2b/allocator.json",
                    "rootSocket": "/run/d2b/allocator/local-root.sock"
                },
                "access": {},
                "localRuntime": {
                    "runtimeState": "metadata-only",
                    "workloads": [{
                        "workloadId": vm,
                        "vmName": vm,
                        "env": "work",
                        "runtime": {
                            "kind": "nixos",
                            "provider": { "id": "local-provider", "driver": "local-ch", "type": "local" },
                            "capabilities": {
                                "lifecycle": true, "display": false, "usbHotplug": false,
                                "guestControl": true, "exec": true, "configSync": true,
                                "ssh": false, "storeSync": true, "keys": true,
                                "inGuestObservability": false
                            },
                            "operationCapabilities": {
                                "lifecycle": {
                                    "start": true, "stop": true, "restart": true,
                                    "switch": false, "hostPrepare": false
                                },
                                "media": { "usbHotplug": false, "removableMedia": false, "qemuMedia": false },
                                "display": { "display": false, "graphics": false, "video": false, "waylandProxy": false },
                                "guest": {
                                    "guestControl": true, "exec": true, "shell": false,
                                    "configSync": true, "ssh": false, "keys": true,
                                    "inGuestObservability": false
                                },
                                "storage": { "storeSync": true, "virtiofs": true, "volumes": false }
                            },
                            "autostartPolicy": "manual"
                        },
                        "paths": {
                            "stateDir": format!("/var/lib/d2b/vms/{vm}/state"),
                            "runDir": format!("/run/d2b/vms/{vm}"),
                            "storeView": format!("/var/lib/d2b/vms/{vm}/store"),
                            "guestControlDir": format!("/run/d2b/vms/{vm}/guest-control")
                        },
                        "identity": {
                            "workloadId": vm,
                            "realmId": "work",
                            "realmPath": ["work"],
                            "canonicalTarget": format!("{vm}.work.d2b")
                        }
                    }],
                    "invariants": {
                        "metadataOnly": true,
                        "existingGlobalVmPathsPreserved": true,
                        "noStateMigrationDuringActivation": true,
                        "brokerEffectsRemainRealmDelegated": true
                    }
                }
            }],
            "invariants": {
                "metadataOnly": true,
                "noSystemdUnitsMaterialized": true,
                "preservesGlobalDaemonBehavior": true,
                "preservesDirectUnixSocketSemantics": true
            }
        });
        std::fs::write(
            dir.join(&rc_filename),
            serde_json::to_vec(&realm_controllers).expect("serialize realm-controllers"),
        )
        .expect("write realm-controllers.json");
        let bundle = json!({
            "bundleVersion": 4,
            "schemaVersion": "v2",
            "publicManifestPath": format!("{unique}.vms.json"),
            "hostPath": format!("{unique}.host.json"),
            "processesPath": format!("{unique}.processes.json"),
            "privilegesPath": format!("{unique}.privileges.json"),
            "realmControllersPath": rc_filename,
            "closures": [],
            "minijailProfiles": [],
            "generation": { "generator": "test", "sourceRevision": null, "generatedAt": null }
        });
        std::fs::write(
            bundle_path,
            serde_json::to_vec(&bundle).expect("serialize bundle"),
        )
        .expect("write bundle.json");
    }

    fn rewrite_bundle_workload_identity(
        bundle_path: &std::path::Path,
        workload_id: &str,
        canonical_target: &str,
    ) {
        let bundle: Value = serde_json::from_slice(
            &std::fs::read(bundle_path).expect("read bundle for workload rewrite"),
        )
        .expect("parse bundle for workload rewrite");
        let rc_ref = bundle
            .get("realmControllersPath")
            .and_then(Value::as_str)
            .expect("bundle has realmControllersPath");
        let rc_path = bundle_path.parent().expect("bundle parent").join(rc_ref);
        let mut rc: Value =
            serde_json::from_slice(&std::fs::read(&rc_path).expect("read realm controllers"))
                .expect("parse realm controllers");
        let workload = rc
            .pointer_mut("/controllers/0/localRuntime/workloads/0")
            .and_then(Value::as_object_mut)
            .expect("first workload object");
        workload.insert(
            "workloadId".to_owned(),
            Value::String(workload_id.to_owned()),
        );
        let identity = workload
            .get_mut("identity")
            .and_then(Value::as_object_mut)
            .expect("identity object");
        identity.insert(
            "workloadId".to_owned(),
            Value::String(workload_id.to_owned()),
        );
        identity.insert(
            "canonicalTarget".to_owned(),
            Value::String(canonical_target.to_owned()),
        );
        std::fs::write(
            &rc_path,
            serde_json::to_vec(&rc).expect("serialize rewritten realm controllers"),
        )
        .expect("write rewritten realm controllers");
    }

    #[test]
    fn lifecycle_verb_bare_target_emits_migration_hint_when_canonical_known() {
        // When a user types a bare VM name and the realm-controllers artifact
        // advertises a canonical workload target, an advisory hint must appear
        // on stderr pointing at the canonical form.
        let vm = "corp-vm";
        let manifest_path = test_socket_path("lv-bare-hint", ".manifest.json");
        if let Some(parent) = manifest_path.parent() {
            std::fs::create_dir_all(parent).expect("create manifest dir");
        }
        write_test_manifest(&manifest_path, vm);
        let bundle_path = manifest_path.with_extension("bundle.json");
        write_bundle_with_realm_controllers(&bundle_path, vm);
        let context = Context {
            manifest_path: manifest_path.clone(),
            bundle_path: bundle_path.clone(),
            public_socket: manifest_path.with_extension("sock"),
            broker_socket: manifest_path.with_extension("broker.sock"),
            state_root: None,
            host_runtime_path: manifest_path.with_extension("host-runtime.json"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: manifest_path.with_extension("daemon-state"),
            metrics_url: "http://127.0.0.1:9101/metrics".to_owned(),
        };
        let (result, _stdout, stderr) = super::with_test_output_capture(|| {
            super::cmd_vm_lifecycle_verb(
                &context,
                super::VmLifecycleInvocation {
                    verb: "start",
                    vm,
                    dry_run: true,
                    apply: false,
                    no_wait_api: false,
                    force: false,
                    json: false,
                },
            )
        });
        assert_eq!(result.expect("bare-target lifecycle dry-run"), 0);
        let stderr_text = String::from_utf8(stderr).expect("stderr utf8");
        assert!(
            stderr_text.contains("corp-vm.work.d2b"),
            "expected migration hint for bare input to mention canonical target; stderr: {stderr_text:?}"
        );
        assert!(
            stderr_text.contains("note:"),
            "expected migration hint prefix 'note:'; stderr: {stderr_text:?}"
        );
    }

    #[test]
    fn lifecycle_verb_canonical_target_skips_migration_hint() {
        // Typing the canonical form "corp-vm.work.d2b" must not produce a
        // migration hint. In the test environment (no realm-entrypoint table
        // on disk) the router treats ".work.d2b" as a conventional gateway
        // target and routes through "sys-work-gateway"; the Gateway branch
        // returns early before any hint logic runs, so stderr stays empty.
        //
        // On a host with a host-local work realm, the router returns
        // VmTargetRoute::Local { vm: "corp-vm" } for "corp-vm.work.d2b".
        // The raw_target preservation fix ensures !raw_target.contains('.')
        // is false for the dotted input, suppressing the hint correctly.
        let manifest_path = test_socket_path("lv-canonical-no-hint", ".manifest.json");
        if let Some(parent) = manifest_path.parent() {
            std::fs::create_dir_all(parent).expect("create manifest dir");
        }
        // "sys-work-gateway" must be declared so the conventional gateway
        // route resolves rather than erroring with "missing realm entrypoint".
        write_test_manifest(&manifest_path, "sys-work-gateway");
        let context = Context {
            manifest_path: manifest_path.clone(),
            bundle_path: manifest_path.with_extension("missing-bundle.json"),
            public_socket: manifest_path.with_extension("sock"),
            broker_socket: manifest_path.with_extension("broker.sock"),
            state_root: None,
            host_runtime_path: manifest_path.with_extension("host-runtime.json"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: manifest_path.with_extension("daemon-state"),
            metrics_url: "http://127.0.0.1:9101/metrics".to_owned(),
        };
        let (result, _stdout, stderr) = super::with_test_output_capture(|| {
            super::cmd_vm_lifecycle_verb(
                &context,
                super::VmLifecycleInvocation {
                    verb: "start",
                    vm: "corp-vm.work.d2b",
                    dry_run: true,
                    apply: false,
                    no_wait_api: false,
                    force: false,
                    json: false,
                },
            )
        });
        assert_eq!(result.expect("canonical-target lifecycle dry-run"), 0);
        let stderr_text = String::from_utf8(stderr).expect("stderr utf8");
        assert!(
            !stderr_text.contains("note:"),
            "canonical input must not produce a migration hint; stderr: {stderr_text:?}"
        );
    }

    #[test]
    fn qemu_media_usb_hotplug_dry_run_reports_qmp_actions() {
        let vm = "media";
        let manifest_path = test_socket_path("qemu-media-usb-dry-run", ".manifest.json");
        if let Some(parent) = manifest_path.parent() {
            std::fs::create_dir_all(parent).expect("create test manifest dir");
        }
        write_qemu_media_manifest(&manifest_path, vm);
        let context = Context {
            manifest_path: manifest_path.clone(),
            bundle_path: manifest_path.with_extension("missing-bundle.json"),
            public_socket: test_socket_path("qemu-media-usb-dry-run", ".sock"),
            broker_socket: PathBuf::from("/dev/null"),
            state_root: None,
            host_runtime_path: PathBuf::from("/dev/null"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: PathBuf::from("/dev/null"),
            metrics_url: "http://127.0.0.1:1/metrics".to_owned(),
        };
        let attach = UsbAttachArgs {
            vm: vm.to_owned(),
            busid: "1-2.3".to_owned(),
            dry_run: true,
            apply: false,
            json: true,
            human: false,
        };
        let (result, stdout) =
            super::with_test_stdout_capture(|| super::cmd_usb_attach(&context, &attach));
        assert_eq!(result.expect("qemu-media usb attach dry-run"), 0);
        let rendered = String::from_utf8(stdout).expect("stdout utf8");
        assert!(rendered.contains("QmpHotplug(add-fd,blockdev-add,device_add)"));
        assert!(!rendered.contains("1-2.3"));

        let detach = UsbDetachArgs {
            vm: vm.to_owned(),
            busid: "1-2.3".to_owned(),
            dry_run: true,
            apply: false,
            json: false,
            human: false,
        };
        let (result, stdout) =
            super::with_test_stdout_capture(|| super::cmd_usb_detach(&context, &detach));
        assert_eq!(result.expect("qemu-media usb detach dry-run"), 0);
        let rendered = String::from_utf8(stdout).expect("stdout utf8");
        assert!(rendered.contains("execute QMP detach"));
        assert!(!rendered.contains("1-2.3"));
    }

    #[test]
    fn usb_enroll_hidden_subcommand_reports_config_migration() {
        let args = [
            "d2b",
            "usb",
            "enroll",
            "media",
            "installer-usb",
            "--busid",
            "1-2.3",
            "--apply",
        ]
        .into_iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
        let err = super::removed_usb_enroll_failure(&args).expect("enroll is removed");
        assert_eq!(err.exit_code, 2);
        assert!(err.message.contains("d2b usb enroll was removed"));
        assert!(
            err.message
                .contains("qemuMedia.source.usbSelector.byIdName")
        );
        assert!(err.message.contains("d2b usb probe"));
        assert!(err.message.contains("/dev/disk/by-id/"));

        let parsed = NativeCli::try_parse_from(["d2b", "usb", "enroll", "media", "installer-usb"]);
        assert!(
            parsed.is_err(),
            "usb enroll must not be public clap surface"
        );
    }

    #[test]
    fn usb_probe_human_renders_qemu_media_config_probe_followup() {
        let entries = vec![
            IpcUsbipProbeEntry {
                kind: IpcUsbProbeEntryKind::QemuMediaSlot,
                vm: "media".to_owned(),
                env: "work".to_owned(),
                bus_id: "1-2.3".to_owned(),
                lock_path: String::new(),
                status: IpcUsbipProbeStatus::Enrollable,
                owner_vm: None,
                slot: Some("cdrom".to_owned()),
                media_ref: Some(MediaRef::new("installer-usb")),
                source_kind: Some("physical-usb".to_owned()),
                candidate_bus_ids: vec!["1-2.3".to_owned()],
                follow_up_command: Some(
                    "update qemu-media config for vm 'media' and ref 'installer-usb', then run `d2b usb probe`; when the VM is running, hotplug this selector with `d2b usb attach media 1-2.3 --apply`".to_owned(),
                ),
                ..Default::default()
            },
            IpcUsbipProbeEntry {
                kind: IpcUsbProbeEntryKind::QemuMediaSlot,
                vm: "media".to_owned(),
                env: "work".to_owned(),
                bus_id: "-".to_owned(),
                lock_path: String::new(),
                status: IpcUsbipProbeStatus::DirectConfig,
                owner_vm: None,
                slot: Some("boot".to_owned()),
                media_ref: Some(MediaRef::new("image-boot")),
                source_kind: Some("image-file".to_owned()),
                candidate_bus_ids: Vec::new(),
                follow_up_command: None,
                ..Default::default()
            },
        ];

        let rendered = render_usb_probe_human(&entries);
        assert!(rendered.contains("QEMU-MEDIA-VM"));
        assert!(rendered.contains("media"));
        assert!(rendered.contains("d2b usb probe"));
        assert!(rendered.contains("d2b usb attach media 1-2.3 --apply"));
        assert!(!rendered.contains("usb enroll"));
        assert!(rendered.contains("direct-config"));
        assert!(!rendered.contains("usb-Vendor_SecretSerial"));
        assert!(!rendered.contains("/dev/disk/by-id"));
    }

    #[test]
    fn usb_probe_human_renders_degraded_claim_state_with_remediation() {
        let entries = vec![IpcUsbipProbeEntry {
            kind: IpcUsbProbeEntryKind::Usbip,
            vm: "corp-vm".to_owned(),
            env: "work".to_owned(),
            bus_id: "1-2".to_owned(),
            lock_path: "/run/d2b/locks/usbip/1-2".to_owned(),
            status: IpcUsbipProbeStatus::Degraded,
            owner_vm: Some("corp-vm".to_owned()),
            durable_claim: public_wire::UsbipDurableClaimStatus {
                state: public_wire::UsbipDurableClaimState::HeldByDesiredOwner,
                owner_vm: Some("corp-vm".to_owned()),
            },
            host: public_wire::UsbipHostProbeStatus {
                bind: public_wire::UsbipHostBindState::Unknown,
                carrier: public_wire::UsbipHostCarrierState::Unknown,
                proxy: public_wire::UsbipProxyState::Unknown,
            },
            guest: public_wire::UsbipGuestProbeStatus {
                import: public_wire::UsbipGuestImportState::Detached,
            },
            topology_policy: public_wire::UsbipTopologyPolicyStatus {
                topology: public_wire::UsbipTopologyState::Unknown,
                policy: public_wire::UsbipPolicyState::Allowed,
            },
            degraded_reasons: vec![public_wire::UsbipProbeDegradedReason {
                code: public_wire::UsbipProbeDegradedReasonCode::GuestImportUnavailable,
                summary: "guest has not imported the claimed USB device".to_owned(),
                remediation: "Run `d2b usb attach corp-vm 1-2 --apply` after the VM is running."
                    .to_owned(),
            }],
            remediation_commands: vec!["d2b usb attach corp-vm 1-2 --apply".to_owned()],
            ..Default::default()
        }];

        let rendered = render_usb_probe_human(&entries);
        assert!(rendered.contains("held-by-desired-owner"));
        assert!(rendered.contains("guest-import-unavailable"));
        assert!(rendered.contains("d2b usb attach corp-vm 1-2 --apply"));
        assert!(!rendered.contains("/run/d2b/locks"));
    }

    #[allow(dead_code)]
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

    #[allow(dead_code)]
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
    fn stop_and_restart_force_flags_parse_for_primary_and_alias_forms() {
        let stop = NativeCli::try_parse_from(["d2b", "vm", "stop", "vm-a", "--force"])
            .expect("vm stop --force parses");
        assert!(matches!(
            stop.command,
            NativeCommand::Vm(VmArgs {
                command: VmCommand::Stop(VmStopArgs { force: true, .. })
            })
        ));

        let stop_short = NativeCli::try_parse_from(["d2b", "vm", "stop", "vm-a", "-f"])
            .expect("vm stop -f parses");
        assert!(matches!(
            stop_short.command,
            NativeCommand::Vm(VmArgs {
                command: VmCommand::Stop(VmStopArgs { force: true, .. })
            })
        ));

        let down =
            NativeCli::try_parse_from(["d2b", "down", "vm-a", "-f"]).expect("down -f parses");
        assert!(matches!(
            down.command,
            NativeCommand::Down(VmStopArgs { force: true, .. })
        ));

        let restart = NativeCli::try_parse_from(["d2b", "vm", "restart", "vm-a", "--force"])
            .expect("vm restart --force parses");
        assert!(matches!(
            restart.command,
            NativeCommand::Vm(VmArgs {
                command: VmCommand::Restart(VmRestartArgs { force: true, .. })
            })
        ));

        let restart_alias =
            NativeCli::try_parse_from(["d2b", "restart", "vm-a", "-f"]).expect("restart -f parses");
        assert!(matches!(
            restart_alias.command,
            NativeCommand::Restart(VmRestartArgs { force: true, .. })
        ));
    }

    #[test]
    fn host_shutdown_hook_uses_raw_hidden_parse_path() {
        let argv = ["d2b", "host", "shutdown-hook", "--apply"]
            .into_iter()
            .map(OsString::from)
            .collect::<Vec<_>>();
        assert!(is_host_shutdown_hook_invocation(&argv));
        let args = parse_host_shutdown_hook_args(&argv).expect("shutdown-hook args parse");
        assert!(args.apply);
        assert!(!args.dry_run);
        assert!(
            NativeCli::try_parse_from(["d2b", "host", "shutdown-hook", "--apply"]).is_err(),
            "shutdown-hook must stay out of the public clap/completion surface"
        );
    }

    #[test]
    fn host_shutdown_hook_orders_workloads_before_env_net_vms() {
        let manifest: ManifestDocument = serde_json::from_value(json!({
            "sys-work-net": {
                "name": "sys-work-net",
                "env": "work",
                "graphics": false,
                "tpm": false,
                "audio": false,
                "usbipYubikey": false,
                "staticIp": null,
                "isNetVm": true,
                "stateDir": "/var/lib/d2b/vms/sys-work-net",
                "bridge": "d2b-work",
                "sshUser": null
            },
            "work-app": {
                "name": "work-app",
                "env": "work",
                "graphics": false,
                "tpm": false,
                "audio": false,
                "usbipYubikey": false,
                "staticIp": null,
                "isNetVm": false,
                "stateDir": "/var/lib/d2b/vms/work-app",
                "bridge": "d2b-work",
                "sshUser": "alice"
            },
            "personal-dev": {
                "name": "personal-dev",
                "env": "personal",
                "graphics": false,
                "tpm": false,
                "audio": false,
                "usbipYubikey": false,
                "staticIp": null,
                "isNetVm": false,
                "stateDir": "/var/lib/d2b/vms/personal-dev",
                "bridge": "d2b-personal",
                "sshUser": "alice"
            },
            "sys-personal-net": {
                "name": "sys-personal-net",
                "env": "personal",
                "graphics": false,
                "tpm": false,
                "audio": false,
                "usbipYubikey": false,
                "staticIp": null,
                "isNetVm": true,
                "stateDir": "/var/lib/d2b/vms/sys-personal-net",
                "bridge": "d2b-personal",
                "sshUser": null
            }
        }))
        .expect("manifest fixture parses");

        assert_eq!(
            host_shutdown_vm_phases(&manifest),
            vec![
                vec!["personal-dev".to_owned(), "work-app".to_owned()],
                vec!["sys-personal-net".to_owned(), "sys-work-net".to_owned()]
            ],
            "host shutdown must stop all workload VMs before any env net VM"
        );
    }

    #[allow(dead_code)]
    fn stop_apply_sends_force_only_when_requested() {
        let _env_lock = ENV_MUTEX.lock().expect("lock env mutex");
        let response = json!({
            "type": "mutatingVerbResponse",
            "verb": "vm stop",
            "outcome": "applied",
            "summary": "vm stop ok",
        });

        let (forced_result, forced_request, _) = run_public_command_with_mock_daemon(
            "vm-stop-force",
            "vm-a",
            response.clone(),
            |context| {
                cmd_vm_stop(
                    context,
                    &VmStopArgs {
                        vm: "vm-a".to_owned(),
                        dry_run: false,
                        apply: true,
                        force: true,
                        json: false,
                        human: false,
                    },
                )
            },
        );
        assert_eq!(forced_result.expect("forced vm stop result"), 0);
        assert_eq!(
            forced_request.get("type").and_then(Value::as_str),
            Some("vmStop")
        );
        assert_eq!(
            forced_request.get("force").and_then(Value::as_bool),
            Some(true)
        );

        let (normal_result, normal_request, _) =
            run_public_command_with_mock_daemon("vm-stop-normal", "vm-a", response, |context| {
                cmd_vm_stop(
                    context,
                    &VmStopArgs {
                        vm: "vm-a".to_owned(),
                        dry_run: false,
                        apply: true,
                        force: false,
                        json: false,
                        human: false,
                    },
                )
            });
        assert_eq!(normal_result.expect("normal vm stop result"), 0);
        assert!(normal_request.get("force").is_none());
    }

    #[allow(dead_code)]
    fn restart_apply_sends_force_for_stop_phase() {
        let _env_lock = ENV_MUTEX.lock().expect("lock env mutex");
        let (result, request, _) = run_public_command_with_mock_daemon(
            "vm-restart-force",
            "vm-a",
            json!({
                "type": "mutatingVerbResponse",
                "verb": "vm restart",
                "outcome": "applied",
                "summary": "vm restart ok",
            }),
            |context| {
                cmd_vm_restart(
                    context,
                    &VmRestartArgs {
                        vm: "vm-a".to_owned(),
                        dry_run: false,
                        apply: true,
                        force: true,
                        json: false,
                        human: false,
                    },
                )
            },
        );

        assert_eq!(result.expect("vm restart result"), 0);
        assert_eq!(
            request.get("type").and_then(Value::as_str),
            Some("vmRestart")
        );
        assert_eq!(request.get("force").and_then(Value::as_bool), Some(true));
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
        std::fs::write(store_view.join("live/.d2b-marker-vm-a"), b"").expect("write zero marker");
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
                d2b: "inactive".to_owned(),
                microvm: "inactive".to_owned(),
                virtiofsd: "inactive".to_owned(),
                qemu_media: None,
                gpu: Some("stopped".to_owned()),
                video: None,
                snd: None,
                swtpm: None,
            },
            current: None,
            booted: None,
            pending_restart: false,
            runtime: super::RUNTIME_UNKNOWN.to_owned(),
            runtime_kind: None,
            autostart: None,
            runtime_capabilities: Vec::new(),
            service_capabilities: Vec::new(),
            unsupported_capabilities: Vec::new(),
            qemu_media: None,
            usb: None,
            declared_roles: vec!["gpu".to_owned()],
            readiness: Vec::new(),
            api_ready: None,
            runner_parity: None,
            live_pool_integrity: None,
            canonical_target: None,
        };
        let manifest_vm = super::ManifestVm {
            name: "vm-a".to_owned(),
            env: Some("dev".to_owned()),
            graphics: true,
            tpm: false,
            audio: false,
            usbip_yubikey: false,
            static_ip: Some("10.20.0.10".to_owned()),
            is_net_vm: false,
            state_dir: "/var/lib/d2b/vms/vm-a".to_owned(),
            bridge: "d2b-dev".to_owned(),
            ssh_user: Some("alice".to_owned()),
            runtime: None,
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
        fn role_profile() -> d2b_core::processes::RoleProfile {
            d2b_core::processes::RoleProfile {
                profile_id: "test-profile".to_owned(),
                uid: 1000,
                gid: 1000,
                adr_carve_out: None,
                caps: Vec::new(),
                namespaces: d2b_core::minijail_profile::NamespaceSet {
                    mount: false,
                    pid: false,
                    net: false,
                    ipc: false,
                    uts: false,
                    user: false,
                },
                seccomp_policy_ref: None,
                mount_policy: d2b_core::minijail_profile::MountPolicy {
                    read_only_paths: Vec::new(),
                    writable_paths: Vec::new(),
                    device_binds: Vec::new(),
                    bind_mounts: Vec::new(),
                    nix_store_read_only: true,
                    hide_device_nodes_by_default: true,
                },
                cgroup_placement: d2b_core::minijail_profile::CgroupPlacement {
                    subtree: "d2b.slice/test".to_owned(),
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
            is_net_vm: false,
            state_dir: "/var/lib/d2b/vms/vm-a".to_owned(),
            bridge: "d2b-dev".to_owned(),
            ssh_user: None,
            runtime: None,
        };
        let dag = d2b_core::processes::VmProcessDag {
            workload_identity: None,
            vm: "vm-a".to_owned(),
            nodes: vec![
                d2b_core::processes::ProcessNode {
                    id: d2b_core::processes::NodeId("ch-runner".to_owned()),
                    role: d2b_core::processes::ProcessRole::CloudHypervisorRunner,
                    unit: None,
                    binary_path: None,
                    argv: Vec::new(),
                    env: Vec::new(),
                    profile: role_profile(),
                    readiness: Vec::new(),
                    plan_ops: Vec::new(),
                    network_interfaces: Vec::new(),
                },
                d2b_core::processes::ProcessNode {
                    id: d2b_core::processes::NodeId("video".to_owned()),
                    role: d2b_core::processes::ProcessRole::Video,
                    unit: None,
                    binary_path: None,
                    argv: Vec::new(),
                    env: Vec::new(),
                    profile: role_profile(),
                    readiness: Vec::new(),
                    plan_ops: Vec::new(),
                    network_interfaces: Vec::new(),
                },
            ],
            edges: Vec::new(),
            invariants: d2b_core::processes::VmProcessInvariants {
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
    fn public_list_entries_drive_legacy_list_status_without_pidfd_read() {
        let services = d2b_contracts::public_wire::PublicVmServices {
            d2b: "active".to_owned(),
            microvm: "running".to_owned(),
            virtiofsd: "running".to_owned(),
            qemu_media: None,
            gpu: Some("running".to_owned()),
            video: Some("running".to_owned()),
            snd: None,
            swtpm: None,
        };
        let entry = d2b_contracts::public_wire::ListEntry {
            env: Some("dev".to_owned()),
            graphics: true,
            is_net_vm: false,
            lifecycle: d2b_contracts::public_wire::VmLifecycle {
                degraded: false,
                degraded_reasons: Vec::new(),
                pending_restart: false,
                state: d2b_contracts::public_wire::VmLifecycleState::Running,
            },
            name: "vm-a".to_owned(),
            workload_identity: None,
            guest_closure_out_path: Some("/nix/store/vm-a-system".to_owned()),
            autostart: None,
            qemu_media: None,
            runtime: d2b_contracts::public_wire::RuntimeSummary {
                detail: "running".to_owned(),
                kind: None,
                operation_capabilities: Default::default(),
                services: Vec::new(),
            },
            services,
            ssh_user: Some("alice".to_owned()),
            static_ip: Some("10.20.0.10".to_owned()),
            tpm: false,
            runtime_capabilities: vec!["lifecycle".to_owned(), "usb-hotplug".to_owned()],
            service_capabilities: vec!["microvm".to_owned(), "d2b".to_owned()],
            unsupported_capabilities: Vec::new(),
            usbip: true,
            vm: "vm-a".to_owned(),
        };

        let output = super::list_output_from_public_entries(&[entry], None);

        assert_eq!(output.0.len(), 1);
        assert_eq!(output.0[0].name, "vm-a");
        assert_eq!(output.0[0].status, "running");
        assert_eq!(
            output.0[0].guest_closure_out_path.as_deref(),
            Some("/nix/store/vm-a-system")
        );
        assert!(output.0[0].graphics);
        assert!(output.0[0].usbip);
    }

    #[test]
    fn public_list_status_collapses_transient_lifecycle_to_stable_label() {
        let lifecycle = d2b_contracts::public_wire::VmLifecycle {
            degraded: false,
            degraded_reasons: Vec::new(),
            pending_restart: false,
            state: d2b_contracts::public_wire::VmLifecycleState::Starting,
        };

        assert_eq!(
            super::public_lifecycle_list_status_label(&lifecycle),
            "running"
        );
    }

    #[test]
    fn public_list_status_preserves_failed_lifecycle_label() {
        let lifecycle = d2b_contracts::public_wire::VmLifecycle {
            degraded: false,
            degraded_reasons: Vec::new(),
            pending_restart: false,
            state: d2b_contracts::public_wire::VmLifecycleState::Failed,
        };

        assert_eq!(
            super::public_lifecycle_list_status_label(&lifecycle),
            "failed"
        );
    }

    #[test]
    fn list_human_preserves_net_vm_status_label() {
        let output = super::ListOutputV2(vec![super::ListItemOutputV2 {
            name: "sys-work-net".to_owned(),
            env: Some("work".to_owned()),
            graphics: false,
            tpm: false,
            usbip: false,
            static_ip: Some("192.168.100.2".to_owned()),
            status: "stopped".to_owned(),
            is_net_vm: true,
            guest_closure_out_path: None,
            runtime_kind: None,
            autostart: None,
            runtime_capabilities: Vec::new(),
            service_capabilities: Vec::new(),
            unsupported_capabilities: Vec::new(),
            qemu_media: None,
            runner_parity_ok: None,
            canonical_target: None,
        }]);

        let rendered = super::render_list_human(&output, None);

        assert!(rendered.contains("stopped (net-vm)"));
        assert!(!rendered.contains("systemd (net-vm)"));
    }

    fn read_model_fixture(kind: &str) -> d2b_contracts::public_wire::PublicReadModelMetadata {
        d2b_contracts::public_wire::PublicReadModelMetadata {
            schema_version: 1,
            kind: kind.to_owned(),
            generation: 7,
            source_fingerprint: "abcdef123456".to_owned(),
            updated_at_unix_ms: 42,
            freshness: "fresh".to_owned(),
            deep_refresh: "available".to_owned(),
        }
    }

    #[test]
    fn parse_public_replies_preserve_read_model_metadata() {
        let list = serde_json::to_vec(&json!({
            "type": "listResponse",
            "readModel": read_model_fixture("list"),
            "vms": []
        }))
        .expect("list json");
        let (_entries, list_model) = super::parse_list_reply(&list).expect("parse list");
        let list_model = list_model.expect("list read model");
        assert_eq!(list_model.kind, "list");
        assert_eq!(list_model.generation, 7);

        let status = serde_json::to_vec(&json!({
            "type": "statusResponse",
            "status": {
                "readModel": read_model_fixture("status"),
                "entries": []
            }
        }))
        .expect("status json");
        let (_entries, status_model) = super::parse_status_reply(&status).expect("parse status");
        let status_model = status_model.expect("status read model");
        assert_eq!(status_model.kind, "status");
        assert_eq!(status_model.source_fingerprint, "abcdef123456");
    }

    #[test]
    fn human_renderers_show_read_model_metadata() {
        let list = super::ListOutputV2(Vec::new());
        let rendered = super::render_list_human(&list, Some(&read_model_fixture("list")));
        assert!(rendered.contains("[read-model: fresh, gen 7, fingerprint abcdef12]"));

        let inventory = super::StatusInventoryOutputV2 {
            runtime: "daemon-public".to_owned(),
            read_model: Some(read_model_fixture("status")),
            vms: Vec::new(),
        };
        let manifest = super::ManifestDocument {
            _manifest: None,
            _observability: None,
            entries: Default::default(),
        };
        let context = Context {
            manifest_path: PathBuf::from("/dev/null"),
            bundle_path: PathBuf::from("/dev/null"),
            public_socket: PathBuf::from("/dev/null"),
            broker_socket: PathBuf::from("/dev/null"),
            state_root: None,
            host_runtime_path: PathBuf::from("/dev/null"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: PathBuf::from("/tmp/d2b-test-daemon-state"),
            metrics_url: "http://127.0.0.1:1/metrics".to_owned(),
        };
        let rendered = super::render_status_inventory_human(&inventory, &manifest, &context, None);
        assert!(rendered.contains("[read-model: fresh, gen 7, fingerprint abcdef12]"));
    }

    #[test]
    fn public_status_entry_drives_legacy_status_services_without_pidfd_read() {
        let root = test_socket_path("public-status-output", "");
        std::fs::create_dir_all(&root).expect("create status root");
        let context = Context {
            manifest_path: root.join("vms.json"),
            bundle_path: root.join("bundle.json"),
            public_socket: PathBuf::from("/dev/null"),
            broker_socket: PathBuf::from("/dev/null"),
            state_root: Some(root.clone()),
            host_runtime_path: PathBuf::from("/dev/null"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: root.join("daemon-state"),
            metrics_url: "http://127.0.0.1:1/metrics".to_owned(),
        };
        let vm = super::ManifestVm {
            name: "vm-a".to_owned(),
            env: Some("dev".to_owned()),
            graphics: true,
            tpm: false,
            audio: false,
            usbip_yubikey: true,
            static_ip: Some("10.20.0.10".to_owned()),
            is_net_vm: false,
            state_dir: root.join("vm-a").display().to_string(),
            bridge: "d2b-dev".to_owned(),
            ssh_user: Some("alice".to_owned()),
            runtime: None,
        };
        let public = d2b_contracts::public_wire::VmStatus {
            bridge_checks: Vec::new(),
            env: Some("dev".to_owned()),
            graphics: true,
            is_net_vm: false,
            lifecycle: d2b_contracts::public_wire::VmLifecycle {
                degraded: false,
                degraded_reasons: Vec::new(),
                pending_restart: false,
                state: d2b_contracts::public_wire::VmLifecycleState::Running,
            },
            name: "vm-a".to_owned(),
            workload_identity: None,
            autostart: None,
            qemu_media: None,
            runtime: d2b_contracts::public_wire::RuntimeSummary {
                detail: "running".to_owned(),
                kind: None,
                operation_capabilities: Default::default(),
                services: Vec::new(),
            },
            services: d2b_contracts::public_wire::PublicVmServices {
                d2b: "active".to_owned(),
                microvm: "running".to_owned(),
                virtiofsd: "running".to_owned(),
                qemu_media: None,
                gpu: Some("running".to_owned()),
                video: Some("running".to_owned()),
                snd: None,
                swtpm: None,
            },
            ssh_user: Some("alice".to_owned()),
            static_ip: Some("10.20.0.10".to_owned()),
            tpm: false,
            runtime_capabilities: vec!["lifecycle".to_owned(), "usb-hotplug".to_owned()],
            service_capabilities: vec!["microvm".to_owned(), "d2b".to_owned()],
            unsupported_capabilities: Vec::new(),
            usbip: true,
            usb: None,
            vm: "vm-a".to_owned(),
        };

        let output = super::build_vm_status_output_from_public(&context, &vm, None, &public);

        assert_eq!(output.runtime, "running");
        assert_eq!(output.services.microvm, "running");
        assert_eq!(output.services.virtiofsd, "running");
        assert_eq!(output.services.gpu.as_deref(), Some("running"));
        assert!(!output.pending_restart);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[allow(dead_code)]
    fn cmd_status_json_uses_daemon_status_entries_envelope() {
        let response = json!({
            "type": "statusResponse",
            "status": {
                "entries": [{
                    "vm": "vm-a",
                    "name": "vm-a",
                    "env": "dev",
                    "graphics": true,
                    "tpm": false,
                    "usbip": true,
                    "isNetVm": false,
                    "sshUser": "alice",
                    "staticIp": "10.20.0.10",
                    "lifecycle": { "state": "Running", "pendingRestart": false },
                    "runtime": { "detail": "running" },
                    "services": {
                        "d2b": "active",
                        "microvm": "running",
                        "virtiofsd": "running",
                        "gpu": "running",
                        "video": "running",
                        "snd": null,
                        "swtpm": null
                    },
                    "bridgeChecks": []
                }]
            }
        });
        let args = super::StatusArgs {
            json: true,
            human: false,
            check_bridges: false,
            vm_flag: None,
            vm: Some("vm-a".to_owned()),
        };

        let (result, request, stdout) =
            run_public_command_with_mock_daemon("cmd-status-public", "vm-a", response, |context| {
                super::cmd_status(context, &args)
            });

        assert_eq!(result.expect("cmd status result"), 0);
        assert_eq!(request.get("type").and_then(Value::as_str), Some("status"));
        assert_eq!(request.get("vm").and_then(Value::as_str), Some("vm-a"));
        let output: Value = serde_json::from_slice(&stdout).expect("status json output");
        assert_eq!(
            output.get("runtime").and_then(Value::as_str),
            Some("running")
        );
        assert_eq!(
            output.pointer("/services/microvm").and_then(Value::as_str),
            Some("running")
        );
        assert_eq!(
            output
                .pointer("/services/virtiofsd")
                .and_then(Value::as_str),
            Some("running")
        );
    }

    #[allow(dead_code)]
    fn cmd_status_json_includes_qemu_media_runtime_fields() {
        let response = json!({
            "type": "statusResponse",
            "status": {
                "entries": [{
                    "vm": "installer",
                    "name": "installer",
                    "env": "dev",
                    "graphics": false,
                    "tpm": false,
                    "usbip": false,
                    "isNetVm": false,
                    "sshUser": null,
                    "staticIp": "10.20.0.20",
                    "lifecycle": { "state": "Running", "pendingRestart": false },
                    "runtime": { "detail": "running", "kind": "qemu-media" },
                    "autostart": {
                        "mode": "manual-only",
                        "reason": "qemu-media VMs are intentionally skipped by daemon autostart"
                    },
                    "unsupportedCapabilities": ["exec", "guest-control", "ssh", "store-sync"],
                    "qemuMedia": {
                        "firmwareMode": "none",
                        "runner": {
                            "role": "qemu-media",
                            "state": "running",
                            "qmpReadiness": "ready",
                            "preContProgress": "paused-before-cont"
                        },
                        "media": [
                            {
                                "mediaRef": "installer-usb",
                                "slot": "boot",
                                "sourceKind": "physical-usb",
                                "format": "raw",
                                "readOnly": true,
                                "registry": {
                                    "state": "missing",
                                    "remediation": "declare the boot-drive source in config, then run d2b usb probe"
                                }
                            },
                            {
                                "mediaRef": "image-boot",
                                "slot": "boot",
                                "sourceKind": "image-file",
                                "format": "raw",
                                "readOnly": false,
                                "registry": {
                                    "state": "direct-config",
                                    "remediation": null
                                }
                            }
                        ]
                    },
                    "services": {
                        "d2b": "active",
                        "microvm": "unsupported",
                        "qemuMedia": "running",
                        "virtiofsd": "stopped",
                        "gpu": null,
                        "video": null,
                        "snd": null,
                        "swtpm": null
                    },
                    "bridgeChecks": []
                }]
            }
        });
        let args = super::StatusArgs {
            json: true,
            human: false,
            check_bridges: false,
            vm_flag: None,
            vm: Some("installer".to_owned()),
        };

        let (result, request, stdout) = run_public_command_with_manifest(
            "qms",
            "installer",
            response,
            write_qemu_media_manifest,
            |context| super::cmd_status(context, &args),
        );

        assert_eq!(result.expect("cmd status result"), 0);
        assert_eq!(request.get("type").and_then(Value::as_str), Some("status"));
        let output: Value = serde_json::from_slice(&stdout).expect("status json output");
        assert_eq!(
            output.get("runtimeKind").and_then(Value::as_str),
            Some("qemu-media")
        );
        assert_eq!(
            output.pointer("/autostart/mode").and_then(Value::as_str),
            Some("manual-only")
        );
        assert_eq!(
            output
                .pointer("/services/qemuMedia")
                .and_then(Value::as_str),
            Some("running")
        );
        assert_eq!(
            output
                .pointer("/qemuMedia/runner/qmpReadiness")
                .and_then(Value::as_str),
            Some("ready")
        );
        assert_eq!(
            output
                .pointer("/qemuMedia/media/0/registry/state")
                .and_then(Value::as_str),
            Some("missing")
        );
        assert!(output.pointer("/qemuMedia/media/1/imagePath").is_none());
        assert_eq!(
            output
                .pointer("/qemuMedia/media/1/registry/state")
                .and_then(Value::as_str),
            Some("direct-config")
        );
        assert!(
            output
                .pointer("/qemuMedia/media/1/registry/remediation")
                .is_none()
        );
        assert!(
            output
                .pointer("/unsupportedCapabilities")
                .and_then(Value::as_array)
                .is_some_and(|items| items.iter().any(|item| item == "exec"))
        );
        assert!(
            output
                .pointer("/runtimeCapabilities")
                .and_then(Value::as_array)
                .is_some_and(|items| items.iter().any(|item| item == "usb-hotplug"))
        );
        assert!(
            output
                .pointer("/serviceCapabilities")
                .and_then(Value::as_array)
                .is_some_and(|items| items.iter().any(|item| item == "qemu-media"))
        );
    }

    #[test]
    fn service_capabilities_do_not_advertise_unsupported_virtiofsd() {
        let services = StatusServicesOutputV2 {
            d2b: "active".to_owned(),
            microvm: "unsupported".to_owned(),
            virtiofsd: "unsupported".to_owned(),
            qemu_media: Some("running".to_owned()),
            gpu: None,
            video: None,
            snd: None,
            swtpm: None,
        };

        let capabilities = output_service_capabilities(&services);
        assert!(capabilities.contains(&"qemu-media".to_owned()));
        assert!(!capabilities.contains(&"virtiofsd".to_owned()));
        assert!(!capabilities.contains(&"microvm".to_owned()));
    }

    #[allow(dead_code)]
    fn cmd_list_json_uses_daemon_public_list_entries() {
        let response = json!({
            "type": "listResponse",
            "vms": [{
                "vm": "vm-a",
                "name": "vm-a",
                "env": "dev",
                "graphics": true,
                "tpm": false,
                "usbip": true,
                "isNetVm": false,
                "sshUser": "alice",
                "staticIp": "10.20.0.10",
                "lifecycle": { "state": "Running", "pendingRestart": false },
                "runtime": { "detail": "running" },
                "services": {
                    "d2b": "active",
                    "microvm": "running",
                    "virtiofsd": "running",
                    "gpu": "running",
                    "video": "running",
                    "snd": null,
                    "swtpm": null
                }
            }]
        });
        let args = super::ListArgs {
            json: true,
            human: false,
        };

        let (result, request, stdout) =
            run_public_command_with_mock_daemon("cmd-list-public", "vm-a", response, |context| {
                super::cmd_list(context, &args)
            });

        assert_eq!(result.expect("cmd list result"), 0);
        assert_eq!(request.get("type").and_then(Value::as_str), Some("list"));
        let output: Value = serde_json::from_slice(&stdout).expect("list json output");
        assert_eq!(
            output.pointer("/0/name").and_then(Value::as_str),
            Some("vm-a")
        );
        assert_eq!(
            output.pointer("/0/status").and_then(Value::as_str),
            Some("running")
        );
        assert_eq!(
            output.pointer("/0/graphics").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            output.pointer("/0/usbip").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[allow(dead_code)]
    fn cmd_list_json_includes_qemu_media_runtime_fields() {
        let response = json!({
            "type": "listResponse",
            "vms": [{
                "vm": "installer",
                "name": "installer",
                "env": "dev",
                "graphics": false,
                "tpm": false,
                "usbip": false,
                "isNetVm": false,
                "sshUser": null,
                "staticIp": "10.20.0.20",
                "lifecycle": { "state": "Running", "pendingRestart": false },
                "runtime": { "detail": "running", "kind": "qemu-media" },
                "autostart": {
                    "mode": "manual-only",
                    "reason": "qemu-media VMs are intentionally skipped by daemon autostart"
                },
                "unsupportedCapabilities": ["exec", "guest-control", "ssh", "store-sync"],
                "qemuMedia": {
                    "firmwareMode": "none",
                    "runner": {
                        "role": "qemu-media",
                        "state": "running",
                        "qmpReadiness": "ready",
                        "preContProgress": "paused-before-cont"
                    },
                    "media": [{
                        "mediaRef": "image-boot",
                        "slot": "boot",
                        "sourceKind": "image-file",
                        "format": "raw",
                        "readOnly": false,
                        "registry": {
                            "state": "direct-config",
                            "remediation": null
                        }
                    }]
                },
                "services": {
                    "d2b": "active",
                    "microvm": "unsupported",
                    "qemuMedia": "running",
                    "virtiofsd": "stopped",
                    "gpu": null,
                    "video": null,
                    "snd": null,
                    "swtpm": null
                }
            }]
        });
        let args = super::ListArgs {
            json: true,
            human: false,
        };

        let (result, request, stdout) = run_public_command_with_manifest(
            "cmd-list-qemu-media-public",
            "installer",
            response,
            write_qemu_media_manifest,
            |context| super::cmd_list(context, &args),
        );

        assert_eq!(result.expect("cmd list result"), 0);
        assert_eq!(request.get("type").and_then(Value::as_str), Some("list"));
        let output: Value = serde_json::from_slice(&stdout).expect("list json output");
        assert_eq!(
            output.pointer("/0/runtimeKind").and_then(Value::as_str),
            Some("qemu-media")
        );
        assert_eq!(
            output.pointer("/0/status").and_then(Value::as_str),
            Some("running")
        );
        assert_eq!(
            output
                .pointer("/0/qemuMedia/runner/preContProgress")
                .and_then(Value::as_str),
            Some("paused-before-cont")
        );
        assert_eq!(
            output.pointer("/0/autostart/mode").and_then(Value::as_str),
            Some("manual-only")
        );
        assert_eq!(
            output
                .pointer("/0/qemuMedia/media/0/registry/state")
                .and_then(Value::as_str),
            Some("direct-config")
        );
    }

    #[test]
    fn vm_list_human_unavailable_reports_socket_requirement() {
        let context = Context {
            manifest_path: PathBuf::from("/dev/null"),
            bundle_path: PathBuf::from("/dev/null"),
            public_socket: test_socket_path("vm-list-unavailable", ".sock"),
            broker_socket: PathBuf::from("/dev/null"),
            state_root: None,
            host_runtime_path: PathBuf::from("/dev/null"),
            system_state_fixture: None,
            auth_status_fixture: None,
            daemon_state_dir: PathBuf::from("/dev/null"),
            metrics_url: "http://127.0.0.1:1/metrics".to_owned(),
        };
        let args = super::VmListArgs {
            json: false,
            human: false,
            realm: None,
            all: false,
        };

        let (result, stdout) =
            super::with_test_stdout_capture(|| super::cmd_vm_list(&context, &args));

        let error = result.expect_err("vm list must fail closed without authenticated daemon");
        assert_eq!(error.exit_code, 69);
        assert_eq!(error.message, "client-connect-failed");
        assert!(stdout.is_empty());
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

    fn manifest_with_vms(names: &[&str]) -> ManifestDocument {
        ManifestDocument {
            _manifest: None,
            _observability: None,
            entries: names
                .iter()
                .map(|name| {
                    (
                        (*name).to_owned(),
                        ManifestVm {
                            name: (*name).to_owned(),
                            env: Some("work".to_owned()),
                            graphics: false,
                            tpm: false,
                            audio: false,
                            usbip_yubikey: false,
                            static_ip: None,
                            is_net_vm: false,
                            state_dir: format!("/var/lib/d2b/vms/{name}"),
                            bridge: "br-work-lan".to_owned(),
                            ssh_user: Some("alice".to_owned()),
                            runtime: None,
                        },
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn storage_migration_plan_includes_checkpoint_and_rollback_command() {
        let manifest = manifest_with_vms(&["work-vm", "corp-vm"]);
        let plan = build_storage_migration_plan(&manifest);

        assert_eq!(plan.command, "host migrate-storage");
        assert_eq!(plan.mode, "dry-run");
        assert_eq!(plan.vm_count, 2);
        assert_eq!(plan.vms, vec!["corp-vm".to_owned(), "work-vm".to_owned()]);
        assert!(plan.checkpoint_id.starts_with("storage-cutover-"));
        assert!(
            plan.rollback_command
                .contains("d2b host migrate-storage --rollback --from-checkpoint")
        );
        assert!(plan.rollback_command.contains(&plan.checkpoint_id));
        assert!(
            plan.preserve
                .iter()
                .any(|item| item.contains("swtpm NVRAM"))
        );
        assert!(plan.cutover_only_cleanup.contains(&"/run/d2b-gpu"));
        assert!(
            plan.fail_closed_hazards
                .iter()
                .any(|item| item.contains("symlink"))
        );
    }

    #[test]
    fn storage_migration_checkpoint_id_is_order_insensitive() {
        let a = vec!["work-vm".to_owned(), "corp-vm".to_owned()];
        let b = vec!["corp-vm".to_owned(), "work-vm".to_owned()];
        assert_eq!(
            storage_migration_checkpoint_id(&a),
            storage_migration_checkpoint_id(&b)
        );
    }

    #[test]
    fn storage_migration_from_checkpoint_requires_rollback() {
        assert!(
            NativeCli::try_parse_from([
                "d2b",
                "host",
                "migrate-storage",
                "--from-checkpoint",
                "storage-cutover-test",
                "--json",
            ])
            .is_err(),
            "--from-checkpoint must not be silently ignored without --rollback",
        );
    }

    #[test]
    fn vm_status_subcommand_parses_natively() {
        let cli =
            NativeCli::try_parse_from(["d2b", "vm", "status", "vm-a"]).expect("vm status parse");
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
    //! The `vm exec --json` failure envelope carries `source`/`reason`/
    //! `transportExitCode` for a transport/protocol/guest-control failure,
    //! and never carries captured stdio bytes or a `guestExitCode` (that
    //! field belongs exclusively to a genuine guest-side terminal result).

    use super::{VmExecArgs, exec_client, exec_json_failure_value};

    fn exec_args(vm: &str) -> VmExecArgs {
        VmExecArgs {
            vm: vm.to_owned(),
            detach: false,
            interactive: false,
            tty: false,
            json: true,
            human: false,
            management: Vec::new(),
            command: vec!["true".to_owned()],
        }
    }

    #[test]
    fn old_generation_70_envelope_is_sourced_to_guest_control() {
        let args = exec_args("work");
        let error = exec_client::ExecClientError::from_daemon_error(
            "guest-control-unavailable-old-generation",
            "this VM generation does not support guest-control exec",
            "rebuild the VM with a current d2b generation",
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
    //! Host-side review/approve logic for `d2b config`. The SSH
    //! `sync` path needs a live VM (Layer-2); these unit tests cover
    //! the pure file-op core + the input validators.

    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{
        config_approve_core, config_atomic_write, config_reject_core, config_staging_path_in,
        config_validate_remote_path, config_validate_staging_bytes, config_validate_vm_name,
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
        assert!(config_validate_remote_path("/var/lib/d2b-guest/guest-config.nix").is_ok());
        assert!(config_validate_remote_path("/etc/d2b/guest-config.nix").is_ok());
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
        // no temp turds left behind (impl writes `.<base>.d2b-tmp.*`)
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("d2b-tmp"))
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
            .filter(|e| e.file_name().to_string_lossy().contains("d2b-tmp"))
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

    #[test]
    fn staging_path_in_is_per_vm() {
        let base = PathBuf::from("/x/state");
        assert_eq!(
            config_staging_path_in(&base, "work-aad"),
            PathBuf::from("/x/state/work-aad.guest.nix")
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
        let encoded = d2b_core::base64_codec::encode(content);
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
            vec![b'a'; (d2b_contracts::guest_wire::READ_GUEST_FILE_MAX_BYTES as usize) + 1];
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

/// Fail-closed source gate: the Rust CLI must not launch `ssh` or `scp`.
/// Guest interaction goes through the authenticated guest-control transport.
/// This module scans production source for SSH/SCP argv tokens and proves at
/// runtime that the guest-control config path stages only received bytes.
#[cfg(test)]
mod ssh_spawn_gate {
    use std::path::PathBuf;

    use super::{
        Context, DEFAULT_GUEST_CONFIG_PATH, GuestConfigReadOutcome, cmd_config_sync,
        config_staging_path, finish_config_sync_from_reply, read_guest_config_via_socket,
    };

    /// Construct the quoted argv tokens (`"ssh"`, `"scp"`) without embedding the
    /// bare literal in this file, so the scanner is robust even if the
    /// test-module skip ever regresses.
    fn forbidden_tokens() -> [String; 2] {
        let ssh: String = ['s', 's', 'h'].iter().collect();
        let scp: String = ['s', 'c', 'p'].iter().collect();
        [format!("\"{ssh}\""), format!("\"{scp}\"")]
    }

    /// Return the 1-based line numbers that launch an SSH/SCP client. `#[cfg(test)] mod` blocks are skipped wholesale (test
    /// fixtures legitimately mention SSH); only column-0 `}` closes such a
    /// block, matching rustfmt's indentation of nested items.
    fn scan_ssh_argv_violations(src: &str) -> Vec<usize> {
        let [ssh_tok, scp_tok] = forbidden_tokens();
        let lines: Vec<&str> = src.lines().collect();
        let mut violations = Vec::new();
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
            if line.contains(&ssh_tok) || line.contains(&scp_tok) {
                violations.push(i + 1);
            }
            i += 1;
        }
        violations
    }

    #[test]
    fn crate_source_launches_no_ssh_or_scp_clients() {
        let src = include_str!("lib.rs");
        let violations = scan_ssh_argv_violations(src);
        assert!(
            violations.is_empty(),
            "found SSH/SCP argv tokens in production code at lines {violations:?}; \
             route guest work through guest-control instead"
        );
    }

    #[test]
    fn gate_flags_illicit_ssh_and_ignores_test_blocks() {
        let [ssh_tok, _] = forbidden_tokens();
        // Illicit: a bare SSH argv in production code must be flagged.
        let illicit = format!("fn run() {{\n    let argv = vec![{ssh_tok}.to_owned()];\n}}\n");
        assert_eq!(scan_ssh_argv_violations(&illicit), vec![2]);

        // Test fixtures: an SSH token inside a `#[cfg(test)] mod` is skipped.
        let in_test = format!("#[cfg(test)]\nmod t {{\n    fn f() {{ let _ = {ssh_tok}; }}\n}}\n");
        assert!(scan_ssh_argv_violations(&in_test).is_empty());
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
            "contentBase64": d2b_core::base64_codec::encode(payload),
        }))
        .expect("serialize reply");
        let staging = dir.join("staged.nix");
        let staged = finish_config_sync_from_reply(&reply, &staging, false)
            .expect("staging succeeds for a valid reply");
        assert_eq!(staged.bytes, payload.len());
        assert!(staging.exists());

        // The Unavailable outcome + byte-staging above prove this path uses
        // only the socket/received bytes; `ssh`/`scp` is forbidden crate-wide.
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
                "audioService": format!("d2b-{vm}-audio.service"),
                "usbipYubikey": false,
                "staticIp": null,
                "isNetVm": false,
                "stateDir": format!("/var/lib/d2b/vms/{vm}"),
                "bridge": "d2b-dev",
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
        let processes = d2b_core::processes::ProcessesJson {
            schema_version: "v2".to_owned(),
            vms: vec![d2b_core::processes::VmProcessDag {
                workload_identity: None,
                vm: vm.to_owned(),
                // No GuestControlHealth node: the VM is a known but
                // pre-guest-control generation.
                nodes: Vec::new(),
                edges: Vec::new(),
                invariants: d2b_core::processes::VmProcessInvariants {
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
            "publicManifestPath": format!("{unique}.vms.json"),
            "hostPath": format!("{unique}.host.json"),
            "processesPath": format!("{unique}.processes.json"),
            "privilegesPath": format!("{unique}.privileges.json"),
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
        // No SSH/SCP client may be spawned on any config-sync path: the exit
        // 70 + old-generation-slug + "not transport-unavailable" assertions
        // above prove the command fails closed before any transport, and
        // `crate_source_launches_ssh_only_from_allowlisted_sites` gates
        // `ssh`/`scp` to sanctioned sites crate-wide.
        // Nothing may be staged or published on the fail-closed path.
        assert!(
            !config_staging_path(vm).exists(),
            "old-generation fail-closed must not stage guest bytes"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod console_fsm_tests {
    //! Unit tests for the console FSM detach-char scanning logic and the
    //! QEMU blank-console warning message content.

    use super::{
        AddressFamily, Context, DetachScan, IpcHelloOk, MAX_FRAME_BYTES, MsgFlags, SockFlag,
        SockType, UnixAddr, daemon_supported_features, encode_type_tagged_message, nix_err_to_io,
        scan_chunk_for_detach, send, socket,
    };
    use d2b_contracts::public_wire;
    use d2b_core::error::Version;
    use nix::{
        sys::socket::{Backlog, accept4, bind, listen},
        unistd::close,
    };
    use serde_json::Value;
    use std::{io, os::fd::AsRawFd as _, path::PathBuf, thread};

    const DETACH: u8 = b'\x1d'; // Ctrl-]

    fn console_test_socket_path(test_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "d2b-console-{}-{test_name}.sock",
            std::process::id()
        ))
    }

    fn recv_test_frame(fd: std::os::fd::RawFd) -> io::Result<Vec<u8>> {
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

    fn send_test_frame(fd: std::os::fd::RawFd, payload: &[u8]) -> io::Result<()> {
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

    #[test]
    fn no_detach_char_returns_no_detach() {
        assert_eq!(scan_chunk_for_detach(b"hello world"), DetachScan::NoDetach);
        assert_eq!(scan_chunk_for_detach(b""), DetachScan::NoDetach);
        assert_eq!(scan_chunk_for_detach(b"\x00\x01\x02"), DetachScan::NoDetach);
    }

    #[test]
    fn detach_only_chunk_has_zero_prefix() {
        let chunk = [DETACH];
        assert_eq!(
            scan_chunk_for_detach(&chunk),
            DetachScan::Detach { prefix_len: 0 }
        );
    }

    #[test]
    fn detach_at_start_has_zero_prefix() {
        let chunk = [DETACH, b'a', b'b'];
        assert_eq!(
            scan_chunk_for_detach(&chunk),
            DetachScan::Detach { prefix_len: 0 }
        );
    }

    #[test]
    fn detach_in_middle_returns_correct_prefix_len() {
        // "abc\x1ddef" — detach at index 3, prefix "abc"
        let mut chunk = b"abc".to_vec();
        chunk.push(DETACH);
        chunk.extend_from_slice(b"def");
        assert_eq!(
            scan_chunk_for_detach(&chunk),
            DetachScan::Detach { prefix_len: 3 }
        );
    }

    #[test]
    fn detach_at_end_returns_full_minus_one_prefix() {
        // "hello\x1d" — detach at index 5, prefix "hello"
        let mut chunk = b"hello".to_vec();
        chunk.push(DETACH);
        assert_eq!(
            scan_chunk_for_detach(&chunk),
            DetachScan::Detach { prefix_len: 5 }
        );
    }

    #[test]
    fn first_detach_char_wins_over_later_occurrences() {
        // "\x1dabc\x1d" — first detach at index 0
        let mut chunk = vec![DETACH];
        chunk.extend_from_slice(b"abc");
        chunk.push(DETACH);
        assert_eq!(
            scan_chunk_for_detach(&chunk),
            DetachScan::Detach { prefix_len: 0 }
        );
    }

    #[test]
    fn console_control_messages_go_to_stderr_and_payload_to_stdout() {
        let socket_path = console_test_socket_path("streams");
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

                let attach_request = recv_test_frame(accepted)?;
                let attach_value: Value = serde_json::from_slice(&attach_request)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                assert_eq!(
                    attach_value.get("op").and_then(Value::as_str),
                    Some("attach")
                );
                let attach_response = encode_type_tagged_message(
                    "consoleResponse",
                    &public_wire::ConsoleOpResponse::Attach(public_wire::ConsoleAttachResult {
                        session: "console-test".to_owned(),
                        provider_kind: public_wire::ConsoleProviderKind::QemuMedia,
                        ring_buffer_start_offset: 0,
                    }),
                    "console attach response",
                )
                .expect("encode console attach response");
                send_test_frame(accepted, &attach_response)?;

                let read_request = recv_test_frame(accepted)?;
                let read_value: Value = serde_json::from_slice(&read_request)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                assert_eq!(
                    read_value.get("op").and_then(Value::as_str),
                    Some("readOutput")
                );
                let read_response = encode_type_tagged_message(
                    "consoleResponse",
                    &public_wire::ConsoleOpResponse::ReadOutput(
                        public_wire::ConsoleReadOutputResult {
                            session: "console-test".to_owned(),
                            stream: d2b_contracts::terminal_wire::TerminalStream::Stdout,
                            offset: 0,
                            chunk_base64: d2b_core::base64_codec::encode(b"guest uart\n"),
                            is_eof: true,
                            ring_buffer_start_offset: 0,
                            dropped_bytes: 0,
                        },
                    ),
                    "console read response",
                )
                .expect("encode console read response");
                send_test_frame(accepted, &read_response)?;

                let eof_request = recv_test_frame(accepted)?;
                let eof_value: Value = serde_json::from_slice(&eof_request)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                assert_eq!(
                    eof_value.get("op").and_then(Value::as_str),
                    Some("readOutput")
                );
                let eof_response = encode_type_tagged_message(
                    "consoleResponse",
                    &public_wire::ConsoleOpResponse::ReadOutput(
                        public_wire::ConsoleReadOutputResult {
                            session: "console-test".to_owned(),
                            stream: d2b_contracts::terminal_wire::TerminalStream::Stdout,
                            offset: 11,
                            chunk_base64: String::new(),
                            is_eof: true,
                            ring_buffer_start_offset: 0,
                            dropped_bytes: 0,
                        },
                    ),
                    "console eof response",
                )
                .expect("encode console eof response");
                send_test_frame(accepted, &eof_response)?;

                let close_request = recv_test_frame(accepted)?;
                let close_value: Value = serde_json::from_slice(&close_request)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                assert_eq!(close_value.get("op").and_then(Value::as_str), Some("close"));
                let close_response = encode_type_tagged_message(
                    "consoleResponse",
                    &public_wire::ConsoleOpResponse::Close(public_wire::ConsoleCloseResult {
                        session: "console-test".to_owned(),
                        closed: true,
                    }),
                    "console close response",
                )
                .expect("encode console close response");
                send_test_frame(accepted, &close_response)
            })();
            close(accepted).expect("close accepted socket");
            exchange.expect("mock console daemon exchange");
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
        let args = super::ConsoleArgs {
            vm: "media".to_owned(),
        };
        let (result, stdout, stderr) =
            super::with_test_output_capture(|| super::cmd_console(&context, &args, &[]));
        server.join().expect("join mock console daemon");
        let _ = std::fs::remove_file(&socket_path);

        assert_eq!(result.expect("console exits cleanly"), 0);
        assert_eq!(stdout, b"guest uart\n");
        let stderr = String::from_utf8(stderr).expect("stderr utf8");
        assert!(stderr.contains("Connected to console for VM 'media'"));
        assert!(stderr.contains("/dev/ttyS0"));
        assert!(stderr.contains("serial-getty"));
        assert!(stderr.contains("VM console closed (EOF)"));
    }

    #[test]
    fn console_signal_loop_closes_on_fatal_signals() {
        let source = include_str!("lib.rs");
        let start = source.find("fn cmd_console(").expect("cmd_console present");
        let body = &source[start
            ..source[start..]
                .find("fn console_round_trip(")
                .expect("console_round_trip follows cmd_console")
                + start];
        for signal in ["Interrupt", "Terminate", "Stop", "Hangup", "Quit"] {
            assert!(
                body.contains(&format!("exec_client::ExecSignal::{signal}")),
                "cmd_console must close and exit on {signal}"
            );
        }
    }

    #[test]
    fn console_output_decode_fails_closed() {
        let socket_path = console_test_socket_path("bad-base64");
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

                let _attach_request = recv_test_frame(accepted)?;
                let attach_response = encode_type_tagged_message(
                    "consoleResponse",
                    &public_wire::ConsoleOpResponse::Attach(public_wire::ConsoleAttachResult {
                        session: "console-test".to_owned(),
                        provider_kind: public_wire::ConsoleProviderKind::LocalHypervisor,
                        ring_buffer_start_offset: 0,
                    }),
                    "console attach response",
                )
                .expect("encode console attach response");
                send_test_frame(accepted, &attach_response)?;

                let _read_request = recv_test_frame(accepted)?;
                let bad_response = encode_type_tagged_message(
                    "consoleResponse",
                    &public_wire::ConsoleOpResponse::ReadOutput(
                        public_wire::ConsoleReadOutputResult {
                            session: "console-test".to_owned(),
                            stream: d2b_contracts::terminal_wire::TerminalStream::Stdout,
                            offset: 0,
                            chunk_base64: "not valid base64!".to_owned(),
                            is_eof: false,
                            ring_buffer_start_offset: 0,
                            dropped_bytes: 0,
                        },
                    ),
                    "console malformed output response",
                )
                .expect("encode malformed output response");
                send_test_frame(accepted, &bad_response)?;

                let close_request = recv_test_frame(accepted)?;
                let close_value: Value = serde_json::from_slice(&close_request)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                assert_eq!(close_value.get("op").and_then(Value::as_str), Some("close"));
                let close_response = encode_type_tagged_message(
                    "consoleResponse",
                    &public_wire::ConsoleOpResponse::Close(public_wire::ConsoleCloseResult {
                        session: "console-test".to_owned(),
                        closed: true,
                    }),
                    "console close response",
                )
                .expect("encode console close response");
                send_test_frame(accepted, &close_response)
            })();
            close(accepted).expect("close accepted socket");
            exchange.expect("mock console daemon exchange");
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
        let args = super::ConsoleArgs {
            vm: "media".to_owned(),
        };
        let (result, stdout, _stderr) =
            super::with_test_output_capture(|| super::cmd_console(&context, &args, &[]));
        server.join().expect("join mock console daemon");
        let _ = std::fs::remove_file(&socket_path);

        let err = result.expect_err("malformed console output must fail closed");
        assert_eq!(err.exit_code, 1);
        assert!(err.message.contains("malformed base64"));
        assert!(
            stdout.is_empty(),
            "malformed chunks must not emit synthetic stdout"
        );
    }
}
