//! Broker wire contract.
//!
//! Every mutating variant carries **only opaque identifiers** +
//! bundle-resolved intent refs. The daemon never names a raw path, a
//! raw nft rule text, a raw route spec, a raw sysctl key/value, a raw
//! ifname set, a raw `/etc/hosts` entry list, a raw uid/gid, raw argv
//! or env, raw caps, or a raw seccomp profile path. The broker uses
//! the opaque IDs to look up the typed intent in its own trusted bundle
//! copy. See `d2b_contracts::types` for the newtype set.

use crate::guest_auth::AUTH_NONCE_LEN;
use crate::types::{
    BundleClosureRef, BundleOpId, MediaRef, PathClass, RoleId, ScopeId, SubjectId, TracingSpanId,
    VmId,
};
use d2b_core::host::IfName;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "payload")]
pub enum BrokerRequest {
    ApplyNftables(ApplyNftablesRequest),
    ApplyNmUnmanaged(ApplyNmUnmanagedRequest),
    ApplyRoute(ApplyRouteRequest),
    ApplySysctl(ApplySysctlRequest),
    BindUnixSocket(BindUnixSocketRequest),
    CreateOrReconcileUsersGroups(CreateOrReconcileUsersGroupsRequest),
    CreatePersistentTap(CreatePersistentTapRequest),
    CreateTapFd(CreateTapFdRequest),
    DelegateCgroupV2(DelegateCgroupV2Request),
    ExportBrokerAudit(ExportBrokerAuditRequest),
    /// Daemon ↔ broker handshake request. The daemon sends its
    /// client_version and supported_features; the broker replies with
    /// [`HelloResponse`] containing the selected wire version. Mirrors
    /// the bootstrap `Hello` shape so the connection layer doesn't need
    /// a side-channel.
    Hello(HelloRequest),
    GuestControlSign(GuestControlSignRequest),
    InjectSecretById(SecretByIdRequest),
    LaunchMinijailChild(LaunchMinijailChildRequest),
    ModprobeIfAllowed(ModprobeIfAllowedRequest),
    OpenCgroupDir(OpenCgroupDirRequest),
    OpenDevice(OpenDeviceRequest),
    OpenFuse(OpenFuseRequest),
    OpenKvm(OpenKvmRequest),
    /// Enroll a physical USB block device for a qemu-media opaque ref.
    /// The daemon supplies only VM/ref plus the transient sysfs busid; the
    /// broker resolves declared policy from the trusted bundle, reads physical
    /// identity as root, and writes root-only registry/rules outside the store.
    QemuMediaEnroll(QemuMediaEnrollRequest),
    /// Refresh redacted qemu-media runtime state from the root-only persistent
    /// registry. The daemon calls this before public status/probe rendering so
    /// `/run` loss after reboot does not make enrollments disappear.
    QemuMediaRefreshRegistry(QemuMediaRefreshRegistryRequest),
    /// Resolve and attach the declared boot source, then continue the paused
    /// qemu-media runner. The broker resolves physical USB registry state or
    /// direct image-file policy from the trusted bundle; the daemon supplies
    /// only the VM id.
    QemuMediaBoot(QemuMediaBootRequest),
    /// Ask the qemu-media guest to shut down through QMP `system_powerdown`.
    /// The daemon supplies only the VM id; raw QMP JSON never crosses the
    /// broker boundary.
    QemuMediaSystemPowerdown(QemuMediaLifecycleRequest),
    /// Read the qemu-media guest/VMM status through QMP `query-status`.
    /// Returns a closed typed enum so polling never leaks raw QMP JSON back to
    /// the daemon.
    QemuMediaQueryStatus(QemuMediaQueryStatusRequest),
    /// Ask the qemu-media VMM to exit through QMP `quit` after the guest is no
    /// longer running.
    QemuMediaQuit(QemuMediaLifecycleRequest),
    /// Resolve an enrolled physical USB selector and execute qemu-media QMP
    /// attach. The busid is a runtime selector only and is redacted from every
    /// success response/audit field.
    QemuMediaAttach(QemuMediaHotplugRequest),
    /// Resolve an enrolled physical USB selector and execute qemu-media QMP
    /// detach. The busid is a runtime selector only and is redacted from every
    /// success response/audit field.
    QemuMediaDetach(QemuMediaHotplugRequest),
    /// Daemon-side reconcile-and-adopt support. The daemon asks the
    /// broker to `pidfd_open(pid)` AND re-verify `/proc/<pid>/stat`
    /// field 22 matches the expected start-time in one atomic call (no
    /// daemon-side syscall surface needed). The pidfd is returned via
    /// SCM_RIGHTS; if start-time drifted the broker closes the fd and
    /// surfaces a typed pidfd-race error.
    OpenPidfd(OpenPidfdRequest),
    OpenVhostNet(OpenVhostNetRequest),
    PauseBroker,
    /// Drain the broker's in-memory ring buffer of ChildReaped events.
    /// Returns [`PollChildReapedResponse`] containing all buffered
    /// notifications in FIFO order; clears the buffer. Idempotent.
    PollChildReaped,
    PrepareRuntimeDir(PrepareDirRequest),
    PrepareStateDir(PrepareDirRequest),
    ReconcileStorageScope(ReconcileStorageScopeRequest),
    ValidateLockSpec(ValidateLockSpecRequest),
    PrepareStoreView(PrepareStoreViewRequest),
    /// Typed broker op that hardlink-farms a VM's resolved closure into
    /// `/var/lib/d2b/vms/<vm>/store/` and atomically swaps the
    /// `current` symlink. Replaces the retired per-VM
    /// `d2b-<vm>-store-sync.service` bash oneshot. The daemon names
    /// only the opaque `bundle_closure_ref` + `vm_id` + expected
    /// `generation_token`; the broker re-derives every closure path from
    /// its trusted bundle copy and derives the collision-free on-disk
    /// `generation_id` itself.
    StoreSync(StoreSyncRequest),
    /// Operator-facing live-pool verification. The daemon names only the
    /// VM id; the broker resolves the trusted store-view intent and reads
    /// host-only `store-view/state` itself. The CLI never reads the
    /// store-view directly.
    StoreVerify(StoreVerifyRequest),
    ReadSecretById(SecretByIdRequest),
    ResumeBroker,
    RotateSecretById(SecretByIdRequest),
    /// Live host installer + migrate writer. Drives the per-host
    /// systemd unit install + `--enable` / `--start` flow (or migrate
    /// writer for existing NixOS hosts). The broker resolves the
    /// installer plan from the trusted bundle's `installer:host` intent
    /// row; the daemon never names raw systemd unit paths or `--enable`
    /// flags on the wire.
    RunHostInstall(RunHostInstallRequest),
    /// Transition an existing systemd-owned VM to daemon-owned without
    /// touching running VMs. Resolves the migrate plan from the bundle's
    /// `migrate:host` intent row.
    RunMigrate(RunMigrateRequest),
    /// Broker-side mutating verb flips for per-VM activation, host GC,
    /// framework-managed SSH key rotation, and known_hosts trust
    /// maintenance.
    RunActivation(RunActivationRequest),
    RunGc(RunGcRequest),
    RunKeysRotate(RunKeysRotateRequest),
    RunHostKeyTrust(RunHostKeyTrustRequest),
    RunRotateKnownHost(RunRotateKnownHostRequest),
    SetBridgePortFlags(SetBridgePortFlagsRequest),
    SetSocketAcl(SetSocketAclRequest),
    SetupMountNamespace(SetupMountNamespaceRequest),
    SignalRunner(SignalRunnerRequest),
    DeregisterRunnerPidfd(DeregisterRunnerPidfdRequest),
    SpawnRunner(SpawnRunnerRequest),
    UpdateHostsFile(UpdateHostsFileRequest),
    UsbipBind(UsbipBindRequest),
    UsbipBindFirewallRule(UsbipBindFirewallRuleRequest),
    UsbipProxyReconcile(UsbipProxyReconcileRequest),
    UsbipUnbind(UsbipUnbindRequest),
    /// Explicit-attach: bind a present sysfs busid for a USB-capable VM
    /// without requiring static bundle firewall/bind intent refs.
    ///
    /// The daemon has already validated: (1) the busid is present in sysfs,
    /// (2) the target VM has `runtime.capabilities.usbHotplug = true`, (3) no
    /// other active claim holds this busid. The broker validates the busid shape,
    /// acquires the per-busid OFD lock, and runs the `usbip bind` helper.
    ///
    /// Currently a typed stub (`Unimplemented`) — the live handler wires the
    /// per-device backend path without restarting shared per-env backends.
    UsbipExplicitBind(UsbipExplicitBindRequest),
    /// Explicit-attach: install a per-busid nftables carve-out scoped
    /// to the target VM's env bridge (not the full per-env USBIP table entry).
    ///
    /// Carries the daemon-validated env bridge identity so the broker can build
    /// the scoped `inet d2b` rule without a bundle firewall intent ref.
    ///
    /// Currently a typed stub (`Unimplemented`).
    UsbipExplicitFirewallRule(UsbipExplicitFirewallRuleRequest),
    ValidateBundle,
    /// Write the per-VM dnsmasq lease file. Replaces leaves of the
    /// retired `microvm-setup@<vm>.service`. Currently a typed stub
    /// (`Unimplemented`) until the live handler is wired.
    ///
    /// TODO: wire to a `live_seed_dnsmasq_lease` handler resolved via
    /// `BundleResolver` (per-VM dnsmasq lease row).
    SeedDnsmasqLease(SeedDnsmasqLeaseRequest),
    /// Bind-mount `/var/lib/d2b/vms/<vm>/store-view` from the
    /// per-VM hardlink farm at `<vm>/store/`. Currently a typed stub
    /// (`Unimplemented`) until the live handler is wired.
    ///
    /// TODO: wire to a `live_bind_mount_from_hardlink_farm` handler
    /// resolved via `BundleResolver::find_store_view_intent`.
    BindMountFromHardlinkFarm(BindMountFromHardlinkFarmRequest),
    /// Enforce the per-leaf ownership/mode matrix on
    /// `/var/lib/d2b/vms/<vm>/`. Currently a typed stub
    /// (`Unimplemented`) until the real check is wired.
    ///
    /// TODO: wire to `d2b_host::ownership_matrix::check`.
    OwnershipMatrixCheck(OwnershipMatrixCheckRequest),
    /// Refuse VM start if `/var/lib/d2b/vms/<vm>/sshd-host-keys/`
    /// drifts from `root:root 0400`. Currently a typed stub
    /// (`Unimplemented`) until the real check is wired.
    ///
    /// TODO: wire to the `O_NOFOLLOW` symlink-rejecting check.
    SshHostKeyPreflight(SshHostKeyPreflightRequest),
    /// Broker-provisioned disk-image creation.
    ///
    /// The daemon dispatches this before `SpawnRunner` for any runner
    /// whose bundle ProcessNode has `DiskInit` plan-ops (currently CH
    /// when `writableStoreOverlay` is enabled). The broker resolves
    /// the target path, size, mode, and ownership from the trusted
    /// bundle — the daemon names only the opaque `vm_id`.
    DiskInit(DiskInitRequest),
}

impl BrokerRequest {
    /// Stable operation name for audit records.
    ///
    /// Mirrors the bootstrap `BootstrapCall::op_name` shape so the
    /// broker audit pipeline (`AuditLog::write_entry`) can be
    /// variant-agnostic between the two wire shapes during the
    /// transition.
    pub fn op_name(&self) -> &'static str {
        match self {
            Self::ApplyNftables(_) => "ApplyNftables",
            Self::ApplyNmUnmanaged(_) => "ApplyNmUnmanaged",
            Self::ApplyRoute(_) => "ApplyRoute",
            Self::ApplySysctl(_) => "ApplySysctl",
            Self::BindUnixSocket(_) => "BindUnixSocket",
            Self::CreateOrReconcileUsersGroups(_) => "CreateOrReconcileUsersGroups",
            Self::CreatePersistentTap(_) => "CreatePersistentTap",
            Self::CreateTapFd(_) => "CreateTapFd",
            Self::DelegateCgroupV2(_) => "DelegateCgroupV2",
            Self::ExportBrokerAudit(_) => "ExportBrokerAudit",
            Self::Hello(_) => "Hello",
            Self::GuestControlSign(_) => "GuestControlSign",
            Self::InjectSecretById(_) => "InjectSecretById",
            Self::LaunchMinijailChild(_) => "LaunchMinijailChild",
            Self::ModprobeIfAllowed(_) => "ModprobeIfAllowed",
            Self::OpenCgroupDir(_) => "OpenCgroupDir",
            Self::OpenDevice(_) => "OpenDevice",
            Self::OpenFuse(_) => "OpenFuse",
            Self::OpenKvm(_) => "OpenKvm",
            Self::QemuMediaEnroll(_) => "QemuMediaEnroll",
            Self::QemuMediaRefreshRegistry(_) => "QemuMediaRefreshRegistry",
            Self::QemuMediaBoot(_) => "QemuMediaBoot",
            Self::QemuMediaSystemPowerdown(_) => "QemuMediaSystemPowerdown",
            Self::QemuMediaQueryStatus(_) => "QemuMediaQueryStatus",
            Self::QemuMediaQuit(_) => "QemuMediaQuit",
            Self::QemuMediaAttach(_) => "QemuMediaAttach",
            Self::QemuMediaDetach(_) => "QemuMediaDetach",
            Self::OpenPidfd(_) => "OpenPidfd",
            Self::OpenVhostNet(_) => "OpenVhostNet",
            Self::PauseBroker => "PauseBroker",
            Self::PollChildReaped => "PollChildReaped",
            Self::PrepareRuntimeDir(_) => "PrepareRuntimeDir",
            Self::PrepareStateDir(_) => "PrepareStateDir",
            Self::ReconcileStorageScope(_) => "ReconcileStorageScope",
            Self::ValidateLockSpec(_) => "ValidateLockSpec",
            Self::PrepareStoreView(_) => "PrepareStoreView",
            Self::StoreSync(_) => "StoreSync",
            Self::StoreVerify(_) => "StoreVerify",
            Self::ReadSecretById(_) => "ReadSecretById",
            Self::ResumeBroker => "ResumeBroker",
            Self::RotateSecretById(_) => "RotateSecretById",
            Self::RunHostInstall(_) => "RunHostInstall",
            Self::RunMigrate(_) => "RunMigrate",
            Self::RunActivation(_) => "RunActivation",
            Self::RunGc(_) => "RunGc",
            Self::RunKeysRotate(_) => "RunKeysRotate",
            Self::RunHostKeyTrust(_) => "RunHostKeyTrust",
            Self::RunRotateKnownHost(_) => "RunRotateKnownHost",
            Self::SetBridgePortFlags(_) => "SetBridgePortFlags",
            Self::SetSocketAcl(_) => "SetSocketAcl",
            Self::SetupMountNamespace(_) => "SetupMountNamespace",
            Self::SignalRunner(_) => "SignalRunner",
            Self::DeregisterRunnerPidfd(_) => "DeregisterRunnerPidfd",
            Self::SpawnRunner(_) => "SpawnRunner",
            Self::UpdateHostsFile(_) => "UpdateHostsFile",
            Self::UsbipBind(_) => "UsbipBind",
            Self::UsbipBindFirewallRule(_) => "UsbipBindFirewallRule",
            Self::UsbipProxyReconcile(_) => "UsbipProxyReconcile",
            Self::UsbipUnbind(_) => "UsbipUnbind",
            Self::UsbipExplicitBind(_) => "UsbipExplicitBind",
            Self::UsbipExplicitFirewallRule(_) => "UsbipExplicitFirewallRule",
            Self::ValidateBundle => "ValidateBundle",
            Self::SeedDnsmasqLease(_) => "SeedDnsmasqLease",
            Self::BindMountFromHardlinkFarm(_) => "BindMountFromHardlinkFarm",
            Self::OwnershipMatrixCheck(_) => "OwnershipMatrixCheck",
            Self::SshHostKeyPreflight(_) => "SshHostKeyPreflight",
            Self::DiskInit(_) => "DiskInit",
        }
    }

    /// Stable category label for the audit's "opaque_target_id"
    /// column. Mirrors `BootstrapCall::opaque_target_id` semantics:
    /// classify the kind of target without leaking caller-supplied
    /// path names. Default is "operation"; the read-only ops have
    /// their own stable labels.
    pub fn opaque_target_id(&self) -> &'static str {
        match self {
            Self::Hello(_) => "daemon-handshake",
            Self::GuestControlSign(_) => "guest-control-auth",
            Self::ValidateBundle => "bundle",
            Self::ExportBrokerAudit(_) => "audit-log",
            Self::PollChildReaped => "pidfd-reap-buffer",
            _ => "operation",
        }
    }
}

/// Broker-side installer driver. The broker resolves the bundle's
/// `installer:host` intent row (synthesised by
/// `d2b_core::bundle_resolver` from the `host.json` + Nix-emitted
/// installer plan), then runs the systemd unit install + `--enable` /
/// `--start` shellouts per the resolved plan. The daemon never names
/// the systemd unit path or `--enable` flag on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunHostInstallRequest {
    pub bundle_installer_intent_ref: BundleOpId,
    #[serde(default)]
    pub enable: bool,
    #[serde(default)]
    pub start: bool,
    #[serde(default)]
    pub no_start: bool,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunHostInstallResponse {
    pub installed: bool,
    pub enabled: bool,
    pub started: bool,
    pub artifacts_written: Vec<String>,
}

/// Broker-side migration driver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunMigrateRequest {
    pub bundle_migrate_intent_ref: BundleOpId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunMigrateResponse {
    pub migrated_vm_count: u32,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ActivationMode {
    Switch,
    Boot,
    Test,
    Rollback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ActivationPhase {
    Prepare,
    Commit,
    #[default]
    MetadataOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunActivationRequest {
    pub bundle_activation_intent_ref: BundleOpId,
    pub mode: ActivationMode,
    #[serde(default)]
    pub phase: ActivationPhase,
    pub vm: String,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunActivationResponse {
    pub mode: ActivationMode,
    pub vm: String,
    #[serde(default)]
    pub generation_number: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest_switch_script_path: Option<String>,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunGcRequest {
    pub bundle_gc_intent_ref: BundleOpId,
    #[serde(default)]
    pub keep_generations: Option<u32>,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunGcResponse {
    #[serde(default)]
    pub keep_generations: Option<u32>,
    pub retained_store_path_count: u32,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunKeysRotateRequest {
    pub bundle_keys_intent_ref: BundleOpId,
    pub vm: String,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunKeysRotateResponse {
    pub vm: String,
    pub key_path: String,
    pub public_key_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunHostKeyTrustRequest {
    pub bundle_trust_intent_ref: BundleOpId,
    pub vm: String,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunHostKeyTrustResponse {
    pub vm: String,
    pub static_ip: String,
    pub known_hosts_path: String,
    pub updated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunRotateKnownHostRequest {
    pub bundle_rotate_known_host_intent_ref: BundleOpId,
    pub vm: String,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunRotateKnownHostResponse {
    pub vm: String,
    pub static_ip: String,
    pub known_hosts_path: String,
    pub removed: bool,
}

/// Daemon ↔ broker handshake request. Carries the daemon's
/// client_version and the wire feature flags it understands so the
/// broker can pick a compatible response version + capability set.
/// Mirrors the bootstrap `Hello { client_version, supported_features }`
/// shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelloRequest {
    pub client_version: String,
    #[serde(default)]
    pub supported_features: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "payload")]
pub enum BrokerResponse {
    Ack(AckResponse),
    CreatePersistentTap(TapReadyResponse),
    CreateTapFd(TapReadyResponse),
    /// Typed broker error envelope returned in place of an op-specific
    /// response when the broker refuses or fails to handle a request.
    /// Mirrors the bootstrap `BrokerResponse::Error` struct-variant
    /// shape so the audit pipeline and daemon-side error propagation
    /// stay shape-compatible across the dispatcher transition.
    Error(BrokerErrorResponse),
    ExportBrokerAudit(ExportBrokerAuditResponse),
    /// Live host install + migrate writer responses.
    RunHostInstall(RunHostInstallResponse),
    RunMigrate(RunMigrateResponse),
    RunActivation(RunActivationResponse),
    RunGc(RunGcResponse),
    RunKeysRotate(RunKeysRotateResponse),
    RunHostKeyTrust(RunHostKeyTrustResponse),
    RunRotateKnownHost(RunRotateKnownHostResponse),
    /// Daemon ↔ broker handshake confirmation response. Returned in
    /// reply to a `BrokerRequest::Hello` so the daemon can
    /// capability-negotiate and the broker can audit the connection
    /// without a separate side-channel.
    Hello(HelloResponse),
    GuestControlSign(GuestControlSignResponse),
    QemuMediaEnroll(QemuMediaEnrollResponse),
    QemuMediaRefreshRegistry(QemuMediaRefreshRegistryResponse),
    QemuMediaBoot(QemuMediaHotplugResponse),
    QemuMediaSystemPowerdown(QemuMediaLifecycleResponse),
    QemuMediaQueryStatus(QemuMediaQueryStatusResponse),
    QemuMediaQuit(QemuMediaLifecycleResponse),
    QemuMediaAttach(QemuMediaHotplugResponse),
    QemuMediaDetach(QemuMediaHotplugResponse),
    /// OpenPidfd response. The pidfd itself is returned via SCM_RIGHTS
    /// on the same frame; the JSON body confirms which `(pid,
    /// start_time_ticks)` the broker verified.
    OpenPidfd(OpenPidfdResponse),
    /// Drain response for `BrokerRequest::PollChildReaped`.
    PollChildReaped(PollChildReapedResponse),
    ReconcileStorageScope(ReconcileStorageScopeResponse),
    SetBridgePortFlags(BridgePortFlagsResponse),
    SignalRunner(SignalRunnerResponse),
    DeregisterRunnerPidfd(DeregisterRunnerPidfdResponse),
    SpawnRunner(SpawnRunnerResponse),
    /// Typed response carrying the activated generation (collision-free
    /// `generation_id` plus the u32 `generation_token`), the resolved
    /// hardlink-farm root, and the count of top-level closure paths
    /// populated. Used by the daemon to surface the swap result in audit
    /// + start traces.
    StoreSync(StoreSyncResponse),
    /// Result of an explicit live-pool verification request.
    StoreVerify(StoreVerifyResponse),
    ValidateLockSpec(ValidateLockSpecResponse),
    ValidateBundle(ValidateBundleResponse),
}

/// Typed broker error envelope for the real wire. Mirrors the
/// bootstrap `BrokerResponse::Error` struct variant fields so the audit
/// pipeline + daemon-side error propagation stay shape-compatible
/// across the dispatcher transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrokerErrorResponse {
    pub kind: String,
    pub operation: String,
    #[serde(default)]
    pub target_wave: Option<String>,
    pub message: String,
    pub action: String,
}

/// Daemon ↔ broker handshake response. Mirrors the bootstrap
/// `BrokerResponse::HelloOk` shape so the connection-level capability
/// negotiation works without a side-channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelloResponse {
    pub server_version: String,
    pub selected_version: String,
    pub capabilities: Vec<String>,
}

/// The broker re-derives the desired nft state from
/// `bundle_nft_intent_ref`. The daemon does NOT pass inline rule text.
/// `desired_hash` is a stable digest of the resolved intent, used for
/// idempotent audit + drift detection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyNftablesRequest {
    pub bundle_nft_intent_ref: BundleOpId,
    pub scope_id: ScopeId,
    #[serde(default)]
    pub desired_hash: Option<String>,
    #[serde(default)]
    pub destroy: bool,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyNmUnmanagedRequest {
    pub bundle_nm_intent_ref: BundleOpId,
    pub scope_id: ScopeId,
    #[serde(default)]
    pub destroy: bool,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyRouteRequest {
    pub bundle_route_intent_ref: BundleOpId,
    pub scope_id: ScopeId,
    #[serde(default)]
    pub destroy: bool,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplySysctlRequest {
    pub bundle_sysctl_intent_ref: BundleOpId,
    pub scope_id: ScopeId,
    #[serde(default)]
    pub destroy: bool,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindUnixSocketRequest {
    pub bundle_socket_intent_ref: BundleOpId,
    pub vm_id: VmId,
    pub role_id: RoleId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateOrReconcileUsersGroupsRequest {
    pub subject_ids: Vec<SubjectId>,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// The broker derives the bridge ifname, owner uid/gid, and TAP
/// attributes from the trusted bundle row anchored by `role_id` +
/// `vm_id`. The legacy wire carried a caller-supplied
/// `ifname_derived: IfName`; that preserved a future bypass of
/// broker-side trusted-bundle resolution, so the field was removed. The
/// broker emits the observed ifname only in the audit record / response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatePersistentTapRequest {
    pub role_id: RoleId,
    pub vm_id: VmId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// See [`CreatePersistentTapRequest`] for the opaque-ID rationale;
/// `CreateTapFd` follows the same contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateTapFdRequest {
    pub role_id: RoleId,
    pub vm_id: VmId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// The slice path is pinned by the bundle
/// (`/sys/fs/cgroup/d2b.slice`). It is **not** taken from caller
/// input — the broker reads it from its own bundle copy via `scope_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DelegateCgroupV2Request {
    pub scope_id: ScopeId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportBrokerAuditRequest {
    pub filter: Option<BrokerAuditFilter>,
    pub since: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrokerAuditFilter {
    pub env: Option<String>,
    pub operation: Option<String>,
    pub vm: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum GuestControlProofRole {
    HostProof,
    GuestProof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum GuestControlDirection {
    HostToGuest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum GuestControlAuthPurpose {
    GuestControlAuthV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GuestBootIdWire(pub String);

impl GuestBootIdWire {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl JsonSchema for GuestBootIdWire {
    fn schema_name() -> String {
        "GuestBootIdWire".to_owned()
    }

    fn json_schema(_gen: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        schemars::schema::Schema::Object(schemars::schema::SchemaObject {
            instance_type: Some(schemars::schema::SingleOrVec::Single(Box::new(
                schemars::schema::InstanceType::String,
            ))),
            string: Some(Box::new(schemars::schema::StringValidation {
                min_length: Some(1),
                max_length: Some(128),
                ..Default::default()
            })),
            ..Default::default()
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestControlSignRequest {
    pub vm_id: VmId,
    pub role: GuestControlProofRole,
    pub protocol_version: u32,
    pub direction: GuestControlDirection,
    pub purpose: GuestControlAuthPurpose,
    pub guest_control_port: u32,
    #[serde(default)]
    pub peer_cid: Option<u32>,
    #[schemars(length(min = 32, max = 32))]
    pub host_nonce: Vec<u8>,
    #[schemars(length(min = 32, max = 32))]
    pub guest_nonce: Vec<u8>,
    pub guest_boot_id: GuestBootIdWire,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities_hash: Option<String>,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

impl GuestControlSignRequest {
    pub fn validate_shape(&self) -> Result<(), &'static str> {
        if self.host_nonce.len() != AUTH_NONCE_LEN || self.guest_nonce.len() != AUTH_NONCE_LEN {
            return Err("nonce-length");
        }
        if self.guest_boot_id.as_str().is_empty() || self.guest_boot_id.as_str().len() > 128 {
            return Err("guest-boot-id");
        }
        match self.role {
            GuestControlProofRole::HostProof if self.capabilities_hash.is_some() => {
                Err("host-proof-capabilities-hash")
            }
            GuestControlProofRole::GuestProof => {
                let Some(hash) = self.capabilities_hash.as_ref() else {
                    return Err("guest-proof-missing-capabilities-hash");
                };
                if hash.is_empty() || hash.len() > 128 {
                    return Err("capabilities-hash");
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestControlSignResponse {
    #[schemars(length(min = 32, max = 32))]
    pub tag: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecretByIdRequest {
    pub opaque_id: String,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// The daemon never passes argv, env, uid/gid, caps, seccomp profile
/// path, or any other launch authority across the wire. The broker
/// reads the full launch context from `bundle.vms[vm_id].roles[role_id]`
/// and constructs the minijail exec line itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LaunchMinijailChildRequest {
    pub vm_id: VmId,
    pub role_id: RoleId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// `module_name` stays as the (already-validated)
/// [`ModuleName`] newtype because it is genuinely a public input —
/// the broker still looks it up in the trusted kernel-module
/// matrix and refuses anything not in the allow list. The matrix
/// itself never crosses the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModprobeIfAllowedRequest {
    /// Kernel-module name. The broker validates this against the
    /// trusted module allowlist; anything not present is refused.
    pub module_name: String,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenCgroupDirRequest {
    pub scope_id: ScopeId,
    pub path_class: PathClass,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenDeviceRequest {
    pub role_id: RoleId,
    pub device_class: String,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenKvmRequest {
    pub role_id: RoleId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// Physical USB enrollment for qemu-media.
///
/// `bus_id` is a transient selector used only by the privileged broker to
/// locate the device under sysfs at enrollment time. It is intentionally not
/// echoed in the success response and is never emitted into Nix-store-backed
/// artifacts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QemuMediaEnrollRequest {
    pub vm_id: VmId,
    pub media_ref: MediaRef,
    pub bus_id: String,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QemuMediaEnrollResponse {
    pub vm_id: VmId,
    pub media_ref: MediaRef,
    pub read_only: bool,
    pub enrolled: bool,
    pub udev_rule_written: bool,
    pub udev_reloaded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QemuMediaRefreshRegistryRequest {
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QemuMediaRefreshRegistryResponse {
    pub record_count: u32,
    pub redacted_index_written: bool,
    pub udev_rule_written: bool,
    pub udev_reloaded: bool,
}

/// qemu-media boot request keyed by VM id only.
///
/// The broker resolves the VM's declared boot source from the trusted bundle.
/// Physical USB boot sources use the root-only enrollment registry; image-file
/// boot sources use the trusted bundle path. Media fds stay inside the broker
/// until QMP consumes them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QemuMediaBootRequest {
    pub vm_id: VmId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QemuMediaLifecycleRequest {
    pub vm_id: VmId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QemuMediaQueryStatusRequest {
    pub vm_id: VmId,
    /// True while daemon shutdown polling is already in progress. EOF,
    /// ECONNRESET, ENOENT, and similar disconnects are then returned as the
    /// closed status `connection-lost-during-shutdown` instead of as noisy
    /// broker errors.
    #[serde(default)]
    pub shutdown_context: bool,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum QemuMediaLifecycleAction {
    SystemPowerdown,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum QemuMediaVmStatus {
    Running,
    Paused,
    Shutdown,
    Suspended,
    Watchdog,
    Debug,
    Inmigrate,
    InternalError,
    IoError,
    Postmigrate,
    Prelaunch,
    FinishMigrate,
    RestoreVm,
    SaveVm,
    GuestPanicked,
    Colo,
    Preconfig,
    Unknown,
    ConnectionLostDuringShutdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QemuMediaLifecycleResponse {
    pub vm_id: VmId,
    pub command: QemuMediaLifecycleAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QemuMediaQueryStatusResponse {
    pub vm_id: VmId,
    pub status: QemuMediaVmStatus,
}

/// qemu-media hotplug request keyed by a runtime USB busid selector.
///
/// The broker compares the current sysfs identity behind `bus_id` with the
/// root-only registry records for `vm_id` and returns only opaque slot/ref
/// information plus QMP command names. The success response never echoes the
/// busid, by-id names, serials, block paths, or the registry path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QemuMediaHotplugRequest {
    pub vm_id: VmId,
    pub bus_id: String,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum QemuMediaHotplugStatus {
    IdentityResolved,
    QmpConnected,
    QmpCapabilities,
    FdAdded,
    BlockdevAdded,
    DeviceAdded,
    DeviceDeleted,
    BlockdevDeleted,
    FdRemoved,
    VmContinued,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QemuMediaHotplugEvent {
    pub status: QemuMediaHotplugStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QemuMediaHotplugResponse {
    pub vm_id: VmId,
    pub media_ref: MediaRef,
    pub slot: String,
    pub read_only: bool,
    pub qmp_commands: Vec<String>,
    pub events: Vec<QemuMediaHotplugEvent>,
}

/// OpenPidfd daemon-side reconcile-and-adopt support. The daemon's
/// `d2bd::supervisor::state::reconcile_and_adopt` loop sends this
/// request for every snapshot the classifier returned `Adopt` for. The
/// broker:
///
/// 1. Calls `pidfd_open(pid)`.
/// 2. Reads `/proc/<pid>/stat` field 22 (start-time ticks).
/// 3. Compares against `expected_start_time_ticks`.
/// 4. On match: returns the pidfd via SCM_RIGHTS + the
///    [`OpenPidfdResponse`] JSON body.
/// 5. On mismatch (pid reuse race): closes the pidfd and surfaces
///    a typed pidfd-race error (audit record carries the observed
///    start-time so the operator can correlate).
///
/// This atomic open-AND-verify closes the critical pid-reuse issue: the
/// daemon could otherwise re-adopt a pidfd that referred to a reused-pid
/// unrelated process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenPidfdRequest {
    /// Per-VM scope the snapshot belongs to.
    pub vm_id: VmId,
    /// Per-VM role identifier (matches the daemon-side
    /// `PidfdKey::role_id`).
    pub role_id: RoleId,
    /// PID the snapshot recorded.
    pub pid: i32,
    /// Field-22 start-time ticks from `/proc/<pid>/stat` at the
    /// time the snapshot was written. The broker re-reads field
    /// 22 AFTER `pidfd_open` and compares; mismatch means the pid
    /// was reused.
    pub expected_start_time_ticks: u64,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// Response body for [`OpenPidfdRequest`] success. The pidfd
/// itself is the first SCM_RIGHTS attachment on the same frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenPidfdResponse {
    pub vm_id: VmId,
    pub role_id: RoleId,
    pub pid: i32,
    /// Echoed back so the daemon can re-verify the match the broker
    /// performed. Equal to `expected_start_time_ticks` from the
    /// request.
    pub verified_start_time_ticks: u64,
    /// Always `0` today; reserved for future multi-fd
    /// SCM_RIGHTS handoffs.
    pub pidfd_index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenVhostNetRequest {
    pub role_id: RoleId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenFuseRequest {
    pub role_id: RoleId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// The concrete `/var/lib/d2b/vms/<vm>` or `/run/d2b/<vm>` path
/// is derived from `vm_id` + `path_class` against the broker-side
/// bundle. The daemon never passes a raw path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrepareDirRequest {
    pub vm_id: VmId,
    pub path_class: PathClass,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrepareStoreViewRequest {
    pub vm_id: VmId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// Store-sync request. The broker resolves the closure intent row from
/// the plain per-VM `vm_id` and refuses the op if `bundle_closure_ref`
/// does not match. The broker also
/// refuses if the wire-supplied `generation_token` does not match the
/// bundle's resolved generation. The token is a content-derived stable
/// equality value (see `closures-json.nix`), not a monotonic counter:
/// the daemon and broker both read it from the same trusted bundle, so
/// a mismatch means a stale daemon is racing the activator and the op
/// is refused fail-closed. It is a display/wire token only and is never
/// used as the on-disk generation key — the broker derives the
/// collision-free `generation_id` (full closure identity, ADR 0027)
/// from its trusted closure copy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoreSyncRequest {
    pub vm_id: VmId,
    pub bundle_closure_ref: BundleClosureRef,
    pub generation_token: u32,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// Store-sync response. Returned after the broker successfully
/// populates the per-VM hardlink farm and swaps the `current` symlink
/// atomically. The `hardlink_farm_path` is the per-VM farm root (i.e.
/// `/var/lib/d2b/vms/<vm>/store-view/`); the active generation
/// directory is reachable via the `current` symlink.
///
/// ADR 0027: `generation_id` is the collision-free on-disk layout key
/// (a SHA-256 over the full ordered closure identity). `generation_token`
/// is the truncated u32 display/wire value carried for backwards
/// compatibility and operator-facing output; it is never used as the
/// on-disk key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoreSyncResponse {
    pub vm: String,
    pub generation_id: String,
    pub generation_token: u32,
    pub hardlink_farm_path: String,
    pub closure_count: u32,
    pub retained_generations: Vec<u32>,
    pub swept_count: u32,
    pub cleanup_deferred: bool,
}

/// Store-verify request. `repair=true` requests the broker's explicit
/// repair path; builds without that path must fail closed instead of
/// returning a success-shaped repair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoreVerifyRequest {
    pub vm_id: VmId,
    #[serde(default)]
    pub repair: bool,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StoreVerifyStatus {
    Ok,
    Drift,
    Unknown,
    Repaired,
    Failed,
    NotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StoreVerifyUnknownReason {
    MarkerOrManifestMissing,
    MarkerOrManifestUnreadable,
    OlderHostGeneration,
    GenerationIdentityUnavailable,
}

/// Store-verify response. Field names intentionally match the public CLI
/// JSON envelope after serde's camelCase conversion on the private wire;
/// the CLI re-renders the signed snake_case envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoreVerifyResponse {
    pub vm: String,
    pub status: StoreVerifyStatus,
    pub checked: u32,
    pub drifted: u32,
    pub repaired: u32,
    #[serde(default)]
    pub unknown_reason: Option<StoreVerifyUnknownReason>,
    #[serde(default)]
    pub audit_ref: Option<String>,
    #[serde(default)]
    pub remediation: Option<String>,
}

/// The broker derives the bridge, port,
/// isolated/neigh_suppress/learning/unicast_flood flags, and matching
/// rule rationale from the trusted bundle row anchored by `vm_id` +
/// `role_id`. The legacy wire carried caller-supplied `bridge: IfName`,
/// `port: IfName`, `isolated: bool`, `neigh_suppress: bool`; these
/// violated the broker's own "daemon never names raw ifnames or raw
/// intent" invariant, so the fields were removed. The broker reads the
/// per-role `BridgePortFlags` row from
/// `bundle.host.environments[*].bridgePortFlags` keyed by `role_id` and
/// applies the documented flag set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetBridgePortFlagsRequest {
    pub vm_id: VmId,
    pub role_id: RoleId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetSocketAclRequest {
    pub bundle_socket_intent_ref: BundleOpId,
    pub vm_id: VmId,
    pub role_id: RoleId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetupMountNamespaceRequest {
    pub vm_id: VmId,
    pub role_id: RoleId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// The managed-block lines come from the bundle's `host::HostsEntry`
/// rows, not the wire. The broker only needs the lookup key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateHostsFileRequest {
    pub bundle_hosts_intent_ref: BundleOpId,
    #[serde(default)]
    pub destroy: bool,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// USBIP live device routing. The daemon supplies only the opaque bind intent
/// id; the broker resolves busid, VM, env, lock path, and physical allowlist
/// from its trusted bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipBindRequest {
    pub bundle_usbip_bind_intent_ref: BundleOpId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// USBIP firewall-rule skeleton. The rule body and the bus_id are
/// derived from the per-busid policy in the trusted bundle
/// (`bundle.usbip.busidLocks[*]`) via the
/// `bundle_usbip_firewall_intent_ref` opaque-ID lookup. The legacy
/// caller-supplied `bus_id: String` + `rule_hash: String` fields were
/// replaced with this opaque reference because the raw `bus_id` was
/// being interpolated into nft rule text without a validating newtype or
/// escaping, and the caller-supplied `rule_hash` allowed the daemon to
/// override the broker's drift-detection digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipBindFirewallRuleRequest {
    pub bundle_usbip_firewall_intent_ref: BundleOpId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipProxyReconcileRequest {
    pub scope_id: ScopeId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipUnbindRequest {
    pub bundle_usbip_bind_intent_ref: BundleOpId,
    /// VM stop/restart tears down active host carrier state while preserving the
    /// host-session same-VM claim so the next start can replay it in the current
    /// host boot. Explicit detach leaves this false and releases the claim after
    /// unbind/ACL revoke.
    #[serde(default)]
    pub preserve_durable_claim: bool,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// Explicit-attach: bind a present sysfs busid for a USB-capable VM
/// without a bundle intent ref. The daemon has already completed:
///  1. sysfs busid presence check (fail-closed if device absent),
///  2. USB-capable gate (`runtime.capabilities.usbHotplug`),
///  3. active-claim exclusivity check (OFD lock read).
///
/// The broker acquires the per-busid OFD lock, runs `usbip bind`, and
/// spawns a per-device backend (not the shared per-env backend).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipExplicitBindRequest {
    /// Daemon-validated sysfs busid (max 31 chars, no metacharacters).
    pub bus_id: String,
    /// USB-capable target VM (must exist in the trusted manifest).
    pub vm: String,
    /// Env the VM belongs to, used for firewall scope and audit.
    pub env: String,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// Explicit-attach: install a per-busid nftables carve-out scoped
/// to the target VM's env bridge. The broker builds the scoped
/// `inet d2b` input rule from `host_uplink_ip` (the env bridge
/// side) and `net_uplink_ip` (the net-VM uplink) so the carve-out is
/// strictly limited to traffic from the owner env's net VM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipExplicitFirewallRuleRequest {
    /// Daemon-validated sysfs busid (max 31 chars, no metacharacters).
    pub bus_id: String,
    /// Env name for audit and rule scoping.
    pub env: String,
    /// The per-env host-uplink IP bound by the USBIP proxy listener.
    pub host_uplink_ip: String,
    /// The per-env net-VM uplink source IP for anti-spoof matching.
    pub net_uplink_ip: String,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AckResponse {
    pub accepted: bool,
    pub operation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TapReadyResponse {
    pub bridge: Option<IfName>,
    pub tap: IfName,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportBrokerAuditResponse {
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BridgePortFlagsResponse {
    pub bridge: IfName,
    pub isolated: bool,
    pub neigh_suppress: bool,
    pub port: IfName,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidateBundleResponse {
    pub valid: bool,
}

/// Runner-signal broker envelope. The live daemon stop/restart path first
/// delivers signals through `d2bd::supervisor::pidfd_table` after
/// `SpawnRunner` pidfd registration; on pidfd `EPERM`, d2bd falls back
/// to this broker-owned live caller via `stop_vm_pidfd_role`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RunnerSignal {
    Term,
    Kill,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignalRunnerRequest {
    pub vm_id: VmId,
    pub role_id: RoleId,
    pub signal: RunnerSignal,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_start_time_ticks: Option<u64>,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignalRunnerResponse {
    pub signaled: bool,
    pub vm_id: VmId,
    pub role_id: RoleId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeregisterRunnerPidfdRequest {
    pub vm_id: VmId,
    pub role_id: RoleId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeregisterRunnerPidfdResponse {
    pub vm_id: VmId,
    pub role_id: RoleId,
    pub removed: bool,
}

/// The daemon never names argv, env, uid/gid, caps,
/// kernel/initrd/cmdline strings, virtiofs sockets, TAP fds, or any
/// other launch authority across the wire. The broker resolves the full
/// role spawn context from `bundle.vms[vm_id].roles[role_id]` anchored
/// by the opaque `bundle_runner_intent_ref`. The wire shape follows the
/// opaque-only contract for every other mutating variant.
///
/// `RunnerRole` selects which argv generator the broker invokes against
/// the bundle data (CH, virtiofsd, or swtpm). Adding new roles requires
/// a bundle schema bump so downstream bundles can declare the new launch
/// context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RunnerRole {
    /// Cloud Hypervisor headless / hybrid VM. Broker invokes
    /// `d2b_host::ch_argv::generate_ch_argv`.
    CloudHypervisor,
    /// QEMU media runtime scaffold. Broker invokes
    /// `d2b_host::qemu_media_argv::generate_qemu_media_argv`.
    QemuMedia,
    /// virtiofsd sidecar; one per `microvm.shares` row. The daemon/bundle
    /// provides argv from `nixos-modules/processes-json.nix`.
    Virtiofsd,
    /// swtpm sidecar (long-lived `swtpm socket ...` process).
    Swtpm,
    /// swtpm pre-start flush (`swtpm_ioctl -i --unix ...`). One-shot.
    SwtpmFlush,
    /// crosvm GPU sidecar. Broker invokes `d2b_host::gpu_argv::generate_gpu_argv`.
    Gpu,
    /// vhost-device-sound audio sidecar. Broker invokes `d2b_host::audio_argv::generate_audio_argv`.
    Audio,
    /// crosvm video-decoder sidecar. Broker invokes `d2b_host::video_argv::generate_video_argv`.
    Video,
    /// socat-based vsock relay sidecar. Broker invokes `d2b_host::vsock_relay_argv::generate_vsock_relay_argv`.
    VsockRelay,
    /// usbip helper sidecar. Broker invokes `d2b_host::usbip_argv::{generate_usbip_bind_argv, generate_usbip_unbind_argv}`.
    Usbip,
    /// OTel host-bridge sidecar (vsock relay folded out of
    /// `d2b-otel-host-bridge.service` into broker SpawnRunner).
    /// Receives pre-opened fds for the obs VM vsock socket and the
    /// d2b OTel host-egress socket; no AF_VSOCK socket creation
    /// capability in the role profile. Broker invokes
    /// `d2b_host::otel_host_bridge_argv::generate_otel_host_bridge_argv`.
    OtelHostBridge,
    /// Host-jailed Wayland filter proxy. Broker invokes
    /// `d2b_host::wayland_proxy_argv::generate_wayland_proxy_argv`.
    /// Empty host capabilities; mandatory `seccompPolicyRef`; no
    /// PipeWire/Pulse socket access. Runs as `d2b-<vm>-wlproxy`
    /// with the real host compositor socket bound read/write at a
    /// fixed in-jail upstream path.
    WaylandProxy,
    /// Broker-spawned console drain helper (ADR 0041). Holds the host end
    /// of the broker-owned console socketpair, drains console output into
    /// a bounded ring buffer, and exposes it to d2bd over a control
    /// socket. One per VM with console enabled.
    ConsoleDrain,
}

impl RunnerRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CloudHypervisor => "cloud-hypervisor",
            Self::QemuMedia => "qemu-media",
            Self::Virtiofsd => "virtiofsd",
            Self::Swtpm => "swtpm",
            Self::SwtpmFlush => "swtpm-flush",
            Self::Gpu => "gpu",
            Self::Audio => "audio",
            Self::Video => "video",
            Self::VsockRelay => "vsock-relay",
            Self::Usbip => "usbip",
            Self::OtelHostBridge => "otel-host-bridge",
            Self::WaylandProxy => "wayland-proxy",
            Self::ConsoleDrain => "console-drain",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpawnRunnerRequest {
    /// VM scope the runner belongs to.
    pub vm_id: VmId,
    /// Per-VM role this runner fills. Must be unique across the VM's
    /// active runners — the daemon's pidfd table is keyed on
    /// `(vm_id, role_id)` and a duplicate registration fails closed.
    pub role_id: RoleId,
    /// Role selector — picks the argv generator the broker applies to
    /// the bundle row anchored by `bundle_runner_intent_ref`.
    pub role: RunnerRole,
    /// Opaque reference into the trusted bundle's runner-intent table.
    /// The broker resolves this to the full launch context (binary
    /// path, argv inputs, uid/gid, capabilities, seccomp policy ref,
    /// cgroup placement, mount namespace, environment) and feeds it
    /// to the matching argv generator.
    pub bundle_runner_intent_ref: BundleOpId,
    /// Optional vsock CID / TAP fd slot allocated by the daemon at
    /// host-prepare time. The broker validates each entry against the
    /// bundle row and refuses any unexpected allocation slot. None
    /// for roles that do not need them (virtiofsd / swtpm).
    #[serde(default)]
    pub runtime_allocations: Vec<RunnerAllocation>,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// Per-runner runtime allocation tuple. Each entry pairs a typed slot
/// kind with the daemon-side opaque reference (a stringified file
/// descriptor slot, vsock CID, or socket path the broker validates
/// against the bundle row).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunnerAllocation {
    pub kind: RunnerAllocationKind,
    /// Opaque reference; the broker interprets per-kind.
    pub opaque_ref: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RunnerAllocationKind {
    /// CH `--vsock cid=N` value (the daemon's allocator decided this
    /// CID at host-prepare time; the broker cross-checks against the
    /// per-VM bundle row).
    VsockCid,
    /// CH `--net fd=N` value when running under
    /// [`crate::broker_wire::CreateTapFdRequest`] — the daemon
    /// references the SCM_RIGHTS slot the broker handed back in the
    /// matching CreateTapFd response.
    TapFdSlot,
    /// CH `--api-socket` path the daemon owns; the broker validates
    /// the path is under `/run/d2b/<vm>/`.
    ApiSocketPath,
}

/// Response to [`SpawnRunnerRequest`]. The pidfd itself is delivered
/// out-of-band as a `SCM_RIGHTS` attachment on the same broker socket
/// frame; this JSON body carries the metadata the daemon's pidfd
/// table requires to validate / reconcile the handle (`(pid,
/// start_time_ticks)` is the pidfd contract).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpawnRunnerResponse {
    pub vm_id: VmId,
    pub role_id: RoleId,
    pub role: RunnerRole,
    /// Child PID. The daemon validates this against the pidfd it
    /// received and against `/proc/<pid>/stat` field 22
    /// (`start_time`).
    pub pid: i32,
    /// Field-22 `start_time` value the broker captured immediately
    /// after `clone()`. Pinned to the pidfd so any restart
    /// reconciliation rejects a stale (pid, start_time) tuple.
    pub start_time_ticks: u64,
    /// Index into the SCM_RIGHTS fd vector the daemon should treat as
    /// the spawned process's pidfd. Always `0` today — kept explicit
    /// so future multi-fd spawn responses (e.g. CH API socket + pidfd)
    /// have an existing wire slot.
    pub pidfd_index: u32,
}

/// Wire envelope wrapping a [`BrokerRequest`] with the authenticated
/// caller context the broker uses for authorization and audit.
///
/// The caller role is derived from `SO_PEERCRED` before dispatch.
/// Broker fallback requests sent by `d2bd` carry the public
/// socket caller role that already passed daemon-side authorization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrokerRequestEnvelope {
    pub request: BrokerRequest,
    #[serde(default)]
    pub caller_role: BrokerCallerRole,
    /// Test-only peer uid override; ignored by the production
    /// broker (which always uses `SO_PEERCRED`).
    #[serde(default)]
    pub test_peer_uid: Option<u32>,
}

/// Caller role classification derived from `SO_PEERCRED` + the
/// `d2b.site.adminUsers` / `d2b.site.launcherUsers`
/// allowlists. Mirrors the legacy `bootstrap::wire::CallerRole`
/// but lives in the production wire crate so the live broker
/// dispatch can take it directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(tag = "role", rename_all = "PascalCase", deny_unknown_fields)]
pub enum BrokerCallerRole {
    AdminUid {
        uid: u32,
    },
    LauncherUid {
        uid: u32,
    },
    RootUid {
        uid: u32,
    },
    #[default]
    NotAuthorized,
}

impl BrokerCallerRole {
    pub fn is_admin_uid(&self) -> bool {
        matches!(self, Self::AdminUid { .. })
    }

    pub fn for_display(&self) -> &'static str {
        match self {
            Self::AdminUid { .. } => "d2b-admin",
            Self::LauncherUid { .. } => "d2b-launcher",
            Self::RootUid { .. } => "RootUid",
            Self::NotAuthorized => "d2b-not-authorized",
        }
    }
}

// ---------------------------------------------------------------
// Typed broker request scaffolds for the host-prep DAG steps. The
// dispatchers currently return `BrokerError::Unimplemented` until real
// handlers are wired. The structs follow the opaque-id discipline: the
// daemon never names raw paths/uids/argv on the wire — only
// bundle-resolved intent references.
// ---------------------------------------------------------------

/// SeedDnsmasqLease request. The broker resolves the per-VM dnsmasq
/// lease intent from the bundle (using `vm_id`) and writes
/// `/var/lib/d2b/dnsmasq/<vm>.leases` with the correct owner / mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SeedDnsmasqLeaseRequest {
    pub vm_id: VmId,
    pub scope_id: ScopeId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// BindMountFromHardlinkFarm request. The broker resolves the
/// `store-view` intent for `vm_id` and creates the bind mount from the
/// per-VM hardlink farm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindMountFromHardlinkFarmRequest {
    pub vm_id: VmId,
    /// Optional opaque pointer at the `store-view` intent row.
    /// `None` means "use the canonical per-VM intent".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_store_view_intent_ref: Option<BundleOpId>,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// OwnershipMatrixCheck request. The broker walks the
/// `/var/lib/d2b/vms/<vm>/` subtree and verifies each leaf against
/// the ownership matrix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnershipMatrixCheckRequest {
    pub vm_id: VmId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// SshHostKeyPreflight request. The broker opens every
/// `/var/lib/d2b/vms/<vm>/sshd-host-keys/ssh_host_*_key` with
/// `O_NOFOLLOW` and refuses if drift from `root:root 0400`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SshHostKeyPreflightRequest {
    pub vm_id: VmId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// Broker-side storage reconciliation request.
///
/// The daemon supplies only a bundle-resolved storage id. The broker looks
/// up the concrete path, owner, mode, kind, cleanup/repair policy, and
/// invariants in its trusted `storage.json`; no raw path or mode crosses
/// the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReconcileStorageScopeRequest {
    pub storage_ref: BundleOpId,
    #[serde(default)]
    pub apply: bool,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum StorageReconcileStatus {
    Clean,
    Created,
    Reused,
    CheckedOnly,
    TemplateUnexpanded,
    Refused,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReconcileStorageScopeResponse {
    pub storage_ref: BundleOpId,
    pub scope: String,
    pub kind: String,
    pub status: StorageReconcileStatus,
    pub applied: bool,
    pub path_hash: String,
}

/// Broker-side synchronization contract validation request.
///
/// The daemon supplies only a lock id. The broker resolves and validates the
/// lock row from trusted `sync.json`; it does not accept raw lock paths or
/// fd-transfer policy from the caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidateLockSpecRequest {
    pub lock_ref: BundleOpId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidateLockSpecResponse {
    pub lock_ref: BundleOpId,
    pub scope: String,
    pub kind: String,
    pub cloexec_required: bool,
    pub fd_passing_mechanism: String,
    pub order_key: String,
}

/// Disk-image provisioning request.
///
/// The daemon sends the VM's opaque `vm_id`; the broker resolves
/// every `DiskInit` plan-op from the trusted bundle's
/// `ProcessNode.plan_ops` for that VM and creates or validates the
/// disk images before runner spawn. Existing `ifAbsent` images are
/// skipped only after fd-bound identity and ext4-superblock validation;
/// declared owner/mode posture drift is repaired automatically when the
/// held fd is safe, and a present unformatted image is repaired only
/// when it is proven empty. Otherwise the broker fails closed.
///
/// Security: the broker NEVER trusts a caller-supplied path. All
/// `target_path`, `size_bytes`, `mode`, `owner_uid`, and `owner_gid`
/// values come from the bundle; the caller supplies only an opaque
/// VM identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiskInitRequest {
    pub vm_id: VmId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// Exit status kind for a broker-reaped child.
///
/// - `Exited`: child called `_exit(n)` / `exit(n)`.
/// - `Signaled`: child was killed by a signal that is NOT SIGKILL.
/// - `Killed`: child was killed specifically by SIGKILL (unexpected termination).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ChildExitKind {
    Exited,
    Signaled,
    Killed,
}

/// Typed exit status carried in [`ChildReapedNotification`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChildExitStatus {
    pub kind: ChildExitKind,
    /// Exit code (present when `kind == "exited"`).
    #[serde(default)]
    pub code: Option<i32>,
    /// Signal number (present when `kind == "signaled"` or `"killed"`).
    #[serde(default)]
    pub signal: Option<i32>,
}

/// One broker-reaped child notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChildReapedNotification {
    /// `"<vm_id>:<role_id>"` key from the broker's pidfd registry.
    pub runner_id: String,
    pub pid: i32,
    pub exit_status: ChildExitStatus,
    /// Unix timestamp milliseconds when the broker called `waitid`.
    pub reaped_at_ms: i64,
}

/// Broker-to-daemon push notifications.
///
/// `#[serde(tag = "kind")]` (internally-tagged, no content wrapper)
/// so a future variant can be added without breaking old daemons;
/// unknown kinds deserialise as `Unknown` (unit variant, `#[serde(other)]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum BrokerNotification {
    ChildReaped(ChildReapedNotification),
    #[serde(other)]
    Unknown,
}

/// Response to `BrokerRequest::PollChildReaped`. Drains and returns all
/// buffered `ChildReaped` notifications in FIFO order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PollChildReapedResponse {
    pub notifications: Vec<ChildReapedNotification>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decode_frame, encode_frame};

    #[test]
    fn validate_bundle_serializes_with_exact_kind() {
        let json = serde_json::to_value(BrokerRequest::ValidateBundle).expect("serializes");
        assert_eq!(json, serde_json::json!({ "kind": "ValidateBundle" }));
    }

    #[test]
    fn broker_caller_role_default_is_not_authorized() {
        assert!(matches!(
            BrokerCallerRole::default(),
            BrokerCallerRole::NotAuthorized
        ));
    }

    #[test]
    fn broker_caller_role_admin_passes_predicate() {
        assert!(BrokerCallerRole::AdminUid { uid: 1000 }.is_admin_uid());
        assert!(!BrokerCallerRole::LauncherUid { uid: 1000 }.is_admin_uid());
    }

    #[test]
    fn broker_caller_role_display_uses_stable_audit_labels() {
        assert_eq!(
            BrokerCallerRole::LauncherUid { uid: 1000 }.for_display(),
            "d2b-launcher"
        );
        assert_eq!(
            BrokerCallerRole::AdminUid { uid: 1000 }.for_display(),
            "d2b-admin"
        );
        assert_eq!(
            BrokerCallerRole::NotAuthorized.for_display(),
            "d2b-not-authorized"
        );
    }

    #[test]
    fn broker_caller_role_round_trips() {
        for role in [
            BrokerCallerRole::AdminUid { uid: 1000 },
            BrokerCallerRole::LauncherUid { uid: 1001 },
            BrokerCallerRole::RootUid { uid: 0 },
            BrokerCallerRole::NotAuthorized,
        ] {
            let json = serde_json::to_string(&role).unwrap();
            let parsed: BrokerCallerRole = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, role);
        }
    }

    #[test]
    fn broker_request_envelope_round_trips_with_admin() {
        let env = BrokerRequestEnvelope {
            request: BrokerRequest::ValidateBundle,
            caller_role: BrokerCallerRole::AdminUid { uid: 1000 },
            test_peer_uid: None,
        };
        let frame = encode_frame(&env).expect("encodes");
        let parsed: BrokerRequestEnvelope =
            decode_frame("BrokerRequestEnvelope", &frame).expect("decodes");
        assert_eq!(parsed, env);
    }

    #[test]
    fn broker_request_envelope_default_caller_role_is_not_authorized() {
        let json = serde_json::json!({
            "request": { "kind": "ValidateBundle" }
        });
        let env: BrokerRequestEnvelope = serde_json::from_value(json).unwrap();
        assert!(matches!(env.caller_role, BrokerCallerRole::NotAuthorized));
    }

    #[test]
    fn run_activation_request_round_trips() {
        let frame = encode_frame(&serde_json::json!({
            "kind": "RunActivation",
            "payload": {
                "bundleActivationIntentRef": "activation:vm:corp-vm",
                "mode": "switch",
                "vm": "corp-vm"
            }
        }))
        .expect("encodes");
        let decoded = decode_frame::<BrokerRequest>("BrokerRequest", &frame).expect("decodes");
        match decoded {
            BrokerRequest::RunActivation(req) => {
                assert_eq!(
                    req.bundle_activation_intent_ref.as_str(),
                    "activation:vm:corp-vm"
                );
                assert_eq!(req.mode, ActivationMode::Switch);
                assert_eq!(req.phase, ActivationPhase::MetadataOnly);
                assert_eq!(req.vm, "corp-vm");
            }
            other => panic!("expected RunActivation, got {other:?}"),
        }
    }

    #[test]
    fn run_activation_request_phase_round_trips() {
        let req = RunActivationRequest {
            bundle_activation_intent_ref: BundleOpId::new("activation:vm:corp-vm"),
            mode: ActivationMode::Switch,
            phase: ActivationPhase::Prepare,
            vm: "corp-vm".to_owned(),
            tracing_span_id: None,
        };
        let json = serde_json::to_string(&req).expect("serialize");
        assert!(json.contains("\"phase\":\"prepare\""));
        let decoded: RunActivationRequest = serde_json::from_str(&json).expect("decode");
        assert_eq!(decoded.phase, ActivationPhase::Prepare);
    }

    #[test]
    fn run_host_key_wire_variants_round_trip() {
        let trust = BrokerResponse::RunHostKeyTrust(RunHostKeyTrustResponse {
            vm: "corp-vm".to_owned(),
            static_ip: "10.20.0.10".to_owned(),
            known_hosts_path: "/var/lib/d2b/known_hosts.d2b".to_owned(),
            updated: true,
        });
        let rotate = BrokerResponse::RunRotateKnownHost(RunRotateKnownHostResponse {
            vm: "corp-vm".to_owned(),
            static_ip: "10.20.0.10".to_owned(),
            known_hosts_path: "/var/lib/d2b/known_hosts.d2b".to_owned(),
            removed: true,
        });
        let trust_json = serde_json::to_string(&trust).expect("serialize trust");
        let rotate_json = serde_json::to_string(&rotate).expect("serialize rotate");
        let decoded_trust: BrokerResponse =
            serde_json::from_str(&trust_json).expect("decode trust");
        let decoded_rotate: BrokerResponse =
            serde_json::from_str(&rotate_json).expect("decode rotate");
        assert_eq!(decoded_trust, trust);
        assert_eq!(decoded_rotate, rotate);
    }

    #[test]
    fn storage_and_sync_requests_are_opaque_only() {
        let storage = encode_frame(&serde_json::json!({
            "kind": "ReconcileStorageScope",
            "payload": {
                "storageRef": "path:run-root",
                "apply": false
            }
        }))
        .expect("encodes");
        let decoded = decode_frame::<BrokerRequest>("BrokerRequest", &storage).expect("decodes");
        match decoded {
            BrokerRequest::ReconcileStorageScope(req) => {
                assert_eq!(req.storage_ref.as_str(), "path:run-root");
                assert!(!req.apply);
            }
            other => panic!("expected ReconcileStorageScope, got {other:?}"),
        }

        let lock = encode_frame(&serde_json::json!({
            "kind": "ValidateLockSpec",
            "payload": {
                "lockRef": "lock:daemon"
            }
        }))
        .expect("encodes");
        let decoded = decode_frame::<BrokerRequest>("BrokerRequest", &lock).expect("decodes");
        match decoded {
            BrokerRequest::ValidateLockSpec(req) => {
                assert_eq!(req.lock_ref.as_str(), "lock:daemon");
            }
            other => panic!("expected ValidateLockSpec, got {other:?}"),
        }
    }

    #[test]
    fn usbip_bind_firewall_rule_round_trips() {
        // The wire shape carries an opaque BundleOpId reference instead
        // of raw bus_id + rule_hash; the broker resolves both
        // server-side from the trusted bundle's per-busid policy.
        let frame = encode_frame(&serde_json::json!({
            "kind": "UsbipBindFirewallRule",
            "payload": { "bundleUsbipFirewallIntentRef": "usbip-fw-1-2" }
        }))
        .expect("encodes");
        let decoded = decode_frame::<BrokerRequest>("BrokerRequest", &frame).expect("decodes");
        match decoded {
            BrokerRequest::UsbipBindFirewallRule(req) => {
                assert_eq!(
                    req.bundle_usbip_firewall_intent_ref.as_str(),
                    "usbip-fw-1-2"
                );
            }
            other => panic!("expected UsbipBindFirewallRule, got {other:?}"),
        }
    }

    #[test]
    fn usbip_proxy_reconcile_carries_optional_trace_context() {
        let frame = encode_frame(&serde_json::json!({
            "kind": "UsbipProxyReconcile",
            "payload": {
                "scopeId": "vm:corp-vm",
                "tracingSpanId": "usb-start-0000000000000001"
            }
        }))
        .expect("encodes");
        let decoded = decode_frame::<BrokerRequest>("BrokerRequest", &frame).expect("decodes");
        match decoded {
            BrokerRequest::UsbipProxyReconcile(req) => {
                assert_eq!(req.scope_id.as_str(), "vm:corp-vm");
                assert_eq!(
                    req.tracing_span_id.as_ref().map(TracingSpanId::as_str),
                    Some("usb-start-0000000000000001")
                );
            }
            other => panic!("expected UsbipProxyReconcile, got {other:?}"),
        }
    }

    /// CreatePersistentTap and CreateTapFd carry only opaque
    /// (role_id, vm_id) on the wire; the broker derives
    /// ifname/owner/attrs from the trusted bundle.
    #[test]
    fn create_persistent_tap_request_is_opaque_only() {
        let frame = encode_frame(&serde_json::json!({
            "kind": "CreatePersistentTap",
            "payload": {
                "roleId": "runner-lan",
                "vmId": "corp-vm"
            }
        }))
        .expect("encodes");
        let decoded = decode_frame::<BrokerRequest>("BrokerRequest", &frame).expect("decodes");
        match decoded {
            BrokerRequest::CreatePersistentTap(req) => {
                assert_eq!(req.role_id.as_str(), "runner-lan");
                assert_eq!(req.vm_id.as_str(), "corp-vm");
            }
            other => panic!("expected CreatePersistentTap, got {other:?}"),
        }
    }

    #[test]
    fn create_tap_fd_request_is_opaque_only() {
        let frame = encode_frame(&serde_json::json!({
            "kind": "CreateTapFd",
            "payload": {
                "roleId": "runner-lan",
                "vmId": "corp-vm"
            }
        }))
        .expect("encodes");
        let decoded = decode_frame::<BrokerRequest>("BrokerRequest", &frame).expect("decodes");
        match decoded {
            BrokerRequest::CreateTapFd(req) => {
                assert_eq!(req.role_id.as_str(), "runner-lan");
                assert_eq!(req.vm_id.as_str(), "corp-vm");
            }
            other => panic!("expected CreateTapFd, got {other:?}"),
        }
    }

    /// SetBridgePortFlags carries only opaque (role_id, vm_id) on the
    /// wire; the broker reads bridge/port names and the desired flag
    /// set from the trusted bundle's per-role BridgePortFlags row.
    #[test]
    fn set_bridge_port_flags_request_is_opaque_only() {
        let frame = encode_frame(&serde_json::json!({
            "kind": "SetBridgePortFlags",
            "payload": {
                "vmId": "corp-vm",
                "roleId": "workload-lan"
            }
        }))
        .expect("encodes");
        let decoded = decode_frame::<BrokerRequest>("BrokerRequest", &frame).expect("decodes");
        match decoded {
            BrokerRequest::SetBridgePortFlags(req) => {
                assert_eq!(req.vm_id.as_str(), "corp-vm");
                assert_eq!(req.role_id.as_str(), "workload-lan");
            }
            other => panic!("expected SetBridgePortFlags, got {other:?}"),
        }
    }

    /// Regression guard: a wire frame that still contains the legacy raw
    /// authority field is rejected by `deny_unknown_fields`. This pins
    /// the opaque-only contract.
    #[test]
    fn set_bridge_port_flags_rejects_raw_bridge_field() {
        let frame = encode_frame(&serde_json::json!({
            "kind": "SetBridgePortFlags",
            "payload": {
                "vmId": "corp-vm",
                "roleId": "workload-lan",
                "bridge": "br-x",
                "port": "tap-x",
                "isolated": true,
                "neighSuppress": false
            }
        }))
        .expect("encodes");
        let result = decode_frame::<BrokerRequest>("BrokerRequest", &frame);
        assert!(result.is_err(), "raw bridge/port/flags must be refused");
    }

    #[test]
    fn create_persistent_tap_rejects_raw_ifname_field() {
        let frame = encode_frame(&serde_json::json!({
            "kind": "CreatePersistentTap",
            "payload": {
                "roleId": "runner-lan",
                "vmId": "corp-vm",
                "ifnameDerived": "d2b-bXXXXXXXX"
            }
        }))
        .expect("encodes");
        let result = decode_frame::<BrokerRequest>("BrokerRequest", &frame);
        assert!(result.is_err(), "raw ifname_derived must be refused");
    }

    #[test]
    fn usbip_bind_firewall_rule_rejects_raw_bus_id_field() {
        let frame = encode_frame(&serde_json::json!({
            "kind": "UsbipBindFirewallRule",
            "payload": {
                "bundleUsbipFirewallIntentRef": "usbip-fw-1-2",
                "busId": "1-2"
            }
        }))
        .expect("encodes");
        let result = decode_frame::<BrokerRequest>("BrokerRequest", &frame);
        assert!(result.is_err(), "raw bus_id must be refused on the W3 wire");
    }

    /// Earlier rejection guards lumped multiple legacy authority fields
    /// into a single test payload, so any one field being accidentally
    /// reintroduced would still be caught — but the guard could not
    /// point at which field. The helper + per-field tests below assert
    /// each removed raw field rejects on its own, so a future regression
    /// that reintroduces exactly one of them fails closed with a
    /// precisely-named test.
    ///
    /// The helper asserts the rejection is specifically
    /// `wire-unknown-field`, not any error, and the per-field test loops
    /// use values matching each field's legacy wire type. Without this,
    /// a future regression that reintroduces a numeric field like
    /// `ownerUid`/`ownerGid`/`mtu` would still pass via serde
    /// type-mismatch on a string value — the gate would see an error and
    /// accept it without proving the wire contract actually refused the
    /// field name.
    fn require_wire_unknown_field_rejection(kind: &str, base: serde_json::Value, unknown: &str) {
        let frame = encode_frame(&serde_json::json!({
            "kind": kind,
            "payload": base,
        }))
        .expect("encodes");
        match decode_frame::<BrokerRequest>("BrokerRequest", &frame) {
            Ok(_) => panic!(
                "{kind} must reject unknown field '{unknown}' (legacy raw authority), but decode succeeded"
            ),
            Err(err) => assert_eq!(
                err.kind().as_str(),
                "wire-unknown-field",
                "{kind} rejected unknown field '{unknown}' but with kind {} (expected wire-unknown-field); message: {}",
                err.kind().as_str(),
                err.message(),
            ),
        }
    }

    /// Legacy authority field with its original wire type. Tightens the
    /// per-field rejection loops so they inject each field with a value
    /// matching its original type (numeric for uid/gid/mtu, bool for
    /// flag fields, string for name/hash fields). Without typed values,
    /// the rejection could pass via serde type-mismatch instead of via
    /// the `deny_unknown_fields` contract.
    fn legacy_value(field: &str) -> serde_json::Value {
        match field {
            "ownerUid" | "ownerGid" | "mtu" => serde_json::json!(1),
            "isolated" | "neighSuppress" => serde_json::json!(true),
            _ => serde_json::json!("legacy"),
        }
    }

    fn opaque_create_tap_payload() -> serde_json::Value {
        serde_json::json!({ "roleId": "runner-lan", "vmId": "corp-vm" })
    }

    fn opaque_set_bridge_port_flags_payload() -> serde_json::Value {
        serde_json::json!({ "vmId": "corp-vm", "roleId": "workload-lan" })
    }

    fn opaque_usbip_firewall_payload() -> serde_json::Value {
        serde_json::json!({ "bundleUsbipFirewallIntentRef": "usbip-fw-1-2" })
    }

    #[test]
    fn create_persistent_tap_rejects_each_legacy_authority_field() {
        for field in [
            "ifnameDerived",
            "bridge",
            "tap",
            "ownerUid",
            "ownerGid",
            "mac",
            "mtu",
        ] {
            let mut payload = opaque_create_tap_payload();
            payload
                .as_object_mut()
                .unwrap()
                .insert(field.to_string(), legacy_value(field));
            require_wire_unknown_field_rejection("CreatePersistentTap", payload, field);
        }
    }

    #[test]
    fn create_tap_fd_rejects_each_legacy_authority_field() {
        for field in [
            "ifnameDerived",
            "bridge",
            "tap",
            "ownerUid",
            "ownerGid",
            "mac",
            "mtu",
        ] {
            let mut payload = opaque_create_tap_payload();
            payload
                .as_object_mut()
                .unwrap()
                .insert(field.to_string(), legacy_value(field));
            require_wire_unknown_field_rejection("CreateTapFd", payload, field);
        }
    }

    #[test]
    fn set_bridge_port_flags_rejects_each_legacy_authority_field() {
        for field in ["bridge", "port", "isolated", "neighSuppress", "rule"] {
            let mut payload = opaque_set_bridge_port_flags_payload();
            payload
                .as_object_mut()
                .unwrap()
                .insert(field.to_string(), legacy_value(field));
            require_wire_unknown_field_rejection("SetBridgePortFlags", payload, field);
        }
    }

    #[test]
    fn usbip_bind_firewall_rule_rejects_each_legacy_authority_field() {
        for field in ["busId", "ruleHash"] {
            let mut payload = opaque_usbip_firewall_payload();
            payload
                .as_object_mut()
                .unwrap()
                .insert(field.to_string(), legacy_value(field));
            require_wire_unknown_field_rejection("UsbipBindFirewallRule", payload, field);
        }
    }

    #[test]
    fn unknown_broker_variant_fails_closed() {
        let frame = encode_frame(&serde_json::json!({
            "kind": "UnknownW4Operation",
            "payload": {}
        }))
        .expect("encodes");
        let error = decode_frame::<BrokerRequest>("BrokerRequest", &frame)
            .expect_err("unknown variant fails closed");
        assert!(
            error.kind().as_str() == "wire-malformed-json"
                || error.kind().as_str() == "wire-version-mismatch",
            "unexpected error kind {}",
            error.kind().as_str()
        );
    }

    #[test]
    fn apply_nftables_request_is_opaque_only() {
        let frame = encode_frame(&serde_json::json!({
            "kind": "ApplyNftables",
            "payload": {
                "bundleNftIntentRef": "nft-corp",
                "scopeId": "scope-corp"
            }
        }))
        .expect("encodes");
        let decoded = decode_frame::<BrokerRequest>("BrokerRequest", &frame).expect("decodes");
        match decoded {
            BrokerRequest::ApplyNftables(req) => {
                assert_eq!(req.bundle_nft_intent_ref.as_str(), "nft-corp");
                assert_eq!(req.scope_id.as_str(), "scope-corp");
            }
            other => panic!("expected ApplyNftables, got {other:?}"),
        }
    }

    #[test]
    fn launch_minijail_child_carries_only_role_and_vm() {
        let frame = encode_frame(&serde_json::json!({
            "kind": "LaunchMinijailChild",
            "payload": {
                "vmId": "corp-vm",
                "roleId": "runner"
            }
        }))
        .expect("encodes");
        let decoded = decode_frame::<BrokerRequest>("BrokerRequest", &frame).expect("decodes");
        match decoded {
            BrokerRequest::LaunchMinijailChild(req) => {
                assert_eq!(req.vm_id.as_str(), "corp-vm");
                assert_eq!(req.role_id.as_str(), "runner");
            }
            other => panic!("expected LaunchMinijailChild, got {other:?}"),
        }
    }

    #[test]
    fn launch_minijail_child_rejects_inline_authority_fields() {
        // Legacy argv, env, uid, gid, caps, and seccomp_profile fields
        // are forbidden — deny_unknown_fields traps them.
        let frame = encode_frame(&serde_json::json!({
            "kind": "LaunchMinijailChild",
            "payload": {
                "vmId": "corp-vm",
                "roleId": "runner",
                "argv": ["/bin/sh"]
            }
        }))
        .expect("encodes");
        let error = decode_frame::<BrokerRequest>("BrokerRequest", &frame)
            .expect_err("argv field must be refused");
        assert!(matches!(
            error.kind().as_str(),
            "wire-unknown-field" | "wire-malformed-json"
        ));
    }

    /// Regression guard: this test was reframed when `ifname_derived`
    /// was removed from `CreateTapFdRequest`. The payload-side
    /// validation it used to assert is now the broker's responsibility
    /// (it derives the ifname from the trusted bundle row keyed by
    /// `role_id` + `vm_id`). What we still want to guarantee here is
    /// that a frame carrying the dropped `ifnameDerived` field is
    /// fail-closed-rejected by the wire layer with `wire-unknown-field`,
    /// preventing a future caller from supplying it.
    #[test]
    fn create_tap_fd_rejects_invalid_ifname() {
        let frame = encode_frame(&serde_json::json!({
            "kind": "CreateTapFd",
            "payload": {
                "ifnameDerived": "bad.name",
                "roleId": "runner",
                "vmId": "corp-vm"
            }
        }))
        .expect("encodes");
        let error = decode_frame::<BrokerRequest>("BrokerRequest", &frame)
            .expect_err("dropped ifnameDerived field must be refused");
        assert_eq!(
            error.kind().as_str(),
            "wire-unknown-field",
            "expected unknown-field rejection; got message: {}",
            error.message()
        );
    }

    /// SpawnRunner carries only opaque IDs (vm_id, role_id,
    /// bundle_runner_intent_ref). The broker resolves the full launch
    /// context (argv inputs, uid/gid, caps, seccomp, cgroup) from the
    /// trusted bundle row anchored by the opaque reference; the daemon
    /// never names argv, env, uid, gid, caps, kernel/initrd paths,
    /// virtiofs sockets, or seccomp profiles on the wire.
    #[test]
    fn spawn_runner_request_is_opaque_only() {
        let frame = encode_frame(&serde_json::json!({
            "kind": "SpawnRunner",
            "payload": {
                "vmId": "corp-vm",
                "roleId": "ch",
                "role": "cloud-hypervisor",
                "bundleRunnerIntentRef": "ch-corp-vm",
                "runtimeAllocations": [
                    { "kind": "vsock-cid", "opaqueRef": "alloc-vsock-1" },
                    { "kind": "api-socket-path", "opaqueRef": "alloc-api-1" }
                ]
            }
        }))
        .expect("encodes");
        let decoded = decode_frame::<BrokerRequest>("BrokerRequest", &frame).expect("decodes");
        match decoded {
            BrokerRequest::SpawnRunner(req) => {
                assert_eq!(req.vm_id.as_str(), "corp-vm");
                assert_eq!(req.role_id.as_str(), "ch");
                assert_eq!(req.role, RunnerRole::CloudHypervisor);
                assert_eq!(req.bundle_runner_intent_ref.as_str(), "ch-corp-vm");
                assert_eq!(req.runtime_allocations.len(), 2);
                assert_eq!(
                    req.runtime_allocations[0].kind,
                    RunnerAllocationKind::VsockCid
                );
                assert_eq!(req.runtime_allocations[0].opaque_ref, "alloc-vsock-1");
                assert_eq!(
                    req.runtime_allocations[1].kind,
                    RunnerAllocationKind::ApiSocketPath
                );
            }
            other => panic!("expected SpawnRunner, got {other:?}"),
        }
    }

    #[test]
    fn spawn_runner_rejects_each_legacy_authority_field() {
        // argv, env, uid, gid, caps, seccomp_profile,
        // kernel/initrd/cmdline, and api_socket_mode are ALL
        // bundle-derived. Wire frames containing them must fail-closed
        // with wire-unknown-field.
        let base = serde_json::json!({
            "vmId": "corp-vm",
            "roleId": "ch",
            "role": "cloud-hypervisor",
            "bundleRunnerIntentRef": "ch-corp-vm"
        });
        for field in [
            "argv",
            "env",
            "uid",
            "gid",
            "caps",
            "seccompProfile",
            "kernelPath",
            "initrdPath",
            "cmdline",
            "apiSocketMode",
            "chBinaryPath",
            "vsockCid",
        ] {
            let mut payload = base.clone();
            payload
                .as_object_mut()
                .unwrap()
                .insert(field.to_string(), legacy_value(field));
            require_wire_unknown_field_rejection("SpawnRunner", payload, field);
        }
    }

    #[test]
    fn spawn_runner_runtime_allocation_unknown_kind_rejected() {
        // The bundle-derived allocation slots are a closed set
        // (vsock-cid, tap-fd-slot, api-socket-path). A wire frame
        // claiming a new kind must fail-closed; future kinds require
        // a wire bump rather than caller-supplied authority.
        let frame = encode_frame(&serde_json::json!({
            "kind": "SpawnRunner",
            "payload": {
                "vmId": "corp-vm",
                "roleId": "ch",
                "role": "cloud-hypervisor",
                "bundleRunnerIntentRef": "ch-corp-vm",
                "runtimeAllocations": [
                    { "kind": "kvm-fd", "opaqueRef": "should-not-cross" }
                ]
            }
        }))
        .expect("encodes");
        assert!(decode_frame::<BrokerRequest>("BrokerRequest", &frame).is_err());
    }

    #[test]
    fn spawn_runner_role_kebab_case_serialization() {
        // Each RunnerRole serializes as the documented kebab-case
        // token so wire compatibility is stable across daemon /
        // broker upgrades.
        let pairs = [
            (RunnerRole::CloudHypervisor, "\"cloud-hypervisor\""),
            (RunnerRole::Virtiofsd, "\"virtiofsd\""),
            (RunnerRole::Swtpm, "\"swtpm\""),
            (RunnerRole::SwtpmFlush, "\"swtpm-flush\""),
            (RunnerRole::Gpu, "\"gpu\""),
            (RunnerRole::Audio, "\"audio\""),
            (RunnerRole::Video, "\"video\""),
            (RunnerRole::VsockRelay, "\"vsock-relay\""),
            (RunnerRole::Usbip, "\"usbip\""),
            (RunnerRole::OtelHostBridge, "\"otel-host-bridge\""),
            (RunnerRole::WaylandProxy, "\"wayland-proxy\""),
            (RunnerRole::ConsoleDrain, "\"console-drain\""),
        ];
        for (role, expected) in pairs {
            assert_eq!(serde_json::to_string(&role).unwrap(), expected);
            assert_eq!(role.as_str(), expected.trim_matches('"'));
        }
    }

    #[test]
    fn signal_runner_request_round_trips() {
        let frame = encode_frame(&serde_json::json!({
            "kind": "SignalRunner",
            "payload": {
                "vmId": "corp-vm",
                "roleId": "ch-runner",
                "signal": "term"
            }
        }))
        .expect("encodes");
        let decoded = decode_frame::<BrokerRequest>("BrokerRequest", &frame).expect("decodes");
        match decoded {
            BrokerRequest::SignalRunner(req) => {
                assert_eq!(req.vm_id.as_str(), "corp-vm");
                assert_eq!(req.role_id.as_str(), "ch-runner");
                assert_eq!(req.signal, RunnerSignal::Term);
            }
            other => panic!("expected SignalRunner, got {other:?}"),
        }
    }

    #[test]
    fn signal_runner_response_round_trips() {
        let response = BrokerResponse::SignalRunner(SignalRunnerResponse {
            signaled: false,
            vm_id: VmId::new("corp-vm"),
            role_id: RoleId::new("ch-runner"),
        });
        let frame = encode_frame(&response).expect("encodes");
        let decoded = decode_frame::<BrokerResponse>("BrokerResponse", &frame).expect("decodes");
        match decoded {
            BrokerResponse::SignalRunner(payload) => {
                assert!(!payload.signaled);
                assert_eq!(payload.vm_id.as_str(), "corp-vm");
                assert_eq!(payload.role_id.as_str(), "ch-runner");
            }
            other => panic!("expected BrokerResponse::SignalRunner, got {other:?}"),
        }
    }

    #[test]
    fn spawn_runner_response_round_trips() {
        // The pidfd is delivered out-of-band over SCM_RIGHTS; the
        // JSON body carries (pid, start_time_ticks, pidfd_index) so
        // the daemon's pidfd table can validate / reconcile the handle.
        let response = BrokerResponse::SpawnRunner(SpawnRunnerResponse {
            vm_id: VmId::new("corp-vm"),
            role_id: RoleId::new("ch"),
            role: RunnerRole::CloudHypervisor,
            pid: 4242,
            start_time_ticks: 987_654_321,
            pidfd_index: 0,
        });
        let frame = encode_frame(&response).expect("encodes");
        let decoded = decode_frame::<BrokerResponse>("BrokerResponse", &frame).expect("decodes");
        match decoded {
            BrokerResponse::SpawnRunner(payload) => {
                assert_eq!(payload.vm_id.as_str(), "corp-vm");
                assert_eq!(payload.role, RunnerRole::CloudHypervisor);
                assert_eq!(payload.pid, 4242);
                assert_eq!(payload.start_time_ticks, 987_654_321);
                assert_eq!(payload.pidfd_index, 0);
            }
            other => panic!("expected BrokerResponse::SpawnRunner, got {other:?}"),
        }
    }
}
