//! Broker wire contract.
//!
//! Every mutating variant carries **only opaque identifiers** +
//! bundle-resolved intent refs. The daemon never names a raw path, a
//! raw nft rule text, a raw route spec, a raw sysctl key/value, a raw
//! ifname set outside the exact Network admission proof, a raw `/etc/hosts`
//! entry list, a raw uid/gid, raw argv
//! or env, raw caps, or a raw seccomp profile path. The broker uses
//! the opaque IDs to look up the typed intent in its own trusted bundle
//! copy. See `d2b_contracts::types` for the newtype set.

use d2b_contracts::audit_wire::validate_audit_page;
pub use d2b_contracts::audit_wire::{AuditExportCursor, AuditExportEntry, AuditExportErrorCode};
use d2b_contracts::types::{
    BundleClosureRef, BundleOpId, MediaRef, PathClass, RoleId, ScopeId, SubjectId, TracingSpanId,
    VmId,
};
use d2b_contracts::workload_identity::WorkloadIdentity;
use d2b_contracts_resource::v3::process::{
    CapabilityClass, EnvironmentClass, NamespaceClass, UserNamespaceSpec,
};
use d2b_contracts_resource::v3::{
    ActivationRunnerInput, IfName, ResourceBundleGenerationId, ResourceGeneration, ResourceRef,
    ResourceUid, execution_policy::ExecutionDomain,
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
    /// One daemon publication of the trusted-context values it currently
    /// holds for one Zone.
    ///
    /// The daemon owns the provider-set revision and the controller/guest
    /// generations, so it publishes them over the same origination leg its
    /// other broker operations use; the broker caches them as durable,
    /// monotonically increasing state and refuses to mint a context block
    /// until it holds a value for the Zone a call names. The reply carries
    /// the broker-epoch nonce every context minted from that point on
    /// carries.
    PublishTrustedContext(PublishTrustedContextValues),
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
    OpenVhostNet(OpenVhostNetRequest),
    ReconcileStorageScope(ReconcileStorageScopeRequest),
    ValidateLockSpec(ValidateLockSpecRequest),
    /// Typed broker op that hardlink-farms a Guest's resolved closure into
    /// its Zone-qualified store view and atomically swaps the
    /// `current` symlink. Replaces the retired per-VM
    /// `d2b-<vm>-store-sync.service` bash oneshot. The daemon names
    /// only the opaque `bundle_closure_ref` + `vm_id` + expected
    /// `generation_token`; the broker re-derives every closure path from
    /// its trusted bundle copy and derives the collision-free on-disk
    /// `generation_id` itself.
    StoreSync(StoreSyncRequest),
    ReadSecretById(SecretByIdRequest),
    RotateSecretById(SecretByIdRequest),
    SetBridgePortFlags(SetBridgePortFlagsRequest),
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
    /// Write the per-VM dnsmasq lease file. Replaces leaves of the
    /// retired `microvm-setup@<vm>.service`. Currently a typed stub
    /// (`Unimplemented`) until the live handler is wired.
    ///
    /// Live handler target: `live_seed_dnsmasq_lease`, resolved through
    /// `BundleResolver` from the per-VM dnsmasq lease row.
    SeedDnsmasqLease(SeedDnsmasqLeaseRequest),
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
    /// Invoke one committed operation through the broker's generic
    /// operation envelope (U10, KTD10).
    ///
    /// This is the generic invocation surface of the retirement template: a
    /// caller names a committed operation and carries the canonical payload
    /// the row's schema admits, and the broker runs the envelope's five
    /// steps - resolve the committed row, authorize the caller against the
    /// row's grants, validate the payload, audit with an invocation id, and
    /// dispatch - in place of one typed dispatch arm per operation. A root
    /// call carries no evidence chain and is authorized as the attested
    /// caller's own class; a handler's nested (sandwich) call carries the
    /// chain it was dispatched under and is authorized against the chain's
    /// initiating principal (KTD6). The reply rides
    /// [`BrokerResponse::EnvelopeInvoke`] with any descriptors the
    /// dispatch minted attached via SCM_RIGHTS.
    ///
    /// The variant replaces per-operation wire variants as their typed arms
    /// retire; a retired variant's name stays gated on the Hello-negotiated
    /// wire version (KTD10) so a straggler peer gets the stale-wire-version
    /// refusal plus an audit record, never a silent malformed-wire drop.
    EnvelopeInvoke(EnvelopeInvokeRequest),
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

/// The environment variable that names the socket a producer of forwarded
/// operations dials.
///
/// One deployment fact, declared once: the broker reads it to find the peer
/// that serves declared handlers, and the daemon that owns the endpoint binds
/// that path. Neither side may invent a second spelling.
pub const FORWARD_SOCKET_ENV: &str = "D2B_BROKER_FORWARD_SOCKET";

/// The refusal code for a forwarded request or response whose fd
/// attachments disagree with their declarations, or whose declared set
/// exceeds the bounded ceiling.
///
/// The code is shared by both legs of the forward carrier,so the broker
/// and the rendezvous cannot drift apart on how an fd-leg failure is named.

pub const FD_LEG: &str = "fd-leg";

/// The most SCM_RIGHTS descriptors one forward frame can carry.


///
/// The receive-side ancillary buffers on both legs are sized
/// `cmsg_space!([RawFd; MAX_FRAME_FDS])`, so a frame with more attachments
/// would be truncated by the transport. A declared set is therefore
/// capped at this constant before dispatch,and a larger declaration is
/// refused with [`FD_LEG`], never delivered as a transport truncation。


pub const MAX_FRAME_FDS: usize =8;

/// The kernel kind one forwarded descriptor must present.from
///
/// The kind is declared per descriptor on the wire,index-aligned with the
/// fd-index declarations,and validated against the received descriptor's
/// fstat mode on the receiving leg;a mismatch is the [`FD_LEG`] refusal。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum FdKind {
    /// A FIFO (pipe) end。



    Fifo,
    /// A socket。



    Socket,
    /// A character device。





    CharDevice,
    /// A block device。





    BlockDevice,
    /// Any descriptor kind.
    ///
    /// Declared by an operation whose leg carries a mixed or
    /// anon-inode set (an fd-inheriting spawn, or descriptors whose
    /// fstat reports no named kind). Admission treats a declared
    /// `Any` as accepting every attached descriptor regardless of
    /// fstat kind - including anon-inodes such as pidfds, whose
    /// fstat mode carries no file type (U10 fd leg).



    Any,
    /// A regular file。







    Regular,
    /// A directory。






    Directory,
}

/// The refusal code for a forwarded request whose broker-attested context
/// block is stale or does not match the values the daemon currently holds.
///
/// The broker is the sole minter of the context block, so every mismatch
/// this code names is a freshness failure or tampering, never a legitimate
/// re-spelling. The code is shared by both legs of the forward carrier: the
/// broker refuses to mint until it holds the daemon's published values, and
/// the rendezvous refuses a request whose context's broker epoch, Zone,
/// provider-set revision, controller generation, or guest generation does
/// not match its own current values.

pub const STALE_CONTEXT: &str = "stale-context";

/// The deadline budget a minted context carries when the operation's
/// committed row declares no tier.
///
/// The value is the rendezvous's historical fixed handler deadline (25 s),
/// carried on the context block instead so that a context-carrying
/// deployment keeps the same per-call bound while the row-level deadline-
/// tier facet (KTD4) is wired; the receiving leg serves the budget the
/// context declares, not a private constant.

pub const DEFAULT_CONTEXT_DEADLINE_MS: u64 = 25_000;

/// The absolute ceiling a context's deadline budget must sit under.
///
/// The budget is broker-minted, so a budget over this ceiling means the
/// block was not minted as the broker wrote it; the receiving leg refuses
/// the call with [`STALE_CONTEXT`] rather than serve an unbounded or
/// oversized handler grant.

pub const MAX_CONTEXT_DEADLINE_MS: u64 = 60_000;

/// The broker-attested context block riding one forwarded request.
///
/// The broker is the sole minter. The block names the authenticating value
/// that binds the call to the attestation (`broker_epoch`: any context
/// minted before a broker restart fails it regardless of generation
/// equality), the Zone and the daemon-owned generational state the broker
/// cached from the daemon's publications (provider-set revision, controller
/// and guest generations), the initiating identity of the call as the
/// broker classified it, and the operation's deadline budget in
/// milliseconds, which the receiving leg serves as the per-call handler
/// deadline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForwardContext {
    /// The broker-epoch nonce the broker minted this block under. A broker
    /// restart bumps it, so every block minted before the restart fails the
    /// rendezvous's epoch check regardless of generation equality.
    pub broker_epoch: u64,
    /// The Zone the invocation runs in, bound to the connection's Zone by
    /// the receiving leg.
    pub zone: String,
    /// The provider-set revision the broker cached from the daemon's
    /// publication for this Zone.
    pub provider_set_revision: u64,
    /// The controller generation the broker cached from the daemon's
    /// publication for this Zone.
    pub controller_generation: u64,
    /// The guest generation the broker cached from the daemon's publication
    /// for this Zone.
    pub guest_generation: u64,
    /// The initiating identity as the broker classified the caller.
    pub initiating_identity: String,
    /// The operation's deadline budget, in milliseconds, served by the
    /// receiving leg as the per-call handler deadline.
    pub deadline_ms: u64,
}

/// The daemon-owned values one publication carries over the established
/// origination leg.
///
/// Provider-set revision and the controller/guest generations are daemon
/// state, so the daemon publishes its current values to the broker over the
/// leg its operations already originate on; the broker caches them as
/// durable, monotonically increasing state and refuses to mint a context
/// until it holds a value for the Zone the call names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishTrustedContextValues {
    /// The Zone the published values describe.
    pub zone: String,
    /// The Zone's current provider-set revision.
    pub provider_set_revision: u64,
    /// The Zone's current controller generation.
    pub controller_generation: u64,
    /// The Zone's current guest generation.
    pub guest_generation: u64,
}

/// The broker's acknowledgement of one [`PublishTrustedContextValues`].
///
/// The reply carries the broker-epoch nonce the broker is currently minting
/// with, so the daemon's rendezvous can refuse every context minted before
/// a broker restart: the epoch it observed stops matching the moment the
/// broker reopens its store and starts minting under a fresh nonce.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishTrustedContextResponse {
    /// The broker-epoch nonce the broker is currently minting with.
    pub broker_epoch: u64,
}

/// One validated, authorized operation forwarded to the process that
/// declares it.
///
/// The broker holds the committed rows and the declaring crate holds the
/// handler, so the dispatch step of the operation envelope forwards rather
/// than serving a family row locally. The payload crosses as the canonical
/// object the envelope already validated against the row's declared shape:
/// the receiving process re-derives no authority from it and the broker
/// never forwards a payload it did not admit.
///
/// The invocation identifier travels with the payload so the peer's record
/// and the broker's record name the same invocation.
///
/// The broker-minted context block rides beside the payload when the broker
/// holds a context store; a context-free carrier stays the pre-attestation
/// mode, and the receiving leg refuses a context it cannot validate with
/// [`STALE_CONTEXT`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForwardOperationRequest {
    /// The committed operation name the call resolved, exactly as the
    /// committed row declares it.
    pub operation: String,
    /// The Zone the invocation runs in.
    pub zone: String,
    /// The invocation identifier the broker's audit record carries.
    pub invocation_id: String,
    /// The canonical payload object the row validated.
    pub payload: serde_json::Value,
    /// The broker-attested context block the broker minted for this
    /// invocation, when the broker holds a context store. Absent on a
    /// context-free carrier; the receiving leg refuses a present block it
    /// cannot validate field-wise with [`STALE_CONTEXT`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<ForwardContext>,
    /// The evidence chain's ordered identities, root first, when this
    /// forwarded invocation is a nested leg of an existing invocation
    /// (U10, KTD6). Absent for a root call. The root invocation id rides
    /// in [`Self::invocation_id`], so the receiving leg re-roots the chain
    /// from the two fields and records the leg as a correlation record
    /// rather than a second root record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_identities: Option<Vec<String>>,
    /// The positions,in the frame's SCM_RIGHTS attachment list,of the
    /// descriptors this request carries. Empty when the request carries none.


    #[serde(default)]
    pub fd_indexes: Vec<u32>,
    /// The kernel kind each declared descriptor must present,index-aligned
    /// with [`Self::fd_indexes`]。

    #[serde(default)]
    pub fd_kinds: Vec<FdKind>,
}

/// How one forwarded invocation ended.
///
/// A refusal keeps the two outcomes apart that a boolean or a unit reply
/// would merge: no process serves the operation (the peer never answers, or
/// answers [`ForwardOperationOutcome::Refused`] with the row's own code),
/// and a served operation that failed carries its own refusal code rather
/// than being reported as an unregistered handler.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ForwardOperationOutcome {
    /// The declared handler ran and returned its canonical result.
    Result {
        /// The canonical result payload the handler returned.
        result: serde_json::Value,
        /// The positions,in the frame's SCM_RIGHTS attachment list,of the
        /// descriptors the answering peer returned. Empty when the response
        /// carries none.


        #[serde(default)]
        fd_indexes: Vec<u32>,
        /// The kernel kind each declared descriptor must present,index-aligned
        /// with the fd-index declarations.


        #[serde(default)]
        fd_kinds: Vec<FdKind>,
    },
    /// The invocation reached no handler, or the handler refused it.
    Refused {
        /// The closed refusal code the refusing process decided.
        code: String,
    },
}

/// The reply to one [`ForwardOperationRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForwardOperationResponse {
    /// How the forwarded invocation ended.
    pub outcome: ForwardOperationOutcome,
}

/// One generic envelope invocation the daemon or a provider handler sends
/// over the origination leg (U10, KTD10).
///
/// The operation names a committed row exactly as the catalog declares it;
/// the payload is the canonical object the envelope validates against the
/// row's declared shape. A root call (a driver invoking a service) carries
/// no chain; a nested call (a provider handler's sandwich leg reaching a
/// broker-generic core) carries the evidence chain it was dispatched under,
/// so the graft rule authorizes the call against the chain's initiating
/// principal and the DB-side records the correlation leg (KTD6).
///
/// The chain crosses as its two parts - the root invocation id and the
/// ordered identities - so the contract crate needs no evidence-chain
/// dependency; the broker reassembles the chain before the envelope
/// admission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvelopeInvokeRequest {
    /// The committed operation name the call resolves to, exactly as the
    /// committed row declares it.
    pub operation: String,
    /// The Zone the invocation runs in.
    pub zone: String,
    /// The canonical payload object the envelope validates against the
    /// row's declared shape.
    pub payload: serde_json::Value,
    /// The root invocation id of the evidence chain a nested call
    /// presents; absent for a root call.
    pub chain_root_invocation_id: Option<String>,
    /// The ordered chain identities, root first; present exactly when
    /// [`Self::chain_root_invocation_id`] is, and never empty then (the
    /// first identity is the initiating principal).
    pub chain_identities: Option<Vec<String>>,
    /// The SCM_RIGHTS request attachments' indexes, in frame order.
    pub fd_indexes: Vec<u32>,
    /// The kernel kinds of the attached descriptors, in frame order.
    pub fd_kinds: Vec<FdKind>,
}

/// The reply to one [`BrokerRequest::EnvelopeInvoke`].
///
/// A success carries the canonical result the dispatch returned plus the
/// descriptors the answering leg minted (via the response frame's
/// SCM_RIGHTS attachments); a refusal carries the envelope's closed
/// refusal code and its detail. Both carry the invocation id the audit
/// record keys on, so a caller can join the reply to the audit log either
/// way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvelopeInvokeResponse {
    /// The committed operation name that ran.
    pub operation: String,
    /// The invocation identifier the audit record carries.
    pub invocation_id: String,
    /// The canonical result object, present exactly when [`Self::refusal`]
    /// is absent.
    pub result: Option<serde_json::Value>,
    /// The closed envelope refusal code, present exactly when the
    /// invocation was refused.
    pub refusal: Option<String>,
    /// Detail of the refusal, when the refusing side contributed one.
    pub detail: Option<String>,
    /// The response frame's SCM_RIGHTS attachments this result owns, by
    /// index.
    pub fd_indexes: Vec<u32>,
    /// The kernel kinds of the returned descriptors, in frame order.
    pub fd_kinds: Vec<FdKind>,
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
            Self::ApplyNftables(_) => "ApplyNftables",
            Self::ApplyNftablesProjection(_) => "ApplyNftablesProjection",
            Self::ApplyNmUnmanaged(_) => "ApplyNmUnmanaged",
            Self::ApplyRoute(_) => "ApplyRoute",
            Self::ApplySysctl(_) => "ApplySysctl",
            Self::CreateOrReconcileUsersGroups(_) => "CreateOrReconcileUsersGroups",
            Self::CreateBridge(_) => "CreateBridge",
            Self::DeleteBridge(_) => "DeleteBridge",
            Self::CreatePersistentTap(_) => "CreatePersistentTap",
            Self::DeletePersistentTap(_) => "DeletePersistentTap",
            Self::CreateTapFd(_) => "CreateTapFd",
            Self::DelegateCgroupV2(_) => "DelegateCgroupV2",
            Self::ExportBrokerAudit(_) => "ExportBrokerAudit",
            Self::Hello(_) => "Hello",
            Self::PublishTrustedContext(_) => "PublishTrustedContext",
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
            Self::PipeWireAudio(_) => "PipeWireAudio",
            Self::StartSystemdUnit(_) => "StartSystemdUnit",
            Self::CheckSystemdUserManager(_) => "CheckSystemdUserManager",
            Self::ObserveSystemdUnit(_) => "ObserveSystemdUnit",
            Self::OpenSystemdUnitPidfd(_) => "OpenSystemdUnitPidfd",
            Self::StopSystemdUnit(_) => "StopSystemdUnit",
            Self::OpenVhostNet(_) => "OpenVhostNet",
            Self::ReconcileStorageScope(_) => "ReconcileStorageScope",
            Self::ValidateLockSpec(_) => "ValidateLockSpec",
            Self::StoreSync(_) => "StoreSync",
            Self::ReadSecretById(_) => "ReadSecretById",
            Self::RotateSecretById(_) => "RotateSecretById",
            Self::SetBridgePortFlags(_) => "SetBridgePortFlags",
            Self::UpdateHostsFile(_) => "UpdateHostsFile",
            Self::UsbipBind(_) => "UsbipBind",
            Self::UsbipBindFirewallRule(_) => "UsbipBindFirewallRule",
            Self::UsbipProxyReconcile(_) => "UsbipProxyReconcile",
            Self::UsbipUnbind(_) => "UsbipUnbind",
            Self::UsbipExplicitBind(_) => "UsbipExplicitBind",
            Self::UsbipExplicitFirewallRule(_) => "UsbipExplicitFirewallRule",
            Self::SeedDnsmasqLease(_) => "SeedDnsmasqLease",
            Self::OwnershipMatrixCheck(_) => "OwnershipMatrixCheck",
            Self::SshHostKeyPreflight(_) => "SshHostKeyPreflight",
            Self::DiskInit(_) => "DiskInit",
            Self::SecurityKeyOpenDevice(_) => "SecurityKeyOpenDevice",
            Self::SecurityKeyApplyUdevRules(_) => "SecurityKeyApplyUdevRules",
            Self::EnvelopeInvoke(_) => "EnvelopeInvoke",
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
            Self::PublishTrustedContext(_) => "trusted-context",
            Self::ExportBrokerAudit(_) => "audit-log",
            Self::EnvelopeInvoke(_) => "envelope",
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
            Self::ReconcileStorageScope(request) => (
                request.storage_ref.to_string(),
                format!("{}:{}", self.op_name(), request.storage_ref),
            ),
            Self::ValidateLockSpec(request) => (
                request.lock_ref.to_string(),
                format!("{}:{}", self.op_name(), request.lock_ref),
            ),
            Self::StoreSync(request) => (
                request.bundle_closure_ref.to_string(),
                format!(
                    "{}:{}:{}:{}",
                    self.op_name(),
                    request.vm_id,
                    request.bundle_closure_ref,
                    request.generation_token
                ),
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
            Self::SetBridgePortFlags(request) => (
                request.vm_id.to_string(),
                format!("{}:{}:{}", self.op_name(), request.vm_id, request.role_id),
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
            Self::ExportBrokerAudit(_)
            | Self::Hello(_)
            | Self::PublishTrustedContext(_)
            | Self::EnvelopeInvoke(_) => return None,
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
                | Self::ExportBrokerAudit(_)
                | Self::Hello(_)
                | Self::PublishTrustedContext(_)
                | Self::EnvelopeInvoke(_)
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
                BrokerRequest::StartSystemdUnit(request)
                | BrokerRequest::ObserveSystemdUnit(request)
                | BrokerRequest::CheckSystemdUserManager(request) => {
                    request
                        .execution_ref
                        .as_ref()
                        .is_some_and(|target| target.resource_type().as_str() == "Guest")
                        && request
                            .guest_execution
                            .as_ref()
                            .is_some_and(GuestExecutionBinding::is_valid)
                }
                BrokerRequest::OpenSystemdUnitPidfd(request) => {
                    request
                        .unit
                        .execution_ref
                        .as_ref()
                        .is_some_and(|target| target.resource_type().as_str() == "Guest")
                        && request
                            .unit
                            .guest_execution
                            .as_ref()
                            .is_some_and(GuestExecutionBinding::is_valid)
                }
                BrokerRequest::StopSystemdUnit(request) => {
                    request
                        .unit
                        .execution_ref
                        .as_ref()
                        .is_some_and(|target| target.resource_type().as_str() == "Guest")
                        && request
                            .unit
                            .guest_execution
                            .as_ref()
                            .is_some_and(GuestExecutionBinding::is_valid)
                }
                _ => true,
            },
        }
    }

    fn request_targets_guest(request: &BrokerRequest) -> bool {
        match request {
            BrokerRequest::StartSystemdUnit(request)
            | BrokerRequest::ObserveSystemdUnit(request)
            | BrokerRequest::CheckSystemdUserManager(request) => {
                request
                    .execution_ref
                    .as_ref()
                    .is_some_and(|target| target.resource_type().as_str() == "Guest")
                    || request.guest_execution.is_some()
            }
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
            _ => false,
        }
    }
}

include!("generated/broker_operation_profiles.rs");

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
    /// Daemon ↔ broker handshake confirmation response. Returned in
    /// reply to a `BrokerRequest::Hello` so the daemon can
    /// capability-negotiate and the broker can audit the connection
    /// without a separate side-channel.
    Hello(HelloResponse),
    /// Acknowledgement of one [`BrokerRequest::PublishTrustedContext`].
    /// Carries the broker-epoch nonce the broker is currently minting
    /// with, so the daemon's receiving leg can refuse every context
    /// minted before a broker restart.
    PublishTrustedContext(PublishTrustedContextResponse),
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
    ReconcileStorageScope(ReconcileStorageScopeResponse),
    SetBridgePortFlags(BridgePortFlagsResponse),
    /// Typed response carrying the activated generation (collision-free
    /// `generation_id` plus the u32 `generation_token`), the resolved
    /// hardlink-farm root, and the count of top-level closure paths
    /// populated. Used by the daemon to surface the swap result in audit
    /// + start traces.
    StoreSync(StoreSyncResponse),
    ValidateLockSpec(ValidateLockSpecResponse),
    /// The reply to one generic envelope invocation
    /// ([`BrokerRequest::EnvelopeInvoke`]): the dispatch's canonical
    /// result or its closed refusal, plus any descriptors the dispatching
    /// leg minted via the response frame's SCM_RIGHTS attachments.
    EnvelopeInvoke(EnvelopeInvokeResponse),
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
    pub zone_uid: ResourceUid,
    pub network_uid: ResourceUid,
    pub network_generation: ResourceGeneration,
    pub attachment_generation: ResourceGeneration,
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
    pub zone_uid: ResourceUid,
    pub network_uid: ResourceUid,
    pub network_generation: ResourceGeneration,
    pub attachment_generation: ResourceGeneration,
    pub bundle_generation: ResourceBundleGenerationId,
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
    pub zone_uid: ResourceUid,
    pub network_uid: ResourceUid,
    pub network_generation: ResourceGeneration,
    pub attachment_generation: ResourceGeneration,
    pub bundle_generation: ResourceBundleGenerationId,
    #[serde(default)]
    pub destroy: bool,
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
/// attributes from the admitted Network identity. Every field in the
/// provenance tuple and the exact admitted interface set is mandatory; there
/// is no legacy env/manifest path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatePersistentTapRequest {
    pub role_id: RoleId,
    pub vm_id: VmId,
    /// Opaque TAP identity bound to the complete Network tuple below.
    pub bundle_tap_intent_ref: BundleOpId,
    pub attachment_id: ResourceUid,
    pub network_generation: ResourceGeneration,
    pub attachment_generation: ResourceGeneration,
    pub zone_uid: ResourceUid,
    pub network_uid: ResourceUid,
    pub bundle_generation: ResourceBundleGenerationId,
    /// Exact interface set copied from the live Network admission proof.
    pub admitted_interface_names: Vec<IfName>,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// Delete one trusted attachment realization without accepting its ifname,
/// path, or ownership marker from the caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeletePersistentTapRequest {
    pub attachment_id: ResourceUid,
    /// Exact Zone/Network identity and installed bundle generation required
    /// for deletion; none are accepted from a caller as a wildcard.
    pub expected_zone_uid: ResourceUid,
    pub expected_network_uid: ResourceUid,
    pub expected_network_generation: ResourceGeneration,
    pub expected_attachment_generation: ResourceGeneration,
    pub expected_bundle_generation: ResourceBundleGenerationId,
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
    pub zone_uid: ResourceUid,
    pub network_uid: ResourceUid,
    pub network_generation: ResourceGeneration,
    pub attachment_generation: ResourceGeneration,
    pub bundle_generation: ResourceBundleGenerationId,
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
    pub zone_uid: ResourceUid,
    pub network_uid: ResourceUid,
    pub network_generation: ResourceGeneration,
    pub attachment_generation: ResourceGeneration,
    pub bundle_generation: ResourceBundleGenerationId,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// See [`CreatePersistentTapRequest`] for the provenance contract;
/// `CreateTapFd` carries the same complete identity tuple and interface proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateTapFdRequest {
    pub role_id: RoleId,
    pub vm_id: VmId,
    /// Opaque TAP identity bound to the complete Network tuple below.
    pub bundle_tap_intent_ref: BundleOpId,
    pub attachment_id: ResourceUid,
    pub network_generation: ResourceGeneration,
    pub attachment_generation: ResourceGeneration,
    pub zone_uid: ResourceUid,
    pub network_uid: ResourceUid,
    pub bundle_generation: ResourceBundleGenerationId,
    /// Exact interface set copied from the live Network admission proof.
    pub admitted_interface_names: Vec<IfName>,
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
        !self.target_uid.as_str().is_empty()
            && self.boot_identity_digest != [0; 32]
            && self.session_generation > 0
            && self.assignment_epoch > 0
            && self.provider_generation > 0
            && self.controller_generation > 0
    }
}

/// OpenPidfd daemon-side reconcile-and-adopt support. The daemon's
/// `d2bd_runtime::supervisor::state::reconcile_and_adopt` loop sends this
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
    /// Exact trusted runner intent for typed Process adoption.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_runner_intent_ref: Option<BundleOpId>,
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
    /// Immutable Zone identity for typed Process adoption.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone_uid: Option<ResourceUid>,
    /// Exact semantic owner of a typed Process resource, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_ref: Option<ResourceRef>,
    /// Selected Process Provider reference for typed Process adoption.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_ref: Option<ResourceRef>,
    /// Provider identity commitment for typed Process adoption.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_identity: Option<[u8; 32]>,
    /// Template identity commitment for typed Process adoption.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_identity: Option<[u8; 32]>,
    /// Desired Process generation for the private runtime scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
    /// Broker-independent commitment to the private runtime scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_scope: Option<[u8; 32]>,
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
    /// Broker-retained Provider-controller bootstrap endpoint, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller_bootstrap_fd_index: Option<u32>,
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
    /// Immutable Zone identity for typed Process observation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone_uid: Option<ResourceUid>,
    /// Exact semantic owner of a typed Process resource, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_ref: Option<ResourceRef>,
    /// Selected Process Provider reference for typed Process observation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_ref: Option<ResourceRef>,
    /// Provider identity commitment for typed Process observation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_identity: Option<[u8; 32]>,
    /// Template identity commitment for typed Process observation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_identity: Option<[u8; 32]>,
    /// Desired Process generation for the private runtime scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
    /// Broker-independent commitment to the private runtime scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_scope: Option<[u8; 32]>,
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

/// Store-sync request. The broker resolves the closure intent row from
/// the opaque `bundle_closure_ref`, verifies that it belongs to the
/// plain `vm_id`, and refuses the op if either identity does not match.
/// The broker also
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

/// The broker derives the bridge, port, and flag set from the complete
/// admitted Network identity. Legacy callers without that context are
/// refused before any link mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetBridgePortFlagsRequest {
    pub vm_id: VmId,
    pub role_id: RoleId,
    /// Complete admitted Network identity for a Network-owned port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_tap_context: Option<NetworkTapContext>,
    #[serde(default)]
    pub tracing_span_id: Option<TracingSpanId>,
}

/// The managed-block lines come from the bundle's `host::HostsEntry`
/// rows, not the wire. The broker only needs the lookup key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateHostsFileRequest {
    pub bundle_hosts_intent_ref: BundleOpId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone_uid: Option<ResourceUid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_uid: Option<ResourceUid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_generation: Option<ResourceGeneration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_generation: Option<ResourceGeneration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_generation: Option<ResourceBundleGenerationId>,
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

/// Runner-signal broker envelope. The live daemon stop/restart path first
/// delivers signals through `d2bd_runtime::supervisor::pidfd_table` after
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
    /// Immutable Zone identity for typed Process control.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone_uid: Option<ResourceUid>,
    /// Exact semantic owner of a typed Process resource, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_ref: Option<ResourceRef>,
    /// Selected Process Provider reference for typed Process control.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_ref: Option<ResourceRef>,
    /// Provider identity commitment for typed Process cleanup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_identity: Option<[u8; 32]>,
    /// Template identity commitment for typed Process cleanup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_identity: Option<[u8; 32]>,
    /// Desired Process generation for the private runtime scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
    /// Broker-independent commitment to the private runtime scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_scope: Option<[u8; 32]>,
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
    /// Immutable Zone identity for typed Process cleanup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone_uid: Option<ResourceUid>,
    /// Exact semantic owner of a typed Process resource, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_ref: Option<ResourceRef>,
    /// Selected Process Provider reference for typed Process cleanup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_ref: Option<ResourceRef>,
    /// Provider identity commitment for typed Process cleanup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_identity: Option<[u8; 32]>,
    /// Template identity commitment for typed Process cleanup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_identity: Option<[u8; 32]>,
    /// Desired Process generation for the private runtime scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
    /// Broker-independent commitment to the private runtime scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_scope: Option<[u8; 32]>,
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
    /// External Provider controller with one inherited bootstrap descriptor.
    ProviderController,
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
    /// `d2b_provider_audio_pipewire::argv`.
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
            Self::ProviderController => "provider-controller",
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

/// Controller-supplied arguments for one typed Process launch.
///
/// The executor never accepts an `argv[0]`: the executable is always the
/// trusted intent's `binary_path`. This type only carries the arguments the
/// owning Process controller appends after it, and only for templates whose
/// trusted bundle row declares that it admits controller arguments. Bounds
/// mirror the spawn preflight so an argument the exec path cannot round-trip
/// is refused at the wire boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunnerLaunchArgs {
    args: Vec<String>,
}

impl RunnerLaunchArgs {
    /// Maximum number of supplied arguments.
    pub const MAX_ARGS: usize = 64;
    /// Maximum byte length of one argument.
    pub const MAX_ARG_BYTES: usize = 4096;
    /// Maximum combined byte length of every supplied argument.
    pub const MAX_TOTAL_BYTES: usize = 16 * 1024;

    /// Validate and construct one bounded argument vector.
    pub fn new(args: Vec<String>) -> Result<Self, RunnerLaunchArgsError> {
        if args.is_empty() {
            return Err(RunnerLaunchArgsError::Empty);
        }
        if args.len() > Self::MAX_ARGS {
            return Err(RunnerLaunchArgsError::TooMany { count: args.len() });
        }
        let mut total = 0usize;
        for (index, arg) in args.iter().enumerate() {
            if arg.is_empty() {
                return Err(RunnerLaunchArgsError::EmptyArgument { index });
            }
            if arg.contains('\0') {
                return Err(RunnerLaunchArgsError::ArgumentWithNul { index });
            }
            if arg.len() > Self::MAX_ARG_BYTES {
                return Err(RunnerLaunchArgsError::ArgumentTooLong { index });
            }
            total = total.saturating_add(arg.len());
        }
        if total > Self::MAX_TOTAL_BYTES {
            return Err(RunnerLaunchArgsError::TotalTooLong { bytes: total });
        }
        Ok(Self { args })
    }

    /// Borrow the validated arguments.
    pub fn as_slice(&self) -> &[String] {
        &self.args
    }
}

/// Closed refusal for one launch-argument vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerLaunchArgsError {
    /// The vector carried no arguments.
    Empty,
    /// More arguments than [`RunnerLaunchArgs::MAX_ARGS`].
    TooMany {
        /// Observed argument count.
        count: usize,
    },
    /// One argument was empty.
    EmptyArgument {
        /// Index of the offending argument.
        index: usize,
    },
    /// One argument contained a NUL byte the exec path cannot round-trip.
    ArgumentWithNul {
        /// Index of the offending argument.
        index: usize,
    },
    /// One argument exceeded [`RunnerLaunchArgs::MAX_ARG_BYTES`].
    ArgumentTooLong {
        /// Index of the offending argument.
        index: usize,
    },
    /// The combined size exceeded [`RunnerLaunchArgs::MAX_TOTAL_BYTES`].
    TotalTooLong {
        /// Observed combined byte length.
        bytes: usize,
    },
}

impl core::fmt::Display for RunnerLaunchArgsError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("launch-args-empty"),
            Self::TooMany { .. } => formatter.write_str("launch-args-too-many"),
            Self::EmptyArgument { .. } => formatter.write_str("launch-args-empty-argument"),
            Self::ArgumentWithNul { .. } => formatter.write_str("launch-args-nul"),
            Self::ArgumentTooLong { .. } => formatter.write_str("launch-args-argument-too-long"),
            Self::TotalTooLong { .. } => formatter.write_str("launch-args-total-too-long"),
        }
    }
}

impl std::error::Error for RunnerLaunchArgsError {}

impl<'de> Deserialize<'de> for RunnerLaunchArgs {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            args: Vec<String>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.args).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpawnRunnerRequest {
    /// VM scope the runner belongs to.
    pub vm_id: VmId,
    /// Per-VM role this runner fills. Legacy runners are unique on
    /// `(vm_id, role_id)`; typed Process runners are additionally fenced by
    /// their private runtime scope and resource incarnation.
    pub role_id: RoleId,
    /// Optional generic Process identity binding. These fields are part of
    /// the registry key and prevent distinct Process resources from
    /// colliding on the legacy VM/role tuple.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_ref: Option<ResourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_uid: Option<ResourceUid>,
    /// Immutable Zone identity for typed Process launch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone_uid: Option<ResourceUid>,
    /// Exact semantic owner of the Process resource, when one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_ref: Option<ResourceRef>,
    /// Immutable semantic-owner UID for owner-scoped runtime bootstrap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_uid: Option<ResourceUid>,
    /// Selected Process Provider reference for typed Process launch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_ref: Option<ResourceRef>,
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
    /// Broker-independent commitment to the private runtime scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_scope: Option<[u8; 32]>,
    /// Typed stdin input admitted only for the activation-nixos runner role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation_input: Option<ActivationRunnerInput>,
    /// Bounded controller-supplied arguments for a typed Process launch.
    ///
    /// The executable is never supplied here: the broker composes `argv[0]`
    /// from the trusted intent's `binary_path` and appends these arguments.
    /// Refused unless the resolved intent's template declares that it admits
    /// controller arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_args: Option<RunnerLaunchArgs>,
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
    /// Number of request-side inherited descriptors attached with SCM_RIGHTS.
    ///
    /// Provider controller launches carry exactly one bootstrap descriptor;
    /// every other runner launch carries zero.
    #[serde(default)]
    pub inherited_fd_count: u16,
    /// Complete admitted Network context required when this runner opens a
    /// VMM TAP through `CreateTapFd`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_tap_context: Option<NetworkTapContext>,
}

/// Admitted Network identity passed through a VMM runner launch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NetworkTapContext {
    pub zone_uid: ResourceUid,
    pub network_uid: ResourceUid,
    pub attachment_id: ResourceUid,
    pub network_generation: ResourceGeneration,
    pub attachment_generation: ResourceGeneration,
    pub bundle_generation: ResourceBundleGenerationId,
    /// Exact interface set copied from the live Network admission proof.
    pub admitted_interface_names: Vec<IfName>,
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
    /// Exact generic Process identity echoed after broker validation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_ref: Option<ResourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_uid: Option<ResourceUid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone_uid: Option<ResourceUid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_ref: Option<ResourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_scope: Option<[u8; 32]>,
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
    /// Provider-controller bootstrap endpoint created and retained by the broker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller_bootstrap_fd_index: Option<u32>,
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
    HostShutdownUid {
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
            Self::HostShutdownUid { .. } => "d2b-host-shutdown",
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
    pub zone_uid: ResourceUid,
    pub network_uid: ResourceUid,
    pub network_generation: ResourceGeneration,
    pub attachment_generation: ResourceGeneration,
    pub bundle_generation: ResourceBundleGenerationId,
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

/// The typed payload the `poll-child-reaped` kernel result carries for a
/// reaped child (the kernel result fields are reaped/exitKind/exitCode/
/// exitSignal/reapedAtMs; this struct remains the daemon-side consumer
/// shape).
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
        assert!(!BrokerCallerRole::HostShutdownUid { uid: 0 }.is_admin_uid());
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
            BrokerCallerRole::HostShutdownUid { uid: 0 },
            BrokerCallerRole::NotAuthorized,
        ] {
            let json = serde_json::to_string(&role).unwrap();
            let parsed: BrokerCallerRole = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, role);
        }
    }

    #[test]
    fn broker_request_envelope_round_trips_with_admin() {
        // U10: the generic envelope carrier now takes the retired
        // process-family variants' place on the wire.
        let env = BrokerRequestEnvelope {
            request: BrokerRequest::EnvelopeInvoke(EnvelopeInvokeRequest {
                operation: "signal-pidfd".to_owned(),
                zone: "zone-a".to_owned(),
                payload: serde_json::json!({ "signal": 15 }),
                chain_root_invocation_id: None,
                chain_identities: None,
                fd_indexes: vec![0],
                fd_kinds: vec![FdKind::Any],
            }),
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
            "request": {
                "kind": "EnvelopeInvoke",
                "payload": {
                    "operation": "signal-pidfd",
                    "zone": "zone-a",
                    "payload": { "signal": 15 },
                    "chainRootInvocationId": null,
                    "chainIdentities": null,
                    "fdIndexes": [0],
                    "fdKinds": ["any"]
                }
            }
        });
        let env: BrokerRequestEnvelope = serde_json::from_value(json).unwrap();
        assert!(matches!(env.caller_role, BrokerCallerRole::NotAuthorized));
        let BrokerRequest::EnvelopeInvoke(invoke) = env.request else {
            panic!("expected the envelope invoke");
        };
        assert_eq!(invoke.operation, "signal-pidfd");
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

    /// CreatePersistentTap and CreateTapFd carry opaque runner identity plus
    /// the complete admitted Network provenance. Kernel names and attributes
    /// remain broker-derived.
    #[test]
    fn create_persistent_tap_request_requires_admitted_provenance() {
        let frame = encode_frame(&serde_json::json!({
            "kind": "CreatePersistentTap",
            "payload": {
                "roleId": "runner-lan",
                "vmId": "corp-vm",
                "bundleTapIntentRef": "network-tap:716a354d3a6a651a0ad54d65cf0a72b764b91b3ed4167c6af551a4949d591019",
                "attachmentId": "123e4567-e89b-42d3-a456-426614174000",
                "networkGeneration": 4,
                "attachmentGeneration": 7,
                "zoneUid": "223e4567-e89b-42d3-a456-426614174001",
                "networkUid": "323e4567-e89b-42d3-a456-426614174002",
                "admittedInterfaceNames": ["d2b-tap0", "d2b-br0"],
                "bundleGeneration": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
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
    fn create_tap_fd_request_requires_admitted_provenance() {
        let frame = encode_frame(&serde_json::json!({
            "kind": "CreateTapFd",
            "payload": {
                "roleId": "runner-lan",
                "vmId": "corp-vm",
                "bundleTapIntentRef": "network-tap:716a354d3a6a651a0ad54d65cf0a72b764b91b3ed4167c6af551a4949d591019",
                "attachmentId": "123e4567-e89b-42d3-a456-426614174000",
                "networkGeneration": 4,
                "attachmentGeneration": 7,
                "zoneUid": "223e4567-e89b-42d3-a456-426614174001",
                "networkUid": "323e4567-e89b-42d3-a456-426614174002",
                "admittedInterfaceNames": ["d2b-tap0", "d2b-br0"],
                "bundleGeneration": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
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

    #[test]
    fn tap_create_rejects_missing_provenance_fields() {
        for kind in ["CreatePersistentTap", "CreateTapFd"] {
            for field in [
                "attachmentId",
                "attachmentGeneration",
                "admittedInterfaceNames",
                "bundleTapIntentRef",
                "networkGeneration",
                "networkUid",
                "zoneUid",
                "bundleGeneration",
            ] {
                let mut payload = opaque_create_tap_payload();
                payload.as_object_mut().unwrap().remove(field);
                let frame = encode_frame(&serde_json::json!({
                    "kind": kind,
                    "payload": payload,
                }))
                .expect("encodes");
                assert!(
                    decode_frame::<BrokerRequest>("BrokerRequest", &frame).is_err(),
                    "{kind} must reject missing {field}"
                );
            }
        }
    }

    #[test]
    fn tap_create_rejects_null_provenance_fields() {
        for kind in ["CreatePersistentTap", "CreateTapFd"] {
            for field in [
                "attachmentId",
                "attachmentGeneration",
                "admittedInterfaceNames",
                "bundleTapIntentRef",
                "networkGeneration",
                "networkUid",
                "zoneUid",
                "bundleGeneration",
            ] {
                let mut payload = opaque_create_tap_payload();
                payload
                    .as_object_mut()
                    .unwrap()
                    .insert(field.to_owned(), serde_json::Value::Null);
                let frame = encode_frame(&serde_json::json!({
                    "kind": kind,
                    "payload": payload,
                }))
                .expect("encodes");
                assert!(
                    decode_frame::<BrokerRequest>("BrokerRequest", &frame).is_err(),
                    "{kind} must reject null {field}"
                );
            }
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
        serde_json::json!({
            "roleId": "runner-lan",
            "vmId": "corp-vm",
            "bundleTapIntentRef": "network-tap:716a354d3a6a651a0ad54d65cf0a72b764b91b3ed4167c6af551a4949d591019",
            "attachmentId": "123e4567-e89b-42d3-a456-426614174000",
            "networkGeneration": 4,
            "attachmentGeneration": 7,
            "zoneUid": "223e4567-e89b-42d3-a456-426614174001",
            "networkUid": "323e4567-e89b-42d3-a456-426614174002",
            "admittedInterfaceNames": ["d2b-tap0", "d2b-br0"],
            "bundleGeneration": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        })
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
            "bundleNftProjectionIntentRef": "network-firewall:223e4567-e89b-42d3-a456-426614174001:323e4567-e89b-42d3-a456-426614174002:work",
            "scopeId": "network:223e4567-e89b-42d3-a456-426614174001:323e4567-e89b-42d3-a456-426614174002",
            "action": "apply",
            "zoneUid": "223e4567-e89b-42d3-a456-426614174001",
            "networkUid": "323e4567-e89b-42d3-a456-426614174002",
            "networkGeneration": 4,
            "attachmentGeneration": 7,
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
            "expectedZoneUid": "223e4567-e89b-42d3-a456-426614174001",
            "expectedNetworkUid": "323e4567-e89b-42d3-a456-426614174002",
            "expectedNetworkGeneration": 7,
            "expectedAttachmentGeneration": 11,
            "expectedBundleGeneration": format!("sha256:{}", "1".repeat(64)),
        });
        let request: DeletePersistentTapRequest =
            serde_json::from_value(payload.clone()).expect("valid fenced tap deletion request");
        assert_eq!(
            request.attachment_id.as_str(),
            "123e4567-e89b-42d3-a456-426614174000"
        );
        assert_eq!(request.expected_network_generation.get(), 7);
        assert_eq!(request.expected_attachment_generation.get(), 11);
        assert_eq!(
            request.expected_network_uid.as_str(),
            "323e4567-e89b-42d3-a456-426614174002"
        );

        for required in [
            "attachmentId",
            "expectedZoneUid",
            "expectedNetworkUid",
            "expectedNetworkGeneration",
            "expectedAttachmentGeneration",
            "expectedBundleGeneration",
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
        // U10: the typed frame is gone; the envelope carrier names the
        // committed operation and the payload is the same opaque-only
        // typed request, so the wire-opacity contract (no argv/env/uid
        // crossings) now guards the envelope payload.
        let frame = encode_frame(&serde_json::json!({
            "kind": "EnvelopeInvoke",
            "payload": {
                "operation": "SpawnRunner",
                "zone": "corp-vm",
                "payload": {
                    "vmId": "corp-vm",
                    "roleId": "ch",
                    "role": "cloud-hypervisor",
                    "bundleRunnerIntentRef": "ch-corp-vm",
                    "runtimeAllocations": [
                        { "kind": "vsock-cid", "opaqueRef": "alloc-vsock-1" },
                        { "kind": "api-socket-path", "opaqueRef": "alloc-api-1" }
                    ]
                },
                "chainRootInvocationId": null,
                "chainIdentities": null,
                "fdIndexes": [],
                "fdKinds": []
            }
        }))
        .expect("encodes");
        let decoded = decode_frame::<BrokerRequest>("BrokerRequest", &frame).expect("decodes");
        let BrokerRequest::EnvelopeInvoke(invoke) = decoded else {
            panic!("expected EnvelopeInvoke");
        };
        assert_eq!(invoke.operation, "SpawnRunner");
        let req: SpawnRunnerRequest =
            serde_json::from_value(invoke.payload).expect("payload is the typed spawn request");
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

    fn spawn_runner_for_profile(role: RunnerRole, execution_ref: &str) -> BrokerRequest {
        // U10: the typed SpawnRunner variant is retired; the profile gates
        // exercise the envelope carrier that now serves the family op.
        let zone = execution_ref.split('/').nth(1).unwrap_or("guest-vm");
        BrokerRequest::EnvelopeInvoke(EnvelopeInvokeRequest {
            operation: "SpawnRunner".to_owned(),
            zone: zone.to_owned(),
            payload: serde_json::to_value(SpawnRunnerRequest {
                vm_id: VmId::new("guest-vm"),
                role_id: RoleId::new(role.as_str()),
                resource_ref: None,
                resource_uid: None,
                zone_uid: None,
                owner_ref: None,
                owner_uid: None,
                provider_ref: None,
                bundle_content_identity: None,
                provider_identity: None,
                template_identity: None,
                generation: None,
                runtime_scope: None,
                activation_input: None,
                launch_args: None,
                sandbox_plan: None,
                role,
                bundle_runner_intent_ref: BundleOpId::new("runner:test"),
                execution_ref: Some(
                    ResourceRef::parse(execution_ref).expect("valid execution ref"),
                ),
                execution_domain: None,
                user_ref: None,
                guest_execution: None,
                runtime_allocations: Vec::new(),
                tracing_span_id: None,
                workload_identity: None,
                inherited_fd_count: 0,
                network_tap_context: None,
            })
            .expect("spawn request serializes"),
            chain_root_invocation_id: None,
            chain_identities: None,
            fd_indexes: vec![],
            fd_kinds: vec![],
        })
    }

    #[test]
    fn profile_spawn_runner_role_matrix_is_closed() {
        // U10 retired the typed SpawnRunner frame at wire v6; the family
        // operation now crosses the profile gate as an EnvelopeInvoke
        // (the envelope carrier is admitted on both profiles), and the
        // role/execution-ref fencing that the typed frame's profile rule
        // enforced moved daemon-side into the family handler. The role
        // ledger stays closed so a future re-admission must be deliberate.
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
            for execution_ref in ["Guest/guest-vm", "Host/host"] {
                let request = spawn_runner_for_profile(role, execution_ref);
                assert!(
                    request.allowed_by_profile(BrokerProfile::Guest),
                    "Guest profile must admit the envelope carrier for {}, the role fence is daemon-side",
                    role.as_str()
                );
                assert!(
                    request.allowed_by_profile(BrokerProfile::Host),
                    "Host profile must admit the envelope carrier for {}, the role fence is daemon-side",
                    role.as_str()
                );
            }
        }
    }

    #[test]
    fn guest_profile_requires_a_complete_target_execution_binding() {
        // U10: the guest-binding admission rule died with the typed
        // variant; the envelope carrier passes the profile gate whatever
        // the binding, and the binding completeness fence moved daemon-
        // side into the family handler (which parses this payload). The
        // binding shape keeps round-tripping here so the payload contract
        // cannot silently drift.
        let complete = spawn_runner_for_profile(RunnerRole::ActivationNixos, "Guest/guest-vm");
        let BrokerRequest::EnvelopeInvoke(mut invoke) = complete else {
            unreachable!("helper always builds EnvelopeInvoke");
        };
        let mut request: SpawnRunnerRequest =
            serde_json::from_value(invoke.payload).expect("typed spawn payload");
        request.guest_execution = Some(GuestExecutionBinding {
            target_uid: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000")
                .expect("Guest UID"),
            boot_identity_digest: [7; 32],
            session_generation: 2,
            assignment_epoch: 3,
            provider_generation: 4,
            controller_generation: 5,
        });
        invoke.payload = serde_json::to_value(&request).expect("payload serializes");
        let completed = BrokerRequest::EnvelopeInvoke(invoke);
        assert!(completed.allowed_by_profile(BrokerProfile::Guest));
        assert!(completed.allowed_by_profile(BrokerProfile::Host));

        request.guest_execution.as_mut().unwrap().assignment_epoch = 0;
        let invalid_payload = serde_json::to_value(&request).expect("payload serializes");
        let mut invoke = match completed {
            BrokerRequest::EnvelopeInvoke(invoke) => invoke,
            _ => unreachable!(),
        };
        invoke.payload = invalid_payload.clone();
        assert!(BrokerRequest::EnvelopeInvoke(invoke).allowed_by_profile(BrokerProfile::Guest));
        // The invalid binding still round-trips through the typed payload.
        let parse_roundtrip: SpawnRunnerRequest =
            serde_json::from_value(invalid_payload).expect("typed spawn payload");
        assert_eq!(
            parse_roundtrip
                .guest_execution
                .as_ref()
                .map(|binding| binding.assignment_epoch),
            Some(0)
        );
    }

    #[test]
    fn spawn_runner_rejects_each_legacy_authority_field() {
        // argv, env, uid, gid, caps, seccomp_profile,
        // kernel/initrd/cmdline, and api_socket_mode are ALL
        // bundle-derived. The envelope carrier admits the generic frame,
        // so the fail-closed rejection now lives in the typed payload
        // parse: any legacy authority field must make the typed spawn
        // request unparseable (deny_unknown_fields).
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
            let frame = encode_frame(&envelope_invoke_json("SpawnRunner", payload))
                .expect("envelope encodes");
            let decoded =
                decode_frame::<BrokerRequest>("BrokerRequest", &frame).expect("envelope decodes");
            let BrokerRequest::EnvelopeInvoke(invoke) = decoded else {
                panic!("expected EnvelopeInvoke");
            };
            assert!(
                serde_json::from_value::<SpawnRunnerRequest>(invoke.payload).is_err(),
                "legacy authority field {field} must fail the typed payload parse"
            );
        }
    }

    #[test]
    fn spawn_runner_runtime_allocation_unknown_kind_rejected() {
        // The bundle-derived allocation slots are a closed set
        // (vsock-cid, tap-fd-slot, api-socket-path). An envelope payload
        // claiming a new kind must fail the typed parse; future kinds
        // require a wire bump rather than caller-supplied authority.
        let frame = encode_frame(&envelope_invoke_json(
            "SpawnRunner",
            serde_json::json!({
                "vmId": "corp-vm",
                "roleId": "ch",
                "role": "cloud-hypervisor",
                "bundleRunnerIntentRef": "ch-corp-vm",
                "runtimeAllocations": [
                    { "kind": "kvm-fd", "opaqueRef": "should-not-cross" }
                ]
            }),
        ))
        .expect("encodes");
        let decoded = decode_frame::<BrokerRequest>("BrokerRequest", &frame).expect("decodes");
        let BrokerRequest::EnvelopeInvoke(invoke) = decoded else {
            panic!("expected EnvelopeInvoke");
        };
        assert!(
            serde_json::from_value::<SpawnRunnerRequest>(invoke.payload).is_err(),
            "unknown allocation kind must fail the typed payload parse"
        );
    }


    fn envelope_invoke_json(operation: &str, payload: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "kind": "EnvelopeInvoke",
            "payload": {
                "operation": operation,
                "zone": "corp-vm",
                "payload": payload,
                "chainRootInvocationId": null,
                "chainIdentities": null,
                "fdIndexes": [],
                "fdKinds": []
            }
        })
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
        // U10: the typed SignalRunner frame is the signal-pidfd kernel
        // envelope leg; the kernel answers with a numeric POSIX signal.
        let frame = encode_frame(&envelope_invoke_json(
            "signal-pidfd",
            serde_json::json!({ "signal": 15 }),
        ))
        .expect("encodes");
        let decoded = decode_frame::<BrokerRequest>("BrokerRequest", &frame).expect("decodes");
        let BrokerRequest::EnvelopeInvoke(invoke) = decoded else {
            panic!("expected EnvelopeInvoke");
        };
        assert_eq!(invoke.operation, "signal-pidfd");
        assert_eq!(
            invoke.payload.get("signal").and_then(serde_json::Value::as_i64),
            Some(15)
        );
    }

    #[test]
    fn cgroup_kill_request_round_trips() {
        // U10: the typed CgroupKill frame is the kill-cgroup kernel
        // envelope leg carrying the delegated-slice leaf path.
        let frame = encode_frame(&envelope_invoke_json(
            "kill-cgroup",
            serde_json::json!({ "cgroupPath": "/sys/fs/cgroup/d2b.slice/vm-a.runner" }),
        ))
        .expect("encodes");
        let decoded = decode_frame::<BrokerRequest>("BrokerRequest", &frame).expect("decodes");
        let BrokerRequest::EnvelopeInvoke(invoke) = decoded else {
            panic!("expected EnvelopeInvoke");
        };
        assert_eq!(invoke.operation, "kill-cgroup");
        assert_eq!(
            invoke.payload.get("cgroupPath").and_then(serde_json::Value::as_str),
            Some("/sys/fs/cgroup/d2b.slice/vm-a.runner")
        );
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
        // U10: the typed response is the envelope result now.
        let response = BrokerResponse::EnvelopeInvoke(EnvelopeInvokeResponse {
            operation: "signal-pidfd".to_owned(),
            invocation_id: "invocation-1".to_owned(),
            result: Some(serde_json::to_value(SignalRunnerResponse {
                signaled: false,
                vm_id: VmId::new("corp-vm"),
                role_id: RoleId::new("ch-runner"),
            })
            .expect("response serializes")),
            refusal: None,
            detail: None,
            fd_indexes: vec![],
            fd_kinds: vec![],
        });
        let frame = encode_frame(&response).expect("encodes");
        let decoded = decode_frame::<BrokerResponse>("BrokerResponse", &frame).expect("decodes");
        let BrokerResponse::EnvelopeInvoke(reply) = decoded else {
            panic!("expected BrokerResponse::EnvelopeInvoke");
        };
        assert_eq!(reply.operation, "signal-pidfd");
        let payload: SignalRunnerResponse =
            serde_json::from_value(reply.result.expect("result present"))
                .expect("typed signal response");
        assert!(!payload.signaled);
        assert_eq!(payload.vm_id.as_str(), "corp-vm");
        assert_eq!(payload.role_id.as_str(), "ch-runner");
    }

    #[test]
    fn spawn_runner_response_round_trips() {
        // The pidfd is delivered out-of-band over SCM_RIGHTS; the
        // envelope result carries (pid, start_time_ticks, pidfd_index)
        // and the fd declarations, so the daemon's pidfd table can
        // validate / reconcile the handle.
        let response = BrokerResponse::EnvelopeInvoke(EnvelopeInvokeResponse {
            operation: "SpawnRunner".to_owned(),
            invocation_id: "invocation-1".to_owned(),
            result: Some(serde_json::to_value(SpawnRunnerResponse {
                vm_id: VmId::new("corp-vm"),
                role_id: RoleId::new("ch"),
                role: RunnerRole::CloudHypervisor,
                resource_ref: None,
                resource_uid: None,
                zone_uid: None,
                owner_ref: None,
                runtime_scope: None,
                pid: 4242,
                start_time_ticks: 987_654_321,
                pidfd_index: 0,
                controller_bootstrap_fd_index: None,
                console_fd_index: None,
                execution_ref: None,
                execution_domain: None,
                user_ref: None,
                guest_execution: None,
                provider_identity: None,
                template_identity: None,
                generation: None,
                bundle_content_identity: None,
            })
            .expect("response serializes")),
            refusal: None,
            detail: None,
            fd_indexes: vec![0],
            fd_kinds: vec![FdKind::Any],
        });
        let frame = encode_frame(&response).expect("encodes");
        let decoded = decode_frame::<BrokerResponse>("BrokerResponse", &frame).expect("decodes");
        let BrokerResponse::EnvelopeInvoke(reply) = decoded else {
            panic!("expected BrokerResponse::EnvelopeInvoke");
        };
        assert_eq!(reply.operation, "SpawnRunner");
        assert_eq!(reply.fd_indexes, vec![0]);
        assert_eq!(reply.fd_kinds, vec![FdKind::Any]);
        let payload: SpawnRunnerResponse =
            serde_json::from_value(reply.result.expect("result present"))
                .expect("typed spawn response");
        assert_eq!(payload.vm_id.as_str(), "corp-vm");
        assert_eq!(payload.role, RunnerRole::CloudHypervisor);
        assert_eq!(payload.pid, 4242);
        assert_eq!(payload.start_time_ticks, 987_654_321);
        assert_eq!(payload.pidfd_index, 0);
    }

    #[test]
    fn envelope_invoke_round_trips_root_and_nested() {
        let root = BrokerRequestEnvelope {
            request: BrokerRequest::EnvelopeInvoke(EnvelopeInvokeRequest {
                operation: "signal-pidfd".to_owned(),
                zone: "zone-a".to_owned(),
                payload: serde_json::json!({}),
                chain_root_invocation_id: None,
                chain_identities: None,
                fd_indexes: vec![],
                fd_kinds: vec![],
            }),
            caller_role: BrokerCallerRole::AdminUid { uid: 1000 },
            test_peer_uid: None,
            audit_join: None,
        };
        let frame = encode_frame(&root).expect("encodes");
        let parsed: BrokerRequestEnvelope =
            decode_frame("BrokerRequestEnvelope", &frame).expect("decodes");
        assert_eq!(parsed, root);
        assert_eq!(parsed.request.op_name(), "EnvelopeInvoke");
        let nested = BrokerRequest::EnvelopeInvoke(EnvelopeInvokeRequest {
            operation: "drain-reap-buffer".to_owned(),
            zone: "zone-a".to_owned(),
            payload: serde_json::json!({}),
            chain_root_invocation_id: Some("invocation-1".to_owned()),
            chain_identities: Some(vec!["daemon".to_owned(), "d2b-provider-process".to_owned()]),
            fd_indexes: vec![],
            fd_kinds: vec![],
        });
        let frame = encode_frame(&nested).expect("encodes");
        let parsed: BrokerRequest =
            decode_frame("BrokerRequest", &frame).expect("decodes");
        assert_eq!(parsed, nested);
        let reply = BrokerResponse::EnvelopeInvoke(EnvelopeInvokeResponse {
            operation: "signal-pidfd".to_owned(),
            invocation_id: "invocation-1".to_owned(),
            result: Some(serde_json::json!({ "signaled": true })),
            refusal: None,
            detail: None,
            fd_indexes: vec![],
            fd_kinds: vec![],
        });
        let frame = encode_frame(&reply).expect("encodes");
        let parsed: BrokerResponse = decode_frame("BrokerResponse", &frame).expect("decodes");
        assert_eq!(parsed, reply);
        let refused = BrokerResponse::EnvelopeInvoke(EnvelopeInvokeResponse {
            operation: "signal-pidfd".to_owned(),
            invocation_id: "invocation-1".to_owned(),
            result: None,
            refusal: Some("unknown-operation".to_owned()),
            detail: None,
            fd_indexes: vec![],
            fd_kinds: vec![],
        });
        let frame = encode_frame(&refused).expect("encodes");
        let parsed: BrokerResponse = decode_frame("BrokerResponse", &frame).expect("decodes");
        assert_eq!(parsed, refused);
    }

    #[test]
    fn forwarded_fd_declarations_round_trip() {
        let request = ForwardOperationRequest {
            chain_identities: None,
            operation: "ProbeOperation".to_owned(),
            zone: "zone-a".to_owned(),
            invocation_id: "invocation-1".to_owned(),
            payload: serde_json::json!({ "label": "x" }),
            context: None,
            fd_indexes: vec![0, 1],
            fd_kinds: vec![FdKind::Fifo, FdKind::Fifo],
        };
        let frame = encode_frame(&request).expect("encodes");
        let decoded = decode_frame::<ForwardOperationRequest>("ForwardOperationRequest", &frame).expect("decodes");
        assert_eq!(decoded, request);

        let outcome = ForwardOperationOutcome::Result {
            result: serde_json::json!({ "ok": true }),
            fd_indexes: vec![0],
            fd_kinds: vec![FdKind::CharDevice],
        };
        let response = ForwardOperationResponse {
            outcome,
        };
        let frame = encode_frame(&response).expect("encodes");
        let decoded = decode_frame::<ForwardOperationResponse>("ForwardOperationResponse", &frame).expect("decodes");
        assert_eq!(
            decoded.outcome,
            ForwardOperationOutcome::Result {
                result: serde_json::json!({ "ok": true }),
                fd_indexes: vec![0],
                fd_kinds: vec![FdKind::CharDevice],
            }
        );
    }

    #[test]
    fn forwarded_frames_without_fd_declarations_decode_as_empty_sets() {
        // The two binaries swap within one generation: an old sender's
        // frame carries no fd fields, and the receiving side must read it as
        // the valid empty set rather than a malformed unknown field.
        let frame = encode_frame(&serde_json::json!({
            "operation": "ProbeOperation",
            "zone": "zone-a",
            "invocationId": "invocation-2",
            "payload": { "label": "x" },
        }))
        .expect("encodes");
        let decoded = decode_frame::<ForwardOperationRequest>("ForwardOperationRequest", &frame).expect("decodes");
        assert!(decoded.fd_indexes.is_empty());
        assert!(decoded.fd_kinds.is_empty());
    }

    #[test]
    fn the_fd_leg_refusal_code_is_the_shared_carrier_code() {
        assert_eq!(FD_LEG, "fd-leg");
        assert_eq!(MAX_FRAME_FDS, 8);
    }

    /// The fixture context a broker would mint: every field the attestation
    /// carries, stated once so the round-trip tests cannot drift from it.
    fn minted_context() -> ForwardContext {
        ForwardContext {
            broker_epoch: 3,
            zone: "zone-a".to_owned(),
            provider_set_revision: 2,
            controller_generation: 4,
            guest_generation: 7,
            initiating_identity: "daemon".to_owned(),
            deadline_ms: DEFAULT_CONTEXT_DEADLINE_MS,
        }
    }

    fn published_values() -> PublishTrustedContextValues {
        PublishTrustedContextValues {
            zone: "zone-a".to_owned(),
            provider_set_revision: 2,
            controller_generation: 4,
            guest_generation: 7,
        }
    }

    #[test]
    fn a_minted_context_round_trips_through_canonical_json() {
        // The context crosses the forward carrier as canonical JSON: the
        // spelling is the camelCase shape both legs serialize, so a field
        // renamed on one side is a decode failure on the other, never a
        // silently dropped comparison value.
        let context = minted_context();
        let frame = encode_frame(&context).expect("encodes");
        let decoded =
            decode_frame::<ForwardContext>("ForwardContext", &frame).expect("decodes");
        assert_eq!(decoded, context);

        let json = serde_json::to_value(&context).expect("serializes");
        assert_eq!(json["brokerEpoch"], 3);
        assert_eq!(json["zone"], "zone-a");
        assert_eq!(json["providerSetRevision"], 2);
        assert_eq!(json["controllerGeneration"], 4);
        assert_eq!(json["guestGeneration"], 7);
        assert_eq!(json["initiatingIdentity"], "daemon");
        assert_eq!(json["deadlineMs"], DEFAULT_CONTEXT_DEADLINE_MS);
        assert_eq!(
            json.as_object().map(|fields| fields.len()),
            Some(7),
            "the context is a closed block: seven fields, nothing else"
        );
    }

    #[test]
    fn a_context_round_trips_inside_the_forward_request() {
        let context = minted_context();
        let request = ForwardOperationRequest {
            chain_identities: None,
            operation: "ProbeOperation".to_owned(),
            zone: "zone-a".to_owned(),
            invocation_id: "invocation-1".to_owned(),
            payload: serde_json::json!({ "label": "x" }),
            context: Some(context.clone()),
            fd_indexes: vec![],
            fd_kinds: vec![],
        };
        let frame = encode_frame(&request).expect("encodes");
        let decoded =
            decode_frame::<ForwardOperationRequest>("ForwardOperationRequest", &frame)
                .expect("decodes");
        assert_eq!(decoded, request);
        assert_eq!(decoded.context, Some(context));
    }

    #[test]
    fn a_frame_without_a_context_decodes_as_absent() {
        // The two binaries swap within one generation: an old sender's frame
        // carries no context block, and the receiving side must read it as
        // the context-free mode rather than a malformed unknown field.
        let frame = encode_frame(&serde_json::json!({
            "operation": "ProbeOperation",
            "zone": "zone-a",
            "invocationId": "invocation-2",
            "payload": { "label": "x" },
        }))
        .expect("encodes");
        let decoded = decode_frame::<ForwardOperationRequest>("ForwardOperationRequest", &frame)
            .expect("decodes");
        assert_eq!(decoded.context, None);
    }

    #[test]
    fn the_daemon_publication_round_trips_and_names_the_shared_code() {
        let values = published_values();
        let frame = encode_frame(&values).expect("encodes");
        let decoded =
            decode_frame::<PublishTrustedContextValues>("PublishTrustedContextValues", &frame)
                .expect("decodes");
        assert_eq!(decoded, values);

        let reply = PublishTrustedContextResponse { broker_epoch: 3 };
        let frame = encode_frame(&reply).expect("encodes");
        let decoded =
            decode_frame::<PublishTrustedContextResponse>("PublishTrustedContextResponse", &frame)
                .expect("decodes");
        assert_eq!(decoded, reply);
    }

    #[test]
    fn the_stale_context_code_is_the_shared_carrier_code() {
        assert_eq!(STALE_CONTEXT, "stale-context");
        // The default budget is the receiving leg's historical fixed
        // deadline, and the ceiling bounds a mutator: both are shared
        // contract, never private numbers.
        assert_eq!(DEFAULT_CONTEXT_DEADLINE_MS, 25_000);
        assert!(MAX_CONTEXT_DEADLINE_MS > DEFAULT_CONTEXT_DEADLINE_MS);
    }
}
