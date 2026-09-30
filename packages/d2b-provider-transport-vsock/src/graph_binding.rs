//! Graph-bound attach authority for one admitted vsock transport relationship.
//!
//! The vsock service keeps no access list of its own. Every attach is a check
//! of live evidence against the fence committed for one admitted
//! `EndpointBinding` relationship, and the Guest, Zone, CID, boot, and
//! reconnect generation a session is pinned to are the evidence the graph
//! committed rather than a list the transport maintains. A revoked
//! relationship stops admitting before any nonce is consumed and before the
//! proof is checked, so a reconnect can never revive revoked authority.

use crate::bridge::ControlRouteToken;
use d2b_contracts_resource::v3::{
    AdmissionStage, BindingKey, BindingRefusal, DesiredRevision, EndpointAttachmentKind,
    EndpointBindingRequest, RefusalReason, ResourceGeneration, StoreIncarnation,
    ZoneDesiredSequence, ZoneId, identity::ReconnectGeneration,
};
use std::{collections::HashMap, error::Error, fmt, sync::Mutex};

/// Highest number of admitted transport relationships one service retains.
///
/// The ceiling is frozen: exceeding it is a refusal, never an allocation
/// growth, so a relationship storm can never become memory growth.
pub const MAX_ADMITTED_TRANSPORT_BINDINGS: usize = 256;

/// The lifecycle a transport observes for one admitted relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationshipPhase {
    /// The relationship is admitted and may open new transports.
    Admitted,
    /// Outstanding use is being driven to its declared safe state.
    Draining,
    /// The relationship is revoked; nothing new may attach to it.
    Revoked,
}

/// The committed evidence one admitted relationship is fenced against.
///
/// The fence is what a stale peer, a stale boot, or a reconnected session is
/// measured against. Advancing any of its fields is the graph saying "older
/// evidence no longer decides this relationship", so every attach attempt
/// carrying the previous value refuses at the matching stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationshipFence {
    store: StoreIncarnation,
    desired_revision: DesiredRevision,
    sequence: ZoneDesiredSequence,
    source_generation: ResourceGeneration,
    consumer_generation: ResourceGeneration,
    minimum_reconnect: ReconnectGeneration,
    phase: RelationshipPhase,
}

impl RelationshipFence {
    /// Construct the fence of a freshly admitted relationship.
    ///
    /// A new relationship starts `Admitted`; every other phase is reached
    /// through a transition, never by declaring it at construction.
    pub fn new(
        store: StoreIncarnation,
        desired_revision: DesiredRevision,
        sequence: ZoneDesiredSequence,
        source_generation: ResourceGeneration,
        consumer_generation: ResourceGeneration,
        minimum_reconnect: ReconnectGeneration,
    ) -> Self {
        Self {
            store,
            desired_revision,
            sequence,
            source_generation,
            consumer_generation,
            minimum_reconnect,
            phase: RelationshipPhase::Admitted,
        }
    }

    /// Borrow the store incarnation the relationship was admitted in.
    pub const fn store(&self) -> &StoreIncarnation {
        &self.store
    }

    /// Return the committed desired revision the relationship is fenced at.
    pub const fn desired_revision(&self) -> DesiredRevision {
        self.desired_revision
    }

    /// Return the Zone desired sequence the relationship is fenced at.
    pub const fn sequence(&self) -> ZoneDesiredSequence {
        self.sequence
    }

    /// Return the source generation the relationship is fenced at.
    pub const fn source_generation(&self) -> ResourceGeneration {
        self.source_generation
    }

    /// Return the consumer generation the relationship is fenced at.
    pub const fn consumer_generation(&self) -> ResourceGeneration {
        self.consumer_generation
    }

    /// Return the lowest reconnect generation this relationship still admits.
    pub const fn minimum_reconnect(&self) -> ReconnectGeneration {
        self.minimum_reconnect
    }

    /// Return the lifecycle phase the relationship is in.
    pub const fn phase(&self) -> RelationshipPhase {
        self.phase
    }

    /// Move the relationship to one lifecycle phase.
    #[must_use]
    pub fn set_phase(mut self, phase: RelationshipPhase) -> Self {
        self.phase = phase;
        self
    }

    /// Move the relationship to the revoking phase.
    #[must_use]
    pub fn revoke(self) -> Self {
        self.set_phase(RelationshipPhase::Revoked)
    }

    /// Move the relationship to its draining phase.
    #[must_use]
    pub fn drain(self) -> Self {
        self.set_phase(RelationshipPhase::Draining)
    }

    /// Fence the relationship at a newer committed desired revision.
    #[must_use]
    pub fn advance_desired_revision(mut self, desired_revision: DesiredRevision) -> Self {
        self.desired_revision = desired_revision;
        self
    }

    /// Fence the relationship at a newer Zone desired sequence.
    #[must_use]
    pub fn advance_sequence(mut self, sequence: ZoneDesiredSequence) -> Self {
        self.sequence = sequence;
        self
    }

    /// Raise the lowest reconnect generation this relationship admits.
    #[must_use]
    pub fn raise_minimum_reconnect(mut self, minimum_reconnect: ReconnectGeneration) -> Self {
        self.minimum_reconnect = minimum_reconnect;
        self
    }
}

/// Live, non-secret observations one peer presents to attach.
///
/// Evidence is measured against the fence and never merges with it: a peer
/// cannot move a fence by presenting evidence, and a fence cannot adopt a
/// peer's claims without the graph having committed them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportAttachEvidence {
    zone: ZoneId,
    store: StoreIncarnation,
    source_generation: ResourceGeneration,
    consumer_generation: ResourceGeneration,
    desired_revision: DesiredRevision,
    sequence: ZoneDesiredSequence,
    reconnect: ReconnectGeneration,
}

impl TransportAttachEvidence {
    /// Construct the evidence one attach attempt presents.
    pub fn new(
        zone: ZoneId,
        store: StoreIncarnation,
        source_generation: ResourceGeneration,
        consumer_generation: ResourceGeneration,
        desired_revision: DesiredRevision,
        sequence: ZoneDesiredSequence,
        reconnect: ReconnectGeneration,
    ) -> Self {
        Self {
            zone,
            store,
            source_generation,
            consumer_generation,
            desired_revision,
            sequence,
            reconnect,
        }
    }

    /// Borrow the Zone the peer claims to be operating in.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the store incarnation the peer observed.
    pub const fn store(&self) -> &StoreIncarnation {
        &self.store
    }

    /// Return the source generation the peer observed.
    pub const fn source_generation(&self) -> ResourceGeneration {
        self.source_generation
    }

    /// Return the consumer generation the peer observed.
    pub const fn consumer_generation(&self) -> ResourceGeneration {
        self.consumer_generation
    }

    /// Return the desired revision the peer observed.
    pub const fn desired_revision(&self) -> DesiredRevision {
        self.desired_revision
    }

    /// Return the Zone desired sequence the peer observed.
    pub const fn sequence(&self) -> ZoneDesiredSequence {
        self.sequence
    }

    /// Return the reconnect generation this attempt is.
    pub const fn reconnect(&self) -> ReconnectGeneration {
        self.reconnect
    }
}

/// One admitted `EndpointBinding` relationship a transport session realizes.
///
/// The relationship carries its own key, its own desired request, and the
/// fence its evidence is measured against, so a transport never decides on
/// its own that a peer may attach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedTransportBinding {
    key: BindingKey,
    request: EndpointBindingRequest,
    fence: RelationshipFence,
}

impl AdmittedTransportBinding {
    /// Construct one admitted relationship from its committed identities.
    pub fn new(
        key: BindingKey,
        request: EndpointBindingRequest,
        fence: RelationshipFence,
    ) -> Self {
        Self {
            key,
            request,
            fence,
        }
    }

    /// Borrow the relationship's KTD3 key.
    pub const fn key(&self) -> &BindingKey {
        &self.key
    }

    /// Borrow the admitted endpoint request.
    pub const fn request(&self) -> &EndpointBindingRequest {
        &self.request
    }

    /// Borrow the fence evidence is measured against.
    pub const fn fence(&self) -> &RelationshipFence {
        &self.fence
    }

    /// Return the attachment kind the relationship was admitted for.
    pub const fn attachment(&self) -> EndpointAttachmentKind {
        self.request.attachment()
    }

    /// Fence this relationship at a newer committed state.
    #[must_use]
    pub fn with_fence(mut self, fence: RelationshipFence) -> Self {
        self.fence = fence;
        self
    }
}

/// The single-use value the attach gate returns; consumed by the open path.
///
/// The route exists only when [`admit_attach`] admitted the presented evidence
/// against the live fence, and it has no public constructor, so an open path
/// cannot skip the gate to obtain one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedTransportRoute {
    key: BindingKey,
    attachment: EndpointAttachmentKind,
    fence: RelationshipFence,
}

impl AdmittedTransportRoute {
    fn new(
        key: BindingKey,
        attachment: EndpointAttachmentKind,
        fence: RelationshipFence,
    ) -> Self {
        Self {
            key,
            attachment,
            fence,
        }
    }

    /// Borrow the relationship this route realizes.
    pub const fn key(&self) -> &BindingKey {
        &self.key
    }

    /// Return the attachment kind the relationship was admitted for.
    pub const fn attachment(&self) -> EndpointAttachmentKind {
        self.attachment
    }

    /// Borrow the fence the gate admitted against.
    pub const fn fence(&self) -> &RelationshipFence {
        &self.fence
    }

    /// Mint the opaque token a privileged in-process control call must carry.
    ///
    /// The token is derived from a live admitted route and has no public
    /// constructor, no reader, and no deserializer, so bytes on a named
    /// stream can never be turned into one.
    pub fn control_token(&self) -> ControlRouteToken {
        ControlRouteToken::for_route(self.key.clone())
    }
}

/// One closed, field-free refusal from the graph-bound attach gate.
///
/// The stage and reason are the graph's own vocabulary, and the code is a
/// stable field-free label: a refusal never echoes a zone, a CID, a handle, or
/// caller-supplied text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportAttachRefusal {
    stage: AdmissionStage,
    reason: RefusalReason,
    code: &'static str,
}

impl TransportAttachRefusal {
    pub(crate) const fn at(
        stage: AdmissionStage,
        reason: RefusalReason,
        code: &'static str,
    ) -> Self {
        Self { stage, reason, code }
    }

    /// The refusal for an attempt naming a relationship this service holds no
    /// admitted binding for.
    ///
    /// A relationship the registry does not hold is not a peer this service
    /// ever admitted, so the attempt is refused as an unauthorized identity
    /// rather than being realized.
    pub const fn relationship_not_admitted() -> Self {
        Self::at(
            AdmissionStage::Authorize,
            RefusalReason::IdentityNotAuthorized,
            "relationship-not-admitted",
        )
    }

    /// Return the enforcing stage.
    pub const fn stage(self) -> AdmissionStage {
        self.stage
    }

    /// Return the typed refusal reason.
    pub const fn reason(self) -> RefusalReason {
        self.reason
    }

    /// Return the closed, stable refusal code.
    pub const fn code(self) -> &'static str {
        self.code
    }
}

impl fmt::Display for TransportAttachRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code)
    }
}

impl Error for TransportAttachRefusal {}

impl From<BindingRefusal> for TransportAttachRefusal {
    fn from(value: BindingRefusal) -> Self {
        Self::at(
            value.stage(),
            value.reason(),
            contract_refusal_code(value.reason()),
        )
    }
}

fn contract_refusal_code(reason: RefusalReason) -> &'static str {
    match reason {
        RefusalReason::PolicySelectionNotAuthorized => "policy-selection-not-authorized",
        RefusalReason::RequiredNamespaceNotAdmitted => "required-namespace-not-admitted",
        RefusalReason::RequiredCapabilityOutsideCeiling => "required-capability-outside-ceiling",
        RefusalReason::RestrictionWeakened => "restriction-weakened",
        RefusalReason::IdentityNotAuthorized => "identity-not-authorized",
        RefusalReason::MandatoryFacetUnsupported => "mandatory-facet-unsupported",
        RefusalReason::SeccompIncompatible => "seccomp-incompatible",
        RefusalReason::LimitExceedsCeiling => "limit-exceeds-ceiling",
        RefusalReason::EmergencyReductionActive => "emergency-reduction-active",
        RefusalReason::TargetSupportMissing => "target-support-missing",
        RefusalReason::ConflictingDeclaration => "conflicting-declaration",
        RefusalReason::SourcePolicyRefused => "source-policy-refused",
        RefusalReason::StaleAuthority => "stale-authority",
        RefusalReason::StoreIncarnationMismatch => "store-incarnation-mismatch",
        RefusalReason::UnprovenEffect => "unproven-effect",
        RefusalReason::UntrustedImplementation => "untrusted-implementation",
    }
}

/// The one ordered gate every graph-bound attach path in this crate runs.
///
/// It stops at the first refusal, so the reported stage is always the earliest
/// reason the attempt is not admitted, and evidence is only ever read: no
/// check here can move a fence, so a reconnect that presents older or foreign
/// evidence cannot make itself current.
pub fn admit_attach(
    binding: &AdmittedTransportBinding,
    evidence: &TransportAttachEvidence,
) -> Result<AdmittedTransportRoute, TransportAttachRefusal> {
    measure(binding.key(), binding.fence(), evidence)?;
    Ok(AdmittedTransportRoute::new(
        binding.key().clone(),
        binding.attachment(),
        binding.fence().clone(),
    ))
}

/// Re-check one already issued route against the fence it was minted from.
///
/// This is the same ordered gate, read from the route rather than from a live
/// relationship, so a session carrying an older route cannot make its own
/// snapshot look current.
pub fn admit_route(
    route: &AdmittedTransportRoute,
    evidence: &TransportAttachEvidence,
) -> Result<AdmittedTransportRoute, TransportAttachRefusal> {
    measure(route.key(), route.fence(), evidence)?;
    Ok(AdmittedTransportRoute::new(
        route.key().clone(),
        route.attachment(),
        route.fence().clone(),
    ))
}

fn measure(
    key: &BindingKey,
    fence: &RelationshipFence,
    evidence: &TransportAttachEvidence,
) -> Result<(), TransportAttachRefusal> {
    if evidence.zone() != key.zone() {
        return Err(TransportAttachRefusal::at(
            AdmissionStage::Authorize,
            RefusalReason::IdentityNotAuthorized,
            "foreign-zone",
        ));
    }
    if evidence.store() != fence.store() {
        return Err(TransportAttachRefusal::at(
            AdmissionStage::Authorize,
            RefusalReason::StoreIncarnationMismatch,
            "store-incarnation-mismatch",
        ));
    }
    match fence.phase() {
        RelationshipPhase::Revoked => {
            return Err(TransportAttachRefusal::at(
                AdmissionStage::Revoke,
                RefusalReason::StaleAuthority,
                "relationship-revoked",
            ));
        }
        RelationshipPhase::Draining => {
            return Err(TransportAttachRefusal::at(
                AdmissionStage::Drain,
                RefusalReason::UnprovenEffect,
                "relationship-draining",
            ));
        }
        RelationshipPhase::Admitted => {}
    }
    if evidence.source_generation() != fence.source_generation() {
        return Err(TransportAttachRefusal::at(
            AdmissionStage::Admit,
            RefusalReason::StaleAuthority,
            "stale-source-generation",
        ));
    }
    if evidence.consumer_generation() != fence.consumer_generation() {
        return Err(TransportAttachRefusal::at(
            AdmissionStage::Admit,
            RefusalReason::StaleAuthority,
            "stale-consumer-generation",
        ));
    }
    if evidence.desired_revision() != fence.desired_revision() {
        return Err(TransportAttachRefusal::at(
            AdmissionStage::Authorize,
            RefusalReason::StaleAuthority,
            "stale-desired-revision",
        ));
    }
    if evidence.sequence() != fence.sequence() {
        return Err(TransportAttachRefusal::at(
            AdmissionStage::Authorize,
            RefusalReason::StaleAuthority,
            "stale-desired-sequence",
        ));
    }
    if evidence.reconnect().get() < fence.minimum_reconnect().get() {
        return Err(TransportAttachRefusal::at(
            AdmissionStage::Activate,
            RefusalReason::StaleAuthority,
            "stale-reconnect-generation",
        ));
    }
    Ok(())
}

/// Fail-closed outcome of a registry-level relationship operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportBindingRefusal {
    /// The registry already holds a relationship under this key.
    AlreadyAdmitted,
    /// The registry is at its frozen ceiling.
    RegistryFull,
    /// No admitted relationship carries this key.
    NotAdmitted,
    /// The registry lock is unavailable after an internal failure.
    RegistryUnavailable,
}

impl TransportBindingRefusal {
    /// Return the closed, stable refusal code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::AlreadyAdmitted => "binding-already-admitted",
            Self::RegistryFull => "binding-registry-full",
            Self::NotAdmitted => "binding-not-admitted",
            Self::RegistryUnavailable => "binding-registry-unavailable",
        }
    }
}

impl fmt::Display for TransportBindingRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for TransportBindingRefusal {}

struct RegistryState {
    ceiling: usize,
    relationships: HashMap<BindingKey, AdmittedTransportBinding>,
}

/// The bounded, service-local set of admitted transport relationships.
///
/// This is the only list that can say a peer may attach, and it holds admitted
/// relationships keyed by their graph identity rather than by peer identity.
/// A peer that is not in here has no attach path at all.
pub struct TransportBindingRegistry {
    state: Mutex<RegistryState>,
}

impl TransportBindingRegistry {
    /// Create an empty registry at the production ceiling.
    #[must_use]
    pub fn new() -> Self {
        Self::with_ceiling(MAX_ADMITTED_TRANSPORT_BINDINGS)
    }

    /// Create an empty registry with a smaller frozen ceiling.
    ///
    /// A ceiling above the production bound is clamped to it, so this
    /// constructor can narrow the registry but never raise it.
    #[must_use]
    pub fn with_ceiling(ceiling: usize) -> Self {
        Self {
            state: Mutex::new(RegistryState {
                ceiling: ceiling.min(MAX_ADMITTED_TRANSPORT_BINDINGS),
                relationships: HashMap::new(),
            }),
        }
    }

    /// Return the frozen ceiling this registry admits up to.
    pub fn ceiling(&self) -> usize {
        self.state.try_lock().map_or(0, |state| state.ceiling)
    }

    /// Admit one relationship and return the route its evidence is measured
    /// against.
    ///
    /// # Errors
    ///
    /// Returns `AlreadyAdmitted` when a relationship already occupies the
    /// key, `RegistryFull` at the frozen ceiling, and `RegistryUnavailable`
    /// when the registry lock is unavailable after an internal failure.
    pub fn admit(
        &self,
        binding: AdmittedTransportBinding,
    ) -> Result<AdmittedTransportRoute, TransportBindingRefusal> {
        let mut state = self
            .state
            .try_lock()
            .map_err(|_| TransportBindingRefusal::RegistryUnavailable)?;
        if state.relationships.contains_key(binding.key()) {
            return Err(TransportBindingRefusal::AlreadyAdmitted);
        }
        if state.relationships.len() >= state.ceiling {
            return Err(TransportBindingRefusal::RegistryFull);
        }
        state
            .relationships
            .insert(binding.key().clone(), binding.clone());
        Ok(AdmittedTransportRoute::new(
            binding.key().clone(),
            binding.attachment(),
            binding.fence().clone(),
        ))
    }

    /// Return the live relationship admitted under one key.
    pub fn binding(&self, key: &BindingKey) -> Option<AdmittedTransportBinding> {
        self.state.try_lock().ok()?.relationships.get(key).cloned()
    }

    /// Re-measure live evidence against the relationship this key names.
    ///
    /// This is the reconnect entry point: a reconnecting peer presents fresh
    /// evidence and receives a fresh route, or the relationship's current
    /// refusal.
    ///
    /// # Errors
    ///
    /// Returns [`TransportAttachRefusal::relationship_not_admitted`] when
    /// this registry holds no relationship under the key, and the ordered
    /// attach refusal otherwise.
    pub fn route(
        &self,
        key: &BindingKey,
        evidence: &TransportAttachEvidence,
    ) -> Result<AdmittedTransportRoute, TransportAttachRefusal> {
        let binding = self
            .binding(key)
            .ok_or_else(TransportAttachRefusal::relationship_not_admitted)?;
        admit_attach(&binding, evidence)
    }

    /// Revoke one live relationship, keeping it observable but unattachable.
    ///
    /// # Errors
    ///
    /// Returns `NotAdmitted` when no relationship carries the key, and
    /// `RegistryUnavailable` when the registry lock is unavailable.
    pub fn revoke(&self, key: &BindingKey) -> Result<(), TransportBindingRefusal> {
        self.transition(key, RelationshipPhase::Revoked)
    }

    /// Move one live relationship into its draining phase.
    ///
    /// # Errors
    ///
    /// Returns `NotAdmitted` when no relationship carries the key, and
    /// `RegistryUnavailable` when the registry lock is unavailable.
    pub fn drain(&self, key: &BindingKey) -> Result<(), TransportBindingRefusal> {
        self.transition(key, RelationshipPhase::Draining)
    }

    fn transition(
        &self,
        key: &BindingKey,
        phase: RelationshipPhase,
    ) -> Result<(), TransportBindingRefusal> {
        let mut state = self
            .state
            .try_lock()
            .map_err(|_| TransportBindingRefusal::RegistryUnavailable)?;
        let binding = state
            .relationships
            .get(key)
            .ok_or(TransportBindingRefusal::NotAdmitted)?
            .clone();
        let fence = binding.fence().clone().set_phase(phase);
        state
            .relationships
            .insert(key.clone(), binding.with_fence(fence));
        Ok(())
    }

    /// Drop one live relationship's authority entirely.
    ///
    /// # Errors
    ///
    /// Returns `NotAdmitted` when no relationship carries the key, and
    /// `RegistryUnavailable` when the registry lock is unavailable.
    pub fn forget(&self, key: &BindingKey) -> Result<(), TransportBindingRefusal> {
        let mut state = self
            .state
            .try_lock()
            .map_err(|_| TransportBindingRefusal::RegistryUnavailable)?;
        state
            .relationships
            .remove(key)
            .map(|_| ())
            .ok_or(TransportBindingRefusal::NotAdmitted)
    }

    /// Return how many relationships are currently admitted.
    ///
    /// A registry whose lock is unavailable admits nothing, so it reports
    /// zero rather than a count it cannot stand behind.
    pub fn len(&self) -> usize {
        self.state
            .try_lock()
            .map_or(0, |state| state.relationships.len())
    }

    /// Return whether no relationship is admitted.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Retire every admitted relationship during service finalization.
    pub fn finalize(&self) {
        if let Ok(mut state) = self.state.try_lock() {
            state.relationships.clear();
        }
    }
}

impl Default for TransportBindingRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TransportBindingRegistry {
    fn drop(&mut self) {
        self.finalize();
    }
}

impl fmt::Debug for TransportBindingRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TransportBindingRegistry(REDACTED)")
    }
}
