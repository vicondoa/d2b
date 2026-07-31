//! Async Network controller state machine and typed child-resource projection.

use std::collections::BTreeMap;
use std::future::Future;

use d2b_contracts::v3::{
    ResourceBundleGenerationId, ResourceGeneration, ResourceRef, ResourceUid,
    execution_policy::{BoundedToken, BudgetSpec, ExecutionPolicy},
    guest::GuestSpec,
    network::{AttachmentGenerationFence, AttachmentHandle, NetworkSpec, cidr_overlaps},
    process::{
        CapabilityClass, EnvironmentClass, ExecutionSpec, MountAccess, MountSpec, ProcessClass,
        ProcessSpec, SandboxSpec, TelemetrySpec,
    },
    volume::{
        AttachmentAccess, AttachmentSettings, AttachmentTransport, CleanupPolicy, CreatePolicy,
        EntryAdoptionPolicy, EntryRestartPolicy, EntryType, ForeignChildPolicy, Invariant,
        LayoutEntry, LeaseClass, QuotaEnforcement, QuotaSpec, RepairPolicy, SensitivityClass,
        SourceKind, SourceSettings, ViewRight, ViewSpec, VolumeAttachment, VolumeKind,
        VolumeSource, VolumeSpec,
    },
};

use crate::artifact::{
    ArtifactCatalogEntry, ArtifactResolutionError, resolve_net_vm_system_artifact,
};
use crate::observe::{NetworkObservation, ObserveDecision, evaluate_observation};
use crate::plan::{ActualState, NetworkReconcilePlan, compute_plan};

/// Config Volume byte ceiling charged to the Host memory budget.
pub const CONFIG_VOLUME_MAX_BYTES: u64 = 4 * 1024 * 1024;
/// Config Volume inode ceiling.
pub const CONFIG_VOLUME_MAX_INODES: u64 = 128;
/// Guest mount path for the read-only config view.
pub const CONFIG_MOUNT_PATH: &str = "/run/d2b/net-config";

/// Closed condition reason emitted by the state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkConditionReason {
    /// A bridge effect failed.
    BridgeCreateError,
    /// Config Volume creation failed terminally.
    ConfigVolumeError,
    /// The reserved User dependency is not Ready.
    UserNotReady,
    /// The backing Volume is not Ready.
    VolumeNotReady,
    /// The Guest is not Ready.
    GuestNotReady,
    /// The Guest Volume attachment is not Ready.
    AttachmentNotReady,
    /// A generation fence was stale and must be refreshed.
    StaleGeneration,
    /// A foreign ownership marker blocked mutation.
    ForeignOwnership,
}

impl NetworkConditionReason {
    /// Return the stable redacted reason code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::BridgeCreateError => "bridge-create-error",
            Self::ConfigVolumeError => "config-volume-error",
            Self::UserNotReady => "user-not-ready",
            Self::VolumeNotReady => "volume-not-ready",
            Self::GuestNotReady => "guest-not-ready",
            Self::AttachmentNotReady => "attachment-not-ready",
            Self::StaleGeneration => "stale-projection-generation",
            Self::ForeignOwnership => "foreign-nft-rule-preserved",
        }
    }
}

/// Closed effect failures. No variant carries a caller or kernel value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkEffectError {
    /// Retryable effect failure.
    Transient,
    /// Bridge creation failed.
    BridgeCreate,
    /// Config Volume creation failed.
    ConfigVolume,
    /// Host memory budget rejected the tmpfs charge.
    HostMemoryBudgetExceeded,
    /// Immutable installed configuration generation changed.
    StaleConfigurationGeneration,
    /// Attachment generation changed.
    StaleAttachmentGeneration,
    /// A foreign ownership marker occupies a trusted slot.
    ForeignOwnership,
    /// CIDRs overlap.
    CidrConflict,
    /// Cross-Zone physical-NIC bridge multiplex was refused.
    CrossZoneL2,
    /// A runtime artifact ID could not be resolved.
    Artifact,
    /// The controller reached an invalid state.
    InvalidState,
}

impl NetworkEffectError {
    /// Return the stable redacted error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Transient => "network-effect-transient",
            Self::BridgeCreate => "bridge-create-error",
            Self::ConfigVolume => "config-volume-error",
            Self::HostMemoryBudgetExceeded => "host-memory-budget-exceeded",
            Self::StaleConfigurationGeneration => "stale-projection-generation",
            Self::StaleAttachmentGeneration => "attachment-generation-mismatch",
            Self::ForeignOwnership => "foreign-nft-rule-preserved",
            Self::CidrConflict => "cidr-conflict",
            Self::CrossZoneL2 => "external-physical-nic-cross-zone-l2",
            Self::Artifact => "net-vm-artifact-resolution",
            Self::InvalidState => "network-controller-invalid-state",
        }
    }
}

impl core::fmt::Display for NetworkEffectError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for NetworkEffectError {}

impl From<ArtifactResolutionError> for NetworkEffectError {
    fn from(_: ArtifactResolutionError) -> Self {
        Self::Artifact
    }
}

/// Readiness input from child resources and private realization state.
#[derive(Clone, PartialEq, Eq)]
pub struct ReconcileInput {
    /// Current desired Network spec.
    pub spec: NetworkSpec,
    /// Authored mDNS toggle carried beside the validated Network spec.
    pub mdns_enabled: bool,
    /// Current immutable Network identity.
    pub network_uid: ResourceUid,
    /// Current Network resource generation.
    pub network_generation: ResourceGeneration,
    /// Immutable installed configuration generation used as the firewall fence.
    pub installed_generation: ResourceBundleGenerationId,
    /// Declared private artifact catalog.
    pub artifact_catalog: Vec<ArtifactCatalogEntry>,
    /// Peer Network specs used for CIDR conflict validation.
    pub peer_networks: Vec<NetworkSpec>,
    /// Reserved User resource is Ready.
    pub user_ready: bool,
    /// Host memory budget can admit the config tmpfs charge.
    pub host_memory_budget_available: u64,
    /// Volume backing readiness.
    pub volume_ready: bool,
    /// Net-VM Guest readiness.
    pub guest_ready: bool,
    /// Volume attachment readiness.
    pub volume_attachment_ready: bool,
    /// Workload VMM owners have closed all attachment FDs.
    pub workload_fds_closed: bool,
    /// Owned child deletion observations.
    pub agent_deleted: bool,
    /// Owned mDNS child deletion observations.
    pub mdns_deleted: bool,
    /// The Volume attachment removal was confirmed.
    pub volume_attachment_removed: bool,
    /// The net-VM Guest deletion was confirmed.
    pub guest_deleted: bool,
    /// The config Volume deletion was confirmed.
    pub volume_deleted: bool,
    /// Retained attachment realizations.
    pub attachments: Vec<AttachmentRealization>,
}

impl core::fmt::Debug for ReconcileInput {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ReconcileInput(<redacted>)")
    }
}

/// Private retained attachment realization.
#[derive(Clone, PartialEq, Eq)]
pub struct AttachmentRealization {
    /// Opaque handle and exact generation fence.
    pub handle: AttachmentHandle,
    /// Whether its owning VMM has closed the FD.
    pub vmm_fd_closed: bool,
}

impl core::fmt::Debug for AttachmentRealization {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("AttachmentRealization(<redacted>)")
    }
}

/// Opaque firewall effect intent.
#[derive(Clone, PartialEq, Eq)]
pub struct FirewallIntent {
    network_uid: ResourceUid,
    expected_generation_id: ResourceBundleGenerationId,
}

impl FirewallIntent {
    /// Construct a projection-scoped immutable-generation intent.
    pub const fn new(
        network_uid: ResourceUid,
        expected_generation_id: ResourceBundleGenerationId,
    ) -> Self {
        Self {
            network_uid,
            expected_generation_id,
        }
    }

    /// Borrow the expected immutable installed generation.
    pub const fn expected_generation_id(&self) -> &ResourceBundleGenerationId {
        &self.expected_generation_id
    }

    /// Borrow the opaque Network identity for the Core effect adapter.
    pub const fn network_uid(&self) -> &ResourceUid {
        &self.network_uid
    }
}

impl core::fmt::Debug for FirewallIntent {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("FirewallIntent(<redacted>)")
    }
}

/// Opaque projection digest returned by the effect adapter.
#[derive(Clone, PartialEq, Eq)]
pub struct FirewallDigest([u8; 32]);

impl FirewallDigest {
    /// Construct from trusted digest bytes.
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl core::fmt::Debug for FirewallDigest {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("FirewallDigest(<redacted>)")
    }
}

/// All host effects injected into the controller.
pub trait NetworkEffectPort: Send + Sync {
    /// Ensure both Network bridges.
    fn create_bridges(
        &self,
        network_uid: &ResourceUid,
    ) -> impl Future<Output = Result<(), NetworkEffectError>> + Send;
    /// Re-apply IPv6 suppression.
    fn apply_sysctls(
        &self,
        network_uid: &ResourceUid,
    ) -> impl Future<Output = Result<(), NetworkEffectError>> + Send;
    /// Apply only this Network's firewall projection.
    fn apply_host_firewall(
        &self,
        intent: &FirewallIntent,
    ) -> impl Future<Output = Result<FirewallDigest, NetworkEffectError>> + Send;
    /// Remove only this Network's firewall projection.
    fn remove_host_firewall(
        &self,
        intent: &FirewallIntent,
    ) -> impl Future<Output = Result<(), NetworkEffectError>> + Send;
    /// Reconcile NetworkManager unmanaged state.
    fn apply_nm_unmanaged(&self) -> impl Future<Output = Result<(), NetworkEffectError>> + Send;
    /// Reconcile host routes.
    fn apply_routes(
        &self,
        network_uid: &ResourceUid,
    ) -> impl Future<Output = Result<(), NetworkEffectError>> + Send;
    /// Reconcile the owned hosts block.
    fn update_hosts(
        &self,
        network_uid: &ResourceUid,
    ) -> impl Future<Output = Result<(), NetworkEffectError>> + Send;
    /// Seed new DHCP reservations.
    fn seed_dhcp(
        &self,
        network_uid: &ResourceUid,
    ) -> impl Future<Output = Result<(), NetworkEffectError>> + Send;
    /// Delete one opaque attachment realization.
    fn delete_persistent_tap(
        &self,
        handle: &AttachmentHandle,
        fence: &AttachmentGenerationFence,
    ) -> impl Future<Output = Result<(), NetworkEffectError>> + Send;
    /// Delete both bridges after every tap confirmation.
    fn delete_bridges(
        &self,
        network_uid: &ResourceUid,
    ) -> impl Future<Output = Result<(), NetworkEffectError>> + Send;
}

/// Child-resource mutation port. It accepts typed specs, never raw host paths.
pub trait NetworkResourcePort: Send + Sync {
    /// Create or update backing-only Volume state and charge its tmpfs quota.
    fn upsert_volume_backing(
        &self,
        spec: &VolumeSpec,
    ) -> impl Future<Output = Result<(), NetworkEffectError>> + Send;
    /// Write all four bounded config payloads through the Volume service.
    fn write_volume_content(
        &self,
        content: &NetworkConfigContent,
    ) -> impl Future<Output = Result<(), NetworkEffectError>> + Send;
    /// Create or update the net-VM Guest.
    fn upsert_guest(
        &self,
        spec: &GuestSpec,
    ) -> impl Future<Output = Result<(), NetworkEffectError>> + Send;
    /// Add the typed read-only Guest attachment.
    fn attach_volume(
        &self,
        attachment: &VolumeAttachment,
    ) -> impl Future<Output = Result<(), NetworkEffectError>> + Send;
    /// Create or update the guest-agent Process.
    fn upsert_agent(
        &self,
        spec: &ProcessSpec,
    ) -> impl Future<Output = Result<(), NetworkEffectError>> + Send;
    /// Reconcile mDNS Process resources from the authored toggle.
    fn reconcile_mdns(
        &self,
        enabled: bool,
    ) -> impl Future<Output = Result<(), NetworkEffectError>> + Send;
    /// Delete agent and mDNS Processes.
    fn delete_processes(&self) -> impl Future<Output = Result<(), NetworkEffectError>> + Send;
    /// Remove the Guest attachment from the Volume.
    fn detach_volume(&self) -> impl Future<Output = Result<(), NetworkEffectError>> + Send;
    /// Delete the net-VM Guest.
    fn delete_guest(&self) -> impl Future<Output = Result<(), NetworkEffectError>> + Send;
    /// Delete the config Volume.
    fn delete_volume(&self) -> impl Future<Output = Result<(), NetworkEffectError>> + Send;
}

/// Four bounded files written through the Volume service.
#[derive(Clone, PartialEq, Eq)]
pub struct NetworkConfigContent {
    /// dnsmasq configuration bytes.
    pub dnsmasq: Vec<u8>,
    /// net-VM nftables configuration bytes.
    pub nftables: Vec<u8>,
    /// routing configuration bytes.
    pub routing: Vec<u8>,
    /// attachment table bytes.
    pub attachments: Vec<u8>,
    digest: [u8; 32],
}

impl NetworkConfigContent {
    /// Return the digest used to request an agent reload.
    pub const fn digest(&self) -> [u8; 32] {
        self.digest
    }
}

impl core::fmt::Debug for NetworkConfigContent {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("NetworkConfigContent(<redacted>)")
    }
}

/// Ordered reconciliation progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconcileProgress {
    /// All desired state converged.
    Ready,
    /// Waiting for a dependency watch event.
    Pending(NetworkConditionReason),
    /// Refresh desired state before retrying a stale effect.
    Requeue(NetworkConditionReason),
    /// Cleanup or reconcile is blocked fail closed.
    Blocked(NetworkConditionReason),
}

/// Strict finalizer stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalizerStage {
    /// Stop workload VMM owners and wait for FD closure.
    WorkloadFdClosure,
    /// Delete retained persistent taps.
    PersistentTaps,
    /// Delete owned agent and mDNS Processes.
    Processes,
    /// Remove the Guest attachment from the Volume.
    VolumeAttachment,
    /// Delete the net-VM Guest.
    Guest,
    /// Delete the config Volume.
    Volume,
    /// Remove host effects and bridges.
    HostFabric,
    /// Finalizer can be cleared.
    Complete,
}

/// Bounded metric label sets. Keys and values are closed enums.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkMetricLabels {
    /// Semantic operation.
    pub operation: &'static str,
    /// Closed outcome.
    pub outcome: &'static str,
    /// Closed error class.
    pub error: &'static str,
}

impl NetworkMetricLabels {
    /// Build labels from closed semantic values only.
    pub const fn new(
        operation: NetworkMetricOperation,
        outcome: NetworkMetricOutcome,
        error: Option<NetworkEffectError>,
    ) -> Self {
        Self {
            operation: operation.label(),
            outcome: outcome.label(),
            error: match error {
                Some(value) => value.code(),
                None => "none",
            },
        }
    }
}

/// Closed metric operation values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkMetricOperation {
    /// Reconcile pass.
    Reconcile,
    /// Observe pass.
    Observe,
    /// Finalizer pass.
    Finalize,
    /// Config Volume sync.
    VolumeSync,
    /// Agent reload.
    AgentReload,
}

impl NetworkMetricOperation {
    const fn label(self) -> &'static str {
        match self {
            Self::Reconcile => "reconcile",
            Self::Observe => "observe",
            Self::Finalize => "finalize",
            Self::VolumeSync => "volume-sync",
            Self::AgentReload => "agent-reload",
        }
    }
}

/// Closed metric outcome values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkMetricOutcome {
    /// Operation converged.
    Success,
    /// Operation will retry.
    Retry,
    /// Operation is blocked.
    Blocked,
}

impl NetworkMetricOutcome {
    const fn label(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Retry => "retry",
            Self::Blocked => "blocked",
        }
    }
}

/// Stateless async Network reconciler.
pub struct NetworkReconciler<E, R> {
    effects: E,
    resources: R,
}

impl<E, R> NetworkReconciler<E, R>
where
    E: NetworkEffectPort,
    R: NetworkResourcePort,
{
    /// Construct with injected effect and resource ports.
    pub const fn new(effects: E, resources: R) -> Self {
        Self { effects, resources }
    }

    /// Compute the effect-free desired-versus-actual plan.
    pub fn plan(&self, input: &ReconcileInput, actual: ActualState) -> NetworkReconcilePlan {
        compute_plan(&input.spec, input.mdns_enabled, actual)
    }

    /// Evaluate projection, sysctl, bridge-port, CIDR, authority, and agent
    /// observations without performing an effect.
    pub fn observe(
        &self,
        observation: NetworkObservation,
    ) -> Result<ObserveDecision, NetworkEffectError> {
        evaluate_observation(observation)
    }

    /// Adopt only observations that are unambiguous and already match desired
    /// state. Adoption never creates or deletes host state.
    pub fn adopt(
        &self,
        observation: NetworkObservation,
    ) -> Result<ObserveDecision, NetworkEffectError> {
        self.observe(observation)
    }

    /// Run ordered reconcile while enforcing every child-readiness barrier.
    pub async fn reconcile(
        &self,
        input: &ReconcileInput,
    ) -> Result<ReconcileProgress, NetworkEffectError> {
        validate_input(input)?;
        if !input.user_ready {
            return Ok(ReconcileProgress::Pending(
                NetworkConditionReason::UserNotReady,
            ));
        }
        if input.host_memory_budget_available < CONFIG_VOLUME_MAX_BYTES {
            return Err(NetworkEffectError::HostMemoryBudgetExceeded);
        }

        self.effects
            .create_bridges(&input.network_uid)
            .await
            .map_err(|_| NetworkEffectError::BridgeCreate)?;
        self.effects.apply_sysctls(&input.network_uid).await?;
        let firewall = FirewallIntent::new(
            input.network_uid.clone(),
            input.installed_generation.clone(),
        );
        match self.effects.apply_host_firewall(&firewall).await {
            Err(NetworkEffectError::StaleConfigurationGeneration) => {
                return Ok(ReconcileProgress::Requeue(
                    NetworkConditionReason::StaleGeneration,
                ));
            }
            Err(NetworkEffectError::ForeignOwnership) => {
                return Ok(ReconcileProgress::Blocked(
                    NetworkConditionReason::ForeignOwnership,
                ));
            }
            result => {
                result?;
            }
        }
        self.effects.apply_nm_unmanaged().await?;
        self.effects.apply_routes(&input.network_uid).await?;
        self.effects.update_hosts(&input.network_uid).await?;
        self.effects.seed_dhcp(&input.network_uid).await?;

        let volume = config_volume_spec("host-system", None)?;
        self.resources
            .upsert_volume_backing(&volume)
            .await
            .map_err(|_| NetworkEffectError::ConfigVolume)?;
        let content = render_config(&input.spec)?;
        self.resources.write_volume_content(&content).await?;
        if !input.volume_ready {
            return Ok(ReconcileProgress::Pending(
                NetworkConditionReason::VolumeNotReady,
            ));
        }

        let artifact = resolve_net_vm_system_artifact(&input.spec, &input.artifact_catalog)?;
        self.resources
            .upsert_guest(&GuestSpec::new(
                ExecutionPolicy::system_default(),
                Some(artifact),
            ))
            .await?;
        if !input.guest_ready {
            return Ok(ReconcileProgress::Pending(
                NetworkConditionReason::GuestNotReady,
            ));
        }

        let attachment = config_volume_attachment("net-vm")?;
        self.resources.attach_volume(&attachment).await?;
        if !input.volume_attachment_ready {
            return Ok(ReconcileProgress::Pending(
                NetworkConditionReason::AttachmentNotReady,
            ));
        }

        self.resources
            .upsert_agent(&guest_agent_process_spec("net-vm")?)
            .await?;
        self.resources.reconcile_mdns(input.mdns_enabled).await?;
        for attachment in &input.attachments {
            if attachment.vmm_fd_closed {
                match self
                    .effects
                    .delete_persistent_tap(&attachment.handle, attachment.handle.generation_fence())
                    .await
                {
                    Err(NetworkEffectError::StaleAttachmentGeneration) => {
                        return Ok(ReconcileProgress::Requeue(
                            NetworkConditionReason::StaleGeneration,
                        ));
                    }
                    Err(NetworkEffectError::ForeignOwnership) => {
                        return Ok(ReconcileProgress::Blocked(
                            NetworkConditionReason::ForeignOwnership,
                        ));
                    }
                    result => result?,
                }
            }
        }
        Ok(ReconcileProgress::Ready)
    }

    /// Advance exactly one strict finalizer stage.
    pub async fn finalize(
        &self,
        input: &ReconcileInput,
    ) -> Result<FinalizerStage, NetworkEffectError> {
        if !input.workload_fds_closed || input.attachments.iter().any(|item| !item.vmm_fd_closed) {
            return Ok(FinalizerStage::WorkloadFdClosure);
        }
        for attachment in &input.attachments {
            match self
                .effects
                .delete_persistent_tap(&attachment.handle, attachment.handle.generation_fence())
                .await
            {
                Err(NetworkEffectError::Transient) => return Ok(FinalizerStage::PersistentTaps),
                Err(NetworkEffectError::StaleAttachmentGeneration) => {
                    return Ok(FinalizerStage::PersistentTaps);
                }
                result => result?,
            }
        }
        if !input.agent_deleted || !input.mdns_deleted {
            self.resources.delete_processes().await?;
            return Ok(FinalizerStage::Processes);
        }
        if !input.volume_attachment_removed {
            self.resources.detach_volume().await?;
            return Ok(FinalizerStage::VolumeAttachment);
        }
        if !input.guest_deleted {
            self.resources.delete_guest().await?;
            return Ok(FinalizerStage::Guest);
        }
        if !input.volume_deleted {
            self.resources.delete_volume().await?;
            return Ok(FinalizerStage::Volume);
        }
        let firewall = FirewallIntent::new(
            input.network_uid.clone(),
            input.installed_generation.clone(),
        );
        self.effects.remove_host_firewall(&firewall).await?;
        self.effects.delete_bridges(&input.network_uid).await?;
        Ok(FinalizerStage::Complete)
    }
}

impl<E, R> core::fmt::Debug for NetworkReconciler<E, R> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("NetworkReconciler(<redacted>)")
    }
}

fn validate_input(input: &ReconcileInput) -> Result<(), NetworkEffectError> {
    if input.peer_networks.iter().any(|peer| {
        cidr_overlaps(input.spec.lan_cidr(), peer.lan_cidr())
            || cidr_overlaps(input.spec.lan_cidr(), peer.uplink_cidr())
            || cidr_overlaps(input.spec.uplink_cidr(), peer.lan_cidr())
            || cidr_overlaps(input.spec.uplink_cidr(), peer.uplink_cidr())
    }) {
        return Err(NetworkEffectError::CidrConflict);
    }
    Ok(())
}

/// Construct the exact backing-only config Volume schema.
pub fn config_volume_spec(
    host_name: &str,
    guest_name: Option<&str>,
) -> Result<VolumeSpec, NetworkEffectError> {
    let owner = ResourceRef::parse("User/net-local-controller")
        .map_err(|_| NetworkEffectError::InvalidState)?;
    let source = VolumeSource::new(
        ResourceRef::parse(&format!("Host/{host_name}"))
            .map_err(|_| NetworkEffectError::InvalidState)?,
        SourceSettings::new(SourceKind::Tmpfs, None)
            .map_err(|_| NetworkEffectError::InvalidState)?,
    )
    .map_err(|_| NetworkEffectError::InvalidState)?;
    let mut layout = vec![
        LayoutEntry::root_directory(owner.clone(), owner.clone(), "0750")
            .map_err(|_| NetworkEffectError::InvalidState)?,
    ];
    for path in [
        "dnsmasq.conf",
        "nftables.rules",
        "routing.conf",
        "attachments.json",
    ] {
        layout.push(
            LayoutEntry::new(
                path,
                EntryType::File,
                owner.clone(),
                owner.clone(),
                "0640",
                None,
                Vec::new(),
                Vec::new(),
                ForeignChildPolicy::Fail,
                true,
                false,
                SensitivityClass::Private,
                CreatePolicy::CreateIfAbsent,
                RepairPolicy::ExactOwner,
                CleanupPolicy::OwnerControlled,
                EntryAdoptionPolicy::AdoptWithLiveOwnerProof,
                EntryRestartPolicy::PreserveAcrossControllerRestart,
                LeaseClass::None,
                vec![Invariant::NoSymlink, Invariant::BrokerOpaqueIdOnly],
            )
            .map_err(|_| NetworkEffectError::InvalidState)?,
        );
    }
    let mut views = BTreeMap::new();
    views.insert(
        "guest-readonly".to_owned(),
        ViewSpec::new("", vec![ViewRight::Read, ViewRight::Traverse])
            .map_err(|_| NetworkEffectError::InvalidState)?,
    );
    let attachments = guest_name
        .map(config_volume_attachment)
        .transpose()?
        .into_iter()
        .collect();
    VolumeSpec::new(
        source,
        VolumeKind::Ephemeral,
        layout,
        views,
        attachments,
        Some(
            QuotaSpec::new(
                Some(CONFIG_VOLUME_MAX_BYTES),
                Some(CONFIG_VOLUME_MAX_INODES),
                QuotaEnforcement::Hard,
            )
            .map_err(|_| NetworkEffectError::InvalidState)?,
        ),
    )
    .map_err(|_| NetworkEffectError::InvalidState)
}

/// Construct the exact read-only virtiofs attachment.
pub fn config_volume_attachment(guest_name: &str) -> Result<VolumeAttachment, NetworkEffectError> {
    VolumeAttachment::new(
        ResourceRef::parse(&format!("Guest/{guest_name}"))
            .map_err(|_| NetworkEffectError::InvalidState)?,
        AttachmentTransport::Virtiofs,
        BoundedToken::parse("guest-readonly").map_err(|_| NetworkEffectError::InvalidState)?,
        AttachmentAccess::ReadOnly,
        CONFIG_MOUNT_PATH,
        AttachmentSettings::default(),
    )
    .map_err(|_| NetworkEffectError::InvalidState)
}

/// Construct the guest-network-namespace agent Process spec.
pub fn guest_agent_process_spec(guest_name: &str) -> Result<ProcessSpec, NetworkEffectError> {
    let sandbox = SandboxSpec::new(
        Vec::new(),
        vec![
            CapabilityClass::NetworkAdmin,
            CapabilityClass::NetworkBind,
            CapabilityClass::NetworkRaw,
        ],
        BoundedToken::parse("strict").map_err(|_| NetworkEffectError::InvalidState)?,
        true,
        false,
        EnvironmentClass::Minimal,
        true,
        Some("0022".to_owned()),
        0,
        None,
    )
    .map_err(|_| NetworkEffectError::InvalidState)?;
    let mount = MountSpec::new(
        ResourceRef::parse("Volume/net-config").map_err(|_| NetworkEffectError::InvalidState)?,
        BoundedToken::parse("guest-readonly").map_err(|_| NetworkEffectError::InvalidState)?,
        CONFIG_MOUNT_PATH,
        MountAccess::ReadOnly,
        true,
    )
    .map_err(|_| NetworkEffectError::InvalidState)?;
    let execution = ExecutionSpec::new(
        ResourceRef::parse(&format!("Guest/{guest_name}"))
            .map_err(|_| NetworkEffectError::InvalidState)?,
        None,
        None,
        ProcessClass::Worker,
        BoundedToken::parse("network-agent").map_err(|_| NetworkEffectError::InvalidState)?,
        None,
        Vec::new(),
        vec![mount],
        sandbox,
        BudgetSpec::default(),
        None,
        Vec::new(),
        TelemetrySpec::default(),
    )
    .map_err(|_| NetworkEffectError::InvalidState)?;
    Ok(ProcessSpec::minimal(execution))
}

/// Render per-Network data into the four config files only.
pub fn render_config(spec: &NetworkSpec) -> Result<NetworkConfigContent, NetworkEffectError> {
    let dnsmasq = format!("lan={}\n", spec.lan_cidr().as_str()).into_bytes();
    let nftables = format!(
        "lan={}\nuplink={}\nblocklist={}\n",
        spec.lan_cidr().as_str(),
        spec.uplink_cidr().as_str(),
        d2b_contracts::v3::network::DEFAULT_HOST_BLOCKLIST.join(",")
    )
    .into_bytes();
    let routing = format!("uplink={}\n", spec.uplink_cidr().as_str()).into_bytes();
    let attachments = format!(
        "[{}]",
        spec.attachments()
            .iter()
            .map(|attachment| attachment.index().to_string())
            .collect::<Vec<_>>()
            .join(",")
    )
    .into_bytes();
    let mut digest_input = Vec::new();
    for bytes in [&dnsmasq, &nftables, &routing, &attachments] {
        digest_input.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
        digest_input.extend_from_slice(bytes);
    }
    Ok(NetworkConfigContent {
        dnsmasq,
        nftables,
        routing,
        attachments,
        digest: crate::nftables::digest_bytes(&digest_input),
    })
}
