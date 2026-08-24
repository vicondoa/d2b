//! The CLI command tree and its compatibility helpers.
//!
//! Retired Guest-control transports are absent; compatibility here is limited
//! to the public command and artifact shapes still owned by the CLI.

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::OsString,
    fmt::Write as _,
    fs,
    io::{self, IoSliceMut, IsTerminal as _, Read as _, Write as _},
    os::fd::{AsRawFd as _, OwnedFd},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

#[allow(unused_imports)]
use clap::{Args, Parser, Subcommand, ValueEnum};
use d2b_contracts::{
    Hello as IpcHello, HelloOk as IpcHelloOk, HelloRejected as IpcHelloRejected, KnownFeatureFlag,
    SemverRange,
    types::{MediaRef, validate_usb_bus_id},
};
use d2b_contracts_broker::broker_wire::{
    AuditExportCursor, StoreVerifyResponse as IpcStoreVerifyResponse,
    StoreVerifyStatus as IpcStoreVerifyStatus,
};
use d2b_contracts_control::{
    cli_output::*,
    public_wire::{
        self, AuditFormat as IpcAuditFormat, AuditRequest as IpcAuditRequest,
        KeyEntry as IpcKeyEntry, KeysShowRequest as IpcKeysShowRequest,
        KeysShowResponse as IpcKeysShowResponse, ListEntry as IpcListEntry,
        ListRequest as IpcListRequest, StatusRequest as IpcStatusRequest,
        UsbProbeEntryKind as IpcUsbProbeEntryKind, UsbipProbeEntry as IpcUsbipProbeEntry,
        UsbipProbeStatus as IpcUsbipProbeStatus, VmLifecycleState as IpcVmLifecycleState,
        VmStatus as IpcVmStatus,
    },
};
use d2b_core::{
    bundle::Bundle, bundle_resolver::HostRuntime, closures::ClosureMetadata,
    error::Error as CoreError, host::HostJson, host_check, processes::ProcessesJson,
    realm_controller_config::RealmControllersJson,
};
use nix::sys::socket::{
    AddressFamily, MsgFlags, SockFlag, SockType, UnixAddr, connect, send, socket,
};
use nix::unistd::Uid;
use rustix::net::sockopt::{Timeout as SocketTimeout, set_socket_timeout};
use rustix::net::{RecvAncillaryBuffer, RecvFlags, recvmsg};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::status_read_model::{
    booted_symlink, build_vm_status_output, build_vm_status_output_from_public, current_symlink,
    list_output_from_manifest, list_output_from_public_entries, public_lifecycle_status_label,
    vm_state_dir,
};
use super::terminal_client::TerminalTransport as _;
use super::{
    EXIT_API_TIMEOUT, MAX_FRAME_BYTES, doctor, exec_client, host_validate, target_routing,
    terminal_client,
};

pub(super) const DEFAULT_MANIFEST_PATH: &str = "/run/current-system/sw/share/d2b/vms.json";
#[cfg(not(test))]
pub(super) const DEFAULT_REALM_ENTRYPOINTS_PATH: &str =
    "/run/current-system/sw/share/d2b/realm-entrypoints.json";
pub(super) const DEFAULT_BUNDLE_PATH: &str = "/etc/d2b/bundle.json";
pub(super) const DEFAULT_PUBLIC_SOCKET: &str = "/run/d2b/public.sock";
pub(super) const DEFAULT_BROKER_SOCKET: &str = "/run/d2b/priv.sock";
pub(super) const DEFAULT_HOST_RUNTIME_PATH: &str = "/var/lib/d2b/runtime/host-runtime.json";
pub(super) const DEFAULT_CLIENT_VERSION_RANGE: &str = ">=0.4.0, <0.5.0";
pub(super) const RUNTIME_UNKNOWN: &str = "unknown";
pub(super) const SYSTEM_TOOL_PATH: &str =
    "/run/current-system/sw/bin:/usr/bin:/usr/sbin:/bin:/sbin";

pub(super) fn system_tool_command(program: &str) -> Command {
    let mut command = Command::new(program);
    command.env("PATH", SYSTEM_TOOL_PATH);
    command
}

/// Location of daemon-persisted state files (`pidfd-table.json`,
/// `kernel-module-report.json`, `autostart-report.json`,
/// `storage-lifecycle-report.json`) that
/// `d2b host doctor --read-only` inspects. Mirrors
/// `d2bd::DEFAULT_DAEMON_STATE_DIR`.
pub(super) const DEFAULT_DAEMON_STATE_DIR: &str = "/var/lib/d2b/daemon-state";
/// No default URL: d2bd does not serve an HTTP metrics endpoint.
/// Set `D2B_METRICS_URL` when an external collector is available.
pub(super) const DEFAULT_METRICS_URL: &str = "";
pub(super) const MAX_REALM_ENTRYPOINTS_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Parser)]
#[command(version)]
pub(super) struct NativeCli {
    #[command(subcommand)]
    pub(super) command: NativeCommand,
}

#[derive(Debug, Subcommand)]
pub(super) enum NativeCommand {
    /// List declared VMs with daemon runtime state when d2bd is reachable.
    List(ListArgs),
    /// Show per-VM runtime status plus bridge health.
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
    /// Inspect current constellation operation and trace state.
    Op(OpArgs),
    /// Per-VM lifecycle verbs (start / stop / restart / list / status) plus the
    /// admin-only Process sub-verb `exec`, which runs commands or an
    /// interactive session inside a VM over authenticated named streams.
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
    /// Clipboard authority operations (picker-driven paste replay via d2b-clipd).
    Clipboard(ClipboardArgs),
}

#[derive(Debug, Args)]
pub(super) struct LaunchArgs {
    /// Canonical workload target or an unambiguous workload id.
    target: String,
    /// Configured launcher item id. Omit to use the declared default or sole item.
    #[arg(long)]
    item: Option<String>,
    /// Emit a structured JSON result.
    #[arg(long, conflicts_with = "human")]
    json: bool,
    /// Force human-readable output.
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
pub(super) struct ListArgs {
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
pub(super) struct StatusArgs {
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
pub(super) struct UsbArgs {
    #[command(subcommand)]
    command: UsbCommand,
}

#[derive(Debug, Subcommand)]
pub(super) enum UsbCommand {
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
pub(super) struct UsbAttachArgs {
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
pub(super) struct UsbDetachArgs {
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
pub(super) struct UsbProbeArgs {
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
pub(super) struct UsbSecurityKeyArgs {
    #[command(subcommand)]
    command: UsbSecurityKeyCommand,
}

#[derive(Debug, Subcommand)]
pub(super) enum UsbSecurityKeyCommand {
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
pub(super) struct UsbSkStatusArgs {
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
pub(super) struct UsbSkSessionsArgs {
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
pub(super) struct UsbSkCancelArgs {
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
pub(super) struct UsbSkTestArgs {
    vm: String,
    #[arg(long)]
    dry_run: bool,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
pub(super) struct AuditArgs {
    #[arg(long)]
    pub(crate) strict: bool,
    #[arg(long, conflicts_with = "human")]
    pub(crate) json: bool,
    #[arg(long, conflicts_with = "json")]
    pub(crate) human: bool,
}

#[derive(Debug, Args)]
pub(super) struct HostArgs {
    #[command(subcommand)]
    command: HostCommand,
}

#[derive(Debug, Subcommand)]
pub(super) enum HostCommand {
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
pub(super) struct HostShutdownHookArgs {
    /// Plan the host-shutdown stop phases without contacting d2bd.
    dry_run: bool,
    /// Apply the host-shutdown stop phases.
    apply: bool,
    json: bool,
}

#[derive(Debug, Args)]
pub(super) struct HostValidateArgs {
    /// Plan: report which readiness validators WOULD be attested.
    /// No evidence is written.
    #[arg(long, conflicts_with = "apply")]
    pub(crate) dry_run: bool,
    /// Apply: write `/var/lib/d2b/validated/<wave>.json` for
    /// every wave whose declared validators are present on disk.
    #[arg(long, conflicts_with = "dry_run")]
    pub(crate) apply: bool,
    /// Restrict to a single wave. Other waves are reported as `skipped`.
    #[arg(long)]
    pub(crate) wave: Option<String>,
    /// Override the per-wave operator signature. When unset, the
    /// verb derives a deterministic sha256 signature from
    /// `hostname|wave|scripts_dir|timestamp`.
    #[arg(long, value_name = "SIGNATURE")]
    pub(crate) operator_signature: Option<String>,
    /// Override the evidence directory. Default: `/var/lib/d2b/validated`.
    #[arg(long, value_name = "PATH")]
    pub(crate) evidence_dir: Option<PathBuf>,
    /// Override the scripts directory. Default: best-effort
    /// discovery of the installed `tests/` share, then `./tests`.
    #[arg(long, value_name = "PATH")]
    pub(crate) scripts_dir: Option<PathBuf>,
    #[arg(long, conflicts_with = "human")]
    pub(crate) json: bool,
    #[arg(long, conflicts_with = "json")]
    pub(crate) human: bool,
}

#[derive(Debug, Args)]
pub(super) struct HostCheckArgs {
    #[arg(long)]
    pub(crate) read_only: bool,
    #[arg(long)]
    pub(crate) strict: bool,
    #[arg(long, conflicts_with = "human")]
    pub(crate) json: bool,
    #[arg(long, conflicts_with = "json")]
    pub(crate) human: bool,
}

#[derive(Debug, Args)]
pub(super) struct HostPrepareArgs {
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
pub(super) struct HostDestroyArgs {
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
pub(super) struct HostDoctorArgs {
    /// Mandatory: doctor is read-only. Mutating forms are separate verbs.
    #[arg(long)]
    pub(crate) read_only: bool,
    #[arg(long, conflicts_with = "human")]
    pub(crate) json: bool,
    #[arg(long, conflicts_with = "json")]
    pub(crate) human: bool,
}

#[derive(Debug, Args)]
pub(super) struct HostMigrateStorageArgs {
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
pub(super) struct HostInstallArgs {
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
pub(super) struct HostReconcileArgs {
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
pub(super) struct AuthArgs {
    #[command(subcommand)]
    command: AuthCommand,
}

#[derive(Debug, Args)]
pub(super) struct RealmArgs {
    #[command(subcommand)]
    command: RealmCommand,
}

#[derive(Debug, Args)]
pub(super) struct OpArgs {
    #[command(subcommand)]
    command: OpCommand,
}

#[derive(Debug, Args)]
pub(super) struct ClipboardArgs {
    #[command(subcommand)]
    command: ClipboardCommand,
}

#[derive(Debug, Subcommand)]
pub(super) enum ClipboardCommand {
    /// Open the picker and request paste replay for the focused target.
    ///
    /// Opens the d2b-clip-picker, waits for a selection, then asks d2b-clipd
    /// to publish the selected payload and trigger paste replay.
    /// Requires d2b-clipd to be running.
    #[command(alias = "picker")]
    Arm(ClipboardArmArgs),
}

#[derive(Debug, Args)]
pub(super) struct ClipboardArmArgs {
    /// Emit a structured JSON envelope.
    #[arg(long, conflicts_with = "human")]
    json: bool,
    /// Emit a human-readable status line.
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

pub(super) const CLIPBOARD_ARM_CONTROL_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(5);

#[derive(Debug, Subcommand)]
pub(super) enum OpCommand {
    /// Inspect current operation/trace state with bounded partial results.
    Inspect(OpInspectArgs),
}

#[derive(Debug, Args)]
pub(super) struct OpInspectArgs {
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
pub(super) enum RealmCommand {
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
pub(super) struct RealmListArgs {
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
pub(super) struct RealmInspectArgs {
    /// Realm path, e.g. `work` or `payments.work`.
    realm: String,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
pub(super) struct RealmEnterArgs {
    /// Realm path, e.g. `work` or `payments.work`.
    realm: String,
}

#[derive(Debug, Args)]
pub(super) struct RealmRunArgs {
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
pub(super) struct VmArgs {
    #[command(subcommand)]
    command: VmCommand,
}

#[derive(Debug, Subcommand)]
pub(super) enum VmCommand {
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
    Display(VmDisplayArgs),
}

#[derive(Debug, Args)]
pub(super) struct VmDisplayArgs {
    #[command(subcommand)]
    command: VmDisplayCommand,
}

#[derive(Debug, Subcommand)]
pub(super) enum VmDisplayCommand {
    /// List active gateway display sessions.
    List(VmDisplayListArgs),
    /// Close a gateway display session by id.
    Close(VmDisplayCloseArgs),
}

#[derive(Debug, Args)]
pub(super) struct VmDisplayListArgs {
    /// Optional realm target to filter, for example `demo.work.d2b`.
    #[arg(long)]
    target: Option<String>,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
pub(super) struct VmDisplayCloseArgs {
    /// Display session id from `d2b vm display list`.
    session_id: String,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

/// `d2b vm exec [-d] [-it] [-i] [-t] <vm> [--env K=V]... [--cwd DIR] -- <cmd...>`
/// Run a command inside a VM. Use `--` before the command, `-it` for an
/// interactive guest PTY, and `-d` to create a detached exec. Detached execs
/// are managed with `d2b vm exec <vm> list`, `logs <id>`, `status <id>`,
/// and `kill <id>`.
#[derive(Debug, Args)]
pub(super) struct VmExecArgs {
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
    /// Set an environment variable in the guest command (`KEY=VALUE`).
    /// Repeatable.
    #[arg(long = "env", value_name = "KEY=VALUE")]
    env: Vec<String>,
    /// Working directory for the guest command.
    #[arg(long = "cwd", value_name = "DIR")]
    cwd: Option<String>,
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
pub(super) enum VmExecManagementCommand {
    List,
    Logs(VmExecLogsArgs),
    Status(VmExecIdArgs),
    Kill(VmExecIdArgs),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct VmExecIdArgs {
    /// Detached exec id returned by `d2b vm exec -d`.
    exec_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct VmExecLogsArgs {
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
pub(super) struct VmStartArgs {
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
pub(super) struct VmStopArgs {
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
pub(super) struct VmRestartArgs {
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
pub(super) struct VmListArgs {
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
pub(super) struct VmStatusArgs {
    /// VM name.
    vm: String,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

// ---- store-lifecycle verbs ----

#[derive(Debug, Args)]
pub(super) struct BuildArgs {
    vm: String,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
pub(super) struct GenerationsArgs {
    vm: String,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
pub(super) struct SwitchArgs {
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
pub(super) struct BootArgs {
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
pub(super) struct TestArgs {
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
pub(super) struct RollbackArgs {
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
pub(super) struct GcArgs {
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
pub(super) struct StoreArgs {
    #[command(subcommand)]
    command: StoreCommand,
}

#[derive(Debug, Subcommand)]
pub(super) enum StoreCommand {
    /// Verify a VM's hardlink-backed live store-view.
    Verify(StoreVerifyArgs),
}

#[derive(Debug, Args)]
pub(super) struct StoreVerifyArgs {
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
pub(super) struct KeysArgs {
    #[command(subcommand)]
    command: KeysCommand,
}

#[derive(Debug, Subcommand)]
pub(super) enum KeysCommand {
    /// List managed keys (per-VM SSH keypair fingerprints).
    List(KeysListArgs),
    /// Show details for a specific VM's managed key.
    Show(KeysShowArgs),
    /// Rotate the framework-managed per-VM SSH keypair. --apply mutates.
    Rotate(KeysRotateArgs),
}

#[derive(Debug, Args)]
pub(super) struct KeysListArgs {
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
pub(super) struct KeysShowArgs {
    vm: String,
    #[arg(long, conflicts_with = "human")]
    json: bool,
    #[arg(long, conflicts_with = "json")]
    human: bool,
}

#[derive(Debug, Args)]
pub(super) struct KeysRotateArgs {
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
pub(super) struct KeysRotateKnownHostArgs {
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
pub(super) struct KeysTrustArgs {
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
pub(super) struct MigrateArgs {
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
pub(super) struct ConsoleArgs {
    /// VM name whose foreground serial console should be attached.
    vm: String,
}

#[derive(Debug, Args)]
pub(super) struct AudioArgs {
    /// Emit machine-readable JSON output.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Option<AudioCommand>,
}

#[derive(Debug, Subcommand)]
pub(super) enum AudioCommand {
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
pub(super) struct AudioStatusArgs {
    /// Optional VM name; omitted lists audio status for every audio-enabled VM.
    vm: Option<String>,
}

#[derive(Debug, Args)]
pub(super) struct AudioToggleArgs {
    /// The new grant state to apply.
    #[arg(value_enum)]
    state: AudioGrantState,
    /// VM name whose audio grant should be changed.
    vm: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(super) enum AudioGrantState {
    /// Enable the selected audio direction.
    On,
    /// Disable the selected audio direction.
    Off,
}

#[derive(Debug, Args)]
pub(super) struct AudioOffArgs {
    /// VM name whose microphone and speaker grants should both be disabled.
    vm: String,
}

#[derive(Debug, Subcommand)]
pub(super) enum AuthCommand {
    Status(AuthStatusArgs),
}

#[derive(Debug, Args)]
pub(super) struct AuthStatusArgs {
    #[arg(long, conflicts_with = "human")]
    pub(crate) json: bool,
    #[arg(long, conflicts_with = "json")]
    pub(crate) human: bool,
    #[arg(long, hide = true)]
    pub(crate) test_uid: Option<u32>,
}

#[derive(Debug)]
pub(crate) struct CliFailure {
    pub(crate) exit_code: i32,
    pub(crate) message: String,
    pub(crate) rendered_stderr: Option<String>,
    pub(crate) admission_recovery: bool,
}

impl CliFailure {
    pub(crate) fn new(exit_code: i32, message: impl Into<String>) -> Self {
        Self {
            exit_code,
            message: message.into(),
            rendered_stderr: None,
            admission_recovery: false,
        }
    }

    pub(crate) fn host_check_probe_error(error: host_check::ProbeError) -> Self {
        let operator_error = CoreError::internal_io(error.opaque_reason);
        Self {
            exit_code: 1,
            message: operator_error.message(),
            rendered_stderr: render_operator_error(&operator_error, Some("host check")),
            admission_recovery: false,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct LegacyContext {
    pub(crate) manifest_path: PathBuf,
    pub(crate) bundle_path: PathBuf,
    pub(crate) public_socket: PathBuf,
    pub(crate) broker_socket: PathBuf,
    pub(crate) state_root: Option<PathBuf>,
    pub(crate) host_runtime_path: PathBuf,
    pub(crate) system_state_fixture: Option<SystemStateFixture>,
    pub(crate) auth_status_fixture: Option<AuthStatusFixture>,
    /// Daemon-persisted state dir (pidfd-table.json,
    /// kernel-module-report.json, autostart-report.json).
    /// Override via `D2B_DAEMON_STATE_DIR`.
    pub(crate) daemon_state_dir: PathBuf,
    /// Prometheus scrape URL the doctor probes for reachability.
    /// Override via `D2B_METRICS_URL`.
    pub(crate) metrics_url: String,
}

// The old helper modules still import this private alias for their pure
// fixtures. Runtime command dispatch uses `context::ZoneContext`.
pub(super) type Context = LegacyContext;

impl LegacyContext {
    pub(crate) fn from_env() -> Result<Self, CliFailure> {
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

    pub(crate) fn load_manifest(&self) -> Result<ManifestDocument, CliFailure> {
        read_json_file(&self.manifest_path).map_err(|err| {
            CliFailure::new(
                1,
                format!("failed to read {}: {err}", self.manifest_path.display()),
            )
        })
    }

    pub(crate) fn load_bundle_context(&self) -> Result<Option<BundleContext>, CliFailure> {
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
pub(super) struct BundleContext {
    pub(crate) host: Option<HostJson>,
    pub(crate) processes: Option<ProcessesJson>,
    pub(crate) closures: BTreeMap<String, ClosureMetadata>,
    pub(crate) host_runtime: Option<HostRuntime>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ManifestDocument {
    #[serde(rename = "_manifest", default)]
    _manifest: Option<Value>,
    #[serde(rename = "_observability", default)]
    _observability: Option<Value>,
    #[serde(flatten)]
    pub(crate) entries: BTreeMap<String, ManifestVm>,
}

impl ManifestDocument {
    pub(crate) fn vms(&self) -> Vec<&ManifestVm> {
        self.entries
            .iter()
            .filter(|(name, _)| !name.starts_with('_'))
            .map(|(_, vm)| vm)
            .collect()
    }

    pub(crate) fn get_vm(&self, name: &str) -> Option<&ManifestVm> {
        self.entries.get(name).filter(|_| !name.starts_with('_'))
    }

    pub(crate) fn bridge_names(&self) -> BTreeSet<String> {
        self.vms()
            .iter()
            .map(|vm| vm.bridge.clone())
            .collect::<BTreeSet<_>>()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ManifestVm {
    pub(crate) name: String,
    pub(crate) env: Option<String>,
    pub(crate) graphics: bool,
    pub(crate) tpm: bool,
    pub(crate) audio: bool,
    pub(crate) usbip_yubikey: bool,
    pub(crate) static_ip: Option<String>,
    pub(crate) is_net_vm: bool,
    pub(crate) state_dir: String,
    pub(crate) bridge: String,
    pub(crate) ssh_user: Option<String>,
    pub(crate) runtime: Option<ManifestRuntime>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ManifestRuntime {
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) capabilities: BTreeMap<String, bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub(super) struct SystemStateFixture {
    units: BTreeMap<String, String>,
    bridges: BTreeMap<String, BridgeHealthFixture>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct BridgeHealthFixture {
    state: String,
    admin: String,
    expected_carrier: String,
    result: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub(super) struct AuthStatusFixture {
    public_reachable: Option<bool>,
    public_version: Option<String>,
    broker_reachable: Option<bool>,
    broker_version: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct BridgeHealthRow {
    pub(crate) name: String,
    pub(crate) state: String,
    pub(crate) admin: String,
    pub(crate) expected_carrier: String,
    pub(crate) result: String,
}

#[derive(Debug, Clone)]
pub(super) struct SocketProbe {
    reachable: bool,
    version: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct HelloOkFrame {
    #[serde(rename = "type")]
    _type_name: String,
    #[serde(flatten)]
    payload: IpcHelloOk,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct HelloRejectedFrame {
    #[serde(rename = "type")]
    _type_name: String,
    #[serde(flatten)]
    _payload: IpcHelloRejected,
    error: DaemonErrorEnvelope,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ErrorFrame {
    #[serde(rename = "type")]
    _type_name: String,
    error: DaemonErrorEnvelope,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct AuditResponseFrame {
    #[serde(rename = "type")]
    _type_name: String,
    #[serde(flatten)]
    payload: public_wire::AuditResponse,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DaemonErrorEnvelope {
    kind: String,
    #[serde(alias = "exitCode", alias = "code")]
    exit_code: u8,
    message: String,
    remediation: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KeysListResponseFrame {
    #[serde(rename = "type")]
    _type_name: String,
    entries: Vec<IpcKeyEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KeysShowResponseFrame {
    #[serde(rename = "type")]
    _type_name: String,
    #[serde(flatten)]
    payload: IpcKeysShowResponse,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ListResponseFrame {
    #[serde(rename = "type")]
    _type_name: String,
    vms: Vec<IpcListEntry>,
    #[serde(default)]
    read_model: Option<d2b_contracts_control::public_wire::PublicReadModelMetadata>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StatusResponseFrame {
    #[serde(rename = "type")]
    _type_name: String,
    status: StatusResponsePayload,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StatusResponsePayload {
    entries: Vec<IpcVmStatus>,
    #[serde(default)]
    read_model: Option<d2b_contracts_control::public_wire::PublicReadModelMetadata>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UsbipProbeResponseFrame {
    #[serde(rename = "type")]
    _type_name: String,
    entries: Vec<IpcUsbipProbeEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StoreVerifyResponseFrame {
    #[serde(rename = "type")]
    _type_name: String,
    #[serde(flatten)]
    payload: IpcStoreVerifyResponse,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GatewayDisplayResponseFrame {
    #[serde(rename = "type")]
    _type_name: String,
    #[serde(flatten)]
    payload: public_wire::GatewayDisplayOpResponse,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WorkloadResponseFrame {
    #[serde(rename = "type")]
    _type_name: String,
    #[serde(flatten)]
    payload: public_wire::WorkloadOpResponse,
}

#[derive(Debug, Clone)]
pub(super) enum AuditSocketOutcome {
    Unreachable,
    Lines(Vec<String>),
}

#[derive(Debug, Clone)]
pub(super) enum KeysSocketOutcome {
    Unavailable,
    List(Vec<IpcKeyEntry>),
    Show(IpcKeysShowResponse),
}

#[derive(Debug, Clone)]
pub(super) enum ListSocketOutcome {
    Unavailable,
    Entries(
        Vec<IpcListEntry>,
        Option<d2b_contracts_control::public_wire::PublicReadModelMetadata>,
    ),
}

#[derive(Debug, Clone)]
pub(super) enum StatusSocketOutcome {
    Unavailable,
    Entries(
        Vec<IpcVmStatus>,
        Option<d2b_contracts_control::public_wire::PublicReadModelMetadata>,
    ),
}

#[derive(Debug, Clone)]
pub(super) enum UsbProbeSocketOutcome {
    Unavailable,
    Entries(Vec<IpcUsbipProbeEntry>),
}

#[derive(Debug, Clone)]
pub(super) enum StoreVerifySocketOutcome {
    Unavailable,
    Response(IpcStoreVerifyResponse),
}

#[derive(Debug, Clone)]
pub(super) enum PublicSocketOutcome {
    Unavailable,
    Unsupported,
    Reply(Vec<u8>),
}

pub(super) fn encode_type_tagged_message<T>(
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

pub(super) fn daemon_supported_features() -> Vec<d2b_contracts::FeatureFlag> {
    vec![
        KnownFeatureFlag::TypedErrors.wire_value(),
        KnownFeatureFlag::StatusCheckBridges.wire_value(),
        KnownFeatureFlag::ExportBrokerAudit.wire_value(),
        KnownFeatureFlag::ConfiguredLaunchV1.wire_value(),
        KnownFeatureFlag::UnsafeLocalProviderV1.wire_value(),
        KnownFeatureFlag::CutoverRunnerV1.wire_value(),
    ]
}

pub(super) fn daemon_hello_frame(type_name: &str) -> Result<Vec<u8>, CliFailure> {
    let hello = IpcHello {
        client_version: SemverRange::new(DEFAULT_CLIENT_VERSION_RANGE).map_err(|err| {
            CliFailure::new(1, format!("failed to build hello version range: {err}"))
        })?,
        supported_features: daemon_supported_features(),
    };
    encode_type_tagged_message(type_name, &hello, "hello request")
}

pub(super) fn daemon_audit_frame(type_name: &str, json_mode: bool) -> Result<Vec<u8>, CliFailure> {
    daemon_audit_frame_with_cursor(type_name, json_mode, None)
}

fn daemon_audit_frame_with_cursor(
    type_name: &str,
    json_mode: bool,
    cursor: Option<AuditExportCursor>,
) -> Result<Vec<u8>, CliFailure> {
    let request = IpcAuditRequest {
        filter: None,
        format: if json_mode {
            IpcAuditFormat::Json
        } else {
            IpcAuditFormat::Human
        },
        since: None,
        cursor,
        limit: 1024,
    };
    encode_type_tagged_message(type_name, &request, "audit request")
}

pub(super) fn is_daemon_unreachable(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

pub(super) fn cli_failure_from_daemon_error(error: DaemonErrorEnvelope) -> CliFailure {
    let message = if error.remediation.is_empty() {
        format!("{}: {}", error.kind, error.message)
    } else {
        format!("{}: {} ({})", error.kind, error.message, error.remediation)
    };
    CliFailure::new(i32::from(error.exit_code), message)
}

pub(super) fn decode_daemon_frame(response: &[u8], context: &str) -> Result<Value, CliFailure> {
    serde_json::from_slice(response)
        .map_err(|err| CliFailure::new(1, format!("failed to decode {context}: {err}")))
}

pub(super) fn parse_hello_reply(response: &[u8]) -> Result<IpcHelloOk, CliFailure> {
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

fn parse_audit_page(
    response: &[u8],
) -> Result<(Vec<String>, Option<AuditExportCursor>, bool), CliFailure> {
    let value = decode_daemon_frame(response, "audit reply")?;
    let Some(type_name) = value.get("type").and_then(Value::as_str) else {
        return Err(CliFailure::new(
            1,
            "daemon audit reply was missing a type discriminator",
        ));
    };
    match type_name {
        "auditResponse" => serde_json::from_value::<AuditResponseFrame>(value)
            .map(|frame| {
                let lines = frame
                    .payload
                    .entries
                    .into_iter()
                    .map(|entry| {
                        entry
                            .record
                            .map(|record| match record {
                                Value::String(line) => line,
                                record => record.to_string(),
                            })
                            .unwrap_or_else(|| {
                                serde_json::json!({
                                    "export_error": entry.error,
                                    "sequence": entry.sequence,
                                })
                                .to_string()
                            })
                    })
                    .collect();
                (lines, frame.payload.next_cursor, frame.payload.complete)
            })
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

pub(super) fn parse_audit_reply(response: &[u8]) -> Result<Vec<String>, CliFailure> {
    parse_audit_page(response).map(|(lines, _, _)| lines)
}

pub(super) fn render_daemon_audit_lines(
    lines: &[String],
    json_mode: bool,
) -> Result<(), CliFailure> {
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

pub(super) fn is_host_shutdown_hook_invocation(raw_args: &[OsString]) -> bool {
    raw_args.get(1).and_then(|arg| arg.to_str()) == Some("host")
        && raw_args.get(2).and_then(|arg| arg.to_str()) == Some("shutdown-hook")
}

pub(super) fn parse_host_shutdown_hook_args(
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

// ============================================================
// `d2b clipboard` - clipboard authority fallback arming
// ============================================================

pub(super) fn cmd_clipboard_arm(
    _context: &LegacyContext,
    args: &ClipboardArmArgs,
) -> Result<i32, CliFailure> {
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

pub(super) fn set_clipboard_arm_timeouts(
    stream: &std::os::unix::net::UnixStream,
) -> std::io::Result<()> {
    let timeout = Some(CLIPBOARD_ARM_CONTROL_TIMEOUT);
    stream.set_read_timeout(timeout)?;
    stream.set_write_timeout(timeout)?;
    Ok(())
}

pub(super) fn clipboard_arm_failure(
    args: &ClipboardArmArgs,
    message: impl Into<String>,
) -> CliFailure {
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
            admission_recovery: false,
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

/// Base directory for host-side config staging. User-local by default
/// (no privileged surface), from `XDG_STATE_HOME` (or `HOME`). Tests
/// override it per-thread via `set_test_staging_base` rather than mutating
/// process-global env.
pub(crate) fn config_staging_base() -> PathBuf {
    #[cfg(test)]
    if let Some(base) = TEST_STAGING_BASE.with(|b| b.borrow().clone()) {
        return base;
    }
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from(".d2b-state"));
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
pub(super) fn set_test_staging_base(base: Option<PathBuf>) {
    TEST_STAGING_BASE.with(|b| *b.borrow_mut() = base);
}

pub(crate) fn config_staging_path_in(base: &Path, vm: &str) -> PathBuf {
    base.join(format!("{vm}.guest.nix"))
}

pub(crate) fn config_staging_path(vm: &str) -> PathBuf {
    config_staging_path_in(&config_staging_base(), vm)
}

/// Reject VM names that are not the framework's `^[a-z][a-z0-9-]*$`
/// shape, so a VM arg can never traverse out of the staging dir.
pub(crate) fn config_validate_vm_name(vm: &str) -> Result<(), CliFailure> {
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

/// Validate the bytes of a staging file before approval. Kept
/// deliberately light - the authoritative eval + containment gate is
/// the per-VM `guestConfigFile` assertion on `d2b switch`. Here we
/// only refuse an empty / non-UTF-8 file so approve cannot silently
/// land a truncated sync.
pub(super) fn config_validate_staging_bytes(bytes: &[u8]) -> Result<(), CliFailure> {
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
pub(crate) fn config_approve_core(staging: &Path, target: &Path) -> Result<usize, CliFailure> {
    config_approve_core_with_digest(staging, target, None)
}

pub(crate) fn config_approve_core_with_digest(
    staging: &Path,
    target: &Path,
    expected_sha256: Option<&str>,
) -> Result<usize, CliFailure> {
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
    if let Some(expected_sha256) = expected_sha256 {
        let actual_sha256 = sha256_hex(&bytes);
        if actual_sha256 != expected_sha256 {
            return Err(CliFailure::new(
                1,
                "config approve: staged content changed after service approval; re-run `d2b config sync`"
                    .to_owned(),
            ));
        }
    }
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
pub(crate) fn config_reject_core(staging: &Path) -> Result<bool, CliFailure> {
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
pub(super) fn warn_pending_staged_config(vm: &str) {
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
pub(super) fn warn_all_pending_staged_configs() {
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
pub(crate) fn config_atomic_write(target: &Path, bytes: &[u8]) -> Result<(), CliFailure> {
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
pub(super) fn sha256_hex(data: &[u8]) -> String {
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

pub(super) fn cmd_launch(context: &LegacyContext, args: &LaunchArgs) -> Result<i32, CliFailure> {
    use d2b_realm_core::{LauncherItemKind, ProtocolToken};

    if !context.public_socket.exists() {
        return Err(CliFailure::new(
            69,
            "launch requires the d2bd public socket; no static or provider fallback is permitted",
        ));
    }
    let mut socket = SeqpacketUnixSocket::connect(&context.public_socket).map_err(|error| {
        CliFailure::new(
            69,
            format!("failed to connect to the d2bd public socket: {error}"),
        )
    })?;
    socket
        .send_frame(&daemon_hello_frame("hello")?)
        .map_err(|error| CliFailure::new(69, format!("failed to send hello frame: {error}")))?;
    let hello = socket
        .recv_frame()
        .map_err(|error| CliFailure::new(69, format!("failed to receive hello reply: {error}")))?;
    let negotiated = parse_hello_reply(&hello)?;
    require_launch_features(&negotiated.capabilities, None)?;

    let list = public_wire::WorkloadOp::List(public_wire::WorkloadListArgs::default());
    let list_response = workload_socket_exchange(&mut socket, &list, "workload list")?;
    let public_wire::WorkloadOpResponse::List(list_result) = list_response else {
        return Err(CliFailure::new(
            76,
            "daemon returned the wrong workload response to list",
        ));
    };
    let workload = select_launch_workload(list_result.workloads, &args.target)?;
    require_launch_features(&negotiated.capabilities, Some(workload.provider_kind))?;
    let item = select_launcher_item(&workload, args.item.as_deref())?;

    if item.kind == LauncherItemKind::Shell {
        return Err(CliFailure::new(
            2,
            "configured shell items require the typed `d2b shell open` command",
        ));
    }

    let item_id = ProtocolToken::parse(item.id.as_str().to_owned())
        .map_err(|_| CliFailure::new(70, "trusted launcher item id is invalid"))?;
    let operation_id = new_launch_operation_id()?;
    let target = workload.identity.canonical_target.clone();
    let launch = public_wire::WorkloadOp::LauncherExec(public_wire::LauncherExecArgs {
        target: target.clone(),
        item_id: item_id.clone(),
        operation_id: operation_id.clone(),
    });
    let response = workload_socket_exchange(&mut socket, &launch, "launcher exec")?;
    let public_wire::WorkloadOpResponse::LauncherExec(result) = response else {
        return Err(CliFailure::new(
            76,
            "daemon returned the wrong workload response to launcher exec",
        ));
    };
    let output = LaunchOutputV1 {
        command: "launch".to_owned(),
        target,
        item_id,
        operation_id,
        disposition: result.disposition,
    };
    if args.json {
        print_json(&output)?;
    } else {
        let disposition = match output.disposition {
            public_wire::LauncherExecDisposition::Committed => "committed",
            public_wire::LauncherExecDisposition::AlreadyCommitted => "already committed",
        };
        print_stdout(&format!(
            "launched {} item {} ({disposition})\n",
            output.target.to_canonical(),
            output.item_id.as_str()
        ));
    }
    Ok(0)
}

pub(super) fn require_launch_features(
    capabilities: &[d2b_contracts::FeatureFlag],
    provider: Option<d2b_realm_core::WorkloadProviderKind>,
) -> Result<(), CliFailure> {
    let has_feature = |expected| {
        capabilities
            .iter()
            .any(|feature| feature.known() == Some(expected))
    };
    if !has_feature(KnownFeatureFlag::ConfiguredLaunchV1) {
        return Err(CliFailure::new(
            70,
            "daemon does not negotiate configured-launch-v1; update d2b and d2bd together",
        ));
    }
    if provider == Some(d2b_realm_core::WorkloadProviderKind::UnsafeLocal)
        && !has_feature(KnownFeatureFlag::UnsafeLocalProviderV1)
    {
        return Err(CliFailure::new(
            70,
            "daemon does not negotiate unsafe-local-provider-v1; no local execution fallback is permitted",
        ));
    }
    Ok(())
}

pub(super) fn workload_provider_kind_label(
    provider: d2b_realm_core::WorkloadProviderKind,
) -> &'static str {
    match provider {
        d2b_realm_core::WorkloadProviderKind::LocalVm => "local-vm",
        d2b_realm_core::WorkloadProviderKind::QemuMedia => "qemu-media",
        d2b_realm_core::WorkloadProviderKind::ProviderManaged => "provider-managed",
        d2b_realm_core::WorkloadProviderKind::UnsafeLocal => "unsafe-local",
    }
}

pub(super) fn select_launch_workload(
    workloads: Vec<public_wire::WorkloadPublicSummary>,
    target: &str,
) -> Result<public_wire::WorkloadPublicSummary, CliFailure> {
    let mut candidates = workloads
        .into_iter()
        .filter(|workload| {
            workload.identity.canonical_target.to_canonical() == target
                || workload.identity.workload_id.as_str() == target
        })
        .collect::<Vec<_>>();
    match candidates.len() {
        1 => Ok(candidates.remove(0)),
        0 => Err(CliFailure::new(
            2,
            format!("workload target `{target}` was not found"),
        )),
        _ => {
            let targets = candidates
                .iter()
                .map(|workload| workload.identity.canonical_target.to_canonical())
                .collect::<Vec<_>>()
                .join(", ");
            Err(CliFailure::new(
                2,
                format!("workload id `{target}` is ambiguous; use one of: {targets}"),
            ))
        }
    }
}

pub(super) fn select_launcher_item(
    workload: &public_wire::WorkloadPublicSummary,
    requested: Option<&str>,
) -> Result<d2b_realm_core::LauncherItemSummary, CliFailure> {
    if let Some(item) = requested {
        return workload
            .launcher_items
            .iter()
            .find(|candidate| candidate.id.as_str() == item)
            .cloned()
            .ok_or_else(|| {
                CliFailure::new(
                    2,
                    format!(
                        "launcher item `{item}` is not configured for `{}`",
                        workload.identity.canonical_target.to_canonical()
                    ),
                )
            });
    }
    if let Some(default_item) = workload.default_item_id.as_ref() {
        return workload
            .launcher_items
            .iter()
            .find(|candidate| &candidate.id == default_item)
            .cloned()
            .ok_or_else(|| {
                CliFailure::new(
                    70,
                    "trusted launcher metadata names a missing default item; rebuild the bundle",
                )
            });
    }
    if let [only] = workload.launcher_items.as_slice() {
        return Ok(only.clone());
    }
    let choices = workload
        .launcher_items
        .iter()
        .map(|item| format!("{} ({})", item.id.as_str(), item.name))
        .collect::<Vec<_>>()
        .join(", ");
    Err(CliFailure::new(
        2,
        if choices.is_empty() {
            "workload has no configured launcher items".to_owned()
        } else {
            format!("launcher item is ambiguous; choose one with --item: {choices}")
        },
    ))
}

#[cfg(test)]
mod workload_launch_tests {
    use super::*;
    use d2b_contracts_control::public_wire::{
        GraphicalLaunchPosture, WorkloadAvailability, WorkloadPublicSummary,
    };
    use d2b_core::workload_identity::{WorkloadIdentity, WorkloadTarget};
    use d2b_realm_core::{
        CapabilitySet, DisplayEnvironmentPosture, EnvironmentPosture, ExecutionIdentityPosture,
        IsolationPosture, LauncherIcon, LauncherItemKind, LauncherItemSummary, ProtocolToken,
        SessionPersistencePosture, WorkloadExecutionPosture, WorkloadProviderKind, WorkloadState,
        ids::{RealmId, WorkloadId},
        realm::RealmPath,
    };

    fn item(id: &str) -> LauncherItemSummary {
        LauncherItemSummary {
            id: ProtocolToken::parse(id).unwrap(),
            name: id.to_owned(),
            icon: LauncherIcon::default(),
            kind: LauncherItemKind::Exec,
            graphical: false,
            capabilities: CapabilitySet::default(),
        }
    }

    pub(super) fn workload(
        workload_id: &str,
        realm: &str,
        items: Vec<LauncherItemSummary>,
        default_item: Option<&str>,
    ) -> WorkloadPublicSummary {
        let realm_id = RealmId::parse(realm).unwrap();
        let identity = WorkloadIdentity::new(
            WorkloadId::parse(workload_id).unwrap(),
            realm_id.clone(),
            RealmPath::new(vec![realm_id]).unwrap(),
            WorkloadTarget::parse(&format!("{workload_id}.{realm}.d2b")).unwrap(),
        );
        WorkloadPublicSummary {
            identity,
            provider_kind: WorkloadProviderKind::UnsafeLocal,
            state: WorkloadState::Stopped,
            execution_posture: WorkloadExecutionPosture {
                isolation: IsolationPosture::UnsafeLocal,
                environment: EnvironmentPosture::SystemdUserManagerAmbient,
                display_environment: DisplayEnvironmentPosture::NotApplicable,
                execution_identity: ExecutionIdentityPosture::AuthenticatedRequesterUid,
                session_persistence: SessionPersistencePosture::UserManagerLifetime,
            },
            availability: WorkloadAvailability::Ready,
            graphical_posture: GraphicalLaunchPosture::NotApplicable,
            capabilities: CapabilitySet::default(),
            launcher_items: items,
            default_item_id: default_item.map(|id| ProtocolToken::parse(id).unwrap()),
        }
    }

    #[test]
    fn target_alias_ambiguity_lists_canonical_choices() {
        let error = select_launch_workload(
            vec![
                workload("browser", "work", vec![item("open")], None),
                workload("browser", "home", vec![item("open")], None),
            ],
            "browser",
        )
        .unwrap_err();
        assert_eq!(error.exit_code, 2);
        assert!(error.message.contains("browser.work.d2b"));
        assert!(error.message.contains("browser.home.d2b"));
    }

    #[test]
    fn item_selection_covers_sole_ambiguous_and_missing_default() {
        let sole = workload("tools", "host", vec![item("only")], None);
        assert_eq!(
            select_launcher_item(&sole, None).unwrap().id.as_str(),
            "only"
        );

        let ambiguous = workload("tools", "host", vec![item("browser"), item("editor")], None);
        let error = select_launcher_item(&ambiguous, None).unwrap_err();
        assert_eq!(error.exit_code, 2);
        assert!(error.message.contains("--item"));

        let missing_default = workload("tools", "host", vec![item("browser")], Some("missing"));
        let error = select_launcher_item(&missing_default, None).unwrap_err();
        assert_eq!(error.exit_code, 70);
        assert!(error.message.contains("rebuild the bundle"));
    }

    #[test]
    fn launch_feature_skew_fails_closed() {
        let error = require_launch_features(&[], None).unwrap_err();
        assert_eq!(error.exit_code, 70);
        assert!(error.message.contains("configured-launch-v1"));

        let configured_only = [KnownFeatureFlag::ConfiguredLaunchV1.wire_value()];
        let error =
            require_launch_features(&configured_only, Some(WorkloadProviderKind::UnsafeLocal))
                .unwrap_err();
        assert_eq!(error.exit_code, 70);
        assert!(error.message.contains("unsafe-local-provider-v1"));
    }
}

pub(super) fn workload_socket_exchange(
    socket: &mut SeqpacketUnixSocket,
    op: &public_wire::WorkloadOp,
    label: &str,
) -> Result<public_wire::WorkloadOpResponse, CliFailure> {
    let request = encode_type_tagged_message("workload", op, label)?;
    socket
        .send_frame(&request)
        .map_err(|error| CliFailure::new(69, format!("failed to send {label}: {error}")))?;
    let response = socket
        .recv_frame()
        .map_err(|error| CliFailure::new(69, format!("failed to receive {label}: {error}")))?;
    let value = decode_daemon_frame(&response, label)?;
    match value.get("type").and_then(Value::as_str) {
        Some("workloadResponse") => serde_json::from_value::<WorkloadResponseFrame>(value)
            .map(|frame| frame.payload)
            .map_err(|error| {
                CliFailure::new(76, format!("failed to decode {label} response: {error}"))
            }),
        Some("error") => {
            let frame: ErrorFrame = serde_json::from_value(value).map_err(|error| {
                CliFailure::new(76, format!("failed to decode {label} error: {error}"))
            })?;
            Err(cli_failure_from_daemon_error(frame.error))
        }
        _ => Err(CliFailure::new(
            76,
            format!("daemon returned an unexpected response to {label}"),
        )),
    }
}

pub(super) fn new_launch_operation_id() -> Result<d2b_realm_core::OperationId, CliFailure> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| CliFailure::new(42, "system clock is before the Unix epoch"))?
        .as_nanos();
    d2b_realm_core::OperationId::parse(format!("launch-{}-{nanos}", std::process::id()))
        .map_err(|_| CliFailure::new(42, "failed to construct a launch operation id"))
}

pub(super) fn cmd_list(context: &LegacyContext, args: &ListArgs) -> Result<i32, CliFailure> {
    let (output, read_model) = match try_list_via_socket(context)? {
        ListSocketOutcome::Entries(entries, rm) => {
            let bundle = context.load_bundle_context().ok().flatten();
            (
                list_output_from_public_entries(&entries, bundle.as_ref()),
                rm,
            )
        }
        ListSocketOutcome::Unavailable => {
            let manifest = context.load_manifest()?;
            let bundle = context.load_bundle_context()?;
            (
                list_output_from_manifest(context, &manifest, bundle.as_ref()),
                None,
            )
        }
    };

    if args.json {
        print_json(&output)?;
    } else {
        print_stdout(&render_list_human(&output, read_model.as_ref()));
    }
    Ok(0)
}

pub(super) fn cmd_status(context: &LegacyContext, args: &StatusArgs) -> Result<i32, CliFailure> {
    let manifest = context.load_manifest()?;

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

    let selected_vm = resolve_selected_vm(context, args)?;
    if !args.json {
        match &selected_vm {
            // Single-VM status only warns about THAT VM's pending edit,
            // never unrelated VMs.
            Some(vm) => warn_pending_staged_config(vm),
            None => warn_all_pending_staged_configs(),
        }
    }
    if let Some(vm_name) = &selected_vm {
        let _ = manifest
            .get_vm(vm_name)
            .ok_or_else(|| CliFailure::new(1, format!("unknown VM '{vm_name}'")))?;
    }
    let socket_status = match try_status_via_socket(context, selected_vm.as_deref())? {
        StatusSocketOutcome::Entries(entries, rm) => Some((entries, rm)),
        StatusSocketOutcome::Unavailable => None,
    };
    let bundle = if socket_status.is_some() {
        context.load_bundle_context().ok().flatten()
    } else {
        context.load_bundle_context()?
    };

    if let Some(vm_name) = selected_vm {
        let vm = manifest
            .get_vm(&vm_name)
            .ok_or_else(|| CliFailure::new(1, format!("unknown VM '{vm_name}'")))?;
        let output = socket_status
            .as_ref()
            .and_then(|(entries, _)| entries.iter().find(|entry| entry.vm == vm.name))
            .map(|entry| build_vm_status_output_from_public(context, vm, bundle.as_ref(), entry))
            .unwrap_or_else(|| build_vm_status_output(context, vm, bundle.as_ref()));
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
        let socket_status = socket_status.as_ref();
        let output = StatusInventoryOutputV2 {
            runtime: if socket_status.is_some() {
                "daemon-public".to_owned()
            } else {
                RUNTIME_UNKNOWN.to_owned()
            },
            read_model: socket_status.as_ref().and_then(|(_, rm)| rm.clone()),
            vms: manifest
                .vms()
                .into_iter()
                .map(|vm| {
                    socket_status
                        .and_then(|(entries, _)| entries.iter().find(|entry| entry.vm == vm.name))
                        .map(|entry| {
                            build_vm_status_output_from_public(context, vm, bundle.as_ref(), entry)
                        })
                        .unwrap_or_else(|| build_vm_status_output(context, vm, bundle.as_ref()))
                })
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

pub(super) fn cmd_audit(
    context: &LegacyContext,
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

pub(super) fn cmd_console(
    context: &LegacyContext,
    args: &ConsoleArgs,
    _original_args: &[OsString],
) -> Result<i32, CliFailure> {
    use d2b_contracts_control::public_wire::{ConsoleOp, ConsoleOpResponse};
    use d2b_contracts_control::terminal_wire::TerminalStream;
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
        .map(|(rows, cols)| d2b_contracts_control::terminal_wire::TerminalSize { rows, cols })
        .unwrap_or(d2b_contracts_control::terminal_wire::TerminalSize { rows: 24, cols: 80 });

    // Attach to the console session.
    let attach_response = console_round_trip(
        &mut socket,
        &ConsoleOp::Attach(d2b_contracts_control::public_wire::ConsoleAttachArgs {
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
    if attach.provider_kind == d2b_contracts_control::public_wire::ConsoleProviderKind::QemuMedia {
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
    let _raw_guard = if is_tty {
        exec_client::FdStateGuard::enter(true, true).ok()
    } else {
        None
    };

    let mut signals = exec_client::install_signals().map_err(|err| {
        CliFailure::new(
            42,
            format!("console: failed to install signal handlers: {err}"),
        )
    })?;

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
                            &ConsoleOp::Resize(
                                d2b_contracts_control::public_wire::ConsoleResizeArgs {
                                    session: session.clone(),
                                    size: d2b_contracts_control::terminal_wire::TerminalSize {
                                        rows,
                                        cols,
                                    },
                                },
                            ),
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
                        &ConsoleOp::Close(d2b_contracts_control::public_wire::ConsoleCloseArgs {
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
                                    d2b_contracts_control::public_wire::ConsoleWriteStdinArgs {
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
                            &ConsoleOp::Close(
                                d2b_contracts_control::public_wire::ConsoleCloseArgs {
                                    session: session.clone(),
                                },
                            ),
                        );
                        print_stderr("\r\nDetached from console.\r\n");
                        return Ok(0);
                    }
                    let chunk_b64 = d2b_core::base64_codec::encode(chunk);
                    let _ = console_round_trip(
                        &mut socket,
                        &ConsoleOp::WriteStdin(
                            d2b_contracts_control::public_wire::ConsoleWriteStdinArgs {
                                session: session.clone(),
                                offset: 0,
                                chunk_base64: chunk_b64,
                                eof: false,
                            },
                        ),
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
            &ConsoleOp::ReadOutput(d2b_contracts_control::public_wire::ConsoleReadOutputArgs {
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
                                &ConsoleOp::Close(
                                    d2b_contracts_control::public_wire::ConsoleCloseArgs {
                                        session: session.clone(),
                                    },
                                ),
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
                            &ConsoleOp::Close(
                                d2b_contracts_control::public_wire::ConsoleCloseArgs {
                                    session: session.clone(),
                                },
                            ),
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
                        &ConsoleOp::Close(d2b_contracts_control::public_wire::ConsoleCloseArgs {
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

/// Encode and send a [`d2b_contracts_control::public_wire::ConsoleOp`] on `socket`, then
/// receive and parse the `consoleResponse` reply. Each call is a complete
/// round-trip.
pub(super) fn console_round_trip(
    socket: &mut SeqpacketUnixSocket,
    op: &d2b_contracts_control::public_wire::ConsoleOp,
) -> Result<d2b_contracts_control::public_wire::ConsoleOpResponse, CliFailure> {
    let frame = encode_console_op_frame(op)?;
    socket
        .send_frame(&frame)
        .map_err(|err| CliFailure::new(69, format!("console op send failed: {err}")))?;
    let reply = socket
        .recv_frame()
        .map_err(|err| CliFailure::new(69, format!("console op recv failed: {err}")))?;
    parse_console_reply(&reply)
}

/// Encode a [`d2b_contracts_control::public_wire::ConsoleOp`] as a JSON wire frame with
/// `"type": "console"`.
pub(super) fn encode_console_op_frame(
    op: &d2b_contracts_control::public_wire::ConsoleOp,
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
pub(super) fn parse_console_reply(
    bytes: &[u8],
) -> Result<d2b_contracts_control::public_wire::ConsoleOpResponse, CliFailure> {
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
/// Returns `DetachScan::Detach` with the number of bytes that appear before the
/// detach char so callers can forward them before closing.
pub(crate) fn scan_chunk_for_detach(chunk: &[u8]) -> DetachScan {
    const DETACH: u8 = b'\x1d';
    match chunk.iter().position(|&b| b == DETACH) {
        None => DetachScan::NoDetach,
        Some(pos) => DetachScan::Detach { prefix_len: pos },
    }
}

pub(super) fn cmd_audio(
    context: &LegacyContext,
    args: &AudioArgs,
    _original_args: &[OsString],
) -> Result<i32, CliFailure> {
    use d2b_contracts_control::public_wire::{
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

pub(super) fn audio_round_trip(
    context: &LegacyContext,
    op: d2b_contracts_control::public_wire::AudioOp,
) -> Result<d2b_contracts_control::public_wire::AudioOpResponse, CliFailure> {
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

pub(super) fn parse_audio_reply(
    bytes: &[u8],
) -> Result<d2b_contracts_control::public_wire::AudioOpResponse, CliFailure> {
    use d2b_contracts_control::public_wire::AudioOpResponse;
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

pub(super) fn render_audio_response(
    _context: &LegacyContext,
    response: &d2b_contracts_control::public_wire::AudioOpResponse,
    json: bool,
) -> Result<i32, CliFailure> {
    use d2b_contracts_control::public_wire::{AudioOpResponse, AudioSetApplied};
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

pub(super) fn format_enforcement(
    posture: &d2b_contracts_control::public_wire::AudioEnforcementPosture,
) -> &'static str {
    use d2b_contracts_control::public_wire::AudioEnforcementPosture;
    match posture {
        AudioEnforcementPosture::HostAndGuest => "host+guest",
        AudioEnforcementPosture::HostOnly => "host",
        AudioEnforcementPosture::GuestOnly => "guest",
        AudioEnforcementPosture::Unsupported => "unsupported",
    }
}

pub(super) fn format_channel(
    channel: &d2b_contracts_control::public_wire::AudioChannel,
) -> &'static str {
    use d2b_contracts_control::public_wire::AudioChannel;
    match channel {
        AudioChannel::Speaker => "speaker",
        AudioChannel::Microphone => "microphone",
    }
}

pub(super) fn cmd_host_check(
    context: &LegacyContext,
    args: &HostCheckArgs,
) -> Result<i32, CliFailure> {
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

pub(super) fn map_host_check_report(report: host_check::HostCheckReport) -> HostCheckOutputV2 {
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

pub(super) fn map_host_check_finding(finding: host_check::HostCheckFinding) -> HostCheckFindingV2 {
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

pub(super) fn map_host_check_severity(
    severity: host_check::HostCheckSeverity,
) -> HostCheckSeverityV2 {
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
pub(super) struct HostErrorEnvelope {
    kind: String,
    code: String,
    exit_code: i32,
    what_was_checked: String,
    observed_state: String,
    remediation: String,
    docs_anchor: String,
}

pub(super) fn host_error_envelope(
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

pub(super) fn emit_host_error(env: &HostErrorEnvelope, json: bool) -> Result<i32, CliFailure> {
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
pub(super) fn daemon_down_envelope(verb: &str) -> HostErrorEnvelope {
    host_error_envelope(
        &format!("d2b {verb} requires d2bd"),
        "daemon-down",
        1,
        "Daemon connectivity at /run/d2b/public.sock.",
        "d2bd is unreachable; the daemon is the only operator surface for mutating verbs.",
        "Start d2bd (systemctl start d2bd d2b-broker.socket) and re-run the same command. See docs/how-to/migrate-d2b-v1-0-to-v1-1.md#recovery-broker-bring-up-troubleshooting for the full bring-up checklist.",
        "docs/reference/error-codes.md#daemon-down",
    )
}

/// Typed `not-yet-implemented` envelope (exit 78) for verbs whose
/// daemon-native handler has not landed yet. No bash fallback ever
/// satisfies these - operators receive the typed envelope and the
/// migration-guide cross-link.
pub(super) fn not_yet_implemented_envelope(verb: &str) -> HostErrorEnvelope {
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
pub(super) enum DeploymentShape {
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

pub(super) fn detect_deployment_shape(
    context: &LegacyContext,
) -> Result<DeploymentShape, CliFailure> {
    // Test override used by golden CLI coverage.
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
    // Default to Tier-0 all-legacy when we can't load a bundle -
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

pub(super) fn cmd_host_prepare(
    context: &LegacyContext,
    args: &HostPrepareArgs,
) -> Result<i32, CliFailure> {
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

pub(super) fn cmd_host_destroy(
    context: &LegacyContext,
    args: &HostDestroyArgs,
) -> Result<i32, CliFailure> {
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

pub(super) fn host_shutdown_vm_phases(manifest: &ManifestDocument) -> Vec<Vec<String>> {
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

pub(super) fn render_host_shutdown_hook_plan(
    phases: &[Vec<String>],
    json: bool,
) -> Result<(), CliFailure> {
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

pub(super) fn cmd_host_shutdown_hook(
    context: &LegacyContext,
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

pub(super) fn cmd_host_doctor(
    context: &LegacyContext,
    args: &HostDoctorArgs,
) -> Result<i32, CliFailure> {
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
pub(super) struct StorageMigrationPlan {
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

pub(super) fn storage_migration_checkpoint_id(vms: &[String]) -> String {
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

pub(super) fn build_storage_migration_plan(manifest: &ManifestDocument) -> StorageMigrationPlan {
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
            "d2b-broker.service stopped",
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
            "/var/lib/d2b/component-session-<vm>",
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

pub(super) fn cmd_host_migrate_storage(
    context: &LegacyContext,
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

pub(super) fn cmd_host_validate(
    _context: &LegacyContext,
    args: &HostValidateArgs,
) -> Result<i32, CliFailure> {
    // The flag helper emits this refusal before returning an error.  Returning
    // that error to the v3 dispatcher would cause it to render a second,
    // generic Zone envelope, so keep the already-emitted host envelope as the
    // complete command result.
    if !args.dry_run && !args.apply {
        return emit_host_error(&missing_mutation_flag_envelope("host validate"), args.json);
    }
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
    // filesystem work - surface a typed envelope instead of a silent
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

pub(super) fn cmd_host_install(
    context: &LegacyContext,
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
            { "step": 1, "what": "place systemd units at /etc/systemd/system/d2bd.service + d2b-broker.socket" },
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

pub(super) fn cmd_host_reconcile(
    context: &LegacyContext,
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

pub(super) fn require_known_vm(
    context: &LegacyContext,
    vm: &str,
    json: bool,
) -> Result<(), CliFailure> {
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
pub(super) struct ResolvedRealmGateway {
    realm: String,
    gateway_vm: String,
    gateway_target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum VmTargetRoute {
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
pub(super) struct RealmEntrypointDocument {
    #[serde(rename = "schemaVersion")]
    _schema_version: u32,
    entries: BTreeMap<String, RealmEntrypointConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RealmEntrypointConfig {
    mode: String,
    gateway: Option<String>,
}

pub(super) fn safe_error_snippet(raw: &str) -> String {
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

pub(super) fn local_realm_entrypoint_config() -> RealmEntrypointConfig {
    RealmEntrypointConfig {
        mode: "host-resident".to_owned(),
        gateway: None,
    }
}

pub(super) fn normalize_realm_entrypoint_entries(
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
pub(super) struct RealmGatewayListEntry {
    realm: String,
    gateway_vm: String,
    gateway_target: String,
    state: String,
}

#[cfg(not(test))]
pub(super) fn realm_entrypoints_path() -> PathBuf {
    env_path("D2B_REALM_ENTRYPOINTS_PATH", DEFAULT_REALM_ENTRYPOINTS_PATH)
}

#[cfg(test)]
pub(super) fn realm_entrypoints_path() -> PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .expect("resolve test realm entrypoints root")
        .join(".d2b-test-missing-realm-entrypoints.json")
}

pub(super) fn load_realm_entrypoint_table()
-> Result<Option<d2b_zone_routing::RealmEntrypointTable>, CliFailure> {
    let path = realm_entrypoints_path();
    load_realm_entrypoint_table_from_path(&path)
}

pub(super) fn load_realm_entrypoint_document_from_path(
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

pub(super) fn load_realm_entrypoint_table_from_path(
    path: &Path,
) -> Result<Option<d2b_zone_routing::RealmEntrypointTable>, CliFailure> {
    let Some(doc) = load_realm_entrypoint_document_from_path(path)? else {
        return Ok(None);
    };
    let mut table = d2b_zone_routing::RealmEntrypointTable::new();
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

pub(super) fn configured_realm_gateways(
    json: bool,
) -> Result<Vec<ResolvedRealmGateway>, CliFailure> {
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

pub(super) fn gateway_vm_from_target_text(
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

pub(super) fn target_name_from_gateway_text(
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

pub(super) fn parse_gateway_target_text(
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

pub(super) fn conventional_gateway_route(
    raw: &str,
    json: bool,
) -> Result<Option<VmTargetRoute>, CliFailure> {
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

pub(super) fn emit_realm_usage_error(
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

pub(super) fn emit_missing_realm_entrypoint(
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

pub(super) fn emit_route_error(
    err: target_routing::RouteError,
    json: bool,
) -> Result<CliFailure, CliFailure> {
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
pub(super) fn print_workload_migration_hint(
    hint: &target_routing::TargetMigrationHint,
    json: bool,
) {
    if json {
        return;
    }
    print_stderr(&format!("note: {hint}\n"));
}

pub(super) fn route_vm_target(
    context: &LegacyContext,
    raw: &str,
    json: bool,
) -> Result<VmTargetRoute, CliFailure> {
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

pub(super) fn route_vm_target_with_table(
    context: &LegacyContext,
    raw: &str,
    json: bool,
    table: Option<d2b_zone_routing::RealmEntrypointTable>,
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
        let table = d2b_zone_routing::RealmEntrypointTable::with_local_default();
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

pub(super) fn resolve_realm_gateway(
    context: &LegacyContext,
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

pub(super) fn gateway_lifecycle_state(
    context: &LegacyContext,
    gateway_vm: &str,
) -> Result<Option<IpcVmLifecycleState>, CliFailure> {
    match try_list_via_socket(context)? {
        ListSocketOutcome::Entries(entries, _) => Ok(entries
            .into_iter()
            .find(|entry| entry.vm == gateway_vm || entry.name == gateway_vm)
            .map(|entry| entry.lifecycle.state)),
        ListSocketOutcome::Unavailable => Ok(None),
    }
}

pub(super) fn gateway_lifecycle_states(
    context: &LegacyContext,
) -> Result<BTreeMap<String, String>, CliFailure> {
    match try_list_via_socket(context)? {
        ListSocketOutcome::Entries(entries, _) => {
            let mut states = BTreeMap::new();
            for entry in entries {
                let label = gateway_state_label(entry.lifecycle.state).to_owned();
                states.insert(entry.vm, label.clone());
                states.insert(entry.name, label);
            }
            Ok(states)
        }
        ListSocketOutcome::Unavailable => Ok(BTreeMap::new()),
    }
}

pub(super) fn gateway_state_allows_exec(state: IpcVmLifecycleState) -> bool {
    matches!(
        state,
        IpcVmLifecycleState::Booted | IpcVmLifecycleState::Running
    )
}

pub(super) fn gateway_state_label(state: IpcVmLifecycleState) -> &'static str {
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

pub(super) fn ensure_realm_gateway_running(
    context: &LegacyContext,
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

pub(super) fn realm_gateway_exec_args(
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
        env: Vec::new(),
        cwd: None,
        vm: gateway_vm,
        json,
        human,
        management: Vec::new(),
        command: argv,
    }
}

pub(super) fn realm_policy_rows(
    context: &LegacyContext,
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

pub(super) fn realm_policy_rows_raw(
    context: &LegacyContext,
) -> Result<Vec<RealmPolicyOutputV1>, CliFailure> {
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

pub(super) fn realm_policy_rows_from_entries(
    context: &LegacyContext,
    entries: BTreeMap<String, RealmEntrypointConfig>,
) -> Result<Vec<RealmPolicyOutputV1>, CliFailure> {
    let gateway_states = gateway_lifecycle_states(context)?;
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

pub(super) fn print_realm_rows_human(rows: &[RealmPolicyOutputV1]) {
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

pub(super) fn print_realm_inspect_human(row: &RealmPolicyOutputV1) {
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

pub(super) fn cmd_realm_list(
    context: &LegacyContext,
    args: &RealmListArgs,
) -> Result<i32, CliFailure> {
    let rows = realm_policy_rows(context, args.json)?;
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

pub(super) fn cmd_realm_inspect(
    context: &LegacyContext,
    args: &RealmInspectArgs,
) -> Result<i32, CliFailure> {
    let rows = realm_policy_rows(context, args.json)?;
    let output = realm_inspect_output(&args.realm, args.json, rows)?;
    if args.json {
        print_json(&output)?;
    } else {
        print_realm_inspect_human(&output.realm);
    }
    Ok(0)
}

pub(super) fn realm_inspect_output(
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

pub(super) fn op_inspect_trace(
    args: &OpInspectArgs,
) -> Result<Option<OpInspectTraceOutputV1>, CliFailure> {
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

pub(super) fn op_inspect_output(
    context: &LegacyContext,
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

pub(super) fn op_inspect_output_from_parts(
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

pub(super) fn cmd_op_inspect(
    context: &LegacyContext,
    args: &OpInspectArgs,
) -> Result<i32, CliFailure> {
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

pub(super) fn cmd_realm_enter(
    context: &LegacyContext,
    args: &RealmEnterArgs,
) -> Result<i32, CliFailure> {
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
    cmd_vm_exec(context, &exec_args)
}

pub(super) fn cmd_realm_run(
    context: &LegacyContext,
    args: &RealmRunArgs,
) -> Result<i32, CliFailure> {
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
    cmd_vm_exec(context, &exec_args)
}

/// Route a legacy `vm <verb> <target>` argument. A local VM name routes
/// to the existing host-daemon fast path (returns `Ok`); a realm/gateway target
/// surfaces a typed, json-aware diagnostic and a non-zero exit - the host daemon
/// holds no realm configuration and cannot dispatch into a realm. The realm's
/// gateway-mode `d2bd` owns gateway-backed targets.
#[cfg(test)]
pub(super) fn guard_local_target(raw: &str, json: bool) -> Result<(), CliFailure> {
    let table = d2b_zone_routing::RealmEntrypointTable::with_local_default();
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

pub(super) fn gateway_operation_id(prefix: &str, target: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    prefix.hash(&mut h);
    target.hash(&mut h);
    std::process::id().hash(&mut h);
    format!("{prefix}-{:016x}", h.finish())
}

pub(super) fn gateway_principal() -> String {
    format!("uid-{}", Uid::current().as_raw())
}

#[cfg(test)]
pub(super) fn gateway_target_from_manifest(
    context: &LegacyContext,
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

pub(super) fn gateway_request_hash(target: &str, argv: &[String]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    target.hash(&mut h);
    argv.hash(&mut h);
    h.finish()
}

pub(super) fn gateway_display_frame(
    op: &public_wire::GatewayDisplayOp,
) -> Result<Vec<u8>, CliFailure> {
    let mut value = serde_json::to_value(op)
        .map_err(|err| CliFailure::new(1, format!("failed to encode gatewayDisplay: {err}")))?;
    let obj = value
        .as_object_mut()
        .ok_or_else(|| CliFailure::new(1, "failed to encode gatewayDisplay request"))?;
    obj.insert(
        "type".to_owned(),
        Value::String("gatewayDisplay".to_owned()),
    );
    serde_json::to_vec(&value)
        .map_err(|err| CliFailure::new(1, format!("failed to serialize gatewayDisplay: {err}")))
}

pub(super) fn dispatch_gateway_display(
    context: &LegacyContext,
    op: public_wire::GatewayDisplayOp,
) -> Result<i32, CliFailure> {
    send_gateway_display(context, op).map(|_| 0)
}

pub(super) fn send_gateway_display(
    context: &LegacyContext,
    op: public_wire::GatewayDisplayOp,
) -> Result<public_wire::GatewayDisplayOpResponse, CliFailure> {
    let frame = gateway_display_frame(&op)?;
    match try_public_socket_request(context, &frame, "gatewayDisplay")? {
        PublicSocketOutcome::Reply(response) => parse_gateway_display_reply(&response),
        PublicSocketOutcome::Unavailable => Err(CliFailure::new(
            70,
            "gatewayDisplay requires d2bd's public socket; start the realm gateway daemon and retry",
        )),
        PublicSocketOutcome::Unsupported => Err(CliFailure::new(
            78,
            "gatewayDisplay is not supported by the running daemon; restart/upgrade the realm gateway daemon",
        )),
    }
}

pub(super) fn cmd_gateway_vm_start(
    context: &LegacyContext,
    target: String,
) -> Result<i32, CliFailure> {
    dispatch_gateway_display(
        context,
        public_wire::GatewayDisplayOp::Start(public_wire::GatewayDisplayStartArgs {
            operation_id: gateway_operation_id("gw-start", &target),
            principal: gateway_principal(),
            request_hash: gateway_request_hash(&target, &[]),
            target,
        }),
    )
}

pub(super) fn cmd_gateway_vm_stop(
    context: &LegacyContext,
    target: String,
) -> Result<i32, CliFailure> {
    dispatch_gateway_display(
        context,
        public_wire::GatewayDisplayOp::Stop(public_wire::GatewayDisplayStopArgs {
            operation_id: gateway_operation_id("gw-stop", &target),
            principal: gateway_principal(),
            request_hash: gateway_request_hash(&target, &[]),
            target,
        }),
    )
}

pub(super) fn cmd_gateway_vm_restart(
    context: &LegacyContext,
    target: String,
) -> Result<i32, CliFailure> {
    cmd_gateway_vm_stop(context, target.clone())?;
    cmd_gateway_vm_start(context, target)
}

pub(super) fn cmd_gateway_vm_exec(
    context: &LegacyContext,
    target: String,
    argv: Vec<String>,
) -> Result<i32, CliFailure> {
    dispatch_gateway_display(
        context,
        public_wire::GatewayDisplayOp::Open(public_wire::GatewayDisplayOpenArgs {
            operation_id: gateway_operation_id("gw-exec", &target),
            principal: gateway_principal(),
            request_hash: gateway_request_hash(&target, &argv),
            target,
            app_argv: argv,
        }),
    )
}

pub(super) fn cmd_vm_display(
    context: &LegacyContext,
    args: &VmDisplayArgs,
) -> Result<i32, CliFailure> {
    match &args.command {
        VmDisplayCommand::List(args) => cmd_vm_display_list(context, args),
        VmDisplayCommand::Close(args) => cmd_vm_display_close(context, args),
    }
}

pub(super) fn cmd_vm_display_list(
    context: &LegacyContext,
    args: &VmDisplayListArgs,
) -> Result<i32, CliFailure> {
    let response = send_gateway_display(
        context,
        public_wire::GatewayDisplayOp::ListDetailed(public_wire::GatewayDisplayListArgs {
            target: args.target.clone(),
        }),
    )?;
    let public_wire::GatewayDisplayOpResponse::ListDetailed(result) = response else {
        return Err(CliFailure::new(
            1,
            "daemon returned an unexpected gatewayDisplay list reply",
        ));
    };
    let output = VmDisplayListOutputV1 {
        command: "vm display list".to_owned(),
        target: args.target.clone(),
        sessions: result
            .sessions
            .into_iter()
            .map(|session| VmDisplaySessionOutputV1 {
                session_id: session.session_id,
                canonical_target: session.target.clone(),
                target: session.target,
                identity_source: VmDisplayIdentitySource::D2bRealmTarget,
                state: session.state,
                operation_id: session.operation_id,
                principal: session.principal,
                capability_preflight: vm_display_capability_preflight_satisfied(),
            })
            .collect(),
    };
    if args.json {
        print_json(&output)?;
    } else {
        if output.sessions.is_empty() {
            print_stdout("No active gateway display sessions\n");
        } else {
            print_stdout(&format!(
                "{:<16} {:<40} {:<12} {:<24} {}\n",
                "SESSION_ID", "TARGET", "STATE", "OPERATION_ID", "PRINCIPAL"
            ));
            for session in &output.sessions {
                print_stdout(&format!(
                    "{:<16} {:<40} {:<12} {:<24} {}\n",
                    session.session_id,
                    session.target,
                    session.state,
                    session.operation_id,
                    session.principal
                ));
            }
        }
    }
    Ok(0)
}

pub(super) fn vm_display_capability_preflight_satisfied() -> VmDisplayCapabilityPreflight {
    VmDisplayCapabilityPreflight {
        status: VmDisplayCapabilityPreflightStatus::Satisfied,
        required_capabilities: vec!["window-forwarding".to_owned()],
        advertised_capabilities: vec!["window-forwarding".to_owned()],
        missing_capabilities: Vec::new(),
    }
}

pub(super) fn cmd_vm_display_close(
    context: &LegacyContext,
    args: &VmDisplayCloseArgs,
) -> Result<i32, CliFailure> {
    let response = send_gateway_display(
        context,
        public_wire::GatewayDisplayOp::Close(public_wire::GatewayDisplayCloseArgs {
            session_id: args.session_id.clone(),
        }),
    )?;
    let public_wire::GatewayDisplayOpResponse::Close(result) = response else {
        return Err(CliFailure::new(
            1,
            "daemon returned an unexpected gatewayDisplay close reply",
        ));
    };
    let output = VmDisplayCloseOutputV1 {
        command: "vm display close".to_owned(),
        session_id: args.session_id.clone(),
        closed: result.closed,
    };
    if args.json {
        print_json(&output)?;
    } else if output.closed {
        print_stdout(&format!(
            "Closed gateway display session {}\n",
            output.session_id
        ));
    } else {
        print_stdout(&format!(
            "Gateway display session {} was not active\n",
            output.session_id
        ));
    }
    Ok(0)
}

pub(super) fn vm_is_qemu_media_runtime(
    context: &LegacyContext,
    vm: &str,
) -> Result<bool, CliFailure> {
    let manifest = context.load_manifest()?;
    Ok(manifest
        .get_vm(vm)
        .and_then(|entry| entry.runtime.as_ref())
        .is_some_and(|runtime| runtime.kind == "qemu-media"))
}

pub(super) fn vm_dag_dry_run_summary(
    verb: &str,
    vm: &str,
    qemu_media: bool,
    force: bool,
) -> serde_json::Value {
    // The DAG the supervisor would drive. Mirrors the structure emitted
    // by the processes::VmProcessDag exporter - for the headless alpha
    // shape (host-reconcile → store-preflight → virtiofsd-ro-store → ch
    // → component-session-health) we summarize the node ids and the
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
                serde_json::json!({"id": "component-session-health",  "role": "component-session-health"}),
            ],
            serde_json::json!([
                {"from": "host-reconcile",     "to": "store-preflight"},
                {"from": "store-preflight",    "to": "virtiofsd-ro-store"},
                {"from": "virtiofsd-ro-store", "to": "ch"},
                {"from": "ch",                 "to": "component-session-health"},
            ]),
            serde_json::json!([
                "component-session-health",
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

pub(super) struct VmLifecycleInvocation<'a> {
    verb: &'a str,
    vm: &'a str,
    dry_run: bool,
    apply: bool,
    no_wait_api: bool,
    force: bool,
    json: bool,
}

pub(super) fn cmd_vm_lifecycle_verb(
    context: &LegacyContext,
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
                return match verb {
                    "start" => cmd_gateway_vm_start(context, target),
                    "stop" => cmd_gateway_vm_stop(context, target),
                    "restart" => cmd_gateway_vm_restart(context, target),
                    _ => unreachable!("unknown gateway lifecycle verb"),
                };
            }
            let summary = serde_json::json!({
                "command": format!("vm {verb}"),
                "mode": "dry-run",
                "target": target,
                "realm": realm,
                "gateway": gateway,
                "gatewayVm": gateway_vm,
                "notes": "realm target would route through the configured gateway entrypoint; --apply preserves the gatewayDisplay compatibility path while the guarded transition path exists.",
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
        // VM lifecycle verbs are daemon-only. The bash-translation
        // bridge has been removed; any failure mode
        // surfaces as a typed envelope via `dispatch_mutating_verb`.
        let request_type = match verb {
            "start" => "vmStart",
            "stop" => "vmStop",
            "restart" => "vmRestart",
            other => other,
        };
        let mut extra_fields = serde_json::Map::new();
        extra_fields.insert("vm".to_owned(), serde_json::Value::String(vm));
        if no_wait_api {
            extra_fields.insert("noWaitApi".to_owned(), serde_json::Value::Bool(true));
        }
        if force {
            extra_fields.insert("force".to_owned(), serde_json::Value::Bool(true));
        }
        return dispatch_mutating_verb(
            context,
            request_type,
            serde_json::Value::Object(extra_fields),
            flags.dry_run,
            flags.apply,
            json,
        );
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
                "vm {verb} --dry-run: would drive the 5-node DAG for vm '{vm}'{force_note} (host-reconcile → store-preflight → virtiofsd-ro-store → ch → component-session-health)\n"
            ));
        }
    }
    Ok(0)
}

pub(super) fn cmd_vm_start(context: &LegacyContext, args: &VmStartArgs) -> Result<i32, CliFailure> {
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

pub(super) fn cmd_vm_stop(context: &LegacyContext, args: &VmStopArgs) -> Result<i32, CliFailure> {
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

pub(super) fn cmd_vm_restart(
    context: &LegacyContext,
    args: &VmRestartArgs,
) -> Result<i32, CliFailure> {
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

pub(super) fn cmd_vm_list(context: &LegacyContext, args: &VmListArgs) -> Result<i32, CliFailure> {
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
        return cmd_vm_exec(context, &exec_args);
    }
    if args.all {
        return cmd_vm_list_all(context, args);
    }
    cmd_vm_list_local(context, args)
}

pub(super) fn cmd_vm_list_all(
    context: &LegacyContext,
    args: &VmListArgs,
) -> Result<i32, CliFailure> {
    let local_entries = match try_list_via_socket(context)? {
        ListSocketOutcome::Entries(entries, _) => entries,
        ListSocketOutcome::Unavailable => Vec::new(),
    };
    let gateway_entries = configured_realm_gateways(args.json)?
        .into_iter()
        .map(|gateway| {
            let state = gateway_lifecycle_state(context, &gateway.gateway_vm)
                .ok()
                .flatten()
                .map(gateway_state_label)
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
                let _ = writeln!(
                    rendered,
                    "{}\t{}\t{}",
                    entry.vm,
                    public_lifecycle_status_label(&entry.lifecycle),
                    entry.runtime.detail
                );
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

pub(super) fn cmd_vm_list_local(
    context: &LegacyContext,
    args: &VmListArgs,
) -> Result<i32, CliFailure> {
    match try_list_via_socket(context)? {
        ListSocketOutcome::Entries(entries, _) => {
            if args.json {
                let body = serde_json::json!({
                    "command": "vm list",
                    "entries": entries,
                });
                let mut rendered = serde_json::to_string_pretty(&body).map_err(|err| {
                    CliFailure::new(1, format!("failed to serialize vm list: {err}"))
                })?;
                rendered.push('\n');
                print_stdout(&rendered);
                return Ok(0);
            }
            if entries.is_empty() {
                print_stdout("vm list: no daemon runtime entries reported\n");
            } else {
                let mut rendered = String::from("VM\tSTATE\tRUNTIME\n");
                for entry in entries {
                    let _ = writeln!(
                        rendered,
                        "{}\t{}\t{}",
                        entry.vm,
                        public_lifecycle_status_label(&entry.lifecycle),
                        entry.runtime.detail
                    );
                }
                print_stdout(&rendered);
            }
        }
        ListSocketOutcome::Unavailable => {
            let note = "vm list requires d2bd's public socket; start or restart d2bd and retry.";
            if args.json {
                let body = serde_json::json!({
                    "command": "vm list",
                    "entries": [],
                    "notes": note,
                });
                let mut rendered = serde_json::to_string_pretty(&body).map_err(|err| {
                    CliFailure::new(1, format!("failed to serialize vm list: {err}"))
                })?;
                rendered.push('\n');
                print_stdout(&rendered);
            } else {
                let mut rendered = String::from("vm list: ");
                rendered.push_str(note);
                rendered.push('\n');
                print_stdout(&rendered);
            }
        }
    }
    Ok(0)
}

pub(super) fn cmd_vm_status(
    context: &LegacyContext,
    args: &VmStatusArgs,
) -> Result<i32, CliFailure> {
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

/// The resource-owner transport: one Resource attach request establishes the
/// Process stream, then every operation is a correlated named-stream frame
/// over the held public.sock seqpacket connection.
pub(super) struct OwnerSocketTransport {
    socket: SeqpacketUnixSocket,
    next_op_id: u64,
    stdin_offset: u64,
    resource_ref: Option<String>,
}

impl Drop for OwnerSocketTransport {
    fn drop(&mut self) {
        if self.resource_ref.is_none() {
            return;
        }
        let request_id = self.next_op_id;
        if request_id == 0 {
            return;
        }
        let frame = public_wire::NamedProcessStreamRequestFrame::new(
            request_id,
            public_wire::NamedProcessStreamRequest::Cancel,
        );
        if let Ok(bytes) = serde_json::to_vec(&frame) {
            let _ = self.socket.send_frame(&bytes);
        }
    }
}

impl terminal_client::TerminalTransport for OwnerSocketTransport {
    type Op = d2b_contracts_control::public_wire::ExecOp;
    type Response = d2b_contracts_control::public_wire::ExecOpResponse;
    type Error = exec_client::ExecClientError;

    fn round_trip(
        &mut self,
        op: &d2b_contracts_control::public_wire::ExecOp,
    ) -> Result<d2b_contracts_control::public_wire::ExecOpResponse, exec_client::ExecClientError>
    {
        let op_id = self.next_op_id;
        self.next_op_id = self.next_op_id.wrapping_add(1);
        if op_id == 0 {
            return Err(exec_client::ExecClientError::protocol(
                "process stream request id exhausted",
            ));
        }
        if !matches!(op, public_wire::ExecOp::Start(_)) && self.resource_ref.is_none() {
            return Err(exec_client::ExecClientError::protocol(
                "process stream operation preceded resource attach",
            ));
        }
        let frame = if let public_wire::ExecOp::Start(start) = op {
            if self.resource_ref.is_some() {
                return Err(exec_client::ExecClientError::protocol(
                    "duplicate process resource attach",
                ));
            }
            encode_process_resource_attach(start, op_id)?
        } else {
            let frame = exec_client::named_stream_request_frame(op, op_id, self.stdin_offset)?;
            serde_json::to_vec(&frame).map_err(|_| {
                exec_client::ExecClientError::protocol(
                    "process named-stream request frame was malformed",
                )
            })?
        };
        self.socket.send_frame(&frame).map_err(|err| {
            exec_client::ExecClientError::transport(format!("process stream send failed: {err}"))
        })?;
        let reply = self.socket.recv_frame().map_err(|err| {
            exec_client::ExecClientError::transport(format!("process stream recv failed: {err}"))
        })?;
        let response = if let public_wire::ExecOp::Start(start) = op {
            let result = decode_process_resource_attach(&reply)?;
            self.resource_ref = Some(format!("EphemeralProcess/exec-{op_id}"));
            let _ = start;
            public_wire::ExecOpResponse::Start(result)
        } else {
            let (response_id, response) = exec_client::named_stream_response_frame(op, &reply)?;
            if response_id != op_id {
                return Err(exec_client::ExecClientError::protocol(
                    "process named-stream response id did not match request",
                ));
            }
            response
        };
        if let public_wire::ExecOpResponse::WriteStdin(result) = &response {
            self.stdin_offset = result.next_offset;
        }
        Ok(response)
    }
}

impl OwnerSocketTransport {
    fn resource_management_round_trip(
        &mut self,
        op: &public_wire::ExecOp,
    ) -> Result<public_wire::ExecOpResponse, exec_client::ExecClientError> {
        let request = encode_process_resource_management(op)?;
        self.socket.send_frame(&request).map_err(|err| {
            exec_client::ExecClientError::transport(format!(
                "process resource request send failed: {err}"
            ))
        })?;
        let reply = self.socket.recv_frame().map_err(|err| {
            exec_client::ExecClientError::transport(format!(
                "process resource response receive failed: {err}"
            ))
        })?;
        exec_client::decode_exec_response_frame(&reply)
    }

    fn resource_detached_create_round_trip(
        &mut self,
        start: &public_wire::ExecStartArgs,
    ) -> Result<public_wire::ExecOpResponse, exec_client::ExecClientError> {
        let request = encode_process_resource_detached_create(start, self.next_op_id)?;
        self.next_op_id = self.next_op_id.wrapping_add(1);
        self.socket.send_frame(&request).map_err(|err| {
            exec_client::ExecClientError::transport(format!(
                "detached Process resource request send failed: {err}"
            ))
        })?;
        let reply = self.socket.recv_frame().map_err(|err| {
            exec_client::ExecClientError::transport(format!(
                "detached Process resource response receive failed: {err}"
            ))
        })?;
        exec_client::decode_exec_response_frame(&reply)
    }
}

fn encode_process_resource_management(
    op: &public_wire::ExecOp,
) -> Result<Vec<u8>, exec_client::ExecClientError> {
    let (method, vm, resource_ref, extra) = match op {
        public_wire::ExecOp::List(args) => ("List", args.vm.clone(), None, serde_json::json!({})),
        public_wire::ExecOp::Status(args) => (
            "Status",
            args.vm.clone(),
            Some(format!("EphemeralProcess/{}", args.exec_id)),
            serde_json::json!({}),
        ),
        public_wire::ExecOp::Logs(args) => (
            "Logs",
            args.vm.clone(),
            Some(format!("EphemeralProcess/{}", args.exec_id)),
            serde_json::json!({
                "stdoutOffset": args.stdout_offset,
                "stderrOffset": args.stderr_offset,
                "maxLen": args.max_len,
            }),
        ),
        public_wire::ExecOp::Kill(args) => (
            "Kill",
            args.vm.clone(),
            Some(format!("EphemeralProcess/{}", args.exec_id)),
            serde_json::json!({}),
        ),
        _ => {
            return Err(exec_client::ExecClientError::protocol(
                "operation is not detached Process resource management",
            ));
        }
    };
    let mut request = serde_json::json!({
        "type": "resourceRequest",
        "method": method,
        "service": "d2b.resource.v3",
        "sessionVerb": "invoke",
        "zoneRef": "Zone/local-root",
        "resourceType": "EphemeralProcess",
        "executionRef": format!("Guest/{vm}"),
    });
    if let Some(resource_ref) = resource_ref {
        request["resourceRef"] = Value::String(resource_ref);
    }
    if let (Some(object), Some(extra)) = (request.as_object_mut(), extra.as_object()) {
        object.extend(
            extra
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
    }
    serde_json::to_vec(&request).map_err(|_| {
        exec_client::ExecClientError::protocol("process resource management request was malformed")
    })
}

fn encode_process_resource_attach(
    start: &public_wire::ExecStartArgs,
    request_id: u64,
) -> Result<Vec<u8>, exec_client::ExecClientError> {
    let resource_ref = format!("EphemeralProcess/exec-{request_id}");
    let initial_size = start
        .term_size
        .map(|size| serde_json::json!({ "rows": size.rows, "cols": size.cols }));
    let request = serde_json::json!({
        "type": "resourceRequest",
        "method": "Create",
        "service": "d2b.resource.v3",
        "sessionVerb": "attach",
        "zoneRef": "Zone/local-root",
        "resourceType": "EphemeralProcess",
        "resourceRef": resource_ref,
        "executionRef": format!("Guest/{}", start.vm),
        "interactive": start.tty,
        "tty": start.tty,
        "initialSize": initial_size,
        "detached": false,
        "argv": start.argv,
        "env": start.env,
        "cwd": start.cwd,
        "opId": request_id,
    });
    serde_json::to_vec(&request).map_err(|_| {
        exec_client::ExecClientError::protocol("process resource attach request was malformed")
    })
}

fn encode_process_resource_detached_create(
    start: &public_wire::ExecStartArgs,
    request_id: u64,
) -> Result<Vec<u8>, exec_client::ExecClientError> {
    let request = serde_json::json!({
        "type": "resourceRequest",
        "method": "Create",
        "service": "d2b.resource.v3",
        "sessionVerb": "invoke",
        "zoneRef": "Zone/local-root",
        "resourceType": "EphemeralProcess",
        "resourceRef": format!("EphemeralProcess/exec-{request_id}"),
        "executionRef": format!("Guest/{}", start.vm),
        "interactive": false,
        "tty": false,
        "initialSize": Value::Null,
        "detached": true,
        "argv": start.argv,
        "env": start.env,
        "cwd": start.cwd,
        "opId": request_id,
    });
    serde_json::to_vec(&request).map_err(|_| {
        exec_client::ExecClientError::protocol(
            "detached Process resource create request was malformed",
        )
    })
}

fn decode_process_resource_attach(
    bytes: &[u8],
) -> Result<public_wire::ExecStartResult, exec_client::ExecClientError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| {
        exec_client::ExecClientError::protocol("process resource attach response was malformed")
    })?;
    if value.get("type").and_then(Value::as_str) == Some("error") {
        let error = value.get("error").unwrap_or(&value);
        let kind = error
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("resource-provider-unavailable");
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("the Process resource attach was refused");
        let remediation = error
            .get("remediation")
            .and_then(Value::as_str)
            .unwrap_or("");
        return Err(exec_client::ExecClientError::from_daemon_error(
            kind,
            message,
            remediation,
        ));
    }
    if value.get("attached").and_then(Value::as_bool) != Some(true) {
        return Err(exec_client::ExecClientError::protocol(
            "process resource attach did not establish a stream",
        ));
    }
    let session = value
        .get("session")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            exec_client::ExecClientError::protocol(
                "process resource attach response omitted its session",
            )
        })?;
    let tty = value.get("tty").and_then(Value::as_bool).unwrap_or(false);
    let stdout_offset = value
        .get("stdoutOffset")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let stderr_offset = value
        .get("stderrOffset")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Ok(public_wire::ExecStartResult {
        session: session.to_owned(),
        tty,
        stdout_offset,
        stderr_offset,
    })
}

/// Typed transport error for an unreachable daemon on the exec path: there is
/// no SSH fallback, so an absent/unreachable daemon is a transport failure.
pub(super) fn exec_daemon_unavailable_error() -> exec_client::ExecClientError {
    exec_client::ExecClientError::transport(
        "vm exec: the d2b daemon is not reachable on its public socket; \
         start d2bd and retry (d2b does not fall back to SSH)",
    )
}

pub(super) fn exec_owner_transport(
    context: &LegacyContext,
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
        next_op_id: 1,
        stdin_offset: 0,
        resource_ref: None,
    })
}

/// Render a typed exec-client error as a CliFailure carrying the CLI exec
/// exit-code contract. The wire `kind` slug + message + remediation are
/// redaction-safe (no argv/env/output bytes).
pub(super) fn exec_error_to_failure(error: exec_client::ExecClientError) -> CliFailure {
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
pub(super) fn exec_terminate(
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
pub(super) fn exec_usage_terminate(
    args: &VmExecArgs,
    message: impl Into<String>,
) -> Result<i32, CliFailure> {
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
pub(super) struct VmExecParsedAction {
    json: bool,
    management: Option<VmExecManagementCommand>,
}

pub(super) fn exec_effective_json(args: &VmExecArgs) -> bool {
    args.json
        || args
            .management
            .iter()
            .any(|value| value.to_str() == Some("--json"))
}

pub(super) fn parse_vm_exec_action(args: &VmExecArgs) -> Result<VmExecParsedAction, String> {
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

pub(super) fn parse_vm_exec_logs_args(words: &[String]) -> Result<VmExecLogsArgs, String> {
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

pub(super) fn parse_vm_exec_u64_flag(flag: &str, value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("vm exec logs: {flag} must be a non-negative integer"))
}

/// Run a command inside a Guest Process resource (FSM). Establishes the
/// resource owner over `public.sock` (admin-only), then multiplexes
/// stdin/stdout/signals over one authenticated named stream. The guest owns
/// the PTY; the CLI only manages host terminal state.
pub(super) fn cmd_vm_exec(context: &LegacyContext, args: &VmExecArgs) -> Result<i32, CliFailure> {
    use d2b_contracts_control::public_wire::{ExecEnvVar, ExecOp, ExecStartArgs, ExecTermSize};

    // 1. Validate flags BEFORE touching host terminal state or the daemon.
    let action = match parse_vm_exec_action(args) {
        Ok(action) => action,
        Err(message) => return exec_usage_terminate(args, message),
    };
    if let Some(management) = action.management.as_ref() {
        let route = route_vm_target(context, &args.vm, action.json)?;
        return match route {
            VmTargetRoute::Local { vm } => cmd_vm_exec_management(context, args, management, &vm),
            VmTargetRoute::Gateway { .. } => exec_usage_terminate(
                args,
                "vm exec: detached management verbs for realm targets are not available on the host; use `d2b realm run <realm> -- d2b vm exec <target> list`",
            ),
        };
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
    // The target-local Process forwards stdin only in PTY mode: its non-TTY
    // validators reject an open stdin, so `-i`/`--interactive` without
    // `-t`/`--tty` would create a stdin-closed exec the CLI then tries to
    // write to. Require a PTY for stdin forwarding rather than fail
    // deterministically once stdin is piped.
    if args.interactive && !args.tty {
        return exec_usage_terminate(
            args,
            "vm exec: -i/--interactive requires -t/--tty; the component-session \
             transport forwards stdin only in PTY mode. Use `-it`, or drop \
             `-i` to run a stdin-closed command.",
        );
    }
    if args.command.is_empty() {
        return exec_usage_terminate(
            args,
            "vm exec: missing command; pass it after `--` (e.g. `d2b vm exec myvm -- ls`)",
        );
    }
    let tty = args.tty;
    let interactive = args.interactive || args.tty;

    let mut env_vars = Vec::with_capacity(args.env.len());
    for (idx, entry) in args.env.iter().enumerate() {
        // Redaction: never echo the raw --env entry - it may carry a
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

    let local_vm = match route_vm_target(context, &args.vm, action.json)? {
        VmTargetRoute::Local { vm } => vm,
        VmTargetRoute::Gateway { target, .. } => {
            if args.detach
                || args.interactive
                || args.tty
                || !args.env.is_empty()
                || args.cwd.is_some()
            {
                return exec_usage_terminate(
                    args,
                    "vm exec: gateway-backed targets currently support non-interactive foreground commands without -d/-i/-t, --env, or --cwd",
                );
            }
            return cmd_gateway_vm_exec(context, target, args.command.clone());
        }
    };

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
        vm: local_vm,
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
    let start_response = match if args.detach {
        let public_wire::ExecOp::Start(start) = &start_op else {
            unreachable!("the command path always builds ExecOp::Start");
        };
        transport.resource_detached_create_round_trip(start)
    } else {
        transport.round_trip(&start_op)
    } {
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
                );
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
                );
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

pub(super) fn cmd_vm_exec_management(
    context: &LegacyContext,
    args: &VmExecArgs,
    management: &VmExecManagementCommand,
    vm: &str,
) -> Result<i32, CliFailure> {
    use d2b_contracts_control::public_wire::{
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
                ExecOp::List(ExecDetachedListArgs { vm: vm.to_owned() }),
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
                    vm: vm.to_owned(),
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
                    vm: vm.to_owned(),
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
                    vm: vm.to_owned(),
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

pub(super) fn exec_send_one_op(
    context: &LegacyContext,
    op: d2b_contracts_control::public_wire::ExecOp,
) -> Result<d2b_contracts_control::public_wire::ExecOpResponse, exec_client::ExecClientError> {
    let mut transport = exec_owner_transport(context)?;
    transport.resource_management_round_trip(&op)
}

pub(super) fn exec_render_detached_create(
    args: &VmExecArgs,
    result: &d2b_contracts_control::public_wire::ExecDetachedCreateResult,
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

pub(super) fn exec_render_detached_list(
    args: &VmExecArgs,
    result: &d2b_contracts_control::public_wire::ExecDetachedListResult,
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

pub(super) fn exec_render_detached_status(
    args: &VmExecArgs,
    result: &d2b_contracts_control::public_wire::ExecDetachedStatusResult,
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

pub(super) fn exec_render_detached_logs(
    args: &VmExecArgs,
    result: &d2b_contracts_control::public_wire::ExecDetachedLogsResult,
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

pub(super) fn exec_decode_detached_logs(
    result: &d2b_contracts_control::public_wire::ExecDetachedLogsResult,
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

pub(super) fn exec_render_detached_kill(
    args: &VmExecArgs,
    result: &d2b_contracts_control::public_wire::ExecDetachedKillResult,
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

pub(super) fn exec_print_json<T: Serialize>(value: &T) -> Result<(), CliFailure> {
    let value = serde_json::to_value(value)
        .map_err(|err| CliFailure::new(1, format!("vm exec: failed to serialize JSON: {err}")))?;
    print_exec_json(&value)
}

pub(super) fn exec_state_label(
    state: d2b_contracts_control::public_wire::ExecState,
) -> &'static str {
    use d2b_contracts_control::public_wire::ExecState;

    match state {
        ExecState::Created => "created",
        ExecState::Running => "running",
        ExecState::Exited => "exited",
        ExecState::Signaled => "signaled",
        ExecState::Cancelled => "cancelled",
        ExecState::SlowConsumerCancelled => "slow-consumer-cancelled",
        ExecState::ProtocolError => "protocol-error",
        ExecState::LostTarget => "lost-target",
        ExecState::Reaped => "reaped",
    }
}

pub(super) fn exec_kill_outcome_label(
    outcome: d2b_contracts_control::public_wire::ExecDetachedKillOutcome,
) -> &'static str {
    use d2b_contracts_control::public_wire::ExecDetachedKillOutcome;

    match outcome {
        ExecDetachedKillOutcome::Cancelling => "cancelling",
        ExecDetachedKillOutcome::AlreadyTerminal => "already-terminal",
    }
}

pub(super) fn exec_terminal_summary(
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

pub(super) fn exec_loss_summary(dropped_bytes: u64, truncated: bool) -> String {
    format!(
        "{dropped_bytes}/{}",
        if truncated { "truncated" } else { "complete" }
    )
}

pub(super) fn exec_list_offsets_summary(
    entry: &d2b_contracts_control::public_wire::ExecDetachedListEntry,
) -> String {
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

pub(super) fn exec_list_loss_summary(
    entry: &d2b_contracts_control::public_wire::ExecDetachedListEntry,
) -> String {
    format!(
        "all={} stdout={} stderr={}",
        exec_loss_summary(entry.dropped_bytes, entry.truncated),
        exec_loss_summary(entry.stdout_dropped_bytes, entry.stdout_truncated),
        exec_loss_summary(entry.stderr_dropped_bytes, entry.stderr_truncated)
    )
}

pub(super) fn exec_logs_incomplete(
    result: &d2b_contracts_control::public_wire::ExecDetachedLogsResult,
) -> bool {
    result.dropped_bytes > 0
        || result.truncated
        || result.stdout_dropped_bytes > 0
        || result.stderr_dropped_bytes > 0
        || result.stdout_truncated
        || result.stderr_truncated
}

pub(super) fn exec_logs_warning(
    result: &d2b_contracts_control::public_wire::ExecDetachedLogsResult,
) -> String {
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
pub(super) fn exec_json_base(args: &VmExecArgs) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    map.insert("command".to_owned(), Value::String("vm exec".to_owned()));
    map.insert("vm".to_owned(), Value::String(args.vm.clone()));
    map
}

/// Append the bounded, charset-safe captured guest output to a JSON envelope.
pub(super) fn exec_json_attach_output(
    map: &mut serde_json::Map<String, Value>,
    host: &exec_client::CapturingHostIo,
) {
    map.insert(
        "stdoutBase64".to_owned(),
        Value::String(d2b_core::base64_codec::encode(host.stdout())),
    );
    map.insert(
        "stderrBase64".to_owned(),
        Value::String(d2b_core::base64_codec::encode(host.stderr())),
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
/// kinds surface through `exec_terminate` as transport/protocol failures.
pub(super) fn exec_json_success_value(
    args: &VmExecArgs,
    outcome: &exec_client::ExecOutcome,
    host: &exec_client::CapturingHostIo,
) -> (Value, i32) {
    use d2b_contracts_control::public_wire::ExecTerminalStatus;

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
pub(super) fn exec_json_success(
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
pub(super) fn exec_json_failure_value(
    args: &VmExecArgs,
    error: &exec_client::ExecClientError,
) -> Value {
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
pub(super) fn print_exec_json(value: &Value) -> Result<(), CliFailure> {
    let mut rendered = serde_json::to_string_pretty(value)
        .map_err(|err| CliFailure::new(1, format!("vm exec: failed to serialize JSON: {err}")))?;
    rendered.push('\n');
    print_stdout(&rendered);
    Ok(())
}

// ---- store-lifecycle CLI verbs ----

pub(super) fn w7_dry_run_summary(verb: &str, vm: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "command": verb,
        "mode": "dry-run",
        "vm": vm,
        "planned": [],
        "notes": format!("d2b {verb} --dry-run reports the planned operation; --apply routes through d2bd → broker."),
    })
}

pub(super) fn cmd_build(context: &LegacyContext, args: &BuildArgs) -> Result<i32, CliFailure> {
    // build is non-destructive - always allowed; never returns
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

pub(super) fn cmd_generations(
    context: &LegacyContext,
    args: &GenerationsArgs,
) -> Result<i32, CliFailure> {
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

pub(super) fn w7_mutating_verb(
    context: &LegacyContext,
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

pub(super) fn cmd_switch(
    context: &LegacyContext,
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

pub(super) fn cmd_boot(
    context: &LegacyContext,
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

pub(super) fn cmd_test(
    context: &LegacyContext,
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

pub(super) fn cmd_rollback(
    context: &LegacyContext,
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

pub(super) fn cmd_gc(
    context: &LegacyContext,
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

pub(super) fn cmd_store_verify(
    context: &LegacyContext,
    args: &StoreVerifyArgs,
) -> Result<i32, CliFailure> {
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

pub(super) fn store_verify_exit_code(status: IpcStoreVerifyStatus) -> i32 {
    match status {
        IpcStoreVerifyStatus::Ok | IpcStoreVerifyStatus::Repaired => 0,
        IpcStoreVerifyStatus::Drift | IpcStoreVerifyStatus::Unknown => 4,
        IpcStoreVerifyStatus::NotFound => 70,
        IpcStoreVerifyStatus::Failed => 78,
    }
}

pub(super) fn store_verify_cli_envelope(response: &IpcStoreVerifyResponse) -> StoreVerifyOutputV2 {
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

pub(super) fn render_store_verify_human(response: &IpcStoreVerifyResponse) -> String {
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

pub(super) fn usb_json_mode(json: bool, human: bool) -> bool {
    if human { false } else { json }
}

pub(super) fn cmd_usb_attach(
    context: &LegacyContext,
    args: &UsbAttachArgs,
) -> Result<i32, CliFailure> {
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

pub(super) fn cmd_usb_detach(
    context: &LegacyContext,
    args: &UsbDetachArgs,
) -> Result<i32, CliFailure> {
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

pub(super) fn removed_usb_enroll_failure(raw_args: &[OsString]) -> Option<CliFailure> {
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
pub(super) fn usb_mutating_verb(
    context: &LegacyContext,
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
            "TargetUsbipImport(attach)",
        ]
    } else {
        vec![
            "TargetUsbipImport(detach)",
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
            "USBIP dry-run reports the daemon → broker bind/lock, firewall, backend/proxy ensurement, reconcile plan, and authenticated target-local import without mutating host or guest state."
        } else {
            "USBIP dry-run reports authenticated target-local import cleanup plus the daemon → broker unbind/reconcile plan without mutating host or guest state."
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
                "d2b {verb} --dry-run: would {action} busid '{bus_id}' for vm '{vm}', reconcile the USBIP proxy, and ask the target-local Process to import the device\n"
            ));
        } else {
            print_stdout(&format!(
                "d2b {verb} --dry-run: would ask the target-local Process to detach busid '{bus_id}' for vm '{vm}', {action} it on the host, and reconcile the USBIP proxy\n"
            ));
        }
    }
    Ok(0)
}

pub(super) fn cmd_usb_probe(
    context: &LegacyContext,
    args: &UsbProbeArgs,
) -> Result<i32, CliFailure> {
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

pub(super) fn render_usb_probe_human(entries: &[IpcUsbipProbeEntry]) -> String {
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

pub(super) fn usb_probe_status_label(status: IpcUsbipProbeStatus) -> &'static str {
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

pub(super) fn durable_claim_label(state: public_wire::UsbipDurableClaimState) -> &'static str {
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

pub(super) fn host_bind_label(state: public_wire::UsbipHostBindState) -> &'static str {
    match state {
        public_wire::UsbipHostBindState::Unbound => "unbound",
        public_wire::UsbipHostBindState::BoundToUsbipHost => "bound-to-usbip-host",
        public_wire::UsbipHostBindState::BoundToUnexpectedDriver => "bound-to-unexpected-driver",
        public_wire::UsbipHostBindState::DeviceMissing => "device-missing",
        public_wire::UsbipHostBindState::Unknown => "unknown",
        public_wire::UsbipHostBindState::NotApplicable => "not-applicable",
    }
}

pub(super) fn host_carrier_label(state: public_wire::UsbipHostCarrierState) -> &'static str {
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

pub(super) fn proxy_label(state: public_wire::UsbipProxyState) -> &'static str {
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

pub(super) fn guest_import_label(state: public_wire::UsbipGuestImportState) -> &'static str {
    match state {
        public_wire::UsbipGuestImportState::Detached => "detached",
        public_wire::UsbipGuestImportState::Imported => "imported",
        public_wire::UsbipGuestImportState::Unavailable => "unavailable",
        public_wire::UsbipGuestImportState::Unknown => "unknown",
        public_wire::UsbipGuestImportState::NotApplicable => "not-applicable",
    }
}

pub(super) fn topology_label(state: public_wire::UsbipTopologyState) -> &'static str {
    match state {
        public_wire::UsbipTopologyState::Match => "match",
        public_wire::UsbipTopologyState::Mismatch => "mismatch",
        public_wire::UsbipTopologyState::Incomplete => "incomplete",
        public_wire::UsbipTopologyState::NotObserved => "not-observed",
        public_wire::UsbipTopologyState::NotApplicable => "not-applicable",
        public_wire::UsbipTopologyState::Unknown => "unknown",
    }
}

pub(super) fn policy_label(state: public_wire::UsbipPolicyState) -> &'static str {
    match state {
        public_wire::UsbipPolicyState::Allowed => "allowed",
        public_wire::UsbipPolicyState::Denied => "denied",
        public_wire::UsbipPolicyState::Missing => "missing",
        public_wire::UsbipPolicyState::NotApplicable => "not-applicable",
        public_wire::UsbipPolicyState::Unknown => "unknown",
    }
}

pub(super) fn reason_code_label(code: public_wire::UsbipProbeDegradedReasonCode) -> &'static str {
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

pub(super) fn usb_sk_json_mode(json: bool, human: bool) -> bool {
    if human { false } else { json }
}

pub(super) fn usb_sk_not_yet_implemented_envelope(verb: &str) -> HostErrorEnvelope {
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

pub(super) fn cmd_usb_sk_status(
    _context: &LegacyContext,
    args: &UsbSkStatusArgs,
) -> Result<i32, CliFailure> {
    let json_mode = usb_sk_json_mode(args.json, args.human);
    emit_host_error(&usb_sk_not_yet_implemented_envelope("status"), json_mode)
}

pub(super) fn cmd_usb_sk_sessions(
    _context: &LegacyContext,
    args: &UsbSkSessionsArgs,
) -> Result<i32, CliFailure> {
    let json_mode = usb_sk_json_mode(args.json, args.human);
    emit_host_error(&usb_sk_not_yet_implemented_envelope("sessions"), json_mode)
}

pub(super) fn cmd_usb_sk_cancel(
    _context: &LegacyContext,
    args: &UsbSkCancelArgs,
) -> Result<i32, CliFailure> {
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

pub(super) fn cmd_usb_sk_test(
    _context: &LegacyContext,
    args: &UsbSkTestArgs,
) -> Result<i32, CliFailure> {
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

pub(super) fn cmd_keys_list(
    context: &LegacyContext,
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

pub(super) fn render_keys_list_human(entries: &[IpcKeyEntry]) -> String {
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

pub(super) fn cmd_keys_show(
    context: &LegacyContext,
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

pub(super) fn w8_mutating_verb(
    context: &LegacyContext,
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

pub(super) fn cmd_keys_rotate(
    context: &LegacyContext,
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

pub(super) fn cmd_keys_rotate_known_host(
    context: &LegacyContext,
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

pub(super) fn cmd_keys_trust(
    context: &LegacyContext,
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

pub(super) fn cmd_migrate(
    context: &LegacyContext,
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
pub(super) const DEFAULT_DRY_RUN_NOTICE: &str = "d2b: NOTICE: defaulting to --dry-run; d2b 1.0 will require explicit --dry-run or --apply (v0.4 bash CLI had no flag requirement).";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MutationFlags {
    dry_run: bool,
    apply: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MutationFlagResolution {
    flags: MutationFlags,
    notice: Option<&'static str>,
}

pub(super) fn resolve_mutation_flags(
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

pub(super) fn require_mutation_flag(
    verb: &str,
    dry_run: bool,
    apply: bool,
    json: bool,
) -> Result<MutationFlags, CliFailure> {
    require_mutation_flag_impl(verb, dry_run, apply, json, true)
}

pub(super) fn require_explicit_mutation_flag(
    verb: &str,
    dry_run: bool,
    apply: bool,
    json: bool,
) -> Result<MutationFlags, CliFailure> {
    require_mutation_flag_impl(verb, dry_run, apply, json, false)
}

pub(super) fn require_mutation_flag_impl(
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
    let exit_code = emit_host_error(&missing_mutation_flag_envelope(verb), json)?;
    Err(CliFailure::new(
        exit_code,
        format!("{verb} refused without --dry-run or --apply"),
    ))
}

pub(super) fn missing_mutation_flag_envelope(verb: &str) -> HostErrorEnvelope {
    host_error_envelope(
        &format!("{verb} requires either --dry-run or --apply"),
        "--apply-or-dry-run-required",
        78,
        &format!("{verb} invocation flags."),
        "Neither --dry-run nor --apply was provided.",
        &format!("Re-run as `d2b {verb} --dry-run` to plan or `d2b {verb} --apply` to mutate.",),
        "docs/reference/error-codes.md#--apply-or-dry-run-required",
    )
}

pub(super) fn cmd_auth_status(
    context: &LegacyContext,
    args: &AuthStatusArgs,
) -> Result<i32, CliFailure> {
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

pub(super) fn resolve_selected_vm(
    context: &LegacyContext,
    args: &StatusArgs,
) -> Result<Option<String>, CliFailure> {
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
pub(super) fn read_vm_api_ready(
    daemon_state_dir: &Path,
    vm_name: &str,
) -> Option<ApiReadyStatusV1> {
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

pub(super) fn live_pool_integrity_unknown(
    reason: &str,
    remediation: String,
) -> LivePoolIntegrityOutputV1 {
    LivePoolIntegrityOutputV1 {
        status: "unknown".to_owned(),
        unknown_reason: Some(reason.to_owned()),
        audit_ref: None,
        repair_attempted: false,
        remediation: Some(remediation),
    }
}

pub(super) fn live_pool_integrity_suspect(
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

pub(super) fn marker_status_for_integrity(store_root: &Path, vm: &str) -> Result<(), &'static str> {
    let marker = store_root.join("live").join(format!(".d2b-marker-{vm}"));
    match std::fs::symlink_metadata(&marker) {
        Ok(meta) if meta.is_file() && meta.len() == 0 => Ok(()),
        Ok(_) => Err("suspect"),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Err("marker_or_manifest_missing"),
        Err(_) => Err("marker_or_manifest_unreadable"),
    }
}

pub(super) fn read_live_pool_integrity(
    context: &LegacyContext,
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

pub(super) fn render_list_human(
    output: &ListOutputV2,
    read_model: Option<&d2b_contracts_control::public_wire::PublicReadModelMetadata>,
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

pub(super) fn render_status_vm_human(
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

pub(super) fn render_status_inventory_human(
    output: &StatusInventoryOutputV2,
    manifest: &ManifestDocument,
    context: &LegacyContext,
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

pub(super) fn render_host_check_human(output: &HostCheckOutputV2) -> String {
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

pub(super) fn render_auth_status_human(output: &AuthStatusOutputV2) -> String {
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

pub(super) fn collect_bridge_rows(
    context: &LegacyContext,
    manifest: &ManifestDocument,
    bundle: Option<&BundleContext>,
) -> Vec<BridgeHealthRow> {
    manifest
        .bridge_names()
        .into_iter()
        .map(|bridge| bridge_health_row(context, bundle, &bridge))
        .collect()
}

pub(super) fn resolve_bridge_probe_name(bundle: Option<&BundleContext>, bridge: &str) -> String {
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

pub(super) fn bridge_health_row(
    context: &LegacyContext,
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
    let output = system_tool_command("ip")
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

pub(super) fn systemctl_state(context: &LegacyContext, unit: &str) -> String {
    if let Some(state) = context
        .system_state_fixture
        .as_ref()
        .and_then(|fixture| fixture.units.get(unit))
    {
        return state.clone();
    }
    let output = system_tool_command("systemctl")
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

pub(super) fn effective_uid() -> u32 {
    Uid::effective().as_raw()
}

pub(super) fn all_known_subcommands() -> Vec<String> {
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

pub(super) fn allowed_subcommands(role: AuthRoleV2) -> BTreeSet<String> {
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

pub(super) fn denied_reason(role: AuthRoleV2, command: &str) -> &'static str {
    match (role, command) {
        (AuthRoleV2::Admin, _) => "allowed",
        (_, "audit") => "audit requires admin role in `d2b.site.adminUsers`.",
        (AuthRoleV2::Launcher, _) => "allowed",
        (AuthRoleV2::None, _) => {
            "this subcommand requires launcher membership or daemon-admin privileges."
        }
    }
}

pub(super) fn parse_uid_env(name: &str) -> BTreeSet<u32> {
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

pub(super) fn env_path(name: &str, default: &str) -> PathBuf {
    env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default))
}

pub(super) fn maybe_load_json_env<T>(name: &str) -> Result<Option<T>, CliFailure>
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

pub(super) fn read_json_file<T>(path: &Path) -> Result<T, io::Error>
where
    T: for<'de> Deserialize<'de>,
{
    let data = fs::read(path)?;
    serde_json::from_slice(&data).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

pub(super) fn read_bundle_json<T>(base_dir: &Path, raw_path: &str) -> Result<Option<T>, CliFailure>
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
/// on any IO or parse error (advisory hint path - never blocks the caller).
pub(super) fn try_canonical_target_for_vm(bundle_path: &Path, vm: &str) -> Option<String> {
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

pub(super) fn try_vm_for_canonical_target(bundle_path: &Path, raw_target: &str) -> Option<String> {
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

pub(super) fn resolve_vm_selector_from_bundle(context: &LegacyContext, selector: &str) -> String {
    try_vm_for_canonical_target(&context.bundle_path, selector)
        .unwrap_or_else(|| selector.to_owned())
}

pub(super) fn print_json<T>(value: &T) -> Result<(), CliFailure>
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
pub(super) static TEST_STDOUT_CAPTURE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(super) fn with_test_stdout_capture<T>(f: impl FnOnce() -> T) -> (T, Vec<u8>) {
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
pub(super) fn with_test_output_capture<T>(f: impl FnOnce() -> T) -> (T, Vec<u8>, Vec<u8>) {
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

pub(super) fn print_stdout(text: &str) {
    let _ = write_stdout_bytes(text.as_bytes());
}

pub(super) fn print_stderr(text: &str) {
    let _ = write_stderr_bytes(text.as_bytes());
}

pub(super) fn write_stdout_bytes(bytes: &[u8]) -> io::Result<()> {
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

pub(super) fn write_stderr_bytes(bytes: &[u8]) -> io::Result<()> {
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

pub(super) fn report_failure(err: CliFailure) -> i32 {
    let mut stderr = io::stderr().lock();
    if let Some(rendered_stderr) = err.rendered_stderr {
        let _ = stderr.write_all(rendered_stderr.as_bytes());
    } else {
        let _ = writeln!(stderr, "d2b: {}", err.message);
    }
    err.exit_code
}

pub(super) fn render_operator_error(
    error: &CoreError,
    owning_command: Option<&str>,
) -> Option<String> {
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

pub(super) fn stdout_is_tty() -> bool {
    io::stdout().is_terminal()
}

// ADR 0017: the `should_fallback_to_legacy` /
// `exec_legacy_passthrough` pair were removed wholesale. Every verb
// the Rust CLI accepts dispatches to clap → typed-envelope; verbs
// clap rejects fall through to the parse-error path. No bash exec
// site survives in the binary crate.

/// Daemon mutating-verb outcome from `try_daemon_mutating_verb`. The CLI uses
/// this to decide whether
/// to (a) print the daemon's plan and exit, (b) surface a typed
/// `not-yet-implemented` envelope (exit 78 per ADR 0015), or (c)
/// surface a `daemon-down` envelope (exit 1).
#[derive(Debug)]
pub(super) enum DaemonVerbOutcome {
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

pub(super) fn daemon_mutating_verb_frame(
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
pub(super) fn try_daemon_mutating_verb(
    context: &LegacyContext,
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

pub(super) fn redact_broker_error_for_cli(
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
                "{op_name} references a bundle intent that the broker did not find. Admin: ask `journalctl -u d2b-broker` for the intent id."
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
                "{op_name} failed at the broker live handler. Admin: inspect `journalctl -u d2b-broker` for the underlying syscall/exit code."
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
                "{op_name} failed: bundle nft script could not be parsed. Admin: inspect `journalctl -u d2b-broker` for the parse error."
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
            "broker operation is not implemented in this build; Admin: install a build with the operation enabled and retry.".to_owned(),
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

pub(super) fn broker_error_envelope(
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

pub(super) fn emit_daemon_mutating_outcome(
    outcome: DaemonVerbOutcome,
    json: bool,
) -> Result<i32, CliFailure> {
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
/// operator path - no bash fallback.
pub(super) fn dispatch_mutating_verb(
    context: &LegacyContext,
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

pub(super) fn probe_socket(path: &Path) -> Result<SocketProbe, CliFailure> {
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

pub(super) fn try_audit_via_socket(
    context: &LegacyContext,
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
    let mut cursor = None;
    let mut lines = Vec::new();
    for _ in 0..1024 {
        let request = daemon_audit_frame_with_cursor("audit", json_mode, cursor.clone())?;
        socket
            .send_frame(&request)
            .map_err(|err| CliFailure::new(1, format!("failed to send audit request: {err}")))?;
        let response = socket
            .recv_frame()
            .map_err(|err| CliFailure::new(1, format!("failed to receive audit reply: {err}")))?;
        let (page, next_cursor, complete) = parse_audit_page(&response)?;
        lines.extend(page);
        if complete {
            return Ok(AuditSocketOutcome::Lines(lines));
        }
        cursor = next_cursor;
        if cursor.is_none() {
            return Err(CliFailure::new(
                1,
                "audit export pagination omitted continuation metadata",
            ));
        }
    }
    Err(CliFailure::new(
        1,
        "audit export exceeded the bounded pagination limit",
    ))
}

pub(super) fn try_keys_list_via_socket(
    context: &LegacyContext,
) -> Result<KeysSocketOutcome, CliFailure> {
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

pub(super) fn try_keys_show_via_socket(
    context: &LegacyContext,
    vm: &str,
) -> Result<KeysSocketOutcome, CliFailure> {
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

pub(super) fn try_list_via_socket(
    context: &LegacyContext,
) -> Result<ListSocketOutcome, CliFailure> {
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

pub(super) fn try_status_via_socket(
    context: &LegacyContext,
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

pub(super) fn try_usb_probe_via_socket(
    context: &LegacyContext,
) -> Result<UsbProbeSocketOutcome, CliFailure> {
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

pub(super) fn try_store_verify_via_socket(
    context: &LegacyContext,
    vm: &str,
    repair: bool,
) -> Result<StoreVerifySocketOutcome, CliFailure> {
    let request = encode_type_tagged_message(
        "storeVerify",
        &d2b_contracts_control::public_wire::StoreVerifyRequest {
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

pub(super) fn try_public_socket_request(
    context: &LegacyContext,
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

pub(super) fn parse_keys_list_reply(bytes: &[u8]) -> Result<Vec<IpcKeyEntry>, CliFailure> {
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

pub(super) fn parse_keys_show_reply(bytes: &[u8]) -> Result<IpcKeysShowResponse, CliFailure> {
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

pub(super) fn parse_list_reply(
    bytes: &[u8],
) -> Result<
    (
        Vec<IpcListEntry>,
        Option<d2b_contracts_control::public_wire::PublicReadModelMetadata>,
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

pub(super) fn parse_status_reply(
    bytes: &[u8],
) -> Result<
    (
        Vec<IpcVmStatus>,
        Option<d2b_contracts_control::public_wire::PublicReadModelMetadata>,
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

pub(super) fn parse_usb_probe_reply(bytes: &[u8]) -> Result<Vec<IpcUsbipProbeEntry>, CliFailure> {
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

pub(super) fn parse_store_verify_reply(bytes: &[u8]) -> Result<IpcStoreVerifyResponse, CliFailure> {
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

pub(super) fn parse_gateway_display_reply(
    bytes: &[u8],
) -> Result<public_wire::GatewayDisplayOpResponse, CliFailure> {
    let value: Value = serde_json::from_slice(bytes).map_err(|err| {
        CliFailure::new(1, format!("failed to parse gatewayDisplay reply: {err}"))
    })?;
    if value.get("type").and_then(Value::as_str) != Some("gatewayDisplayResponse") {
        return Err(CliFailure::new(
            1,
            "daemon returned an unexpected reply to gatewayDisplay".to_owned(),
        ));
    }
    serde_json::from_value::<GatewayDisplayResponseFrame>(value)
        .map(|frame| frame.payload)
        .map_err(|err| CliFailure::new(1, format!("failed to decode gatewayDisplay reply: {err}")))
}

pub(crate) struct SeqpacketUnixSocket {
    fd: OwnedFd,
}

impl SeqpacketUnixSocket {
    #[cfg(test)]
    pub(crate) fn from_owned_fd(fd: OwnedFd) -> Self {
        Self { fd }
    }

    pub(crate) fn connect(path: &Path) -> io::Result<Self> {
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

    pub(crate) fn send_frame(&mut self, payload: &[u8]) -> io::Result<()> {
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

    pub(crate) fn set_io_timeout(&self, timeout: Duration) -> io::Result<()> {
        set_socket_timeout(&self.fd, SocketTimeout::Recv, Some(timeout))
            .map_err(io::Error::from)?;
        set_socket_timeout(&self.fd, SocketTimeout::Send, Some(timeout))
            .map_err(io::Error::from)?;
        Ok(())
    }

    pub(crate) fn recv_frame(&mut self) -> io::Result<Vec<u8>> {
        let mut buffer = vec![0_u8; MAX_FRAME_BYTES + 4];
        let mut iov = [IoSliceMut::new(&mut buffer)];
        // The resource and legacy daemon protocols never carry descriptors.
        // Allocate enough ancillary space to observe the bounded descriptor
        // range, then reject every recognized control message instead of
        // silently discarding it.
        let mut ancillary_bytes = [0_u8; rustix::cmsg_space!(ScmRights(32))];
        let mut ancillary = RecvAncillaryBuffer::new(&mut ancillary_bytes);
        let received = recvmsg(&self.fd, &mut iov, &mut ancillary, RecvFlags::empty())
            .map_err(io::Error::from)?;
        if received.flags.contains(RecvFlags::TRUNC) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "oversized seqpacket frame",
            ));
        }
        if received.bytes < 4 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "short frame from seqpacket socket",
            ));
        }
        let expected = u32::from_le_bytes(buffer[..4].try_into().expect("frame prefix")) as usize;
        if expected > MAX_FRAME_BYTES || expected + 4 != received.bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "malformed seqpacket frame",
            ));
        }
        if ancillary.drain().next().is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "ancillary data is not permitted on the CLI transport",
            ));
        }
        Ok(buffer[4..4 + expected].to_vec())
    }
}

#[cfg(test)]
mod cli_transport_contract_tests {
    use super::{MAX_FRAME_BYTES, SeqpacketUnixSocket};
    use nix::sys::socket::{AddressFamily, MsgFlags, SockFlag, SockType, send, socketpair};
    use rustix::net::{SendAncillaryBuffer, SendAncillaryMessage, SendFlags, sendmsg};
    use std::{
        io::IoSlice,
        os::fd::{AsFd as _, AsRawFd as _},
    };

    #[test]
    fn legacy_seqpacket_client_rejects_oversized_declared_packets() {
        let (client, server) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .expect("create seqpacket pair");
        let mut socket = SeqpacketUnixSocket { fd: client };
        let outbound = socket
            .send_frame(&vec![0_u8; MAX_FRAME_BYTES + 1])
            .expect_err("outbound oversized frame must fail closed");
        assert_eq!(outbound.kind(), std::io::ErrorKind::InvalidInput);
        let payload_len = MAX_FRAME_BYTES + 1;
        let mut frame = Vec::with_capacity(4);
        frame.extend_from_slice(&(payload_len as u32).to_le_bytes());
        send(server.as_raw_fd(), &frame, MsgFlags::empty()).expect("send oversized declaration");
        let error = socket
            .recv_frame()
            .expect_err("oversized declaration must fail closed");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("malformed"));
    }

    #[test]
    fn legacy_seqpacket_client_rejects_ancillary_file_descriptors() {
        let (client, server) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .expect("create seqpacket pair");
        let file = std::fs::File::open("/dev/null").expect("open descriptor fixture");
        let rights = [file.as_fd()];
        let mut control_bytes = [0_u8; rustix::cmsg_space!(ScmRights(1))];
        let mut control = SendAncillaryBuffer::new(&mut control_bytes);
        assert!(control.push(SendAncillaryMessage::ScmRights(&rights)));
        let frame = 0_u32.to_le_bytes();
        let iov = [IoSlice::new(&frame)];
        sendmsg(&server, &iov, &mut control, SendFlags::empty()).expect("send ancillary frame");
        let mut socket = SeqpacketUnixSocket { fd: client };
        let error = socket
            .recv_frame()
            .expect_err("ancillary data must fail closed");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("ancillary"));
    }
}

pub(super) fn read_symlink_target(path: &Path) -> Option<String> {
    fs::read_link(path)
        .ok()
        .map(|target| target.display().to_string())
}

pub(super) fn nix_err_to_io(err: nix::errno::Errno) -> io::Error {
    io::Error::from_raw_os_error(err as i32)
}

#[cfg(test)]
mod host_install_dispatch_tests {
    use clap::Parser;
    use std::{
        ffi::OsString,
        io,
        os::fd::{AsRawFd as _, RawFd},
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
        AddressFamily, HostInstallArgs, IpcHelloOk, LegacyContext, MAX_FRAME_BYTES, MsgFlags,
        NativeCli, SockFlag, SockType, UnixAddr, VmStartArgs, cmd_vm_start,
        daemon_supported_features, encode_type_tagged_message, nix_err_to_io, public_wire, send,
        socket,
    };
    use d2b_contracts::Version;

    static ENV_MUTEX: Mutex<()> = Mutex::new(());
    static TEST_SOCKET_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn missing_daemon_context() -> LegacyContext {
        let missing_manifest = test_socket_path("missing-daemon", ".missing-manifest.json");
        LegacyContext {
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
        let context = LegacyContext {
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

        let context = LegacyContext {
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
        let context = LegacyContext {
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
        let context = LegacyContext {
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

    #[test]
    fn cmd_status_accepts_canonical_workload_target_selector() {
        let manifest_path = test_socket_path("status-workload-canonical", ".manifest.json");
        if let Some(parent) = manifest_path.parent() {
            std::fs::create_dir_all(parent).expect("manifest parent");
        }
        write_test_manifest(&manifest_path, "work-aad");
        let bundle_path = manifest_path.with_extension("bundle.json");
        write_bundle_with_realm_controllers(&bundle_path, "work-aad");
        rewrite_bundle_workload_identity(&bundle_path, "aad", "aad.work.d2b");
        let context = LegacyContext {
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
        let mut table = d2b_zone_routing::RealmEntrypointTable::with_local_default();
        table.gateway_backed(
            d2b_realm_core::RealmPath::new(vec![d2b_realm_core::RealmId::parse("work").unwrap()])
                .unwrap(),
            d2b_realm_core::TargetName::parse("corp-gateway.local.d2b").unwrap(),
        );
        let context = LegacyContext {
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
        // `corp-vm.work.d2b` already has the `.d2b` suffix - env-style detection
        // must not reject it. This test verifies there is no false positive.
        let manifest_path = test_socket_path("env-style-no-false-positive", ".manifest.json");
        if let Some(parent) = manifest_path.parent() {
            std::fs::create_dir_all(parent).expect("manifest parent");
        }
        write_test_manifest(&manifest_path, "vm-a");
        let mut table = d2b_zone_routing::RealmEntrypointTable::with_local_default();
        // Make `work` a local realm so the route resolves without a daemon.
        table.host_resident(
            d2b_realm_core::RealmPath::new(vec![d2b_realm_core::RealmId::parse("work").unwrap()])
                .unwrap(),
        );
        let context = test_context(manifest_path.clone());

        let (result, _stdout) = super::with_test_stdout_capture(|| {
            super::route_vm_target_with_table(&context, "corp-vm.work.d2b", false, Some(table))
        });
        // Must not produce an env-style error - the result may be Ok (Local) or a
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
        let context = LegacyContext {
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

    #[test]
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

    #[test]
    fn gateway_display_frame_serializes_lifecycle_open_list_and_close_requests() {
        let start = super::gateway_display_frame(&public_wire::GatewayDisplayOp::Start(
            public_wire::GatewayDisplayStartArgs {
                target: "demo.work.d2b".to_owned(),
                operation_id: "gw-start-1".to_owned(),
                principal: "uid-1000".to_owned(),
                request_hash: 7,
            },
        ))
        .unwrap();
        let start_v: Value = serde_json::from_slice(&start).unwrap();
        assert_eq!(
            start_v.get("type").and_then(Value::as_str),
            Some("gatewayDisplay")
        );
        assert_eq!(start_v.get("op").and_then(Value::as_str), Some("start"));

        let stop = super::gateway_display_frame(&public_wire::GatewayDisplayOp::Stop(
            public_wire::GatewayDisplayStopArgs {
                target: "demo.work.d2b".to_owned(),
                operation_id: "gw-stop-1".to_owned(),
                principal: "uid-1000".to_owned(),
                request_hash: 9,
            },
        ))
        .unwrap();
        let stop_v: Value = serde_json::from_slice(&stop).unwrap();
        assert_eq!(
            stop_v.get("type").and_then(Value::as_str),
            Some("gatewayDisplay")
        );
        assert_eq!(stop_v.get("op").and_then(Value::as_str), Some("stop"));

        let open = super::gateway_display_frame(&public_wire::GatewayDisplayOp::Open(
            public_wire::GatewayDisplayOpenArgs {
                target: "demo.work.d2b".to_owned(),
                operation_id: "gw-exec-1".to_owned(),
                principal: "uid-1000".to_owned(),
                app_argv: vec!["foot".to_owned()],
                request_hash: 8,
            },
        ))
        .unwrap();
        let open_v: Value = serde_json::from_slice(&open).unwrap();
        assert_eq!(
            open_v.get("type").and_then(Value::as_str),
            Some("gatewayDisplay")
        );
        assert_eq!(open_v.get("op").and_then(Value::as_str), Some("open"));
        assert_eq!(
            open_v
                .get("args")
                .and_then(|a| a.get("appArgv"))
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(Value::as_str),
            Some("foot")
        );

        let list = super::gateway_display_frame(&public_wire::GatewayDisplayOp::List(
            public_wire::GatewayDisplayListArgs {
                target: Some("demo.work.d2b".to_owned()),
            },
        ))
        .unwrap();
        let list_v: Value = serde_json::from_slice(&list).unwrap();
        assert_eq!(
            list_v.get("type").and_then(Value::as_str),
            Some("gatewayDisplay")
        );
        assert_eq!(list_v.get("op").and_then(Value::as_str), Some("list"));

        let list_detailed = super::gateway_display_frame(
            &public_wire::GatewayDisplayOp::ListDetailed(public_wire::GatewayDisplayListArgs {
                target: Some("demo.work.d2b".to_owned()),
            }),
        )
        .unwrap();
        let list_detailed_v: Value = serde_json::from_slice(&list_detailed).unwrap();
        assert_eq!(
            list_detailed_v.get("type").and_then(Value::as_str),
            Some("gatewayDisplay")
        );
        assert_eq!(
            list_detailed_v.get("op").and_then(Value::as_str),
            Some("list-detailed")
        );

        let close = super::gateway_display_frame(&public_wire::GatewayDisplayOp::Close(
            public_wire::GatewayDisplayCloseArgs {
                session_id: "s0".to_owned(),
            },
        ))
        .unwrap();
        let close_v: Value = serde_json::from_slice(&close).unwrap();
        assert_eq!(
            close_v.get("type").and_then(Value::as_str),
            Some("gatewayDisplay")
        );
        assert_eq!(close_v.get("op").and_then(Value::as_str), Some("close"));
    }

    #[test]
    fn gateway_display_reply_parser_accepts_bounded_list_response() {
        let response = serde_json::json!({
            "type": "gatewayDisplayResponse",
            "op": "list-detailed",
            "result": {
                "sessions": [{
                    "sessionId": "s0",
                    "target": "demo.work.d2b",
                    "state": "running",
                    "operationId": "op-1",
                    "principal": "uid-1000"
                }]
            }
        });
        let parsed = super::parse_gateway_display_reply(&serde_json::to_vec(&response).unwrap())
            .expect("gateway display list response parses");
        let public_wire::GatewayDisplayOpResponse::ListDetailed(result) = parsed else {
            panic!("expected detailed list response");
        };
        assert_eq!(result.sessions.len(), 1);
        assert_eq!(result.sessions[0].operation_id, "op-1");
        assert_eq!(result.sessions[0].principal, "uid-1000");
        let rendered = format!("{result:?}");
        for forbidden in ["foot", "SharedAccessKey", "/run/", "waypipe"] {
            assert!(
                !rendered.contains(forbidden),
                "gateway display reply leaked {forbidden}: {rendered}"
            );
        }
    }

    #[test]
    fn gateway_display_reply_parser_accepts_close_response() {
        let response = serde_json::json!({
            "type": "gatewayDisplayResponse",
            "op": "close",
            "result": {
                "closed": true
            }
        });
        let parsed = super::parse_gateway_display_reply(&serde_json::to_vec(&response).unwrap())
            .expect("gateway display close response parses");
        let public_wire::GatewayDisplayOpResponse::Close(result) = parsed else {
            panic!("expected close response");
        };
        assert!(result.closed);
    }

    /// Per-thread guard that overrides the config-staging base for a test and
    /// clears it on drop - replaces the old `D2B_CONFIG_STAGING_DIR` env
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
        let received = nix::sys::socket::recv(fd, &mut buffer, flags).map_err(nix_err_to_io)?;
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

    fn test_context(manifest_path: PathBuf) -> LegacyContext {
        LegacyContext {
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
        let context = test_context(manifest_path.clone());
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

        let rows =
            super::realm_policy_rows_from_entries(&context, entries).expect("realm rows render");
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
        let context = test_context(manifest_path.clone());
        let mut entries = std::collections::BTreeMap::new();
        entries.insert(
            "work".to_owned(),
            super::RealmEntrypointConfig {
                mode: "gateway-backed".to_owned(),
                gateway: Some("sys-work-gateway.local.d2b".to_owned()),
            },
        );
        let rows = super::realm_policy_rows_from_entries(
            &context,
            super::normalize_realm_entrypoint_entries(entries).unwrap(),
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
        let context = test_context(manifest_path.clone());

        let mut unknown_mode = std::collections::BTreeMap::new();
        unknown_mode.insert(
            "work".to_owned(),
            super::RealmEntrypointConfig {
                mode: "surprise".to_owned(),
                gateway: None,
            },
        );
        let err = super::realm_policy_rows_from_entries(&context, unknown_mode)
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
        let err = super::realm_policy_rows_from_entries(&context, missing_gateway)
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
        let context = test_context(manifest_path.clone());
        let args = super::OpInspectArgs {
            trace_id: Some("trace-1".to_owned()),
            span_id: Some("span-1".to_owned()),
            json: true,
            human: false,
        };
        let output = super::op_inspect_output(&context, &args).expect("op inspect renders");
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

    fn write_bundle_with_realm_controllers(bundle_path: &std::path::Path, vm: &str) {
        let dir = bundle_path.parent().expect("bundle parent dir");
        std::fs::create_dir_all(dir).expect("create bundle dir");
        let unique = bundle_path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("bundle filename");
        let realm_controllers_name = format!("{unique}.realm-controllers.json");
        let runtime = json!({
            "kind": "nixos",
            "provider": {
                "id": "local-provider",
                "driver": "local-ch",
                "type": "local"
            },
            "capabilities": {
                "lifecycle": true,
                "display": false,
                "usbHotplug": false,
                "exec": true,
                "configSync": true,
                "ssh": false,
                "storeSync": true,
                "keys": true,
                "inGuestObservability": false
            },
            "operationCapabilities": {
                "lifecycle": {
                    "start": true,
                    "stop": true,
                    "restart": true,
                    "switch": false,
                    "hostPrepare": false
                },
                "media": {
                    "usbHotplug": false,
                    "removableMedia": false,
                    "qemuMedia": false
                },
                "display": {
                    "display": false,
                    "graphics": false,
                    "video": false,
                    "waylandProxy": false
                },
                "guest": {
                    "exec": true,
                    "shell": false,
                    "configSync": true,
                    "ssh": false,
                    "keys": true,
                    "inGuestObservability": false
                },
                "storage": {
                    "storeSync": true,
                    "virtiofs": true,
                    "volumes": false
                }
            },
            "autostartPolicy": "manual"
        });
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
                    "providers": [{
                        "kind": "nixos",
                        "provider": {
                            "id": "local-provider",
                            "driver": "local-ch",
                            "type": "local"
                        },
                        "capabilities": runtime["capabilities"],
                        "operationCapabilities": runtime["operationCapabilities"],
                        "autostartPolicy": "manual"
                    }],
                    "workloads": [{
                        "workloadId": vm,
                        "vmName": vm,
                        "env": "work",
                        "runtime": runtime,
                        "paths": {
                            "stateDir": format!("/var/lib/d2b/vms/{vm}/state"),
                            "runDir": format!("/run/d2b/vms/{vm}"),
                            "storeView": format!("/var/lib/d2b/vms/{vm}/store"),
                            "componentSessionDir": format!("/run/d2b/vms/{vm}/component-session")
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
            dir.join(&realm_controllers_name),
            serde_json::to_vec(&realm_controllers).expect("serialize realm controllers"),
        )
        .expect("write realm controllers");
        let bundle = json!({
            "bundleVersion": 11,
            "schemaVersion": "v2",
            "publicManifestPath": format!("{unique}.vms.json"),
            "hostPath": format!("{unique}.host.json"),
            "processesPath": format!("{unique}.processes.json"),
            "privilegesPath": format!("{unique}.privileges.json"),
            "realmControllersPath": realm_controllers_name,
            "closures": [],
            "minijailProfiles": [],
            "generation": {
                "generator": "test",
                "sourceRevision": null,
                "generatedAt": null
            }
        });
        std::fs::write(
            bundle_path,
            serde_json::to_vec(&bundle).expect("serialize bundle"),
        )
        .expect("write bundle");
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
            .expect("bundle has realm controllers path");
        let rc_path = bundle_path.parent().expect("bundle parent").join(rc_ref);
        let mut realm_controllers: Value =
            serde_json::from_slice(&std::fs::read(&rc_path).expect("read realm controllers"))
                .expect("parse realm controllers");
        let workload = realm_controllers
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
            serde_json::to_vec(&realm_controllers).expect("serialize rewritten realm controllers"),
        )
        .expect("write rewritten realm controllers");
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

        let context = LegacyContext {
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
        F: FnOnce(&LegacyContext) -> Result<i32, super::CliFailure>,
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
        F: FnOnce(&LegacyContext) -> Result<i32, super::CliFailure>,
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

        let context = LegacyContext {
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
}

#[cfg(test)]
mod console_fsm_tests {
    //! Unit tests for the console FSM detach-char scanning logic and the
    //! QEMU blank-console warning message content.

    use super::{
        AddressFamily, DetachScan, IpcHelloOk, LegacyContext, MAX_FRAME_BYTES, MsgFlags, SockFlag,
        SockType, UnixAddr, daemon_supported_features, encode_type_tagged_message, nix_err_to_io,
        scan_chunk_for_detach, send, socket,
    };
    use d2b_contracts::Version;
    use d2b_contracts_control::public_wire;
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
        let received =
            nix::sys::socket::recv(fd, &mut buffer, MsgFlags::empty()).map_err(nix_err_to_io)?;
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
        // "abc\x1ddef" - detach at index 3, prefix "abc"
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
        // "hello\x1d" - detach at index 5, prefix "hello"
        let mut chunk = b"hello".to_vec();
        chunk.push(DETACH);
        assert_eq!(
            scan_chunk_for_detach(&chunk),
            DetachScan::Detach { prefix_len: 5 }
        );
    }

    #[test]
    fn first_detach_char_wins_over_later_occurrences() {
        // "\x1dabc\x1d" - first detach at index 0
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
                            stream: d2b_contracts_control::terminal_wire::TerminalStream::Stdout,
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
                            stream: d2b_contracts_control::terminal_wire::TerminalStream::Stdout,
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

        let context = LegacyContext {
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
        let source = include_str!("legacy.rs");
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
                            stream: d2b_contracts_control::terminal_wire::TerminalStream::Stdout,
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

        let context = LegacyContext {
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
