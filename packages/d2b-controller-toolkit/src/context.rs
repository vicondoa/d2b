//! Identity-bound context supplied to one reconcile pass.

use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use d2b_contracts::v3::{
    ConfigurationGeneration, ControllerGeneration, ResourceGeneration, ResourceRef, ResourceUid,
    ZoneId, ZoneRevision,
};

/// Closed reason set used for queue coalescing and dispatch selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TriggerReason {
    SpecGenerationChanged,
    OwnedResourceChanged,
    DependencyChanged,
    DependencyReady,
    DeletionRequested,
    FinalizerRequired,
    ControllerGenerationChanged,
    ProviderGenerationChanged,
    PolicyChanged,
    SecurityPolicyChanged,
    ArtifactOrImageChanged,
    ExecutionStatusChanged,
    ScheduledObserve,
    AssessUpdateDue,
    UpgradeRequested,
    ExpeditedMutation,
    RetryDue,
    ManualReconcile,
    StartupRelist,
}

impl TriggerReason {
    /// Whether this reason must survive coalescing and convergence suppression.
    pub const fn is_non_droppable(self) -> bool {
        matches!(
            self,
            Self::SpecGenerationChanged
                | Self::OwnedResourceChanged
                | Self::DeletionRequested
                | Self::FinalizerRequired
                | Self::ControllerGenerationChanged
                | Self::ProviderGenerationChanged
                | Self::PolicyChanged
                | Self::SecurityPolicyChanged
                | Self::DependencyChanged
                | Self::DependencyReady
                | Self::ScheduledObserve
                | Self::AssessUpdateDue
                | Self::UpgradeRequested
                | Self::ExpeditedMutation
                | Self::RetryDue
                | Self::ManualReconcile
        )
    }

    /// Whether this reason requires the update-currency assessment path.
    pub const fn requires_update_assessment(self) -> bool {
        matches!(
            self,
            Self::SpecGenerationChanged
                | Self::ControllerGenerationChanged
                | Self::ProviderGenerationChanged
                | Self::SecurityPolicyChanged
                | Self::ArtifactOrImageChanged
                | Self::DependencyChanged
                | Self::AssessUpdateDue
        )
    }
}

/// Deterministic, duplicate-free trigger collection.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TriggerSet(BTreeSet<TriggerReason>);

impl TriggerSet {
    /// Construct a reason set from an iterator.
    pub fn new(reasons: impl IntoIterator<Item = TriggerReason>) -> Self {
        Self(reasons.into_iter().collect())
    }

    /// Add one reason.
    pub fn insert(&mut self, reason: TriggerReason) {
        self.0.insert(reason);
    }

    /// Merge all reasons from another admitted hint.
    pub fn union_with(&mut self, other: &Self) {
        self.0.extend(other.0.iter().copied());
    }

    /// Test whether a reason is present.
    pub fn contains(&self, reason: TriggerReason) -> bool {
        self.0.contains(&reason)
    }

    /// Return the number of distinct reasons.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether no reason is present.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterate in stable enum order.
    pub fn iter(&self) -> impl Iterator<Item = TriggerReason> + '_ {
        self.0.iter().copied()
    }

    /// Whether any reason requires update assessment.
    pub fn requires_update_assessment(&self) -> bool {
        self.0
            .iter()
            .any(|reason| reason.requires_update_assessment())
    }
}

/// Immutable identity for one resource incarnation.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceKey {
    zone: ZoneId,
    resource_ref: ResourceRef,
    uid: ResourceUid,
}

impl ResourceKey {
    /// Construct a Zone-local resource key.
    pub fn new(zone: ZoneId, resource_ref: ResourceRef, uid: ResourceUid) -> Self {
        Self {
            zone,
            resource_ref,
            uid,
        }
    }

    /// Borrow the Zone identity.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the canonical resource reference.
    pub const fn resource_ref(&self) -> &ResourceRef {
        &self.resource_ref
    }

    /// Borrow the immutable resource UID.
    pub const fn uid(&self) -> &ResourceUid {
        &self.uid
    }
}

impl core::fmt::Debug for ResourceKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ResourceKey")
            .field("resource_type", self.resource_ref.resource_type())
            .field("has_zone", &true)
            .field("has_uid", &true)
            .finish()
    }
}

/// Fresh target body read immediately before a handler starts.
#[derive(Clone, PartialEq, Eq)]
pub struct ResourceSnapshot {
    key: ResourceKey,
    revision: ZoneRevision,
    generation: ResourceGeneration,
    canonical_json: Vec<u8>,
    deleting: bool,
}

impl ResourceSnapshot {
    /// Construct a fresh resource snapshot.
    pub fn new(
        key: ResourceKey,
        revision: ZoneRevision,
        generation: ResourceGeneration,
        canonical_json: Vec<u8>,
        deleting: bool,
    ) -> Self {
        Self {
            key,
            revision,
            generation,
            canonical_json,
            deleting,
        }
    }

    /// Borrow the immutable identity.
    pub const fn key(&self) -> &ResourceKey {
        &self.key
    }

    /// Return the fresh revision.
    pub const fn revision(&self) -> ZoneRevision {
        self.revision
    }

    /// Return the desired-state generation.
    pub const fn generation(&self) -> ResourceGeneration {
        self.generation
    }

    /// Borrow canonical resource bytes.
    pub fn canonical_json(&self) -> &[u8] {
        &self.canonical_json
    }

    /// Whether deletion has been requested.
    pub const fn deleting(&self) -> bool {
        self.deleting
    }
}

impl core::fmt::Debug for ResourceSnapshot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ResourceSnapshot")
            .field("key", &self.key)
            .field("revision", &self.revision)
            .field("generation", &self.generation)
            .field(
                "canonical_json",
                &format_args!("<{} bytes>", self.canonical_json.len()),
            )
            .field("deleting", &self.deleting)
            .finish()
    }
}

/// Base-only dependency snapshot from the same Zone as the target.
#[derive(Clone, PartialEq, Eq)]
pub struct DependencySnapshot {
    resource: ResourceSnapshot,
}

impl DependencySnapshot {
    /// Wrap a base-only dependency resource.
    pub fn new(resource: ResourceSnapshot) -> Self {
        Self { resource }
    }

    /// Borrow the dependency resource.
    pub const fn resource(&self) -> &ResourceSnapshot {
        &self.resource
    }
}

impl core::fmt::Debug for DependencySnapshot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DependencySnapshot")
            .field("resource", &self.resource)
            .finish()
    }
}

/// Authenticated controller and execution identity fixed at registration.
#[derive(Clone, PartialEq, Eq)]
pub struct ControllerIdentity {
    zone: ZoneId,
    controller_ref: ResourceRef,
    controller_generation: ControllerGeneration,
    provider_ref: ResourceRef,
    provider_generation: ResourceGeneration,
    process_ref: ResourceRef,
    host_ref: ResourceRef,
    guest_ref: Option<ResourceRef>,
}

impl ControllerIdentity {
    /// Construct an identity whose Zone is fixed by the registered session.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        zone: ZoneId,
        controller_ref: ResourceRef,
        controller_generation: ControllerGeneration,
        provider_ref: ResourceRef,
        provider_generation: ResourceGeneration,
        process_ref: ResourceRef,
        host_ref: ResourceRef,
        guest_ref: Option<ResourceRef>,
    ) -> Result<Self, ContextError> {
        if controller_ref.resource_type().as_str() != "Process"
            || provider_ref.resource_type().as_str() != "Provider"
            || process_ref.resource_type().as_str() != "Process"
            || host_ref.resource_type().as_str() != "Host"
            || guest_ref
                .as_ref()
                .is_some_and(|guest| guest.resource_type().as_str() != "Guest")
        {
            return Err(ContextError::InvalidControllerIdentity);
        }
        Ok(Self {
            zone,
            controller_ref,
            controller_generation,
            provider_ref,
            provider_generation,
            process_ref,
            host_ref,
            guest_ref,
        })
    }

    /// Borrow the authenticated Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Return the controller generation.
    pub const fn controller_generation(&self) -> ControllerGeneration {
        self.controller_generation
    }

    /// Return the Provider generation.
    pub const fn provider_generation(&self) -> ResourceGeneration {
        self.provider_generation
    }
}

impl core::fmt::Debug for ControllerIdentity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ControllerIdentity")
            .field("controller_type", self.controller_ref.resource_type())
            .field("controller_generation", &self.controller_generation)
            .field("provider_type", self.provider_ref.resource_type())
            .field("provider_generation", &self.provider_generation)
            .field("process_type", self.process_ref.resource_type())
            .field("host_type", self.host_ref.resource_type())
            .field(
                "guest_type",
                &self.guest_ref.as_ref().map(ResourceRef::resource_type),
            )
            .field("has_zone", &true)
            .finish()
    }
}

/// Correlation identifiers fixed for one pass.
#[derive(Clone, PartialEq, Eq)]
pub struct OperationContext {
    operation_id: String,
    idempotency_key: String,
    correlation_id: String,
    trace_id: Option<String>,
}

impl OperationContext {
    /// Construct bounded opaque identifiers.
    pub fn new(
        operation_id: impl Into<String>,
        idempotency_key: impl Into<String>,
        correlation_id: impl Into<String>,
        trace_id: Option<String>,
    ) -> Result<Self, ContextError> {
        let value = Self {
            operation_id: operation_id.into(),
            idempotency_key: idempotency_key.into(),
            correlation_id: correlation_id.into(),
            trace_id,
        };
        if [
            value.operation_id.as_str(),
            value.idempotency_key.as_str(),
            value.correlation_id.as_str(),
        ]
        .into_iter()
        .any(|field| field.is_empty() || field.len() > 256)
            || value
                .trace_id
                .as_ref()
                .is_some_and(|field| field.is_empty() || field.len() > 256)
        {
            return Err(ContextError::InvalidOperationIdentity);
        }
        Ok(value)
    }

    /// Borrow the operation ID for protocol correlation.
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    /// Borrow the idempotency key.
    pub fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }
}

impl core::fmt::Debug for OperationContext {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("OperationContext(<redacted>)")
    }
}

/// Cloneable cancellation signal carrying no authority.
#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    /// Mark the pass cancelled.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// Observe cancellation.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

impl core::fmt::Debug for Cancellation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("Cancellation")
            .field(&self.is_cancelled())
            .finish()
    }
}

/// Typed durable-commit evidence consumed by an expedited pass.
///
/// Production issuance remains inside the registered transport adapter. The
/// toolkit exposes no public constructor or field accessors.
pub struct CommittedRevisionProof {
    resource_uid: ResourceUid,
    generation: ResourceGeneration,
    revision: ZoneRevision,
    operation_id: String,
}

impl CommittedRevisionProof {
    pub(crate) fn issue(
        resource_uid: ResourceUid,
        generation: ResourceGeneration,
        revision: ZoneRevision,
        operation_id: String,
    ) -> Self {
        Self {
            resource_uid,
            generation,
            revision,
            operation_id,
        }
    }
}

impl core::fmt::Debug for CommittedRevisionProof {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CommittedRevisionProof")
            .field("has_resource_uid", &true)
            .field("generation", &self.generation)
            .field("revision", &self.revision)
            .field("has_operation_id", &true)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EffectGate {
    Ordinary,
    ExpeditedPending,
    ExpeditedCommitted,
}

/// A borrowed proof that external effects are permitted for this pass.
pub struct EffectPermit<'context> {
    _context: &'context ReconcileContext,
}

impl core::fmt::Debug for EffectPermit<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("EffectPermit(<redacted>)")
    }
}

/// One fresh, Zone-checked reconcile invocation context.
pub struct ReconcileContext {
    identity: ControllerIdentity,
    target: ResourceKey,
    revision: ZoneRevision,
    generation: ResourceGeneration,
    reasons: TriggerSet,
    high_water_revision: ZoneRevision,
    operation: OperationContext,
    attempt: u32,
    deadline_tick: u64,
    cancellation: Cancellation,
    policy_revision: u64,
    api_revision: u64,
    configuration_revision: ConfigurationGeneration,
    effect_gate: EffectGate,
}

impl ReconcileContext {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn ordinary(
        identity: ControllerIdentity,
        target: &ResourceSnapshot,
        dependencies: &[DependencySnapshot],
        reasons: TriggerSet,
        high_water_revision: ZoneRevision,
        operation: OperationContext,
        attempt: u32,
        deadline_tick: u64,
        cancellation: Cancellation,
        policy_revision: u64,
        api_revision: u64,
        configuration_revision: ConfigurationGeneration,
    ) -> Result<Self, ContextError> {
        Self::new(
            identity,
            target,
            dependencies,
            reasons,
            high_water_revision,
            operation,
            attempt,
            deadline_tick,
            cancellation,
            policy_revision,
            api_revision,
            configuration_revision,
            EffectGate::Ordinary,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn expedited_pending(
        identity: ControllerIdentity,
        target: &ResourceSnapshot,
        dependencies: &[DependencySnapshot],
        reasons: TriggerSet,
        high_water_revision: ZoneRevision,
        operation: OperationContext,
        attempt: u32,
        deadline_tick: u64,
        cancellation: Cancellation,
        policy_revision: u64,
        api_revision: u64,
        configuration_revision: ConfigurationGeneration,
    ) -> Result<Self, ContextError> {
        Self::new(
            identity,
            target,
            dependencies,
            reasons,
            high_water_revision,
            operation,
            attempt,
            deadline_tick,
            cancellation,
            policy_revision,
            api_revision,
            configuration_revision,
            EffectGate::ExpeditedPending,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new(
        identity: ControllerIdentity,
        target: &ResourceSnapshot,
        dependencies: &[DependencySnapshot],
        reasons: TriggerSet,
        high_water_revision: ZoneRevision,
        operation: OperationContext,
        attempt: u32,
        deadline_tick: u64,
        cancellation: Cancellation,
        policy_revision: u64,
        api_revision: u64,
        configuration_revision: ConfigurationGeneration,
        effect_gate: EffectGate,
    ) -> Result<Self, ContextError> {
        if identity.zone != *target.key.zone()
            || dependencies
                .iter()
                .any(|dependency| dependency.resource.key.zone() != target.key.zone())
        {
            return Err(ContextError::ZoneMismatch);
        }
        if high_water_revision < target.revision {
            return Err(ContextError::HighWaterBehindSnapshot);
        }
        Ok(Self {
            identity,
            target: target.key.clone(),
            revision: target.revision,
            generation: target.generation,
            reasons,
            high_water_revision,
            operation,
            attempt,
            deadline_tick,
            cancellation,
            policy_revision,
            api_revision,
            configuration_revision,
            effect_gate,
        })
    }

    pub(crate) fn bind_committed_proof(
        mut self,
        proof: CommittedRevisionProof,
    ) -> Result<Self, ContextError> {
        if self.effect_gate != EffectGate::ExpeditedPending {
            return Err(ContextError::UnexpectedCommitProof);
        }
        if proof.resource_uid != *self.target.uid()
            || proof.generation != self.generation
            || proof.revision != self.revision
            || proof.operation_id != self.operation.operation_id
        {
            return Err(ContextError::CommitProofMismatch);
        }
        self.effect_gate = EffectGate::ExpeditedCommitted;
        Ok(self)
    }

    /// Borrow the registered identity.
    pub const fn identity(&self) -> &ControllerIdentity {
        &self.identity
    }

    /// Borrow the target key.
    pub const fn target(&self) -> &ResourceKey {
        &self.target
    }

    /// Return the fresh target revision.
    pub const fn revision(&self) -> ZoneRevision {
        self.revision
    }

    /// Return the target generation.
    pub const fn generation(&self) -> ResourceGeneration {
        self.generation
    }

    /// Borrow all coalesced reasons.
    pub const fn reasons(&self) -> &TriggerSet {
        &self.reasons
    }

    /// Return the admitted high-water revision.
    pub const fn high_water_revision(&self) -> ZoneRevision {
        self.high_water_revision
    }

    /// Borrow operation correlation.
    pub const fn operation(&self) -> &OperationContext {
        &self.operation
    }

    /// Return the one-based attempt number.
    pub const fn attempt(&self) -> u32 {
        self.attempt
    }

    /// Return the monotonic deadline tick.
    pub const fn deadline_tick(&self) -> u64 {
        self.deadline_tick
    }

    /// Borrow cancellation state.
    pub const fn cancellation(&self) -> &Cancellation {
        &self.cancellation
    }

    /// Return policy, API, and configuration revisions.
    pub const fn revisions(&self) -> (u64, u64, ConfigurationGeneration) {
        (
            self.policy_revision,
            self.api_revision,
            self.configuration_revision,
        )
    }

    /// Whether this pass is bound to an expedited committed mutation.
    pub const fn is_expedited(&self) -> bool {
        matches!(
            self.effect_gate,
            EffectGate::ExpeditedPending | EffectGate::ExpeditedCommitted
        )
    }

    /// Borrow a non-reusable effect permit after all gates pass.
    pub fn authorize_effect(&self) -> Result<EffectPermit<'_>, ContextError> {
        match self.effect_gate {
            EffectGate::Ordinary | EffectGate::ExpeditedCommitted => {
                Ok(EffectPermit { _context: self })
            }
            EffectGate::ExpeditedPending => Err(ContextError::CommitProofRequired),
        }
    }
}

impl core::fmt::Debug for ReconcileContext {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ReconcileContext")
            .field("identity", &self.identity)
            .field("target", &self.target)
            .field("revision", &self.revision)
            .field("generation", &self.generation)
            .field("reasons", &self.reasons)
            .field("high_water_revision", &self.high_water_revision)
            .field("operation", &self.operation)
            .field("attempt", &self.attempt)
            .field("deadline_tick", &self.deadline_tick)
            .field("cancellation", &self.cancellation)
            .field("policy_revision", &self.policy_revision)
            .field("api_revision", &self.api_revision)
            .field("configuration_revision", &self.configuration_revision)
            .field("effect_gate", &self.effect_gate)
            .finish()
    }
}

/// Invalid context or expedited proof binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextError {
    ZoneMismatch,
    HighWaterBehindSnapshot,
    InvalidControllerIdentity,
    InvalidOperationIdentity,
    CommitProofRequired,
    UnexpectedCommitProof,
    CommitProofMismatch,
}

impl core::fmt::Display for ContextError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::ZoneMismatch => "reconcile inputs must have one registered Zone",
            Self::HighWaterBehindSnapshot => "high-water revision is behind the fresh snapshot",
            Self::InvalidControllerIdentity => {
                "controller identity must bind Process, Provider, Host, and optional Guest types"
            }
            Self::InvalidOperationIdentity => "operation identity is empty or oversized",
            Self::CommitProofRequired => "expedited effects require durable commit proof",
            Self::UnexpectedCommitProof => "commit proof is not valid for this pass",
            Self::CommitProofMismatch => "commit proof does not match the fresh target",
        })
    }
}

impl std::error::Error for ContextError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(zone: &str, name: &str, uid: &str) -> ResourceKey {
        ResourceKey::new(
            ZoneId::parse(zone).unwrap(),
            ResourceRef::parse(&format!("Process/{name}")).unwrap(),
            ResourceUid::parse(uid).unwrap(),
        )
    }

    fn snapshot(zone: &str, name: &str, uid: &str) -> ResourceSnapshot {
        ResourceSnapshot::new(
            key(zone, name, uid),
            ZoneRevision::new(4),
            ResourceGeneration::new(2).unwrap(),
            b"{}".to_vec(),
            false,
        )
    }

    fn identity(zone: &str) -> ControllerIdentity {
        ControllerIdentity::new(
            ZoneId::parse(zone).unwrap(),
            ResourceRef::parse("Process/controller").unwrap(),
            ControllerGeneration::new(3).unwrap(),
            ResourceRef::parse("Provider/runtime").unwrap(),
            ResourceGeneration::new(4).unwrap(),
            ResourceRef::parse("Process/controller").unwrap(),
            ResourceRef::parse("Host/system").unwrap(),
            None,
        )
        .unwrap()
    }

    fn operation() -> OperationContext {
        OperationContext::new("op-1", "idem-1", "corr-1", None).unwrap()
    }

    fn pending_context() -> ReconcileContext {
        let target = snapshot("work", "app", "123e4567-e89b-42d3-a456-426614174000");
        ReconcileContext::expedited_pending(
            identity("work"),
            &target,
            &[],
            TriggerSet::new([TriggerReason::ExpeditedMutation]),
            ZoneRevision::new(4),
            operation(),
            1,
            20,
            Cancellation::default(),
            5,
            6,
            ConfigurationGeneration::new(7).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn trigger_union_keeps_non_droppable_causes() {
        let mut reasons = TriggerSet::new([
            TriggerReason::SpecGenerationChanged,
            TriggerReason::OwnedResourceChanged,
        ]);
        reasons.union_with(&TriggerSet::new([
            TriggerReason::DeletionRequested,
            TriggerReason::PolicyChanged,
        ]));

        assert_eq!(reasons.len(), 4);
        for reason in [
            TriggerReason::OwnedResourceChanged,
            TriggerReason::DeletionRequested,
            TriggerReason::PolicyChanged,
        ] {
            assert!(reasons.contains(reason));
            assert!(reason.is_non_droppable());
        }
    }

    #[test]
    fn controller_identity_rejects_type_confusion_before_registration() {
        assert_eq!(
            ControllerIdentity::new(
                ZoneId::parse("work").unwrap(),
                ResourceRef::parse("Process/controller").unwrap(),
                ControllerGeneration::new(3).unwrap(),
                ResourceRef::parse("Guest/not-a-provider").unwrap(),
                ResourceGeneration::new(4).unwrap(),
                ResourceRef::parse("Process/controller").unwrap(),
                ResourceRef::parse("Host/system").unwrap(),
                None,
            )
            .unwrap_err(),
            ContextError::InvalidControllerIdentity
        );
    }

    #[test]
    fn zone_mismatch_is_rejected_before_context_mint() {
        let target = snapshot("work", "app", "123e4567-e89b-42d3-a456-426614174000");
        let dependency = DependencySnapshot::new(snapshot(
            "personal",
            "dependency",
            "123e4567-e89b-42d3-a456-426614174001",
        ));
        let result = ReconcileContext::ordinary(
            identity("work"),
            &target,
            &[dependency],
            TriggerSet::new([TriggerReason::DependencyChanged]),
            ZoneRevision::new(4),
            operation(),
            1,
            20,
            Cancellation::default(),
            5,
            6,
            ConfigurationGeneration::new(7).unwrap(),
        );

        assert_eq!(result.unwrap_err(), ContextError::ZoneMismatch);
    }

    #[test]
    fn expedited_effect_is_denied_until_matching_proof_is_consumed() {
        let pending = pending_context();
        assert_eq!(
            pending.authorize_effect().unwrap_err(),
            ContextError::CommitProofRequired
        );

        let proof = CommittedRevisionProof::issue(
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            ResourceGeneration::new(2).unwrap(),
            ZoneRevision::new(4),
            "op-1".to_owned(),
        );
        let committed = pending.bind_committed_proof(proof).unwrap();
        assert!(committed.authorize_effect().is_ok());
    }

    #[test]
    fn mismatched_proof_never_mints_effect_permission() {
        let proof = CommittedRevisionProof::issue(
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174009").unwrap(),
            ResourceGeneration::new(2).unwrap(),
            ZoneRevision::new(4),
            "op-1".to_owned(),
        );
        assert_eq!(
            pending_context().bind_committed_proof(proof).unwrap_err(),
            ContextError::CommitProofMismatch
        );
    }

    #[test]
    fn protected_context_diagnostics_are_redacted() {
        const ZONE: &str = "debug-zone-sentinel";
        const NAME: &str = "debug-name-sentinel";
        const UID: &str = "deadbeef-dead-4bad-8bad-deadbeef0001";
        const OPERATION: &str = "debug-operation-sentinel";

        let target = snapshot(ZONE, NAME, UID);
        let context = ReconcileContext::ordinary(
            identity(ZONE),
            &target,
            &[],
            TriggerSet::new([TriggerReason::ManualReconcile]),
            ZoneRevision::new(4),
            OperationContext::new(OPERATION, OPERATION, OPERATION, Some(OPERATION.to_owned()))
                .unwrap(),
            1,
            20,
            Cancellation::default(),
            5,
            6,
            ConfigurationGeneration::new(7).unwrap(),
        )
        .unwrap();

        assert_eq!(context.target().zone().as_str(), ZONE);
        assert_eq!(context.target().resource_ref().name().as_str(), NAME);
        assert_eq!(context.target().uid().as_str(), UID);
        assert_eq!(context.operation().operation_id(), OPERATION);
        let debug = format!("{context:?}");
        for sentinel in [ZONE, NAME, UID, OPERATION] {
            assert!(!debug.contains(sentinel), "{debug}");
        }
    }
}
