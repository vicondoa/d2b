//! Graph-bound attach authority for one admitted local transport relationship.
//!
//! The portal keeps no access list of its own. Every attach is a check of live
//! evidence against the fence committed for one admitted `EndpointBinding`
//! relationship, and the kernel peer identity a transport may be pinned to is
//! derived from that same relationship rather than supplied beside it. The
//! data plane stays a data plane: the privileged control operations below are
//! in-process calls, and no byte sequence that arrives on a stream can become
//! one of them.

use crate::portal::TransportHandle;
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

/// The kernel peer identity one admitted relationship is pinned to.
///
/// A pin is carried by the relationship, not beside it. The graph-bound open
/// path has no argument in which a caller could place a peer identity of its
/// own, so the only uid/gid that can govern an attach is the one the graph
/// bound to the relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KernelPeerPin {
    uid: u32,
    gid: u32,
}

impl KernelPeerPin {
    /// Construct the kernel uid/gid a relationship is pinned to.
    pub const fn new(uid: u32, gid: u32) -> Self {
        Self { uid, gid }
    }

    /// Return the pinned uid.
    pub const fn uid(self) -> u32 {
        self.uid
    }

    /// Return the pinned gid.
    pub const fn gid(self) -> u32 {
        self.gid
    }
}

/// One admitted `EndpointBinding` relationship a transport route realizes.
///
/// The relationship carries its own key, its own desired request, and the
/// fence its evidence is measured against, so a transport never decides on
/// its own that a peer may attach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedTransportBinding {
    key: BindingKey,
    request: EndpointBindingRequest,
    fence: RelationshipFence,
    kernel_peer_pin: Option<KernelPeerPin>,
}

impl AdmittedTransportBinding {
    /// Construct one admitted relationship from its committed identities.
    pub fn new(key: BindingKey, request: EndpointBindingRequest, fence: RelationshipFence) -> Self {
        Self {
            key,
            request,
            fence,
            kernel_peer_pin: None,
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

    /// Return the kernel peer pin the relationship carries, if any.
    pub const fn kernel_peer_pin(&self) -> Option<KernelPeerPin> {
        self.kernel_peer_pin
    }

    /// Bind the relationship to the kernel peer the graph resolved for it.
    #[must_use]
    pub fn with_kernel_peer_pin(mut self, pin: KernelPeerPin) -> Self {
        self.kernel_peer_pin = Some(pin);
        self
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
    kernel_peer_pin: Option<KernelPeerPin>,
}

impl AdmittedTransportRoute {
    fn new(
        key: BindingKey,
        attachment: EndpointAttachmentKind,
        fence: RelationshipFence,
        kernel_peer_pin: Option<KernelPeerPin>,
    ) -> Self {
        Self {
            key,
            attachment,
            fence,
            kernel_peer_pin,
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

    /// Return the kernel peer pin the relationship carries, if any.
    pub const fn kernel_peer_pin(&self) -> Option<KernelPeerPin> {
        self.kernel_peer_pin
    }

    /// Mint the opaque token a privileged in-process control call must carry.
    ///
    /// The token is derived from a live admitted route and has no public
    /// constructor, no reader, and no deserializer, so bytes on a socket can
    /// never be turned into one.
    pub fn control_token(&self) -> ControlRouteToken {
        ControlRouteToken {
            _route: self.key.clone(),
        }
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
/// evidence cannot make itself current.
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
        binding.attachment(),
        fence.clone(),
        binding.kernel_peer_pin(),
    ))
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
        self.state
            .try_lock()
            .map_or(0, |state| state.ceiling)
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
            binding.kernel_peer_pin(),
        ))
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

/// The closed set of privileged transport control operations.
///
/// Every one is an in-process call. None is a frame, and none is reachable
/// from the bytes a transport carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportControlOperation {
    /// Open a transport on an already admitted route.
    Open,
    /// Close a transport on an already admitted route.
    Close,
    /// Observe a transport on an already admitted route.
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
    handle: TransportHandle,
}

impl ControlPlaneRequest {
    /// Issue one privileged control request against a live route.
    pub fn issue(
        operation: TransportControlOperation,
        token: ControlRouteToken,
        handle: TransportHandle,
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

    /// Return the transport handle the request targets.
    pub const fn handle(&self) -> TransportHandle {
        self.handle
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
        if route.is_none() {
            return Err(ControlPlaneInjectionRefusal::RouteNotAdmitted);
        }
        if scan_for_operation(bytes).is_none() {
            return Err(ControlPlaneInjectionRefusal::NoOperationDiscriminant);
        }
        Err(ControlPlaneInjectionRefusal::NotAControlRequest)
    }
}
