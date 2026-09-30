//! Graph-bound attach authority for one admitted relay carriage.
//!
//! The Relay Provider keeps no access list of its own. Every carriage is a
//! check of live evidence against the fence committed for one admitted
//! `CredentialBinding` (and, where the relay also realizes an egress endpoint,
//! one admitted `EndpointBinding`), so a relay identity - a SAS, a bearer, a
//! socket - is never local grant authority. The data plane stays a data plane:
//! the privileged control operations below are in-process calls, and no byte
//! sequence that arrives on a relay stream can become one of them.
//!
//! ## Two audiences, two layers, never interchangeable
//!
//! This module owns [`RELAY_CREDENTIAL_AUDIENCE`], the *graph* audience: the
//! bounded lower-kebab token a `CredentialBinding` declares it delivers for.
//! [`crate::auth::RELAY_TOKEN_RESOURCE`] is the *wire* audience: the Entra
//! resource a SAS or bearer is minted for inside `auth.rs`. They are
//! deliberately different values at different layers. The wire audience is a
//! URL and cannot even be spelled as a `BoundedToken`, so the graph audience
//! must not be a copy of it, and neither may be reused for the other: a
//! relationship admitted for a wrong-layer audience would authorize a
//! credential whose bearer is minted for something else entirely.

use std::{collections::HashMap, error::Error, fmt, sync::Mutex};

use d2b_contracts_resource::v3::{
    AdmissionStage, BindingKey, BindingRefusal, BoundedToken, CredentialBindingRequest,
    CredentialOperation, DesiredRevision, RefusalReason, ResourceGeneration, StoreIncarnation,
    ZoneDesiredSequence, ZoneId, identity::ReconnectGeneration,
};

/// The graph audience a relay `CredentialBinding` must declare.
///
/// This is the bounded token the resource graph admits a delivery for, not the
/// wire-protocol audience the SAS or bearer is minted for; see the module
/// documentation for why the two layers must stay separate.
pub const RELAY_CREDENTIAL_AUDIENCE: &str = "relay-egress";

/// Highest number of admitted transport relationships one Provider retains.
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
/// carrying the previous value refuses at the matching stage. Revocation is
/// a fence transition like any other: it is retained rather than dropped, so a
/// reconnect that presents the same or newer evidence still refuses instead of
/// minting a second carriage from the connection that just released.
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

    /// Move the relationship to the draining phase.
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

/// One admitted `CredentialBinding` relationship a relay carriage realizes.
///
/// The relationship carries its own key, its own desired request, and the
/// fence its evidence is measured against, so a transport never decides on
/// its own that a peer may attach. The admitted audience and operation set
/// live on the request, not beside it, so a caller cannot widen them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedTransportBinding {
    key: BindingKey,
    request: CredentialBindingRequest,
    fence: RelationshipFence,
}

impl AdmittedTransportBinding {
    /// Construct one admitted relationship from its committed identities.
    pub fn new(key: BindingKey, request: CredentialBindingRequest, fence: RelationshipFence) -> Self {
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

    /// Borrow the admitted credential request.
    pub const fn request(&self) -> &CredentialBindingRequest {
        &self.request
    }

    /// Borrow the fence evidence is measured against.
    pub const fn fence(&self) -> &RelationshipFence {
        &self.fence
    }

    /// Return the audience the relationship was admitted for.
    pub const fn audience(&self) -> &BoundedToken {
        self.request.audience()
    }

    /// Whether the relationship admits `operation` for its admitted audience.
    pub fn admits_operation(&self, operation: CredentialOperation) -> bool {
        self.request.admits_operation(operation)
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
    audience: BoundedToken,
    fence: RelationshipFence,
}

impl AdmittedTransportRoute {
    fn new(key: BindingKey, audience: BoundedToken, fence: RelationshipFence) -> Self {
        Self {
            key,
            audience,
            fence,
        }
    }

    /// Borrow the relationship this route realizes.
    pub const fn key(&self) -> &BindingKey {
        &self.key
    }

    /// Borrow the audience the gate admitted for.
    pub const fn audience(&self) -> &BoundedToken {
        &self.audience
    }

    /// Borrow the fence the gate admitted against.
    pub const fn fence(&self) -> &RelationshipFence {
        &self.fence
    }

    /// Mint the opaque token a privileged in-process control call must carry.
    ///
    /// The token is derived from a live admitted route and has no public
    /// constructor, no reader, and no deserializer, so bytes on a relay stream
    /// can never be turned into one.
    pub fn control_token(&self) -> ControlRouteToken {
        ControlRouteToken {
            _route: self.key.clone(),
        }
    }

    /// Mint the carriage handle a privileged in-process control call targets.
    pub fn carriage(&self) -> RelayCarriageHandle {
        RelayCarriageHandle {
            _route: self.key.clone(),
        }
    }
}

/// The two admitted relationships one relay carriage realizes.
///
/// A relay carriage is credential delivery, and it may additionally realize
/// the Provider's own egress endpoint. Both halves are graph relationships:
/// the delivery names no peer, no token, and no socket, so a relay identity
/// cannot stand in for either of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedRelayDelivery {
    credential: AdmittedTransportBinding,
    endpoint: Option<AdmittedTransportBinding>,
}

impl AdmittedRelayDelivery {
    /// Compose one carriage from its admitted relationships.
    #[must_use]
    pub const fn new(
        credential: AdmittedTransportBinding,
        endpoint: Option<AdmittedTransportBinding>,
    ) -> Self {
        Self {
            credential,
            endpoint,
        }
    }

    /// Borrow the admitted credential relationship.
    pub const fn credential(&self) -> &AdmittedTransportBinding {
        &self.credential
    }

    /// Borrow the admitted egress-endpoint relationship, when the carriage
    /// realizes one.
    pub const fn endpoint(&self) -> Option<&AdmittedTransportBinding> {
        self.endpoint.as_ref()
    }

    /// Borrow the relationship's KTD3 key.
    pub const fn key(&self) -> &BindingKey {
        self.credential.key()
    }

    /// Borrow the consumer the admitted credential is delivered to.
    ///
    /// This is the only execution identity a relay credential read may be
    /// requested for: the credential stays in the execution context the graph
    /// bound it to.
    pub const fn credential_consumer_ref(&self) -> &d2b_contracts::ResourceRef {
        self.credential.request().consumer_ref()
    }

    /// Borrow the Zone both relationships are admitted in.
    pub const fn zone(&self) -> &ZoneId {
        self.credential.key().zone()
    }
}

/// One closed, field-free refusal from the graph-bound attach gate.
///
/// The stage and reason are the graph's own vocabulary, and the code is a
/// stable field-free label: a refusal never echoes a zone, a path, a handle,
/// or caller-supplied text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportAttachRefusal {
    stage: AdmissionStage,
    reason: RefusalReason,
    code: &'static str,
}

impl TransportAttachRefusal {
    const fn at(stage: AdmissionStage, reason: RefusalReason, code: &'static str) -> Self {
        Self { stage, reason, code }
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
/// evidence cannot make itself current, and a reconnect that presents newer
/// evidence still refuses against a revoked fence.
pub fn admit_attach(
    binding: &AdmittedTransportBinding,
    evidence: &TransportAttachEvidence,
) -> Result<AdmittedTransportRoute, TransportAttachRefusal> {
    if evidence.zone() != binding.key().zone() {
        return Err(TransportAttachRefusal::at(
            AdmissionStage::Authorize,
            RefusalReason::IdentityNotAuthorized,
            "foreign-zone",
        ));
    }
    let fence = binding.fence();
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
    Ok(AdmittedTransportRoute::new(
        binding.key().clone(),
        binding.audience().clone(),
        fence.clone(),
    ))
}

/// Check the relay's own delivery classes on one admitted relationship.
///
/// This is a *class* check, not an evidence check: it asks whether the
/// relationship the graph committed actually delivers a relay token for the
/// relay's declared graph audience. It runs before any credential byte is
/// read, so a relationship admitted for a different audience or without
/// `AcquireToken` never reaches a credential Provider.
pub fn admit_relay_credential_delivery(
    binding: &AdmittedTransportBinding,
) -> Result<(), TransportAttachRefusal> {
    if binding.audience().as_str() != RELAY_CREDENTIAL_AUDIENCE {
        return Err(TransportAttachRefusal::at(
            AdmissionStage::Admit,
            RefusalReason::TargetSupportMissing,
            "relay-audience-not-admitted",
        ));
    }
    if !binding.admits_operation(CredentialOperation::AcquireToken) {
        return Err(TransportAttachRefusal::at(
            AdmissionStage::Admit,
            RefusalReason::RequiredCapabilityOutsideCeiling,
            "relay-operation-not-admitted",
        ));
    }
    Ok(())
}

/// Run the whole gate one relay carriage must pass, in order.
///
/// The evidence checks run first, on the credential relationship and then on
/// the egress endpoint when the carriage realizes one, and only then is the
/// relationship's own delivery class checked. The route is the credential
/// relationship's, because that is the relationship the credential read is
/// bound to.
///
/// # Errors
///
/// Returns the first [`TransportAttachRefusal`] the carriage earns; see
/// [`admit_attach`] and [`admit_relay_credential_delivery`] for the codes.
pub fn admit_relay_delivery(
    delivery: &AdmittedRelayDelivery,
    evidence: &TransportAttachEvidence,
) -> Result<AdmittedTransportRoute, TransportAttachRefusal> {
    let route = admit_attach(delivery.credential(), evidence)?;
    if let Some(endpoint) = delivery.endpoint() {
        admit_attach(endpoint, evidence)?;
    }
    admit_relay_credential_delivery(delivery.credential())?;
    Ok(route)
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

/// The bounded, Provider-local set of admitted transport relationships.
///
/// This is the only list that can say a relay carriage may open, and it holds
/// admitted relationships keyed by their graph identity rather than by relay
/// identity. A carriage whose relationship is not in here has no open path at
/// all. Revocation retains the relationship in a revoked phase rather than
/// dropping it, so a reconnect that presents the same or newer evidence still
/// refuses instead of minting a second carriage from a released one.
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
        let route = AdmittedTransportRoute::new(
            binding.key().clone(),
            binding.audience().clone(),
            binding.fence().clone(),
        );
        state
            .relationships
            .insert(binding.key().clone(), binding);
        Ok(route)
    }

    /// Return the live relationship admitted under one key.
    pub fn binding(&self, key: &BindingKey) -> Option<AdmittedTransportBinding> {
        self.state.try_lock().ok()?.relationships.get(key).cloned()
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

    /// Retire every admitted relationship during Provider finalization.
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

/// The closed set of privileged transport control operations.
///
/// Every one is an in-process call. None is a frame, and none is reachable
/// from the bytes a transport carries. `Open` maps to the Provider's
/// `open_under_delivery`, `Close` to `RelayConnection::close`, and `Observe`
/// to `RelayConnection::observe_transport`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportControlOperation {
    /// Open a carriage on an already admitted delivery.
    Open,
    /// Close a carriage on an already admitted delivery.
    Close,
    /// Observe a carriage on an already admitted delivery.
    Observe,
}

const OPEN_DISCRIMINANT: &[u8] = b"open";
const CLOSE_DISCRIMINANT: &[u8] = b"close";
const OBSERVE_DISCRIMINANT: &[u8] = b"observe";

fn scan_for_operation(bytes: &[u8]) -> Option<TransportControlOperation> {
    let candidates = [
        (OPEN_DISCRIMINANT, TransportControlOperation::Open),
        (CLOSE_DISCRIMINANT, TransportControlOperation::Close),
        (OBSERVE_DISCRIMINANT, TransportControlOperation::Observe),
    ];
    for (name, operation) in candidates {
        if bytes.windows(name.len()).any(|window| window == name) {
            return Some(operation);
        }
    }
    None
}

/// What a real scan found in stream-carried bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CarriageClass {
    /// The bytes name no privileged control operation at all.
    Data,
    /// The bytes name one privileged control operation.
    ControlShaped,
}

impl CarriageClass {
    /// Whether the bytes are shaped like a control request.
    pub const fn is_control_shaped(self) -> bool {
        matches!(self, Self::ControlShaped)
    }
}

/// Scan stream-carried bytes for a privileged control-operation discriminant.
///
/// The scan is real: it reads the bytes. What it cannot do is turn them into
/// authority, which is why its result is a classification and not a request.
pub fn classify_carriage(bytes: &[u8]) -> CarriageClass {
    if scan_for_operation(bytes).is_some() {
        CarriageClass::ControlShaped
    } else {
        CarriageClass::Data
    }
}

/// Why stream-carried bytes could not become a control-plane request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlPlaneInjectionRefusal {
    /// Carriage carries no operation discriminant: it is data by construction.
    NoOperationDiscriminant,
    /// The bytes are shaped like a control request but carry no route token.
    NotAControlRequest,
    /// No admitted route backs this attempt.
    RouteNotAdmitted,
}

impl ControlPlaneInjectionRefusal {
    /// Return the closed, stable refusal code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::NoOperationDiscriminant => "no-operation-discriminant",
            Self::NotAControlRequest => "not-a-control-request",
            Self::RouteNotAdmitted => "route-not-admitted",
        }
    }
}

impl fmt::Display for ControlPlaneInjectionRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for ControlPlaneInjectionRefusal {}

/// The opaque authorization a privileged control request must carry.
///
/// It is derived from a live admitted route and deliberately has no public
/// constructor, no accessor, no `Clone`, and no deserializer: the only way to
/// hold one is to be holding an admitted route, and the only way to use it is
/// to spend it on one in-process call. The bound relationship is retained
/// rather than read, because reading it would turn the token into evidence
/// about a route a caller does not own.
pub struct ControlRouteToken {
    _route: BindingKey,
}

impl fmt::Debug for ControlRouteToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ControlRouteToken(OPAQUE)")
    }
}

/// The admitted relationship a privileged control request acts on.
///
/// It names the graph relationship, not a socket and not a peer. `Open` acts
/// on the relationship's delivery; `Close` and `Observe` act on the live
/// `RelayConnection` that relationship already opened. There is no constructor
/// a caller can reach, so a handle cannot name a relationship the gate did not
/// admit.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelayCarriageHandle {
    _route: BindingKey,
}

impl RelayCarriageHandle {
    /// Borrow the relationship this handle acts on.
    pub const fn relationship(&self) -> &BindingKey {
        &self._route
    }
}

/// A privileged control request.
///
/// Constructible only against a live admitted route, because issuing one
/// requires the route token that only
/// [`AdmittedTransportRoute::control_token`] can mint and that this type
/// consumes.
#[derive(Debug)]
pub struct ControlPlaneRequest {
    operation: TransportControlOperation,
    _token: ControlRouteToken,
    handle: RelayCarriageHandle,
}

impl ControlPlaneRequest {
    /// Issue one privileged control request against a live route.
    pub fn issue(
        operation: TransportControlOperation,
        token: ControlRouteToken,
        handle: RelayCarriageHandle,
    ) -> Self {
        Self {
            operation,
            _token: token,
            handle,
        }
    }

    /// Return the operation the request performs.
    pub const fn operation(&self) -> TransportControlOperation {
        self.operation
    }

    /// Borrow the carriage the request targets.
    pub const fn carriage(&self) -> &RelayCarriageHandle {
        &self.handle
    }

    /// The one function stream-carried bytes could reach if a transport ever
    /// parsed its data plane for control.
    ///
    /// It scans `bytes` for a control-operation discriminant, and it always
    /// refuses: carriage can name an operation, but it cannot carry the route
    /// token that would make the operation privileged, and no byte sequence is
    /// a deserializer for [`ControlRouteToken`]. This function is the only
    /// plausible injection seam in the crate, and it is closed.
    ///
    /// # Errors
    ///
    /// Returns `RouteNotAdmitted` when no admitted route backs the attempt,
    /// `NoOperationDiscriminant` when the bytes name no control operation, and
    /// `NotAControlRequest` when they do name one without a route token.
    pub fn from_carriage(
        route: Option<&AdmittedTransportRoute>,
        bytes: &[u8],
    ) -> Result<Self, ControlPlaneInjectionRefusal> {
        let Some(_route) = route else {
            return Err(ControlPlaneInjectionRefusal::RouteNotAdmitted);
        };
        if !classify_carriage(bytes).is_control_shaped() {
            return Err(ControlPlaneInjectionRefusal::NoOperationDiscriminant);
        }
        Err(ControlPlaneInjectionRefusal::NotAControlRequest)
    }
}
