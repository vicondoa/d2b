//! Broker wire contract.
//!
//! Every mutating variant carries **only opaque identifiers** +
//! bundle-resolved intent refs. The daemon never names a raw path, a
//! raw nft rule text, a raw route spec, a raw sysctl key/value, a raw
//! ifname set, a raw `/etc/hosts` entry list, a raw uid/gid, raw argv
//! or env, raw caps, or a raw seccomp profile path. The broker uses
//! the opaque IDs to look up the typed intent in its own trusted bundle
//! copy. See `d2b_contracts::types` for the newtype set.

use d2b_contracts::audit_wire::validate_audit_page;
pub use d2b_contracts::audit_wire::{AuditExportCursor, AuditExportEntry, AuditExportErrorCode};
pub use d2b_contracts::store_verify_wire::{
    StoreVerifyRequest, StoreVerifyResponse, StoreVerifyStatus, StoreVerifyUnknownReason,
};
use d2b_contracts::types::{
    BundleClosureRef, BundleOpId, MediaRef, PathClass, RoleId, ScopeId, SubjectId, TracingSpanId,
    VmId,
};
use d2b_contracts::workload_identity::WorkloadIdentity;
use d2b_contracts_resource::v3::process::{
    CapabilityClass, EnvironmentClass, NamespaceClass, UserNamespaceSpec,
};
use d2b_contracts_resource::v3::{
    ActivationRunnerInput, ArtifactId, IfName, ResourceBundleGenerationId, ResourceGeneration,
    ResourceRef, ResourceUid,
    execution_policy::ExecutionDomain, storage::ZoneStoreId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "payload")]
pub enum BrokerRequest {
    /// Authenticate and apply one source-to-target NixOS generation
    /// handoff. The broker resolves all host effects from its trusted
    /// installed-generation state; no path or command crosses the wire.
    ApplyHostGenerationHandoff(crate::host_generation::ApplyHostGenerationHandoff),
    /// Launch one operation-scoped cutover runner before control-plane drain.
    ///
    /// The request must carry exactly one SCM_RIGHTS bootstrap descriptor.
    /// The broker resolves the runner executable from its trusted server
    /// configuration and never accepts a path or command over the wire.
    LaunchCutoverRunner(LaunchCutoverRunnerRequest),
    /// Append one durable, operation-scoped cutover audit record.
    CutoverAudit(CutoverAuditRequest),
    /// Dispatch one closed operation-scoped cutover or reset effect.
    CutoverEffect(CutoverEffectRequest),
    ApplyNftables(ApplyNftablesRequest),
    /// Apply or remove one Provider-owned nftables projection.
    ///
    /// Distinct from [`BrokerRequest::ApplyNftables`], which owns the
    /// framework's own `inet d2b` table: this op carries a projection a
    /// Provider owns, and its action is a closed enum rather than a
    /// boolean, so a caller cannot express a third meaning.
    ///
    /// The live handler refuses a request whose fence differs from the
    /// installed generation, mutating nothing and requeueing as stale.
    ApplyNftablesProjection(ApplyNftablesProjectionRequest),
    ApplyNmUnmanaged(ApplyNmUnmanagedRequest),
    ApplyRoute(ApplyRouteRequest),
    ApplySysctl(ApplySysctlRequest),
    BindUnixSocket(BindUnixSocketRequest),
    CreateOrReconcileUsersGroups(CreateOrReconcileUsersGroupsRequest),
    /// Create the bridge an environment's links attach to. The daemon
    /// names only the opaque bundle intent ref and scope; the broker
    /// derives the bridge ifname and its attributes from its own trusted
    /// bundle copy.
    ///
    /// The live handler suppresses IPv6 on the link before bringing it up.
    CreateBridge(CreateBridgeRequest),
    /// Delete a bridge this framework created. Follows the same
    /// opaque-identifier contract as [`BrokerRequest::CreateBridge`].
    ///
    /// The live handler removes the bridge only after its TAP removals are
    /// confirmed.
    DeleteBridge(DeleteBridgeRequest),
    CreatePersistentTap(CreatePersistentTapRequest),
    /// Delete a persistent TAP this framework created. Follows the same
    /// opaque-identifier contract as
    /// [`BrokerRequest::CreatePersistentTap`].
    ///
    /// The live handler requires both generation fences to match and the
    /// VMM descriptor to be closed before removing the TAP.
    DeletePersistentTap(DeletePersistentTapRequest),
    CreateTapFd(CreateTapFdRequest),
    DelegateCgroupV2(DelegateCgroupV2Request),
    ExportBrokerAudit(ExportBrokerAuditRequest),
    /// Daemon ↔ broker handshake request. The daemon sends its
    /// client_version and supported_features; the broker replies with
    /// [`HelloResponse`] containing the selected wire version. Mirrors
    /// the bootstrap `Hello` shape so the connection layer doesn't need
    /// a side-channel.
    Hello(HelloRequest),
    InjectSecretById(SecretByIdRequest),
    LaunchMinijailChild(LaunchMinijailChildRequest),
    ModprobeIfAllowed(ModprobeIfAllowedRequest),
    OpenCgroupDir(OpenCgroupDirRequest),
    OpenDevice(OpenDeviceRequest),
    OpenFuse(OpenFuseRequest),
    /// Resolve a configured FIDO security-key stable selector from the
    /// trusted bundle, open the physical hidraw node as root, and
    /// return the fd to `d2bd` via `SCM_RIGHTS`. `d2bd` manages the
    /// long-lived CTAPHID relay session; the broker only opens the
    /// device. The daemon names only `vm_id` and an opaque
    /// `selector_id` (resolved from the trusted bundle); the broker
    /// never names raw device paths on the wire.
    OpenHidrawSecurityKey(OpenHidrawSecurityKeyRequest),
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
    /// Obtain a pidfd for the peer of exactly one accepted Unix socket.
    ///
    /// The accepted socket is the sole SCM_RIGHTS request attachment. The
    /// request body deliberately contains no descriptor number, PID,
    /// credential tuple, path, or subject claim.
    OpenPeerPidfdFromAcceptedSocket(OpenPeerPidfdFromAcceptedSocketRequest),
    /// Observe one broker-owned runner after validating its retained
    /// pidfd-backed identity against the trusted bundle.
    ObserveRunner(ObserveRunnerRequest),
    /// Apply one bounded host-side PipeWire effect for a trusted audio
    /// runner. The broker resolves all executable and runtime details from
    /// the signed runner intent; the daemon supplies only opaque identities
    /// and a closed action.
    PipeWireAudio(PipeWireAudioRequest),
    /// Start one trusted non-forking transient systemd unit. The broker
    /// resolves executable, argv, uid/gid, environment, and cgroup
    /// placement from the bundle runner intent.
    StartSystemdUnit(StartSystemdUnitRequest),
    /// Check whether the exact user manager selected by the trusted runner
    /// intent is reachable. The manager connection never crosses the broker
    /// boundary.
    CheckSystemdUserManager(CheckSystemdUserManagerRequest),
    /// Observe one trusted transient systemd unit without opening a pidfd.
    ObserveSystemdUnit(ObserveSystemdUnitRequest),
    /// Re-open a pidfd after re-verifying a trusted transient unit identity.
    OpenSystemdUnitPidfd(OpenSystemdUnitPidfdRequest),
    /// Stop one exact transient systemd unit identity.
    StopSystemdUnit(StopSystemdUnitRequest),
    /// Resolve one signed Zone storage-row id against the trusted bundle,
    /// provision or open its database inode, and return the owned database
    /// descriptor via `SCM_RIGHTS`. No path, mode, owner, or marker value
    /// crosses this boundary.
    OpenZoneStore(OpenZoneStoreRequest),
    OpenVhostNet(OpenVhostNetRequest),
    PauseBroker,
    /// Drain the broker's in-memory ring buffer of ChildReaped events.
    /// Returns [`PollChildReapedResponse`] containing all buffered
    /// notifications in FIFO order; clears the buffer. Idempotent.
    PollChildReaped,
    PrepareRuntimeDir(PrepareDirRequest),
    PrepareStateDir(PrepareDirRequest),
    /// Adopt a known legacy swtpm state through the broker-owned journal.
    MigrateLegacySwtpmState(MigrateLegacySwtpmStateRequest),
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
    /// Kill exactly one trusted runner cgroup leaf during intentional
    /// teardown. The broker resolves the leaf from its trusted runner
    /// intent; callers never provide a cgroup path.
    CgroupKill(CgroupKillRequest),
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
    /// Currently a typed stub (`Unimplemented`) - the live handler wires the
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
    /// Record the broker durability evidence that closes an authenticated
    /// resource-bundle activation commit. The request carries only the
    /// canonical audit join; the broker does not accept resource rows or
    /// caller-selected host effects over this operation.
    ResourceActivationAudit(ResourceActivationAuditRequest),
    ValidateBundle,
    /// Write the per-VM dnsmasq lease file. Replaces leaves of the
    /// retired `microvm-setup@<vm>.service`. Currently a typed stub
    /// (`Unimplemented`) until the live handler is wired.
    ///
    /// Live handler target: `live_seed_dnsmasq_lease`, resolved through
    /// `BundleResolver` from the per-VM dnsmasq lease row.
    SeedDnsmasqLease(SeedDnsmasqLeaseRequest),
    /// Bind-mount `/var/lib/d2b/vms/<vm>/store-view` from the
    /// per-VM hardlink farm at `<vm>/store/`. Currently a typed stub
    /// (`Unimplemented`) until the live handler is wired.
    ///
    /// Live handler target: `live_bind_mount_from_hardlink_farm`, resolved
    /// through `BundleResolver::find_store_view_intent`.
    BindMountFromHardlinkFarm(BindMountFromHardlinkFarmRequest),
    /// Enforce the per-leaf ownership/mode matrix on
    /// `/var/lib/d2b/vms/<vm>/`. Currently a typed stub
    /// (`Unimplemented`) until the real check is wired.
    ///
    /// Live handler target: `d2b_host::ownership_matrix::check`.
    OwnershipMatrixCheck(OwnershipMatrixCheckRequest),
    /// Refuse VM start if `/var/lib/d2b/vms/<vm>/sshd-host-keys/`
    /// drifts from `root:root 0400`. Currently a typed stub
    /// (`Unimplemented`) until the real check is wired.
    ///
    /// Live handler target: the `O_NOFOLLOW` symlink-rejecting check.
    SshHostKeyPreflight(SshHostKeyPreflightRequest),
    /// Broker-provisioned disk-image creation.
    ///
    /// The daemon dispatches this before `SpawnRunner` for any runner
    /// whose bundle ProcessNode has `DiskInit` plan-ops (currently CH
    /// when `writableStoreOverlay` is enabled). The broker resolves
    /// the target path, size, mode, and ownership from the trusted
    /// bundle - the daemon names only the opaque `vm_id`.
    DiskInit(DiskInitRequest),
    /// Open the FIDO/CTAP hidraw node for the named device selector.
    ///
    /// The broker resolves the stable device label against the trusted
    /// bundle security-key device table, performs sysfs-presence and
    /// FIDO-class checks, opens the exact hidraw node, and returns
    /// the fd via `SCM_RIGHTS`. The daemon holds the fd for the CTAP
    /// relay session lifetime.
    ///
    /// Typed stub - live handler target: `live_security_key_open_device`.
    SecurityKeyOpenDevice(d2b_contracts::security_key::SecurityKeyOpenDeviceRequest),
    /// Apply udev group grants for configured FIDO hidraw nodes.
    ///
    /// Writes broker-generated udev rules granting the
    /// `d2b-security-key` group ownership of the configured
    /// vendor/product/serial-matched hidraw nodes. Called once during
    /// host activation or when the device selector list changes.
    ///
    /// Typed stub - live handler target: `live_security_key_apply_udev_rules`.
    SecurityKeyApplyUdevRules(d2b_contracts::security_key::SecurityKeyApplyUdevRulesRequest),
}

/// Path-free result of a source-to-target generation handoff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyHostGenerationHandoffResponse {
    pub target: d2b_contracts_resource::v3::ResourceRef,
    pub state: crate::host_generation::HandoffState,
    pub source_generation: u64,
    pub target_generation: u64,
    pub source_remains_usable: bool,
    pub summary: String,
}

/// Request to launch the one-shot cutover runner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LaunchCutoverRunnerRequest {
    /// Opaque operation identity used to derive runner-owned state.
    pub operation_id: BundleOpId,
    /// Required index of the single bootstrap fd attachment.
    pub bootstrap_fd_index: u32,
    /// Digest of the capability transferred over the bootstrap fd. The broker
    /// binds this value to the operation before spawning the runner.
    pub capability_digest: CanonicalAuditDigest,
    /// Capability expiry copied from the single-use bootstrap.
    pub expires_at_ms: u64,
}

/// Response from the cutover runner launch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LaunchCutoverRunnerResponse {
    /// Opaque operation identity.
    pub operation_id: BundleOpId,
    /// Child pid observed at launch.
    pub pid: i32,
    /// `/proc/<pid>/stat` start-time captured by the broker.
    pub start_time_ticks: u64,
    /// Index of the returned pidfd, when the broker supplies one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pidfd_index: Option<u32>,
}

/// Closed transition vocabulary for cutover audit publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CutoverAuditTransition {
    /// A hold request was durably recorded.
    HoldRequested,
    /// A hold clear/resume was durably recorded.
    HoldCleared,
    /// A phase began after its journal record.
    PhaseStarted,
    /// A phase completed after its typed effect.
    PhaseCompleted,
    /// A typed effect began.
    EffectStarted,
    /// A typed effect completed.
    EffectCompleted,
    /// A terminal outcome was recorded.
    Terminal,
}

impl CutoverAuditTransition {
    /// Return the stable audit disposition label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HoldRequested => "hold-requested",
            Self::HoldCleared => "hold-cleared",
            Self::PhaseStarted => "phase-started",
            Self::PhaseCompleted => "phase-completed",
            Self::EffectStarted => "effect-started",
            Self::EffectCompleted => "effect-completed",
            Self::Terminal => "terminal",
        }
    }
}

/// Request to publish one durable cutover audit transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CutoverAuditRequest {
    /// Operation identity bound by the runner capability.
    pub operation_id: BundleOpId,
    /// Current U3 phase number.
    pub phase: u8,
    /// Closed transition kind.
    pub transition: CutoverAuditTransition,
    /// Digest of the immutable operation request.
    pub request_digest: CanonicalAuditDigest,
    /// Digest of a bounded hold reason, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_digest: Option<CanonicalAuditDigest>,
}

/// Durable audit publication response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CutoverAuditResponse {
    /// Stable record identity returned only after fsync and directory sync.
    pub record_id: CanonicalAuditDigest,
}

/// Closed authority carried by a cutover effect request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CutoverEffectAuthority {
    /// Host-wide cutover authority.
    Cutover,
    /// Zone-scoped reset authority.
    ResetZone,
    /// Provider-scoped reset authority.
    ResetProvider,
    /// Guest-scoped reset authority.
    ResetGuest,
}

/// Closed effect vocabulary shared with the U3 allowlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CutoverEffectKind {
    HostDrain,
    CutoverDisposition,
    ResourceStoreCreate,
    ProviderInstall,
    ZoneActivation,
    GuestActivation,
    Verification,
    CutoverFinalization,
    ScopedZoneReset,
    ScopedProviderReset,
    ScopedGuestReset,
    DestroyDurableVolume,
    PreserveSource,
    QuarantineDestination,
    CutoverBroker,
    ClosureActivation,
    ApplyAdmission,
}

/// Typed payloads for cutover effects that reuse existing broker operations.
///
/// Every variant is itself a closed broker request. It carries no host path,
/// command, uid/gid, or free-form mutation text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "payload")]
pub enum CutoverEffectPayload {
    None,
    ApplyAdmission(CutoverAdmissionRequest),
    Storage(ReconcileStorageScopeRequest),
    ZoneStore(OpenZoneStoreRequest),
    StoreSync(StoreSyncRequest),
    StoreVerify(StoreVerifyRequest),
    Verification(CutoverVerificationRequest),
    Activation(RunActivationRequest),
    Systemd(Box<SystemdUnitRequest>),
    Quarantine {
        staged_id: BundleOpId,
        source_id: BundleOpId,
        marker_digest: CanonicalAuditDigest,
    },
    Finalization {
        artifacts: Vec<ArtifactId>,
        disposition_digest: CanonicalAuditDigest,
        consent_digest: CanonicalAuditDigest,
    },
    DestroyDurableVolume {
        storage_ref: BundleOpId,
        marker_digest: CanonicalAuditDigest,
        consent_digest: CanonicalAuditDigest,
    },
}

/// Broker-owned phase-9 verification admission. The supplied Zone identities
/// are expected evidence bindings only; the broker returns the live
/// observations from its trusted bundle and host checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CutoverVerificationRequest {
    pub expected_zone_ids: Vec<BundleOpId>,
}

/// One broker-owned Zone verification observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CutoverZoneVerification {
    pub zone_id: BundleOpId,
    pub healthy: bool,
}

/// Broker-owned phase-9 verification observations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CutoverVerificationResponse {
    pub zones: Vec<CutoverZoneVerification>,
    pub sources_preserved: bool,
    pub identity_digests_match: bool,
    pub candidate_current: bool,
}

/// Broker-owned apply admission observations. These fields are never
/// caller-authored; the broker derives them from its trusted bundle and live
/// host ownership state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CutoverAdmissionResponse {
    pub candidate_current: bool,
    pub markers_valid: bool,
    pub ownership_valid: bool,
    pub predicates_hold: bool,
}

/// Typed request for the broker-owned apply admission observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CutoverAdmissionRequest {
    /// Candidate artifact resolved and verified before host drain.
    pub system_artifact_id: Option<ArtifactId>,
    /// Preserved source artifact resolved and verified before host drain.
    pub source_system_artifact_id: Option<ArtifactId>,
}

/// Closed replay behavior for an operation-scoped effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CutoverReplayClass {
    Repeatable,
    ReopenByJournaledIdentity,
    QuarantineOnly,
}

impl CutoverEffectAuthority {
    /// Return whether this authority permits the effect kind.
    pub const fn permits(self, effect: CutoverEffectKind) -> bool {
        match self {
            Self::Cutover => !matches!(
                effect,
                CutoverEffectKind::ScopedZoneReset
                    | CutoverEffectKind::ScopedProviderReset
                    | CutoverEffectKind::ScopedGuestReset
                    | CutoverEffectKind::DestroyDurableVolume
            ),
            Self::ResetZone => matches!(
                effect,
                CutoverEffectKind::ApplyAdmission
                    | CutoverEffectKind::ScopedZoneReset
                    | CutoverEffectKind::DestroyDurableVolume
                    | CutoverEffectKind::PreserveSource
                    | CutoverEffectKind::QuarantineDestination
                    | CutoverEffectKind::Verification
            ),
            Self::ResetProvider => matches!(
                effect,
                CutoverEffectKind::ApplyAdmission
                    | CutoverEffectKind::ScopedProviderReset
                    | CutoverEffectKind::DestroyDurableVolume
                    | CutoverEffectKind::PreserveSource
                    | CutoverEffectKind::QuarantineDestination
                    | CutoverEffectKind::Verification
            ),
            Self::ResetGuest => matches!(
                effect,
                CutoverEffectKind::ApplyAdmission
                    | CutoverEffectKind::ScopedGuestReset
                    | CutoverEffectKind::DestroyDurableVolume
                    | CutoverEffectKind::PreserveSource
                    | CutoverEffectKind::QuarantineDestination
                    | CutoverEffectKind::Verification
            ),
        }
    }
}

/// Typed operation-scoped effect request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CutoverEffectRequest {
    pub operation_id: BundleOpId,
    pub authority: CutoverEffectAuthority,
    pub phase: u8,
    pub effect_id: BundleOpId,
    pub effect: CutoverEffectKind,
    pub replay_class: CutoverReplayClass,
    pub request_digest: CanonicalAuditDigest,
    pub capability_digest: CanonicalAuditDigest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<BundleOpId>,
    /// Existing typed generation handoff used only by `ClosureActivation`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff: Option<crate::host_generation::ApplyHostGenerationHandoff>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<CutoverEffectPayload>,
}

/// Typed effect result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CutoverEffectOutcome {
    Succeeded,
    Failed,
    Ambiguous,
}

/// Typed operation-scoped effect response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CutoverEffectResponse {
    pub outcome: CutoverEffectOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<BundleOpId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<CutoverVerificationResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admission: Option<CutoverAdmissionResponse>,
    pub audit_record_id: CanonicalAuditDigest,
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
            Self::ApplyHostGenerationHandoff(_) => "ApplyHostGenerationHandoff",
            Self::LaunchCutoverRunner(_) => "LaunchCutoverRunner",
            Self::CutoverAudit(_) => "CutoverAudit",
            Self::CutoverEffect(_) => "CutoverEffect",
            Self::ApplyNftables(_) => "ApplyNftables",
            Self::ApplyNftablesProjection(_) => "ApplyNftablesProjection",
            Self::ApplyNmUnmanaged(_) => "ApplyNmUnmanaged",
            Self::ApplyRoute(_) => "ApplyRoute",
            Self::ApplySysctl(_) => "ApplySysctl",
            Self::BindUnixSocket(_) => "BindUnixSocket",
            Self::CreateOrReconcileUsersGroups(_) => "CreateOrReconcileUsersGroups",
            Self::CreateBridge(_) => "CreateBridge",
            Self::DeleteBridge(_) => "DeleteBridge",
            Self::CreatePersistentTap(_) => "CreatePersistentTap",
            Self::DeletePersistentTap(_) => "DeletePersistentTap",
            Self::CreateTapFd(_) => "CreateTapFd",
            Self::DelegateCgroupV2(_) => "DelegateCgroupV2",
            Self::ExportBrokerAudit(_) => "ExportBrokerAudit",
            Self::Hello(_) => "Hello",
            Self::InjectSecretById(_) => "InjectSecretById",
            Self::LaunchMinijailChild(_) => "LaunchMinijailChild",
            Self::ModprobeIfAllowed(_) => "ModprobeIfAllowed",
            Self::OpenCgroupDir(_) => "OpenCgroupDir",
            Self::OpenDevice(_) => "OpenDevice",
            Self::OpenFuse(_) => "OpenFuse",
            Self::OpenHidrawSecurityKey(_) => "OpenHidrawSecurityKey",
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
            Self::OpenPeerPidfdFromAcceptedSocket(_) => "OpenPeerPidfdFromAcceptedSocket",
            Self::ObserveRunner(_) => "ObserveRunner",
            Self::PipeWireAudio(_) => "PipeWireAudio",
            Self::StartSystemdUnit(_) => "StartSystemdUnit",
            Self::CheckSystemdUserManager(_) => "CheckSystemdUserManager",
            Self::ObserveSystemdUnit(_) => "ObserveSystemdUnit",
            Self::OpenSystemdUnitPidfd(_) => "OpenSystemdUnitPidfd",
            Self::StopSystemdUnit(_) => "StopSystemdUnit",
            Self::OpenZoneStore(_) => "OpenZoneStore",
            Self::OpenVhostNet(_) => "OpenVhostNet",
            Self::PauseBroker => "PauseBroker",
            Self::PollChildReaped => "PollChildReaped",
            Self::PrepareRuntimeDir(_) => "PrepareRuntimeDir",
            Self::PrepareStateDir(_) => "PrepareStateDir",
            Self::MigrateLegacySwtpmState(_) => "MigrateLegacySwtpmState",
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
            Self::CgroupKill(_) => "CgroupKill",
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
            Self::ResourceActivationAudit(_) => "ResourceActivationAudit",
            Self::ValidateBundle => "ValidateBundle",
            Self::SeedDnsmasqLease(_) => "SeedDnsmasqLease",
            Self::BindMountFromHardlinkFarm(_) => "BindMountFromHardlinkFarm",
            Self::OwnershipMatrixCheck(_) => "OwnershipMatrixCheck",
            Self::SshHostKeyPreflight(_) => "SshHostKeyPreflight",
            Self::DiskInit(_) => "DiskInit",
            Self::SecurityKeyOpenDevice(_) => "SecurityKeyOpenDevice",
            Self::SecurityKeyApplyUdevRules(_) => "SecurityKeyApplyUdevRules",
        }
    }

    /// Return whether this request is admitted by a fixed broker profile.
    ///
    /// The profile is selected by the broker process at startup. It is not
    /// carried on the wire, so a request cannot switch or widen the active
    /// authority domain.
    pub fn allowed_by_profile(&self, profile: BrokerProfile) -> bool {
        profile.allows_request(self)
    }

    /// Stable category label for the audit's "opaque_target_id"
    /// column. Mirrors `BootstrapCall::opaque_target_id` semantics:
    /// classify the kind of target without leaking caller-supplied
    /// path names. Default is "operation"; the read-only ops have
    /// their own stable labels.
    pub fn opaque_target_id(&self) -> &'static str {
        match self {
            Self::Hello(_) => "daemon-handshake",
            Self::ValidateBundle => "bundle",
            Self::ResourceActivationAudit(_) => "resource-activation-audit",
            Self::ExportBrokerAudit(_) => "audit-log",
            Self::PollChildReaped => "pidfd-reap-buffer",
            Self::OpenZoneStore(_) => "zone-store",
            Self::OpenPeerPidfdFromAcceptedSocket(_) => "accepted-socket",
            _ => "operation",
        }
    }

    /// Return the canonical join identities for a typed durability request.
    ///
    /// The material is assembled from authoritative typed fields only. It
    /// deliberately does not use the display category, opaque target label,
    /// or a serialization of the whole request.
    pub fn authoritative_audit_join(&self) -> Option<(String, String)> {
        let (scope, operation) = match self {
            Self::ApplyNftables(request) => (
                request.scope_id.to_string(),
                format!(
                    "{}:{}:{}",
                    self.op_name(),
                    request.bundle_nft_intent_ref,
                    request.destroy
                ),
            ),
            Self::ApplyNftablesProjection(request) => (
                request.scope_id.to_string(),
                format!(
                    "{}:{}:{:?}:{}",
                    self.op_name(),
                    request.bundle_nft_projection_intent_ref,
                    request.action,
                    request.expected_generation_id.as_str()
                ),
            ),
            Self::ApplyNmUnmanaged(request) => (
                request.scope_id.to_string(),
                format!(
                    "{}:{}:{}",
                    self.op_name(),
                    request.bundle_nm_intent_ref,
                    request.destroy
                ),
            ),
            Self::ApplyRoute(request) => (
                request.scope_id.to_string(),
                format!(
                    "{}:{}:{}",
                    self.op_name(),
                    request.bundle_route_intent_ref,
                    request.destroy
                ),
            ),
            Self::ApplySysctl(request) => (
                request.scope_id.to_string(),
                format!(
                    "{}:{}:{}",
                    self.op_name(),
                    request.bundle_sysctl_intent_ref,
                    request.destroy
                ),
            ),
            Self::BindUnixSocket(request) => (
                request.vm_id.to_string(),
                format!("{}:{}:{}", self.op_name(), request.vm_id, request.role_id),
            ),
            Self::CreateOrReconcileUsersGroups(request) => (
                request
                    .subject_ids
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
                format!("{}:{}", self.op_name(), request.subject_ids.len()),
            ),
            Self::CreateBridge(request) => (
                request.scope_id.to_string(),
                format!("{}:{}", self.op_name(), request.bundle_bridge_intent_ref),
            ),
            Self::DeleteBridge(request) => (
                request.scope_id.to_string(),
                format!("{}:{}", self.op_name(), request.bundle_bridge_intent_ref),
            ),
            Self::CreatePersistentTap(request) => (
                request.vm_id.to_string(),
                format!("{}:{}:{}", self.op_name(), request.vm_id, request.role_id),
            ),
            Self::CreateTapFd(request) => (
                request.vm_id.to_string(),
                format!("{}:{}:{}", self.op_name(), request.vm_id, request.role_id),
            ),
            Self::DeletePersistentTap(request) => (
                request.attachment_id.to_string(),
                format!(
                    "{}:{}:{}:{}",
                    self.op_name(),
                    request.attachment_id,
                    request.expected_network_generation.get(),
                    request.expected_attachment_generation.get()
                ),
            ),
            Self::DelegateCgroupV2(request) => (
                request.scope_id.to_string(),
                format!("{}:{}", self.op_name(), request.scope_id),
            ),
            Self::InjectSecretById(request)
            | Self::ReadSecretById(request)
            | Self::RotateSecretById(request) => (
                request.opaque_id.clone(),
                format!("{}:{}", self.op_name(), request.opaque_id),
            ),
            Self::LaunchMinijailChild(request) => (
                request.vm_id.to_string(),
                format!("{}:{}:{}", self.op_name(), request.vm_id, request.role_id),
            ),
            Self::ModprobeIfAllowed(request) => (
                request.module_name.clone(),
                format!("{}:{}", self.op_name(), request.module_name),
            ),
            Self::OpenCgroupDir(request) => (
                request.scope_id.to_string(),
                format!(
                    "{}:{}:{:?}",
                    self.op_name(),
                    request.scope_id,
                    request.path_class
                ),
            ),
            Self::OpenDevice(request) => (
                request.role_id.to_string(),
                format!(
                    "{}:{}:{}",
                    self.op_name(),
                    request.role_id,
                    request.device_class
                ),
            ),
            Self::OpenFuse(request) => (
                request.role_id.to_string(),
                format!("{}:{}", self.op_name(), request.role_id),
            ),
            Self::OpenKvm(request) => (
                request.role_id.to_string(),
                format!("{}:{}", self.op_name(), request.role_id),
            ),
            Self::OpenVhostNet(request) => (
                request.role_id.to_string(),
                format!("{}:{}", self.op_name(), request.role_id),
            ),
            Self::OpenHidrawSecurityKey(request) => (
                request.vm_id.to_string(),
                format!(
                    "{}:{}:{}",
                    self.op_name(),
                    request.vm_id,
                    request.selector_id
                ),
            ),
            Self::QemuMediaEnroll(request) => (
                request.vm_id.to_string(),
                format!("{}:{}:{}", self.op_name(), request.vm_id, request.media_ref),
            ),
            Self::QemuMediaRefreshRegistry(_) => {
                ("qemu-media-registry".to_owned(), self.op_name().to_owned())
            }
            Self::QemuMediaBoot(request) => (
                request.vm_id.to_string(),
                format!("{}:{}", self.op_name(), request.vm_id),
            ),
            Self::QemuMediaSystemPowerdown(request) => (
                request.vm_id.to_string(),
                format!("{}:{}", self.op_name(), request.vm_id),
            ),
            Self::QemuMediaQueryStatus(request) => (
                request.vm_id.to_string(),
                format!("{}:{}", self.op_name(), request.vm_id),
            ),
            Self::QemuMediaQuit(request) => (
                request.vm_id.to_string(),
                format!("{}:{}", self.op_name(), request.vm_id),
            ),
            Self::QemuMediaAttach(request) | Self::QemuMediaDetach(request) => (
                request.vm_id.to_string(),
                format!("{}:{}:{}", self.op_name(), request.vm_id, request.bus_id),
            ),
            Self::OpenPidfd(request) => (
                request.vm_id.to_string(),
                format!("{}:{}:{}", self.op_name(), request.vm_id, request.role_id),
            ),
            Self::OpenPeerPidfdFromAcceptedSocket(_) => return None,
            Self::ResourceActivationAudit(request) => (
                request.audit_join.zone_id.as_str().to_owned(),
                request.audit_join.operation_identity.as_str().to_owned(),
            ),
            Self::ObserveRunner(request) => (
                request.vm_id.to_string(),
                format!("{}:{}:{}", self.op_name(), request.vm_id, request.role_id),
            ),
            Self::PipeWireAudio(request) => (
                request.vm_id.to_string(),
                format!("{}:{}:{}", self.op_name(), request.vm_id, request.role_id),
            ),
            Self::StartSystemdUnit(request) => (
                request.vm_id.to_string(),
                format!("{}:{}:{}", self.op_name(), request.vm_id, request.role_id),
            ),
            Self::CheckSystemdUserManager(request) => (
                request.vm_id.to_string(),
                format!("{}:{}:{}", self.op_name(), request.vm_id, request.role_id),
            ),
            Self::ObserveSystemdUnit(request) => (
                request.vm_id.to_string(),
                format!("{}:{}:{}", self.op_name(), request.vm_id, request.role_id),
            ),
            Self::OpenSystemdUnitPidfd(request) => (
                request.unit.vm_id.to_string(),
                format!(
                    "{}:{}:{}",
                    self.op_name(),
                    request.unit.vm_id,
                    request.unit.role_id
                ),
            ),
            Self::StopSystemdUnit(request) => (
                request.unit.vm_id.to_string(),
                format!(
                    "{}:{}:{}",
                    self.op_name(),
                    request.unit.vm_id,
                    request.unit.role_id
                ),
            ),
            Self::CgroupKill(request) => (
                request.vm_id.to_string(),
                format!("{}:{}:{}", self.op_name(), request.vm_id, request.role_id),
            ),
            Self::SignalRunner(request) => (
                request.vm_id.to_string(),
                format!("{}:{}:{}", self.op_name(), request.vm_id, request.role_id),
            ),
            Self::DeregisterRunnerPidfd(request) => (
                request.vm_id.to_string(),
                format!("{}:{}:{}", self.op_name(), request.vm_id, request.role_id),
            ),
            Self::OpenZoneStore(request) => (
                request.zone_store_id.as_str().to_owned(),
                format!("{}:{}", self.op_name(), request.zone_store_id.as_str()),
            ),
            Self::PrepareRuntimeDir(request) | Self::PrepareStateDir(request) => (
                request.vm_id.to_string(),
                format!(
                    "{}:{}:{:?}",
                    self.op_name(),
                    request.vm_id,
                    request.path_class
                ),
            ),
            Self::MigrateLegacySwtpmState(request) => (
                request.vm_id.to_string(),
                format!(
                    "{}:{}:{}",
                    self.op_name(),
                    request.vm_id,
                    request.bundle_legacy_swtpm_intent_ref
                ),
            ),
            Self::ReconcileStorageScope(request) => (
                request.storage_ref.to_string(),
                format!("{}:{}", self.op_name(), request.storage_ref),
            ),
            Self::ValidateLockSpec(request) => (
                request.lock_ref.to_string(),
                format!("{}:{}", self.op_name(), request.lock_ref),
            ),
            Self::PrepareStoreView(request) => (
                request.vm_id.to_string(),
                format!("{}:{}", self.op_name(), request.vm_id),
            ),
            Self::StoreVerify(request) => (
                request.vm_id.to_string(),
                format!("{}:{}", self.op_name(), request.vm_id),
            ),
            Self::StoreSync(request) => (
                request.vm_id.to_string(),
                format!(
                    "{}:{}:{}:{}",
                    self.op_name(),
                    request.vm_id,
                    request.bundle_closure_ref,
                    request.generation_token
                ),
            ),
            Self::RunHostInstall(request) => (
                request.bundle_installer_intent_ref.to_string(),
                format!(
                    "{}:{}:{}:{}",
                    self.op_name(),
                    request.bundle_installer_intent_ref,
                    request.enable,
                    request.start
                ),
            ),
            Self::RunMigrate(request) => (
                request.bundle_migrate_intent_ref.to_string(),
                format!("{}:{}", self.op_name(), request.bundle_migrate_intent_ref),
            ),
            Self::ApplyHostGenerationHandoff(request) => (
                request.target.to_canonical_string(),
                format!(
                    "{}:{}:{}:{}",
                    self.op_name(),
                    request.target.to_canonical_string(),
                    request.intent.source_generation,
                    request.intent.target_generation
                ),
            ),
            Self::LaunchCutoverRunner(request) => (
                request.operation_id.to_string(),
                format!(
                    "{}:{}:{}",
                    self.op_name(),
                    request.operation_id,
                    request.bootstrap_fd_index
                ),
            ),
            Self::RunActivation(request) => (
                request.vm.clone(),
                format!(
                    "{}:{}:{}:{:?}:{:?}",
                    self.op_name(),
                    request.bundle_activation_intent_ref,
                    request.vm,
                    request.mode,
                    request.phase
                ),
            ),
            Self::RunGc(request) => (
                request.bundle_gc_intent_ref.to_string(),
                format!("{}:{}", self.op_name(), request.bundle_gc_intent_ref),
            ),
            Self::RunKeysRotate(request) => (
                request.vm.clone(),
                format!(
                    "{}:{}:{}",
                    self.op_name(),
                    request.bundle_keys_intent_ref,
                    request.vm
                ),
            ),
            Self::RunHostKeyTrust(request) => (
                request.vm.clone(),
                format!(
                    "{}:{}:{}",
                    self.op_name(),
                    request.bundle_trust_intent_ref,
                    request.vm
                ),
            ),
            Self::RunRotateKnownHost(request) => (
                request.vm.clone(),
                format!(
                    "{}:{}:{}",
                    self.op_name(),
                    request.bundle_rotate_known_host_intent_ref,
                    request.vm
                ),
            ),
            Self::SetBridgePortFlags(request) => (
                request.vm_id.to_string(),
                format!("{}:{}:{}", self.op_name(), request.vm_id, request.role_id),
            ),
            Self::SetSocketAcl(request) => (
                request.vm_id.to_string(),
                format!("{}:{}:{}", self.op_name(), request.vm_id, request.role_id),
            ),
            Self::SetupMountNamespace(request) => (
                request.vm_id.to_string(),
                format!("{}:{}:{}", self.op_name(), request.vm_id, request.role_id),
            ),
            Self::SpawnRunner(request) => (
                request.vm_id.to_string(),
                format!(
                    "{}:{}:{}:{}",
                    self.op_name(),
                    request.vm_id,
                    request.role_id,
                    request.bundle_runner_intent_ref
                ),
            ),
            Self::UpdateHostsFile(request) => (
                request.bundle_hosts_intent_ref.to_string(),
                format!(
                    "{}:{}:{}",
                    self.op_name(),
                    request.bundle_hosts_intent_ref,
                    request.destroy
                ),
            ),
            Self::UsbipBind(request) => (
                request.bundle_usbip_bind_intent_ref.to_string(),
                format!(
                    "{}:{}",
                    self.op_name(),
                    request.bundle_usbip_bind_intent_ref
                ),
            ),
            Self::UsbipUnbind(request) => (
                request.bundle_usbip_bind_intent_ref.to_string(),
                format!(
                    "{}:{}:{}",
                    self.op_name(),
                    request.bundle_usbip_bind_intent_ref,
                    request.preserve_durable_claim
                ),
            ),
            Self::UsbipBindFirewallRule(request) => (
                request.bundle_usbip_firewall_intent_ref.to_string(),
                format!(
                    "{}:{}",
                    self.op_name(),
                    request.bundle_usbip_firewall_intent_ref
                ),
            ),
            Self::UsbipProxyReconcile(request) => (
                request.scope_id.to_string(),
                format!("{}:{}", self.op_name(), request.scope_id),
            ),
            Self::UsbipExplicitBind(request) => (
                request.vm.clone(),
                format!("{}:{}:{}", self.op_name(), request.vm, request.env),
            ),
            Self::UsbipExplicitFirewallRule(request) => (
                request.env.clone(),
                format!(
                    "{}:{}:{}",
                    self.op_name(),
                    request.env,
                    request.host_uplink_ip
                ),
            ),
            Self::SeedDnsmasqLease(request) => (
                request.vm_id.to_string(),
                format!("{}:{}:{}", self.op_name(), request.vm_id, request.scope_id),
            ),
            Self::BindMountFromHardlinkFarm(request) => (
                request.vm_id.to_string(),
                format!(
                    "{}:{}:{:?}",
                    self.op_name(),
                    request.vm_id,
                    request.bundle_store_view_intent_ref
                ),
            ),
            Self::OwnershipMatrixCheck(request) => (
                request.vm_id.to_string(),
                format!("{}:{}", self.op_name(), request.vm_id),
            ),
            Self::SshHostKeyPreflight(request) => (
                request.vm_id.to_string(),
                format!("{}:{}", self.op_name(), request.vm_id),
            ),
            Self::DiskInit(request) => (
                request.vm_id.to_string(),
                format!("{}:{}", self.op_name(), request.vm_id),
            ),
            Self::SecurityKeyOpenDevice(request) => (
                request.device_label.as_str().to_owned(),
                format!("{}:{}", self.op_name(), request.session_id.as_str()),
            ),
            Self::SecurityKeyApplyUdevRules(request) => (
                request.bundle_udev_intent_ref.clone(),
                format!("{}:{}", self.op_name(), request.bundle_udev_intent_ref),
            ),
            Self::ValidateBundle
            | Self::ExportBrokerAudit(_)
            | Self::Hello(_)
            | Self::CutoverAudit(_)
            | Self::CutoverEffect(_)
            | Self::PauseBroker
            | Self::PollChildReaped
            | Self::ResumeBroker => return None,
        };
        Some((
            d2b_contracts_resource::v3::canonical_digest("d2b:broker-zone:v2", scope.as_bytes()),
            d2b_contracts_resource::v3::canonical_digest(
                "d2b:broker-operation:v2",
                operation.as_bytes(),
            ),
        ))
    }

    /// Return whether this request participates in authoritative audit join.
    ///
    /// This is the allocation-free companion of [`Self::authoritative_audit_join`].
    pub fn requires_authoritative_audit_join(&self) -> bool {
        !matches!(
            self,
            Self::OpenPeerPidfdFromAcceptedSocket(_)
                | Self::ValidateBundle
                | Self::ExportBrokerAudit(_)
                | Self::CutoverAudit(_)
                | Self::CutoverEffect(_)
                | Self::Hello(_)
                | Self::PauseBroker
                | Self::PollChildReaped
                | Self::ResumeBroker
        )
    }
}

/// Fixed privileged-broker authority profiles.
///
/// Host and Guest use the same wire and executable, but each process starts
/// with one closed operation catalog. The catalog is deliberately kept next
/// to the wire operation names so adding a request requires an explicit
/// profile decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BrokerProfile {
    /// Host and realm authorities may use the complete host catalog.
    Host,
    /// Guest authorities may use only local process effects and read-only
    /// broker lifecycle operations.
    Guest,
}

impl BrokerProfile {
    /// Stable process-start profile label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Guest => "guest",
        }
    }

    /// Closed Host operation catalog.
    pub const fn host_operations() -> &'static [&'static str] {
        HOST_OPERATION_CATALOG
    }

    /// Closed Guest operation catalog.
    pub const fn guest_operations() -> &'static [&'static str] {
        GUEST_OPERATION_CATALOG
    }

    /// Return the operation catalog for this profile.
    pub const fn operations(self) -> &'static [&'static str] {
        match self {
            Self::Host => Self::host_operations(),
            Self::Guest => Self::guest_operations(),
        }
    }

    /// Check the stable operation name against the profile catalog.
    pub fn allows_operation(self, operation: &str) -> bool {
        self.operations().contains(&operation)
    }

    /// Check both the closed catalog and profile-specific target constraints.
    pub fn allows_request(self, request: &BrokerRequest) -> bool {
        if !self.allows_operation(request.op_name()) {
            return false;
        }
        match self {
            Self::Host => !Self::request_targets_guest(request),
            Self::Guest => match request {
                BrokerRequest::SpawnRunner(request) => {
                    request
                        .execution_ref
                        .as_ref()
                        .is_some_and(|target| target.resource_type().as_str() == "Guest")
                        && GUEST_LOCAL_RUNNER_ROLES.contains(&request.role)
                        && request.guest_execution.as_ref().is_some_and(GuestExecutionBinding::is_valid)
                }
                BrokerRequest::StartSystemdUnit(request)
                | BrokerRequest::ObserveSystemdUnit(request)
                | BrokerRequest::CheckSystemdUserManager(request) => request
                    .execution_ref
                    .as_ref()
                    .is_some_and(|target| target.resource_type().as_str() == "Guest")
                    && request
                        .guest_execution
                        .as_ref()
                        .is_some_and(GuestExecutionBinding::is_valid),
                BrokerRequest::OpenPidfd(request) => request
                    .guest_execution
                    .as_ref()
                    .is_some_and(GuestExecutionBinding::is_valid),
                BrokerRequest::ObserveRunner(request) => request
                    .guest_execution
                    .as_ref()
                    .is_some_and(GuestExecutionBinding::is_valid),
                BrokerRequest::OpenSystemdUnitPidfd(request) => request
                    .unit
                    .execution_ref
                    .as_ref()
                    .is_some_and(|target| target.resource_type().as_str() == "Guest")
                    && request
                        .unit
                        .guest_execution
                        .as_ref()
                        .is_some_and(GuestExecutionBinding::is_valid),
                BrokerRequest::StopSystemdUnit(request) => request
                    .unit
                    .execution_ref
                    .as_ref()
                    .is_some_and(|target| target.resource_type().as_str() == "Guest")
                    && request
                        .unit
                        .guest_execution
                        .as_ref()
                        .is_some_and(GuestExecutionBinding::is_valid),
                BrokerRequest::SignalRunner(request) => request
                    .guest_execution
                    .as_ref()
                    .is_some_and(GuestExecutionBinding::is_valid),
                BrokerRequest::DeregisterRunnerPidfd(request) => request
                    .guest_execution
                    .as_ref()
                    .is_some_and(GuestExecutionBinding::is_valid),
                _ => true,
            },
        }
    }

    fn request_targets_guest(request: &BrokerRequest) -> bool {
        match request {
            BrokerRequest::SpawnRunner(request) => request
                .execution_ref
                .as_ref()
                .is_some_and(|target| target.resource_type().as_str() == "Guest")
                || request.guest_execution.is_some(),
            BrokerRequest::StartSystemdUnit(request)
            | BrokerRequest::ObserveSystemdUnit(request)
            | BrokerRequest::CheckSystemdUserManager(request) => request
                .execution_ref
                .as_ref()
                .is_some_and(|target| target.resource_type().as_str() == "Guest")
                || request.guest_execution.is_some(),
            BrokerRequest::OpenSystemdUnitPidfd(request) => {
                request.unit.guest_execution.is_some()
                    || request
                        .unit
                        .execution_ref
                        .as_ref()
                        .is_some_and(|target| target.resource_type().as_str() == "Guest")
            }
            BrokerRequest::StopSystemdUnit(request) => {
                request.unit.guest_execution.is_some()
                    || request
                        .unit
                        .execution_ref
                        .as_ref()
                        .is_some_and(|target| target.resource_type().as_str() == "Guest")
            }
            BrokerRequest::OpenPidfd(request) => request.guest_execution.is_some(),
            BrokerRequest::ObserveRunner(request) => request.guest_execution.is_some(),
            BrokerRequest::SignalRunner(request) => request.guest_execution.is_some(),
            BrokerRequest::DeregisterRunnerPidfd(request) => request.guest_execution.is_some(),
            _ => false,
        }
    }
}

/// Every request currently defined by the broker wire. Host mode is closed
/// over this list rather than using an open-ended default.
pub const HOST_OPERATION_CATALOG: &[&str] = &[
    "ApplyHostGenerationHandoff",
    "LaunchCutoverRunner",
    "CutoverAudit",
    "CutoverEffect",
    "ApplyNftables",
    "ApplyNftablesProjection",
    "ApplyNmUnmanaged",
    "ApplyRoute",
    "ApplySysctl",
    "BindUnixSocket",
    "CreateOrReconcileUsersGroups",
    "CreateBridge",
    "DeleteBridge",
    "CreatePersistentTap",
    "DeletePersistentTap",
    "CreateTapFd",
    "DelegateCgroupV2",
    "ExportBrokerAudit",
    "Hello",
    "InjectSecretById",
    "LaunchMinijailChild",
    "ModprobeIfAllowed",
    "OpenCgroupDir",
    "OpenDevice",
    "OpenFuse",
    "OpenHidrawSecurityKey",
    "OpenKvm",
    "QemuMediaEnroll",
    "QemuMediaRefreshRegistry",
    "QemuMediaBoot",
    "QemuMediaSystemPowerdown",
    "QemuMediaQueryStatus",
    "QemuMediaQuit",
    "QemuMediaAttach",
    "QemuMediaDetach",
    "OpenPidfd",
    "OpenPeerPidfdFromAcceptedSocket",
    "ObserveRunner",
    "PipeWireAudio",
    "StartSystemdUnit",
    "CheckSystemdUserManager",
    "ObserveSystemdUnit",
    "OpenSystemdUnitPidfd",
    "StopSystemdUnit",
    "OpenZoneStore",
    "OpenVhostNet",
    "PauseBroker",
    "PollChildReaped",
    "PrepareRuntimeDir",
    "PrepareStateDir",
    "MigrateLegacySwtpmState",
    "ReconcileStorageScope",
    "ValidateLockSpec",
    "PrepareStoreView",
    "StoreSync",
    "StoreVerify",
    "ReadSecretById",
    "ResumeBroker",
    "RotateSecretById",
    "RunHostInstall",
    "RunMigrate",
    "RunActivation",
    "RunGc",
    "RunKeysRotate",
    "RunHostKeyTrust",
    "RunRotateKnownHost",
    "SetBridgePortFlags",
    "SetSocketAcl",
    "SetupMountNamespace",
    "CgroupKill",
    "SignalRunner",
    "DeregisterRunnerPidfd",
    "SpawnRunner",
    "UpdateHostsFile",
    "UsbipBind",
    "UsbipBindFirewallRule",
    "UsbipProxyReconcile",
    "UsbipUnbind",
    "UsbipExplicitBind",
    "UsbipExplicitFirewallRule",
    "ResourceActivationAudit",
    "ValidateBundle",
    "SeedDnsmasqLease",
    "BindMountFromHardlinkFarm",
    "OwnershipMatrixCheck",
    "SshHostKeyPreflight",
    "DiskInit",
    "SecurityKeyOpenDevice",
    "SecurityKeyApplyUdevRules",
];

/// Guest-local process and broker lifecycle effects. Host networking,
/// devices, storage, realm, cutover, and allocator operations are intentionally
/// absent from this catalog.
pub const GUEST_OPERATION_CATALOG: &[&str] = &[
    "Hello",
    "ExportBrokerAudit",
    "ValidateBundle",
    "OpenPidfd",
    "OpenPeerPidfdFromAcceptedSocket",
    "ObserveRunner",
    "StartSystemdUnit",
    "CheckSystemdUserManager",
    "ObserveSystemdUnit",
    "OpenSystemdUnitPidfd",
    "StopSystemdUnit",
    "PollChildReaped",
    "PrepareRuntimeDir",
    "PrepareStateDir",
    "SetupMountNamespace",
    "CgroupKill",
    "SignalRunner",
    "DeregisterRunnerPidfd",
    "SpawnRunner",
];

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
    /// Durable generation artifact selected by the caller.
    #[serde(default)]
    pub system_artifact_id: Option<ArtifactId>,
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
    /// Result of one source-to-target generation handoff.
    ApplyHostGenerationHandoff(ApplyHostGenerationHandoffResponse),
    /// Result of launching the operation-scoped cutover runner.
    LaunchCutoverRunner(LaunchCutoverRunnerResponse),
    /// Durable cutover audit publication result.
    CutoverAudit(CutoverAuditResponse),
    /// Typed operation-scoped effect result.
    CutoverEffect(CutoverEffectResponse),
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
    QemuMediaEnroll(QemuMediaEnrollResponse),
    QemuMediaRefreshRegistry(QemuMediaRefreshRegistryResponse),
    QemuMediaBoot(QemuMediaHotplugResponse),
    QemuMediaSystemPowerdown(QemuMediaLifecycleResponse),
    QemuMediaQueryStatus(QemuMediaQueryStatusResponse),
    QemuMediaQuit(QemuMediaLifecycleResponse),
    QemuMediaAttach(QemuMediaHotplugResponse),
    QemuMediaDetach(QemuMediaHotplugResponse),
    /// `OpenHidrawSecurityKey` response. The hidraw fd is returned via
    /// `SCM_RIGHTS` alongside this envelope. The response body carries
    /// only the resolved stable selector label (never the raw device
    /// path) so the audit log and daemon can correlate the fd to the
    /// configured key.
    OpenHidrawSecurityKey(OpenHidrawSecurityKeyResponse),
    /// OpenPidfd response. The pidfd itself is returned via SCM_RIGHTS
    /// on the same frame; the JSON body confirms which `(pid,
    /// start_time_ticks)` the broker verified.
    OpenPidfd(OpenPidfdResponse),
    /// Response for [`BrokerRequest::OpenPeerPidfdFromAcceptedSocket`].
    /// The only attachment is the returned close-on-exec pidfd.
    OpenPeerPidfdFromAcceptedSocket(OpenPeerPidfdFromAcceptedSocketResponse),
    /// Observation of a broker-owned runner. No pidfd is returned because
    /// the operation is a status query over the broker's retained registry.
    ObserveRunner(ObserveRunnerResponse),
    /// Result of one broker-owned PipeWire effect. Raw node identifiers and
    /// runtime paths never cross the wire.
    PipeWireAudio(PipeWireAudioResponse),
    /// StartSystemdUnit response. The exact-main pidfd is returned via
    /// SCM_RIGHTS alongside this identity envelope.
    StartSystemdUnit(StartSystemdUnitResponse),
    /// Result of a same-UID user-manager reachability check.
    CheckSystemdUserManager(CheckSystemdUserManagerResponse),
    /// Observation of a transient systemd unit. `None` is represented by
    /// `present = false` and a zero identity.
    ObserveSystemdUnit(ObserveSystemdUnitResponse),
    /// Re-open response for a previously verified transient unit.
    OpenSystemdUnitPidfd(OpenSystemdUnitPidfdResponse),
    /// Stop response for an exact transient unit identity.
    StopSystemdUnit(StopSystemdUnitResponse),
    /// `OpenZoneStore` response. The database descriptor is the sole
    /// `SCM_RIGHTS` attachment on the same frame; the JSON body contains
    /// only opaque identity and disposition metadata.
    OpenZoneStore(OpenZoneStoreResponse),
    /// Response for [`BrokerRequest::ResourceActivationAudit`].
    ResourceActivationAudit(ResourceActivationAuditResponse),
    /// Drain response for `BrokerRequest::PollChildReaped`.
    PollChildReaped(PollChildReapedResponse),
    ReconcileStorageScope(ReconcileStorageScopeResponse),
    MigrateLegacySwtpmState(MigrateLegacySwtpmStateResponse),
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
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrokerErrorResponse {
    pub kind: String,
    pub operation: String,
    #[serde(default)]
    pub target_wave: Option<String>,
    pub message: String,
    pub action: String,
}

impl core::fmt::Debug for BrokerErrorResponse {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BrokerErrorResponse")
            .field("kind", &self.kind)
            .field("operation", &self.operation)
            .field("has_target_wave", &self.target_wave.is_some())
            .field("message", &"<redacted>")
            .field("action", &"<redacted>")
            .finish()
    }
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

/// The action an [`ApplyNftablesProjectionRequest`] carries.
///
/// Closed on purpose: the framework's own nftables op spells its two
/// directions as a `destroy` boolean, which leaves "neither" and "both"
/// expressible in a future field pair. A projection names exactly one of
/// two directions and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum NftablesProjectionAction {
    /// Install the resolved projection.
    Apply,
    /// Remove the resolved projection.
    Remove,
}

/// The broker re-derives the desired projection from
/// `bundle_nft_projection_intent_ref`. As with
/// [`ApplyNftablesRequest`], the daemon passes no inline rule text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyNftablesProjectionRequest {
    pub bundle_nft_projection_intent_ref: BundleOpId,
    pub scope_id: ScopeId,
    pub action: NftablesProjectionAction,
    /// Immutable installed bundle generation the projection was resolved from.
    pub expected_generation_id: ResourceBundleGenerationId,
    #[serde(default)]
    pub desired_hash: Option<String>,
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
    /// Optional v3 attachment realization identity. Legacy host-prep callers
    /// omit these fields; Network Provider callers supply all three so the
    /// broker can adopt and generation-fence the persistent TAP on restart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_id: Option<ResourceUid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_generation: Option<ResourceGeneration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_generation: Option<ResourceGeneration>,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// Delete one trusted attachment realization without accepting its ifname,
/// path, or ownership marker from the caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeletePersistentTapRequest {
    pub attachment_id: ResourceUid,
    pub expected_network_generation: ResourceGeneration,
    pub expected_attachment_generation: ResourceGeneration,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// The broker derives the bridge ifname and its attributes from the
/// trusted bundle row anchored by `bundle_bridge_intent_ref` +
/// `scope_id`. As with the TAP ops, no caller-supplied ifname crosses
/// the wire; the observed ifname appears only in the audit record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateBridgeRequest {
    pub bundle_bridge_intent_ref: BundleOpId,
    pub scope_id: ScopeId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// See [`CreateBridgeRequest`] for the opaque-ID rationale;
/// `DeleteBridge` follows the same contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteBridgeRequest {
    pub bundle_bridge_intent_ref: BundleOpId,
    pub scope_id: ScopeId,
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
/// input - the broker reads it from its own bundle copy via `scope_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DelegateCgroupV2Request {
    pub scope_id: ScopeId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportBrokerAuditRequest {
    pub filter: Option<BrokerAuditFilter>,
    pub since: Option<String>,
    #[serde(default)]
    pub cursor: Option<AuditExportCursor>,
    #[serde(default = "default_audit_export_limit")]
    pub limit: u32,
}

impl core::fmt::Debug for ExportBrokerAuditRequest {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ExportBrokerAuditRequest")
            .field("has_filter", &self.filter.is_some())
            .field("has_since", &self.since.is_some())
            .field("has_cursor", &self.cursor.is_some())
            .field("limit", &self.limit)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrokerAuditFilter {
    pub env: Option<String>,
    pub operation: Option<String>,
    pub vm: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(default)]
    pub severity: Option<BrokerAuditSeverity>,
}

impl core::fmt::Debug for BrokerAuditFilter {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("BrokerAuditFilter(<redacted>)")
    }
}

/// Closed severity predicate for broker audit export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum BrokerAuditSeverity {
    Info,
    Warning,
    Error,
    Denied,
}

fn default_audit_export_limit() -> u32 {
    256
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
/// `ModuleName` newtype because it is genuinely a public input -
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

/// Exact authenticated binding for a Guest-local Process lifecycle.
///
/// The values are commitments, not raw boot identifiers or transport
/// handles. A Guest broker requires this tuple for every target-local
/// Process operation and revalidates the boot commitment against its own
/// kernel before executing the request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestExecutionBinding {
    pub target_uid: ResourceUid,
    pub boot_identity_digest: [u8; 32],
    pub session_generation: u64,
    pub assignment_epoch: u64,
    pub provider_generation: u64,
    pub controller_generation: u64,
}

impl GuestExecutionBinding {
    /// Return whether all Guest execution commitments are populated.
    pub fn is_valid(&self) -> bool {
        self.target_uid.as_str().len() > 0
            && self.boot_identity_digest != [0; 32]
            && self.session_generation > 0
            && self.assignment_epoch > 0
            && self.provider_generation > 0
            && self.controller_generation > 0
    }
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
    /// Optional generic Process identity binding. Legacy VM runner callers
    /// omit these fields and retain the historical VM/role key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_ref: Option<ResourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_uid: Option<ResourceUid>,
    /// Exact Guest target/session binding for target-local Process adoption.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest_execution: Option<GuestExecutionBinding>,
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

/// A request whose authority is the sole attached accepted Unix socket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenPeerPidfdFromAcceptedSocketRequest {}

/// Response metadata for an accepted-socket-bound peer pidfd handoff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenPeerPidfdFromAcceptedSocketResponse {
    /// The sole SCM_RIGHTS pidfd attachment index.
    pub pidfd_index: u32,
}

/// Observe one runner by its trusted `(vm_id, role_id)` identity. The
/// broker resolves the intent reference again and refuses stale or
/// ambiguous ownership rather than trusting caller-supplied process data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObserveRunnerRequest {
    pub vm_id: VmId,
    pub role_id: RoleId,
    pub role: RunnerRole,
    pub bundle_runner_intent_ref: BundleOpId,
    /// Optional generic Process identity binding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_ref: Option<ResourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_uid: Option<ResourceUid>,
    /// Exact Guest target/session binding for target-local Process adoption.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest_execution: Option<GuestExecutionBinding>,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// Verified runner observation returned by the broker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObserveRunnerResponse {
    pub vm_id: VmId,
    pub role_id: RoleId,
    pub present: bool,
    pub pid: i32,
    pub start_time_ticks: u64,
    pub cgroup_verified: bool,
    pub executable_verified: bool,
}

/// Audio channel selected by a broker-owned PipeWire effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PipeWireAudioChannel {
    /// Playback stream.
    Speaker,
    /// Capture stream.
    Microphone,
}

/// Closed host-side PipeWire action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum PipeWireAudioAction {
    /// Set the stream mute state.
    SetGrant { on: bool },
    /// Set the stream level in the inclusive 0..=100 range.
    SetLevel { percent: u8 },
}

/// Request one bounded broker-owned PipeWire effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PipeWireAudioRequest {
    /// Opaque VM identity resolved against the trusted bundle.
    pub vm_id: VmId,
    /// Opaque audio runner role identity.
    pub role_id: RoleId,
    /// Signed runner intent that supplies the PipeWire effect tools and
    /// runtime environment.
    pub bundle_runner_intent_ref: BundleOpId,
    /// Stream direction.
    pub channel: PipeWireAudioChannel,
    /// Closed effect action.
    pub action: PipeWireAudioAction,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// Response to [`PipeWireAudioRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PipeWireAudioResponse {
    pub vm_id: VmId,
    pub role_id: RoleId,
    /// Whether the requested host effect was applied.
    pub applied: bool,
    /// Whether the broker could reach the PipeWire session.
    pub host_ready: bool,
    /// Whether exactly one matching stream was found.
    pub node_present: bool,
}

/// Closed systemd execution domain. User-manager execution remains subject to
/// same-UID verification by the broker; no manager address crosses the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SystemdUnitDomain {
    /// The host system manager.
    System,
    /// The verified per-user manager.
    User,
}

/// Stable systemd identity returned only after the broker has queried the
/// manager and re-read the process start time under the pidfd boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SystemdUnitIdentity {
    /// systemd's 16-byte InvocationID.
    pub invocation_id: [u8; 16],
    /// Digest of the exact ControlGroup path; the path never crosses IPC.
    pub cgroup_identity: [u8; 32],
    /// MainPID verified against the unit and `/proc`.
    pub main_pid: u32,
    /// `/proc/<pid>/stat` field-22 start time.
    pub start_time_ticks: u64,
    /// Owning Provider digest bound to the unit identity.
    pub provider_identity: [u8; 32],
    /// Component template digest bound to the unit identity.
    pub template_identity: [u8; 32],
    /// Process resource generation bound to the unit identity.
    pub generation: u64,
    /// Content identity of the broker-resolved trusted bundle.
    pub bundle_content_identity: String,
    /// Exact Guest target/session binding, when this is Guest-local.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest_execution: Option<GuestExecutionBinding>,
}

/// Shared trusted request fields for systemd unit operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SystemdUnitRequest {
    /// Execution target represented by the trusted runner intent.
    pub vm_id: VmId,
    /// Process role identifier within the execution target.
    pub role_id: RoleId,
    /// Optional generic Process identity binding used in unit names and
    /// authorization. Legacy VM runner callers omit these fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_ref: Option<ResourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_uid: Option<ResourceUid>,
    /// Closed runner role selecting the trusted bundle launch plan.
    pub role: RunnerRole,
    /// Opaque bundle reference resolved only by the broker.
    pub bundle_runner_intent_ref: BundleOpId,
    /// Content identity the broker must resolve for this unit.
    pub bundle_content_identity: String,
    /// Process Provider identity digest.
    pub provider_identity: [u8; 32],
    /// Component template identity digest.
    pub template_identity: [u8; 32],
    /// Nonzero Process resource generation.
    pub generation: u64,
    /// System or verified user manager.
    pub domain: SystemdUnitDomain,
    /// Canonical Host or Guest execution target, when supplied by a v3
    /// Process ticket. Legacy VM runner callers omit this field.
    #[serde(default)]
    pub execution_ref: Option<ResourceRef>,
    /// Canonical User resource bound to a user-domain launch.
    #[serde(default)]
    pub user_ref: Option<ResourceRef>,
    /// Exact Guest target/session binding for target-local Process operations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest_execution: Option<GuestExecutionBinding>,
    /// Typed sandbox requirements enforced by the broker's systemd launch
    /// adapter. Legacy VM runner callers omit this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_plan: Option<SandboxLaunchPlan>,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// Request to start one transient systemd unit.
pub type StartTransientUnitRequest = SystemdUnitRequest;
/// Compatibility spelling used by the BrokerRequest variant.
pub type StartSystemdUnitRequest = StartTransientUnitRequest;
/// Request to check the trusted per-user systemd manager.
pub type CheckSystemdUserManagerRequest = SystemdUnitRequest;

/// Request to observe one transient systemd unit.
pub type ObserveSystemdUnitRequest = SystemdUnitRequest;

/// Request to re-open a pidfd after identity re-verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenSystemdUnitPidfdRequest {
    /// Trusted unit selector and binding inputs.
    #[serde(flatten)]
    pub unit: SystemdUnitRequest,
    /// Identity observed before the local descriptor was requested.
    pub expected: SystemdUnitIdentity,
}

/// Request to stop one exact transient systemd unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StopSystemdUnitRequest {
    /// Trusted unit selector and binding inputs.
    #[serde(flatten)]
    pub unit: SystemdUnitRequest,
    /// Identity that must still match before the stop is sent.
    pub expected: SystemdUnitIdentity,
    /// Graceful drain or forced termination.
    pub class: SystemdStopClass,
}

/// Stop class for transient systemd units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SystemdStopClass {
    /// Request systemd to stop the unit and wait for it to become inactive.
    Drain,
    /// Kill the exact unit cgroup and verify it becomes inactive.
    Terminate,
}

/// Start response. The exact-main pidfd is the first SCM_RIGHTS fd.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartTransientUnitResponse {
    pub vm_id: VmId,
    pub role_id: RoleId,
    pub identity: SystemdUnitIdentity,
    pub pidfd_index: u32,
}
/// Compatibility spelling used by the BrokerResponse variant.
pub type StartSystemdUnitResponse = StartTransientUnitResponse;

/// Response from a same-UID per-user manager reachability check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckSystemdUserManagerResponse {
    pub vm_id: VmId,
    pub role_id: RoleId,
    pub available: bool,
}

/// Observation response. `present = false` has no identity payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObserveSystemdUnitResponse {
    pub vm_id: VmId,
    pub role_id: RoleId,
    pub present: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<SystemdUnitIdentity>,
}

/// Re-open response. The exact-main pidfd is the first SCM_RIGHTS fd.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenSystemdUnitPidfdResponse {
    pub vm_id: VmId,
    pub role_id: RoleId,
    pub identity: SystemdUnitIdentity,
    pub pidfd_index: u32,
}

/// Stop response after systemd confirmed the unit is inactive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StopSystemdUnitResponse {
    pub vm_id: VmId,
    pub role_id: RoleId,
    pub stopped: bool,
}

/// Open one broker-resolved Zone resource store. The request is deliberately
/// one typed opaque id: all path, marker, ownership, filesystem, locking, and
/// publication authority remains in the signed storage-row artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenZoneStoreRequest {
    pub zone_store_id: ZoneStoreId,
}

/// Terminal disposition of a Zone store open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ZoneStoreDisposition {
    Provisioned,
    Opened,
}

impl ZoneStoreDisposition {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Provisioned => "provisioned",
            Self::Opened => "opened",
        }
    }
}

/// Response to [`OpenZoneStoreRequest`]. The database fd is always the only
/// descriptor attached to the response frame and is selected by `fd_index`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenZoneStoreResponse {
    pub zone_store_id: ZoneStoreId,
    pub store_identity: String,
    pub disposition: ZoneStoreDisposition,
    pub fd_index: u32,
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

/// Broker op that resolves a configured FIDO security-key stable
/// selector and opens the physical hidraw node for `d2bd`. The
/// daemon never names raw hidraw paths; the broker re-derives the
/// node from its trusted bundle copy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenHidrawSecurityKeyRequest {
    /// Opaque VM identifier from the trusted manifest.
    pub vm_id: VmId,
    /// Opaque stable-selector id that the broker resolves against
    /// its trusted bundle's security-key device registry.
    pub selector_id: String,
    /// Exact Device resource admitted by Core.
    pub device_ref: ResourceRef,
    /// Core-derived Host physical-backing authority digest.
    pub authority_key: String,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// Derive the exact Device-selector binding accepted by the privileged broker.
pub fn security_key_authority_binding(device_ref: &ResourceRef, selector_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"d2b:security-key-authority/v1");
    hasher.update([0]);
    hasher.update(device_ref.to_canonical_string().as_bytes());
    hasher.update([0]);
    hasher.update(selector_id.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

/// Confirmation that the broker opened the security-key hidraw node.
/// The hidraw fd itself is returned via `SCM_RIGHTS` on the same
/// seqpacket frame; this body carries only scrubbed metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenHidrawSecurityKeyResponse {
    /// The stable selector label that was resolved (no raw path).
    pub selector_resolved: String,
    /// Closed-set device-class label confirming the node is a
    /// FIDO-class HID device.
    pub device_class: String,
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

/// Opaque request for one trusted legacy swtpm migration or inventory probe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MigrateLegacySwtpmStateRequest {
    pub bundle_legacy_swtpm_intent_ref: BundleOpId,
    pub vm_id: VmId,
    /// When true, inspect the broker-owned legacy inventory without mutating
    /// state. The closed outcome is used by Core to seal the migration
    /// decision before the Provider reconcile starts.
    #[serde(default)]
    pub probe_only: bool,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// Closed migration outcome returned by the broker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LegacySwtpmMigrationOutcome {
    Migrated,
    AlreadyMigrated,
    NotApplicable,
    Pending,
    Failed,
    Ambiguous,
    /// The broker inventory proves that legacy state exists and adoption is
    /// required before a new TPM state can be ensured.
    AdoptionRequired,
    /// The broker inventory proves that no prior state exists.
    NeverProvisioned,
}

impl LegacySwtpmMigrationOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Migrated => "migrated",
            Self::AlreadyMigrated => "already-migrated",
            Self::NotApplicable => "not-applicable",
            Self::Pending => "pending",
            Self::Failed => "failed",
            Self::Ambiguous => "ambiguous",
            Self::AdoptionRequired => "adoption-required",
            Self::NeverProvisioned => "never-provisioned",
        }
    }
}

/// Result of one broker-owned legacy swtpm migration attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MigrateLegacySwtpmStateResponse {
    pub outcome: LegacySwtpmMigrationOutcome,
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
/// used as the on-disk generation key - the broker derives the
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

#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportBrokerAuditResponse {
    pub entries: Vec<AuditExportEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<AuditExportCursor>,
    pub complete: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExportBrokerAuditResponseWire {
    entries: Vec<AuditExportEntry>,
    #[serde(default)]
    next_cursor: Option<AuditExportCursor>,
    complete: bool,
}

impl<'de> Deserialize<'de> for ExportBrokerAuditResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ExportBrokerAuditResponseWire::deserialize(deserializer)?;
        validate_audit_page(wire.complete, wire.next_cursor.as_ref())
            .map_err(serde::de::Error::custom)?;
        Ok(Self {
            entries: wire.entries,
            next_cursor: wire.next_cursor,
            complete: wire.complete,
        })
    }
}

impl core::fmt::Debug for ExportBrokerAuditResponse {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ExportBrokerAuditResponse")
            .field("entry_count", &self.entries.len())
            .field("has_next_cursor", &self.next_cursor.is_some())
            .field("complete", &self.complete)
            .finish()
    }
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

/// Authenticated resource-bundle activation audit join. Both identities are
/// broker-derived canonical SHA-256 values; no resource payload or path is
/// accepted over the wire.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceActivationAuditRequest {
    pub audit_join: AuditJoinContext,
}

impl core::fmt::Debug for ResourceActivationAuditRequest {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ResourceActivationAuditRequest(<redacted>)")
    }
}

/// Confirmation that the broker appended the activation durability record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceActivationAuditResponse {
    pub recorded: bool,
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
    /// Optional generic Process identity binding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_ref: Option<ResourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_uid: Option<ResourceUid>,
    /// Exact Guest target/session binding for target-local Process control.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest_execution: Option<GuestExecutionBinding>,
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
pub struct CgroupKillRequest {
    pub vm_id: VmId,
    pub role_id: RoleId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeregisterRunnerPidfdRequest {
    pub vm_id: VmId,
    pub role_id: RoleId,
    /// Optional exact process identity. Deregistration must not remove a
    /// replacement runner that reused the VM/role tuple.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_start_time_ticks: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_ref: Option<ResourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_uid: Option<ResourceUid>,
    /// Exact Guest target/session binding for target-local Process cleanup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest_execution: Option<GuestExecutionBinding>,
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
/// `RunnerRole` identifies the bundle-owned runner shape consumed by the
/// broker. Provider-specific argv planning is not performed in this crate.
/// Adding new roles requires a bundle schema bump so downstream bundles can
/// declare the new launch context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RunnerRole {
    /// Cloud Hypervisor headless / hybrid VM. The runtime Provider owns argv
    /// planning; the broker consumes the bundle-authoritative launch shape.
    CloudHypervisor,
    /// QEMU media runtime scaffold. The runtime Provider owns argv planning;
    /// the broker consumes the bundle-authoritative launch shape.
    QemuMedia,
    /// Target-local one-shot NixOS activation runner. Guest mode may spawn
    /// this role only from the bundle-authoritative process intent.
    ActivationNixos,
    /// virtiofsd sidecar; one per `microvm.shares` row. The daemon/bundle
    /// provides argv from `nixos-modules/processes-json.nix`.
    Virtiofsd,
    /// swtpm sidecar (long-lived `swtpm socket ...` process).
    Swtpm,
    /// swtpm pre-start flush (`swtpm_ioctl -i --unix ...`). One-shot.
    SwtpmFlush,
    /// crosvm GPU sidecar. Broker invokes the device GPU Provider argv generator.
    Gpu,
    /// vhost-device-sound audio sidecar. Broker invokes
    /// `d2b_provider_audio_pipewire::generate_audio_argv`.
    Audio,
    /// crosvm video-decoder sidecar. Broker invokes the device GPU Provider argv generator.
    Video,
    /// socat-based vsock relay sidecar. The transport-vsock Provider owns
    /// argv planning; the broker consumes the bundle-authoritative shape.
    VsockRelay,
    /// usbip helper sidecar. Broker invokes the device USBIP Provider argv generator.
    Usbip,
    /// OTel host-bridge sidecar (vsock relay folded out of
    /// `d2b-otel-host-bridge.service` into broker SpawnRunner).
    /// Receives pre-opened fds for the obs VM vsock socket and the
    /// d2b OTel host-egress socket; no AF_VSOCK socket creation
    /// capability in the role profile. The bundle remains authoritative.
    OtelHostBridge,
    /// Host-jailed Wayland proxy. The display Provider owns argv planning and
    /// the bundle remains authoritative.
    /// Empty host capabilities; mandatory `seccompPolicyRef`; no
    /// PipeWire/Pulse socket access. Runs as `d2b-<vm>-wlproxy`
    /// with the real host compositor socket bound read/write at a
    /// fixed in-jail upstream path.
    WaylandProxy,
}

/// Typed semantic sandbox plan compiled by the daemon and re-validated by
/// the privileged broker before a runner is spawned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SandboxLaunchPlan {
    pub digest: String,
    pub domain: ExecutionDomain,
    pub namespace_classes: Vec<NamespaceClass>,
    pub capability_classes: Vec<CapabilityClass>,
    pub seccomp_class: d2b_contracts_resource::v3::execution_policy::BoundedToken,
    pub no_new_privileges: bool,
    pub start_root: bool,
    pub environment_class: EnvironmentClass,
    pub read_only_root: bool,
    pub umask: Option<String>,
    pub oom_score_adj: i32,
    pub user_namespace: Option<UserNamespaceSpec>,
}

impl RunnerRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CloudHypervisor => "cloud-hypervisor",
            Self::QemuMedia => "qemu-media",
            Self::ActivationNixos => "activation-nixos-runner",
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
        }
    }
}

/// Closed set of runner roles that a Guest broker may spawn locally.
///
/// The current runner vocabulary contains mostly host-side VM, device, relay,
/// observability, and compositor helpers. The activation runner is the one
/// explicitly Guest-local role; future Guest Process roles must be added here
/// together with their signed bundle contract and identity fencing.
pub const GUEST_LOCAL_RUNNER_ROLES: &[RunnerRole] = &[RunnerRole::ActivationNixos];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpawnRunnerRequest {
    /// VM scope the runner belongs to.
    pub vm_id: VmId,
    /// Per-VM role this runner fills. Must be unique across the VM's
    /// active runners - the daemon's pidfd table is keyed on
    /// `(vm_id, role_id)` and a duplicate registration fails closed.
    pub role_id: RoleId,
    /// Optional generic Process identity binding. These fields are part of
    /// the registry key and prevent distinct Process resources from
    /// colliding on the legacy VM/role tuple.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_ref: Option<ResourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_uid: Option<ResourceUid>,
    /// Content identity of the daemon's trusted bundle snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_content_identity: Option<String>,
    /// Provider/template identity expected by the daemon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_identity: Option<[u8; 32]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_identity: Option<[u8; 32]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
    /// Typed stdin input admitted only for the activation-nixos runner role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation_input: Option<ActivationRunnerInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_plan: Option<SandboxLaunchPlan>,
    /// Role selector - picks the argv generator the broker applies to
    /// the bundle row anchored by `bundle_runner_intent_ref`.
    pub role: RunnerRole,
    /// Opaque reference into the trusted bundle's runner-intent table.
    /// The broker resolves this to the full launch context (binary
    /// path, argv inputs, uid/gid, capabilities, seccomp policy ref,
    /// cgroup placement, mount namespace, environment) and feeds it
    /// to the matching argv generator.
    pub bundle_runner_intent_ref: BundleOpId,
    /// Canonical Host or Guest execution target bound by the Process ticket.
    /// Legacy VM runner callers omit this additive field.
    #[serde(default)]
    pub execution_ref: Option<ResourceRef>,
    /// Canonical execution domain bound by the Process ticket.
    #[serde(default)]
    pub execution_domain: Option<ExecutionDomain>,
    /// Canonical User resource bound to a user-domain launch.
    #[serde(default)]
    pub user_ref: Option<ResourceRef>,
    /// Exact Guest target/session binding for target-local Process launches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest_execution: Option<GuestExecutionBinding>,
    /// Optional vsock CID / TAP fd slot allocated by the daemon at
    /// host-prepare time. The broker validates each entry against the
    /// bundle row and refuses any unexpected allocation slot. None
    /// for roles that do not need them (virtiofsd / swtpm).
    #[serde(default)]
    pub runtime_allocations: Vec<RunnerAllocation>,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
    /// Universal workload identity from the realm-native model.
    ///
    /// Additive: present for VMs that are declared as realm workloads;
    /// absent (`None`) for VMs that predate realm workload declarations.
    /// The broker treats `None` as "no realm identity available" and does
    /// not reject the request - this field is for audit, observability, and
    /// routing purposes only. The backend-specific runtime config
    /// (`vm_id`, `role`, `role_id`, `bundle_runner_intent_ref`) is always
    /// carried in the existing typed fields, never inside this identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload_identity: Option<WorkloadIdentity>,
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
    /// [`crate::broker_wire::CreateTapFdRequest`] - the daemon
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
    /// Resolved execution binding and content identities echoed by the
    /// broker after validating the request against its trusted bundle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_ref: Option<ResourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_domain: Option<ExecutionDomain>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_ref: Option<ResourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest_execution: Option<GuestExecutionBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_identity: Option<[u8; 32]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_identity: Option<[u8; 32]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_content_identity: Option<String>,
    /// Child PID. The daemon validates this against the pidfd it
    /// received and against `/proc/<pid>/stat` field 22
    /// (`start_time`).
    pub pid: i32,
    /// Field-22 `start_time` value the broker captured immediately
    /// after `clone()`. Pinned to the pidfd so any restart
    /// reconciliation rejects a stale (pid, start_time) tuple.
    pub start_time_ticks: u64,
    /// Index into the SCM_RIGHTS fd vector the daemon should treat as
    /// the spawned process's pidfd. Always `0` today - kept explicit
    /// so future multi-fd spawn responses (e.g. CH API socket + pidfd)
    /// have an existing wire slot.
    pub pidfd_index: u32,
    /// Optional index into the SCM_RIGHTS fd vector for a provider-specific
    /// console stream. qemu-media uses this for the daemon-owned peer of the
    /// socketpair whose other end was passed to QEMU.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub console_fd_index: Option<u32>,
}

/// Canonical opaque digest carried by the broker audit join context.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct CanonicalAuditDigest(pub String);

impl CanonicalAuditDigest {
    /// Parse the exact lower-case SHA-256 wire spelling.
    pub fn parse(value: impl Into<String>) -> Result<Self, &'static str> {
        let value = value.into();
        if d2b_contracts_resource::v3::is_canonical_digest(&value) {
            Ok(Self(value))
        } else {
            Err("canonical-audit-digest-invalid")
        }
    }

    /// Borrow the digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for CanonicalAuditDigest {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("CanonicalAuditDigest(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for CanonicalAuditDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

/// Explicit Zone and operation identity carried with broker requests.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditJoinContext {
    pub zone_id: CanonicalAuditDigest,
    pub operation_identity: CanonicalAuditDigest,
}

impl core::fmt::Debug for AuditJoinContext {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("AuditJoinContext(<redacted>)")
    }
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
    /// Explicit canonical join identities for broker/resource durability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_join: Option<AuditJoinContext>,
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
    /// Operation-scoped runner peer admitted only to closed cutover ops.
    CutoverRunner {
        operation_id: BundleOpId,
        capability_digest: CanonicalAuditDigest,
    },
    #[default]
    NotAuthorized,
}

impl BrokerCallerRole {
    pub fn is_admin_uid(&self) -> bool {
        matches!(self, Self::AdminUid { .. })
    }

    /// Return whether this is the operation-scoped runner peer class.
    pub fn is_cutover_runner(&self) -> bool {
        matches!(self, Self::CutoverRunner { .. })
    }

    pub fn for_display(&self) -> &'static str {
        match self {
            Self::AdminUid { .. } => "d2b-admin",
            Self::LauncherUid { .. } => "d2b-launcher",
            Self::RootUid { .. } => "RootUid",
            Self::CutoverRunner { .. } => "d2b-cutover-runner",
            Self::NotAuthorized => "d2b-not-authorized",
        }
    }
}

// ---------------------------------------------------------------
// Typed broker request scaffolds for the host-prep DAG steps. The
// dispatchers currently return `BrokerError::Unimplemented` until real
// handlers are wired. The structs follow the opaque-id discipline: the
// daemon never names raw paths/uids/argv on the wire - only
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
    use d2b_contracts::{decode_frame, encode_frame};

    #[test]
    fn validate_bundle_serializes_with_exact_kind() {
        let json = serde_json::to_value(BrokerRequest::ValidateBundle).expect("serializes");
        assert_eq!(json, serde_json::json!({ "kind": "ValidateBundle" }));
    }

    #[test]
    fn pipewire_audio_request_is_opaque_and_closed() {
        let request = BrokerRequest::PipeWireAudio(PipeWireAudioRequest {
            vm_id: VmId::new("corp-vm"),
            role_id: RoleId::new("audio"),
            bundle_runner_intent_ref: BundleOpId::new("runner:vm:corp-vm:role:audio"),
            channel: PipeWireAudioChannel::Speaker,
            action: PipeWireAudioAction::SetLevel { percent: 75 },
            tracing_span_id: None,
        });
        let json = serde_json::to_value(&request).expect("serializes");
        assert_eq!(json["kind"], "PipeWireAudio");
        assert_eq!(json["payload"]["vmId"], "corp-vm");
        assert_eq!(
            json["payload"]["action"],
            serde_json::json!({"kind": "setLevel", "value": {"percent": 75}})
        );
        assert_eq!(request.op_name(), "PipeWireAudio");
        assert!(request.authoritative_audit_join().is_some());
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
            BrokerCallerRole::CutoverRunner {
                operation_id: BundleOpId::new("op"),
                capability_digest: CanonicalAuditDigest::parse(
                    "sha256:".to_owned() + &"a".repeat(64)
                )
                .unwrap(),
            }
            .for_display(),
            "d2b-cutover-runner"
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
            BrokerCallerRole::CutoverRunner {
                operation_id: BundleOpId::new("op"),
                capability_digest: CanonicalAuditDigest::parse(
                    "sha256:".to_owned() + &"a".repeat(64),
                )
                .unwrap(),
            },
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
            audit_join: None,
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
            system_artifact_id: None,
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
    fn open_zone_store_request_round_trips_with_only_opaque_id() {
        let frame = encode_frame(&serde_json::json!({
            "kind": "OpenZoneStore",
            "payload": {
                "zoneStoreId": "zone-store-local-root"
            }
        }))
        .expect("encodes");
        let decoded = decode_frame::<BrokerRequest>("BrokerRequest", &frame).expect("decodes");
        match decoded {
            BrokerRequest::OpenZoneStore(req) => {
                assert_eq!(req.zone_store_id.as_str(), "zone-store-local-root");
            }
            other => panic!("expected OpenZoneStore, got {other:?}"),
        }

        let response = BrokerResponse::OpenZoneStore(OpenZoneStoreResponse {
            zone_store_id: ZoneStoreId::parse("zone-store-local-root").expect("id"),
            store_identity: "sha256:".to_owned() + &"a".repeat(64),
            disposition: ZoneStoreDisposition::Opened,
            fd_index: 0,
        });
        let encoded = serde_json::to_string(&response).expect("serialize response");
        let decoded: BrokerResponse = serde_json::from_str(&encoded).expect("decode response");
        assert_eq!(decoded, response);
    }

    #[test]
    fn open_zone_store_rejects_paths_and_extra_authority_fields() {
        for field in ["path", "parentDirectoryId", "owner", "mode", "marker"] {
            let frame = encode_frame(&serde_json::json!({
                "kind": "OpenZoneStore",
                "payload": {
                    "zoneStoreId": "zone-store-local-root",
                    field: "/var/lib/d2b/zones/local-root/store.redb"
                }
            }))
            .expect("encodes");
            let error = decode_frame::<BrokerRequest>("BrokerRequest", &frame)
                .expect_err("caller authority must be rejected");
            assert_eq!(
                error.kind().as_str(),
                "wire-unknown-field",
                "unexpected rejection for {field}: {}",
                error.message()
            );
        }
    }

    #[test]
    fn open_zone_store_rejects_path_injection_in_opaque_id() {
        let frame = encode_frame(&serde_json::json!({
            "kind": "OpenZoneStore",
            "payload": {
                "zoneStoreId": "zone-store-local-root/../../outside"
            }
        }))
        .expect("encodes");
        let error = decode_frame::<BrokerRequest>("BrokerRequest", &frame)
            .expect_err("path-shaped storage id must be rejected");
        assert!(
            error.kind().as_str() == "wire-invalid-field"
                || error.kind().as_str() == "wire-malformed-json",
            "unexpected error kind {}",
            error.kind().as_str()
        );
    }

    #[test]
    fn open_zone_store_unknown_operation_fails_closed() {
        let frame = encode_frame(&serde_json::json!({
            "kind": "OpenZoneStoreFuture",
            "payload": {
                "zoneStoreId": "zone-store-local-root"
            }
        }))
        .expect("encodes");
        let error = decode_frame::<BrokerRequest>("BrokerRequest", &frame)
            .expect_err("unknown operation must be denied");
        assert!(
            error.kind().as_str() == "wire-malformed-json"
                || error.kind().as_str() == "wire-version-mismatch",
            "unexpected error kind {}",
            error.kind().as_str()
        );
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
    /// ifname/owner/attrs from the trusted bundle. The v3 attachment
    /// identity and generation fences are optional typed fields used only
    /// for broker-owned restart adoption and finalization.
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
    /// reintroduced would still be caught - but the guard could not
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
    /// type-mismatch on a string value - the gate would see an error and
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
    fn apply_nftables_projection_requires_installed_generation_fence() {
        let generation_id = format!("sha256:{}", "1".repeat(64));
        let mut payload = serde_json::json!({
            "bundleNftProjectionIntentRef": "nft-projection:work",
            "scopeId": "scope:work",
            "action": "apply",
            "expectedGenerationId": generation_id,
        });
        let request: ApplyNftablesProjectionRequest =
            serde_json::from_value(payload.clone()).expect("valid fenced projection request");
        let encoded = serde_json::to_value(request).expect("projection request serializes");
        assert_eq!(encoded["expectedGenerationId"], generation_id);

        payload
            .as_object_mut()
            .unwrap()
            .remove("expectedGenerationId");
        assert!(serde_json::from_value::<ApplyNftablesProjectionRequest>(payload).is_err());
    }

    #[test]
    fn delete_persistent_tap_requires_opaque_id_and_both_generation_fences() {
        let mut payload = serde_json::json!({
            "attachmentId": "123e4567-e89b-42d3-a456-426614174000",
            "expectedNetworkGeneration": 7,
            "expectedAttachmentGeneration": 11,
        });
        let request: DeletePersistentTapRequest =
            serde_json::from_value(payload.clone()).expect("valid fenced tap deletion request");
        assert_eq!(
            request.attachment_id.as_str(),
            "123e4567-e89b-42d3-a456-426614174000"
        );
        assert_eq!(request.expected_network_generation.get(), 7);
        assert_eq!(request.expected_attachment_generation.get(), 11);

        for required in [
            "attachmentId",
            "expectedNetworkGeneration",
            "expectedAttachmentGeneration",
        ] {
            let mut missing = payload.clone();
            missing.as_object_mut().unwrap().remove(required);
            assert!(serde_json::from_value::<DeletePersistentTapRequest>(missing).is_err());
        }

        payload
            .as_object_mut()
            .unwrap()
            .insert("vmId".to_owned(), serde_json::json!("corp-vm"));
        assert!(serde_json::from_value::<DeletePersistentTapRequest>(payload).is_err());
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
        // are forbidden - deny_unknown_fields traps them.
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

    fn spawn_runner_for_profile(role: RunnerRole, execution_ref: &str) -> BrokerRequest {
        BrokerRequest::SpawnRunner(SpawnRunnerRequest {
            vm_id: VmId::new("guest-vm"),
            role_id: RoleId::new(role.as_str()),
            resource_ref: None,
            resource_uid: None,
            bundle_content_identity: None,
            provider_identity: None,
            template_identity: None,
            generation: None,
            activation_input: None,
            sandbox_plan: None,
            role,
            bundle_runner_intent_ref: BundleOpId::new("runner:test"),
            execution_ref: Some(ResourceRef::parse(execution_ref).expect("valid execution ref")),
            execution_domain: None,
            user_ref: None,
            guest_execution: None,
            runtime_allocations: Vec::new(),
            tracing_span_id: None,
            workload_identity: None,
        })
    }

    #[test]
    fn profile_spawn_runner_role_matrix_is_closed() {
        // No current RunnerRole is Guest-local. Guest Process resources use
        // the dedicated local effect classes instead; a future runner role
        // must be added to this allowlist deliberately.
        let guest_local_roles: &[RunnerRole] = &[];
        let all_roles = [
            RunnerRole::CloudHypervisor,
            RunnerRole::QemuMedia,
            RunnerRole::Virtiofsd,
            RunnerRole::Swtpm,
            RunnerRole::SwtpmFlush,
            RunnerRole::Gpu,
            RunnerRole::Audio,
            RunnerRole::Video,
            RunnerRole::VsockRelay,
            RunnerRole::Usbip,
            RunnerRole::OtelHostBridge,
            RunnerRole::WaylandProxy,
        ];

        for role in all_roles {
            let expected_guest = guest_local_roles.contains(&role);
            assert_eq!(
                spawn_runner_for_profile(role, "Guest/guest-vm")
                    .allowed_by_profile(BrokerProfile::Guest),
                expected_guest,
                "Guest profile role {} must match the closed Guest-local allowlist",
                role.as_str()
            );
            assert!(
                !spawn_runner_for_profile(role, "Host/host")
                    .allowed_by_profile(BrokerProfile::Guest),
                "Guest profile must reject Host execution ref for {}",
                role.as_str()
            );
            assert!(
                !spawn_runner_for_profile(role, "Guest/guest-vm")
                    .allowed_by_profile(BrokerProfile::Host),
                "Host profile must reject Guest execution ref {}",
                role.as_str()
            );
            assert!(
                spawn_runner_for_profile(role, "Host/host").allowed_by_profile(BrokerProfile::Host),
                "Host profile behavior changed for Host execution ref {}",
                role.as_str()
            );
        }
    }

    #[test]
    fn guest_profile_requires_a_complete_target_execution_binding() {
        let mut request = spawn_runner_for_profile(
            RunnerRole::ActivationNixos,
            "Guest/guest-vm",
        );
        request = match request {
            BrokerRequest::SpawnRunner(mut request) => {
                request.guest_execution = Some(GuestExecutionBinding {
                    target_uid: ResourceUid::parse(
                        "123e4567-e89b-42d3-a456-426614174000",
                    )
                    .expect("Guest UID"),
                    boot_identity_digest: [7; 32],
                    session_generation: 2,
                    assignment_epoch: 3,
                    provider_generation: 4,
                    controller_generation: 5,
                });
                BrokerRequest::SpawnRunner(request)
            }
            _ => unreachable!("helper always builds SpawnRunner"),
        };
        assert!(request.allowed_by_profile(BrokerProfile::Guest));
        assert!(!request.allowed_by_profile(BrokerProfile::Host));

        let BrokerRequest::SpawnRunner(mut invalid) = request else {
            unreachable!("helper always builds SpawnRunner");
        };
        invalid.guest_execution.as_mut().unwrap().assignment_epoch = 0;
        assert!(!BrokerRequest::SpawnRunner(invalid).allowed_by_profile(BrokerProfile::Guest));
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
    fn cgroup_kill_request_round_trips() {
        let frame = encode_frame(&serde_json::json!({
            "kind": "CgroupKill",
            "payload": {
                "vmId": "corp-vm",
                "roleId": "ch-runner"
            }
        }))
        .expect("encodes");
        let decoded = decode_frame::<BrokerRequest>("BrokerRequest", &frame).expect("decodes");
        match decoded {
            BrokerRequest::CgroupKill(req) => {
                assert_eq!(req.vm_id.as_str(), "corp-vm");
                assert_eq!(req.role_id.as_str(), "ch-runner");
            }
            other => panic!("expected CgroupKill, got {other:?}"),
        }
    }

    #[test]
    fn user_manager_check_round_trips() {
        let frame = encode_frame(&serde_json::json!({
            "kind": "CheckSystemdUserManager",
            "payload": {
                "vmId": "guest-vm",
                "roleId": "audio",
                "role": "audio",
                "bundleRunnerIntentRef": "intent",
                "bundleContentIdentity": "bundle",
                "providerIdentity": vec![1_u8; 32],
                "templateIdentity": vec![2_u8; 32],
                "generation": 3,
                "domain": "user"
            }
        }))
        .expect("encodes");
        let decoded = decode_frame::<BrokerRequest>("BrokerRequest", &frame).expect("decodes");
        assert!(matches!(
            decoded,
            BrokerRequest::CheckSystemdUserManager(request)
                if request.domain == SystemdUnitDomain::User
        ));
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
            console_fd_index: None,
            execution_ref: None,
            execution_domain: None,
            user_ref: None,
            guest_execution: None,
            provider_identity: None,
            template_identity: None,
            generation: None,
            bundle_content_identity: None,
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
