//! One broker-owned source reservation service (U8, KTD9; R5-R6, R19-R24,
//! R36-R41).
//!
//! # One claim, one owner
//!
//! The source provider still decides semantic admission (KTD2): a
//! [`BindingAdmission`] arrives here already authorized against the source's
//! own policy, grants, and realization support. What this service owns is the
//! *physical or logical claim on the source* - exactly one reservation owner
//! serializes that claim, so a Process and a Guest asking for incompatible
//! writable use of one source cannot each obtain a writer. The arbitration
//! key is the source, so one decision governs both transports no matter which
//! consumer kind or realization backend the two requests name (AE5, AE8).
//!
//! Every entry point is reached through an admitted Operation, so nothing
//! here accepts a bare identity: [`BindingReservationService::admit_binding`]
//! takes the admission the source provider produced, and every later call
//! takes the handle that admission minted.
//!
//! # Attenuated realization legs
//!
//! A binding-owned helper needs the parent's exact source, but it is not a
//! second peer writer or a competing device claim (AE27). A helper leg binds
//! the parent reservation, the helper's own identity and desired revision, an
//! explicitly permitted operation subset, the exact source, and the broker
//! epoch. It cannot introduce another source, cannot hold a right the parent
//! was not admitted for, and cannot outlive the parent's revocation.
//!
//! # No second ledger
//!
//! The pending/effect/close/release record is carried by
//! [`crate::state_cells`], the broker's existing single-owner,
//! durable-before-granted state mechanism, under [`RESERVATION_CELL`]. A claim
//! is durably pre-committed before its handle exists, completed when the
//! source-side effect is observed, and retired when the reservation is
//! released, so a restart reconciles or refuses instead of reminting access
//! (R41). There is no second durable format, no family-specific storage
//! variant, and no second lifecycle vocabulary: the observed state here is the
//! contracts crate's own [`BindingLifecycleState`] / [`CompletionCondition`] /
//! [`ReleaseOutcome`] rather than a parallel enum.
//!
//! # Ordering, not waiting
//!
//! No method here blocks, waits, or recurses. The pre-drain ordering is data
//! the owner advances stage by stage from observed evidence, so a binding
//! owner driving it never holds a mailbox waiting for a descendant (KTD10).

use std::collections::{BTreeMap, BTreeSet};

use d2b_contracts_resource::v3::{
    AdmissionStage, BindingAdmission, BindingArbitration, BindingKey, BindingLifecycleState,
    BindingObservation, BindingSlotIndex, BindingSpecFingerprint, BoundedToken,
    CallableOperation, CompletionCondition, DesiredRevision, OperationImplementation,
    RefusalReason, ReleaseOutcome, RequestedRights, ResourceUid, SourceReservation, ZoneId,
};

use crate::catalog::CellDurability;
use crate::state_cells::{CellStore, CellStoreError, ConsumeDecision};

/// The state cell the reservation owner records its durable claims in.
pub const RESERVATION_CELL: &str = "binding-reservation";

/// How many holders one claim may have at once under shared arbitration.
const MAX_SHARED_HOLDERS: usize = 64;

/// How many realization legs one reservation may bind.
const MAX_RESERVATION_LEGS: usize = 64;

// ---------------------------------------------------------------------------
// Fence epoch
// ---------------------------------------------------------------------------

/// The broker's fence token for one claim.
///
/// Every leg carries the epoch it was minted under. Revocation advances it,
/// which is what makes "a leg cannot outlive the parent's revocation" a
/// comparison rather than a promise: a leg whose epoch is behind the claim's
/// current one is refused even when every other field still matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReservationEpoch(u64);

impl ReservationEpoch {
    /// The epoch a freshly admitted claim is minted under.
    pub const INITIAL: Self = Self(1);

    /// The raw monotone value.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Whether this epoch is still the claim's current fence.
    pub const fn is_current(self, claim: Self) -> bool {
        self.0 == claim.0
    }

    fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

// ---------------------------------------------------------------------------
// Leg capabilities
// ---------------------------------------------------------------------------

/// One capability a leg may exercise over its parent's reservation.
///
/// The set is closed and every entry only ever reduces use or recovers known
/// state, which is what keeps a leg from becoming a second authority path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReservationCapability {
    /// Read the parent's current reservation evidence.
    Observe,
    /// Detach the consumer from the prepared relationship.
    Detach,
    /// Block new use of the parent reservation.
    Revoke,
    /// Release the parent reservation.
    Release,
}

impl ReservationCapability {
    /// The complete control lane a leg may hold.
    ///
    /// This is the whole permitted-operation subset: observe, detach, revoke,
    /// release. It contains no capability that grants use, so neither a
    /// realization leg nor a cleanup leg can serve a consumer through it.
    pub const CLEANUP_LANE: [Self; 4] = [Self::Observe, Self::Detach, Self::Revoke, Self::Release];

    /// Whether this capability is inside the control lane.
    pub const fn is_cleanup_lane(self) -> bool {
        matches!(
            self,
            Self::Observe | Self::Detach | Self::Revoke | Self::Release
        )
    }
}

/// Which lane one bound leg runs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegLane {
    /// A realization leg: the helper consumes the parent's source under the
    /// admitted right, inside an explicitly permitted capability subset.
    Realization,
    /// A cleanup leg: the helper may only observe, detach, revoke, and
    /// release. It holds no right at all.
    Cleanup,
}

// ---------------------------------------------------------------------------
// Leg identity
// ---------------------------------------------------------------------------

/// A helper's own identity, bound to one desired revision.
///
/// The revision is part of the identity on purpose: a helper re-declared at a
/// new desired revision is a new leg, and the old one's grant cannot be
/// carried across the change (R35).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LegIdentity {
    name: BoundedToken,
    uid: ResourceUid,
    desired_revision: DesiredRevision,
}

impl LegIdentity {
    /// Bind one helper identity to its committed desired revision.
    pub const fn new(
        name: BoundedToken,
        uid: ResourceUid,
        desired_revision: DesiredRevision,
    ) -> Self {
        Self {
            name,
            uid,
            desired_revision,
        }
    }

    /// Borrow the helper's stable local name.
    pub const fn name(&self) -> &BoundedToken {
        &self.name
    }

    /// Borrow the helper's store-assigned identity.
    pub const fn uid(&self) -> &ResourceUid {
        &self.uid
    }

    /// The desired revision this leg was minted for.
    pub const fn desired_revision(&self) -> DesiredRevision {
        self.desired_revision
    }
}

// ---------------------------------------------------------------------------
// Declared drain implementation
// ---------------------------------------------------------------------------

/// The predeclared implementation a missing helper's cleanup leg runs under.
///
/// This is the sole control-lane exception permitting a cleanup child. It is
/// bound to the existing reservation, it may be a different declared
/// Operation than the one the ordinary use ran, and it carries no right: the
/// lane it opens is [`LegLane::Cleanup`], so it can observe, detach, revoke,
/// and release, and it can neither serve the consumer nor obtain another
/// source claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainImplementation {
    operation: CallableOperation,
    capabilities: BTreeSet<ReservationCapability>,
}

impl DrainImplementation {
    /// Record a predeclared drain implementation over a capability subset.
    ///
    /// A capability outside [`ReservationCapability::CLEANUP_LANE`] is refused
    /// here, so a declaration cannot smuggle a use-granting capability into
    /// the control lane.
    pub fn new(
        operation: CallableOperation,
        capabilities: impl IntoIterator<Item = ReservationCapability>,
    ) -> Result<Self, ReservationError> {
        let capabilities = capabilities.into_iter().collect::<BTreeSet<_>>();
        if capabilities.iter().any(|capability| !capability.is_cleanup_lane()) {
            return Err(ReservationError::refuse(
                AdmissionStage::Drain,
                RefusalReason::PolicySelectionNotAuthorized,
            ));
        }
        Ok(Self {
            operation,
            capabilities,
        })
    }

    /// The declared Operation the cleanup leg runs.
    pub const fn operation(&self) -> &CallableOperation {
        &self.operation
    }

    /// The declared implementation identity of that Operation.
    pub fn implementation(&self) -> &OperationImplementation {
        self.operation.implementation()
    }

    /// Whether this implementation is a different declared Operation than the
    /// one the ordinary use ran. It may be: a cleanup leg is allowed to name
    /// a different Operation as long as the lane stays the control lane.
    pub fn differs_from(&self, use_implementation: &OperationImplementation) -> bool {
        self.operation.implementation() != use_implementation
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Why the reservation owner refused one call.
#[derive(Debug, PartialEq)]
pub enum ReservationError {
    /// A typed refusal with the enforcing stage and the reason.
    Refused {
        /// The stage that refused.
        stage: AdmissionStage,
        /// Why it refused.
        reason: RefusalReason,
    },
    /// The relationship's observed state forbids the requested transition.
    UnexpectedState {
        /// What the relationship was in.
        observed: BindingLifecycleState,
    },
    /// The handle names a claim this owner does not hold.
    UnknownReservation,
    /// The claim's durable record could not be read or written.
    Record(CellStoreError),
}

impl ReservationError {
    fn refuse(stage: AdmissionStage, reason: RefusalReason) -> Self {
        Self::Refused { stage, reason }
    }

    /// The typed refusal this error carries, when it is one.
    pub const fn refusal(&self) -> Option<(AdmissionStage, RefusalReason)> {
        match self {
            Self::Refused { stage, reason } => Some((*stage, *reason)),
            _ => None,
        }
    }
}

impl std::fmt::Display for ReservationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused { stage, reason } => {
                write!(formatter, "reservation refused at {stage:?}: {reason:?}")
            }
            Self::UnexpectedState { observed } => {
                write!(formatter, "reservation refused: unexpected state {observed:?}")
            }
            Self::UnknownReservation => write!(formatter, "reservation refused: unknown claim"),
            Self::Record(error) => write!(formatter, "reservation record: {error}"),
        }
    }
}

impl std::error::Error for ReservationError {}

// ---------------------------------------------------------------------------
// Legs
// ---------------------------------------------------------------------------

/// The observed state of one bound helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegObservation {
    /// The helper was not found. A missing helper can only be replaced through
    /// a declared drain implementation, never by reminting its ordinary leg.
    Missing,
    /// The helper is running and still holds the parent's source.
    Live,
    /// The helper's use of the parent's source is closed and proven.
    Finalized,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Leg {
    identity: LegIdentity,
    lane: LegLane,
    rights: Option<RequestedRights>,
    operation: OperationImplementation,
    capabilities: BTreeSet<ReservationCapability>,
    epoch: ReservationEpoch,
    observation: LegObservation,
}

impl Leg {
    fn permits(&self, capability: ReservationCapability) -> bool {
        self.capabilities.contains(&capability)
    }
}

/// The handle one bound leg is driven by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegHandle {
    reservation: SourceReservation,
    parent: BindingKey,
    identity: LegIdentity,
    epoch: ReservationEpoch,
}

impl LegHandle {
    /// Borrow the parent reservation this leg is bounded by.
    pub const fn reservation(&self) -> &SourceReservation {
        &self.reservation
    }

    /// Borrow the admitted relationship that owns this leg.
    pub const fn parent(&self) -> &BindingKey {
        &self.parent
    }

    /// Borrow the helper's own identity.
    pub const fn identity(&self) -> &LegIdentity {
        &self.identity
    }

    /// The fence epoch this leg was minted under.
    pub const fn epoch(&self) -> ReservationEpoch {
        self.epoch
    }
}

/// The handle one admitted claim is driven by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReservationHandle {
    reservation: SourceReservation,
    binding: BindingKey,
    epoch: ReservationEpoch,
}

impl ReservationHandle {
    /// Borrow the source-owned reservation identity.
    pub const fn reservation(&self) -> &SourceReservation {
        &self.reservation
    }

    /// Borrow the admitted relationship.
    pub const fn binding(&self) -> &BindingKey {
        &self.binding
    }

    /// The fence epoch this claim was minted under.
    ///
    /// The owner keeps one handle for the whole relationship, so its handle
    /// stays current across the revocation it installs. The epoch is compared
    /// against a leg's, not the owner's: [`Self::fence`] advances the claim's
    /// fence and retires the grants minted under the previous one.
    pub const fn epoch(&self) -> ReservationEpoch {
        self.epoch
    }
}

/// The grant one authorized leg use produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegGrant {
    reservation: SourceReservation,
    epoch: ReservationEpoch,
    source: ResourceUid,
    lane: LegLane,
    rights: Option<RequestedRights>,
    capability: ReservationCapability,
}

impl LegGrant {
    /// Borrow the parent reservation this grant is bounded by.
    pub const fn reservation(&self) -> &SourceReservation {
        &self.reservation
    }

    /// The fence epoch this grant is valid under.
    pub const fn epoch(&self) -> ReservationEpoch {
        self.epoch
    }

    /// The exact source this grant names. A leg can never name another one.
    pub const fn source(&self) -> &ResourceUid {
        &self.source
    }

    /// Which lane produced this grant.
    pub const fn lane(&self) -> LegLane {
        self.lane
    }

    /// The right this leg holds. A cleanup leg holds none.
    pub const fn rights(&self) -> Option<RequestedRights> {
        self.rights
    }

    /// The capability this grant authorizes.
    pub const fn capability(&self) -> ReservationCapability {
        self.capability
    }
}

// ---------------------------------------------------------------------------
// Pre-drain stages (KTD10)
// ---------------------------------------------------------------------------

/// One stage of the ordered pre-drain (KTD10).
///
/// The stages are recorded separately rather than collapsed into "busy" or
/// "done" because the ordering requirement is precise: new use is blocked
/// first, the consumer is detached while required helpers still exist, the
/// helpers are finalized next, and only then is the reservation released.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DrainStage {
    /// The request was cancelled before any reservation or helper existed.
    /// Pre-drain is a no-op here: there is nothing to drain.
    NotApplicable,
    /// New use is blocked and the epoch has advanced.
    Fenced,
    /// The consumer no longer uses the prepared relationship. This is
    /// satisfied by observation when the relationship was `Active`, and is
    /// already satisfied for a relationship that never became `Active`.
    ConsumerDetached,
    /// Every helper's use of the parent's source is closed and proven.
    HelpersFinalized,
    /// The reservation is released and its durable record is retired.
    ReservationReleased,
}

impl DrainStage {
    /// Whether this stage is the last one.
    pub const fn is_complete(self) -> bool {
        matches!(self, Self::ReservationReleased)
    }
}

/// The ordered pre-drain the owner should perform next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrainPlan {
    reached: DrainStage,
    next: Option<DrainStage>,
}

impl DrainPlan {
    /// The furthest stage this claim has proven.
    pub const fn reached(self) -> DrainStage {
        self.reached
    }

    /// The stage to perform now, or `None` when the pre-drain is finished or
    /// has nothing to do.
    pub const fn next(self) -> Option<DrainStage> {
        self.next
    }

    /// Whether the pre-drain is finished.
    pub const fn is_complete(self) -> bool {
        self.next.is_none()
    }
}

/// Evidence one pre-drain stage was actually reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrainEvidence {
    /// The consumer is observed detached from the prepared relationship.
    pub consumer_detached: bool,
    /// Every helper's use of the parent's source is observed closed.
    pub helpers_finalized: bool,
}

/// What one claim's release did to its source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseReport {
    /// The reservation is released and the source keeps its other holders. A
    /// consumer leaving never deletes a source another consumer still uses
    /// (AE17); the source resource's own deletion is a separate decision.
    ReleasedSourceRetained,
    /// The reservation is released and this was the claim's last holder, so
    /// the source claim itself is free.
    ReleasedSourceFreed,
    /// The release had already happened.
    AlreadyReleased,
}

// ---------------------------------------------------------------------------
// Stage prerequisites
// ---------------------------------------------------------------------------

/// One stage-specific prerequisite recorded for a relationship (KTD10).
///
/// The dependency graph records committed identity, source preparation,
/// consumer completion, and release as separate stages. Cycle detection runs
/// over the activation and drain stage graphs only, so a helper leg that
/// references its own parent reservation does not create an activation
/// requirement that the parent be `Active` - it needs the reservation, never
/// the consumer's use of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReservationStage {
    /// The row's committed identity exists.
    CommittedIdentity,
    /// The source side of the relationship is prepared.
    SourcePrepared,
    /// The consumer side of the relationship completed.
    ConsumerCompletion,
    /// The relationship is released.
    Release,
}

impl ReservationStage {
    /// Every stage, in activation order.
    pub const ALL: [Self; 4] = [
        Self::CommittedIdentity,
        Self::SourcePrepared,
        Self::ConsumerCompletion,
        Self::Release,
    ];

    /// The stable label a diagnostic renders.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CommittedIdentity => "committed-identity",
            Self::SourcePrepared => "source-prepared",
            Self::ConsumerCompletion => "consumer-completion",
            Self::Release => "release",
        }
    }
}

/// One node of a stage graph.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum StageNode {
    /// The admitted relationship that owns the claim.
    Parent,
    /// One bound helper.
    Leg(LegIdentity),
}

// ---------------------------------------------------------------------------
// The claim
// ---------------------------------------------------------------------------

/// One consumer's hold on the arbitrated source claim.
///
/// The fence lives on the holder, not on the claim: one consumer's revocation
/// blocks that consumer's use and retires the grants its legs hold, while a
/// peer still using the same source keeps its own use (AE17). A claim-level
/// fence would let one consumer's teardown cut off a consumer that never
/// asked to leave.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Holder {
    admission: BindingAdmission,
    observation: BindingObservation,
    /// Whether new use is blocked for this relationship. Separate from the
    /// observed state so the fence never erases the evidence that decided
    /// what the pre-drain has to wait for.
    fenced: bool,
    /// This holder's fence token. A leg is minted under it and is retired
    /// when revocation advances it.
    fence: ReservationEpoch,
    /// The furthest pre-drain stage this holder has proven.
    reached: DrainStage,
    /// The drain implementation predeclared for this relationship, if any.
    drain_implementation: Option<DrainImplementation>,
}

impl Holder {
    fn new(admission: BindingAdmission) -> Self {
        Self {
            observation: BindingObservation::new(
                BindingLifecycleState::Admitted,
                CompletionCondition::Pending,
                CompletionCondition::Pending,
                ReleaseOutcome::Outstanding,
            ),
            admission,
            fenced: false,
            fence: ReservationEpoch::INITIAL,
            reached: DrainStage::NotApplicable,
            drain_implementation: None,
        }
    }

    /// Whether this relationship still admits new use and new legs.
    fn admits_new_use(&self) -> bool {
        !self.fenced && self.observation.state().admits_new_use()
    }

    /// Whether the consumer actually used the relationship.
    ///
    /// `Unknown` is included: an uncertain observation may be resolved by
    /// observing the consumer detach, never by assuming it never ran.
    fn requires_detach_evidence(&self) -> bool {
        matches!(
            self.observation.state(),
            BindingLifecycleState::Active | BindingLifecycleState::Unknown
        )
    }
}

/// One arbitrated claim on one source.
///
/// This is the unit the owner serializes: several consumers can hold it at
/// once under shared arbitration, but the arbitrating side of the source has
/// exactly one holder, so two incompatible requests cannot each obtain a
/// writer (AE5, AE8). The legs belong to the relationship that minted them,
/// so a leg is found through its parent holder rather than through the claim.
#[derive(Debug, Clone)]
struct SourceClaim {
    zone: ZoneId,
    source_uid: ResourceUid,
    reservation: SourceReservation,
    arbitration: BindingArbitration,
    /// The single arbitrating holder, when one exists.
    writer: Option<Holder>,
    /// Every non-arbitrating holder.
    readers: BTreeMap<BindingKey, Holder>,
    legs: BTreeMap<BindingKey, BTreeMap<LegIdentity, Leg>>,
    /// Whether the claim has been fully released and is free for reassignment.
    released: bool,
}

impl SourceClaim {
    fn holders(&self) -> usize {
        usize::from(self.writer.is_some()) + self.readers.len()
    }

    fn holder(&self, key: &BindingKey) -> Option<&Holder> {
        self.writer
            .as_ref()
            .filter(|holder| holder.admission.key() == key)
            .or_else(|| self.readers.get(key))
    }

    fn holder_mut(&mut self, key: &BindingKey) -> Option<&mut Holder> {
        if self
            .writer
            .as_ref()
            .is_some_and(|holder| holder.admission.key() == key)
        {
            return self.writer.as_mut();
        }
        self.readers.get_mut(key)
    }

    /// Whether this claim may still take a new holder.
    fn admits_new_use(&self) -> bool {
        !self.released
    }

    fn legs_of(&self, key: &BindingKey) -> Option<&BTreeMap<LegIdentity, Leg>> {
        self.legs.get(key)
    }
}

// ---------------------------------------------------------------------------
// The service
// ---------------------------------------------------------------------------

/// The single broker owner of every source reservation.
///
/// One instance per broker process. It holds the arbitrated claims, the
/// consumer slot index, and the durable claim records; nothing else in the
/// broker serializes a source claim, so there is exactly one answer to "who
/// holds this source right now" (KTD9).
pub struct BindingReservationService {
    claims: BTreeMap<ResourceUid, SourceClaim>,
    slots: BindingSlotIndex,
    store: std::sync::Arc<CellStore>,
}

impl std::fmt::Debug for BindingReservationService {
    /// Bounded and identity-free: a claim's source identity, reservation id,
    /// and leg identities are the private transport material this owner holds,
    /// so the projection counts them without naming them.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let legs = self
            .claims
            .values()
            .map(|claim| claim.legs.len())
            .sum::<usize>();
        formatter
            .debug_struct("BindingReservationService")
            .field("claim_count", &self.claims.len())
            .field("leg_count", &legs)
            .field("occupied_slot_count", &self.slots.entries().count())
            .finish_non_exhaustive()
    }
}

impl BindingReservationService {
    /// Construct the service over one broker state store.
    pub fn new(store: std::sync::Arc<CellStore>) -> Self {
        Self {
            claims: BTreeMap::new(),
            slots: BindingSlotIndex::new(),
            store,
        }
    }

    /// Every durable claim record this owner can recover, in invocation order.
    ///
    /// A record proves a claim was taken, not that its effect completed, so
    /// the owner reconciles or refuses from the outcome (R41). The read is
    /// deliberately not filtered by principal: recovery must be able to see a
    /// record whose principal does not match in order to refuse it.
    pub fn recoverable_claims(&self) -> Vec<crate::state_cells::DurableClaim> {
        self.store.durable_claims(RESERVATION_CELL)
    }

    /// The observed lifecycle of one claim.
    pub fn claim_state(&self, handle: &ReservationHandle) -> Result<BindingLifecycleState, ReservationError> {
        Ok(self.holder(handle)?.observation.state())
    }

    /// Whether one relationship is fenced against new use.
    pub fn is_fenced(&self, handle: &ReservationHandle) -> Result<bool, ReservationError> {
        Ok(self.holder(handle)?.fenced)
    }

    /// Bind a predeclared drain implementation to one admitted relationship.
    ///
    /// Declared while the relationship is admitted, never minted during a
    /// drain: this is what a missing helper's cleanup leg runs under, and
    /// without it the claim stays fenced and unreleased rather than reminting
    /// the helper's ordinary leg.
    pub fn declare_drain_implementation(
        &mut self,
        handle: &ReservationHandle,
        implementation: DrainImplementation,
    ) -> Result<(), ReservationError> {
        let holder = self.holder_mut(handle)?;
        if holder.fenced {
            return Err(ReservationError::refuse(
                AdmissionStage::Drain,
                RefusalReason::StaleAuthority,
            ));
        }
        holder.drain_implementation = Some(implementation);
        Ok(())
    }

    // -- Admission ---------------------------------------------------------

    /// Admit one relationship against the arbitrated source claim.
    ///
    /// `admission` is the source provider's own semantic decision; this call
    /// adds only the physical claim. `fingerprint` is the admitted
    /// declaration's digest, which the consumer slot index uses to tell a
    /// re-declaration from a conflicting one.
    pub fn admit_binding(
        &mut self,
        admission: BindingAdmission,
        reservation: SourceReservation,
        fingerprint: &BindingSpecFingerprint,
    ) -> Result<ReservationHandle, ReservationError> {
        let key = admission.key().clone();
        if reservation.zone() != key.zone() || reservation.source_uid() != key.source_uid() {
            return Err(ReservationError::refuse(
                AdmissionStage::Reserve,
                RefusalReason::ConflictingDeclaration,
            ));
        }
        // The consumer slot index is keyed independently of the source-owned
        // key, so a conflicting simultaneous declaration is refused here even
        // when the two name different sources.
        self.slots.declare(&key, fingerprint).map_err(|_| {
            ReservationError::refuse(
                AdmissionStage::Normalize,
                RefusalReason::ConflictingDeclaration,
            )
        })?;

        let rights = admission.rights();
        let source_uid = key.source_uid().clone();
        let claim = self.claims.entry(source_uid.clone()).or_insert_with(|| SourceClaim {
            zone: key.zone().clone(),
            source_uid: source_uid.clone(),
            reservation: reservation.clone(),
            arbitration: admission.arbitration(),
            writer: None,
            readers: BTreeMap::new(),
            legs: BTreeMap::new(),
            released: false,
        });
        if claim.arbitration != admission.arbitration() {
            return Err(ReservationError::refuse(
                AdmissionStage::Reserve,
                RefusalReason::SourcePolicyRefused,
            ));
        }
        if !claim.admits_new_use() {
            return Err(ReservationError::refuse(
                AdmissionStage::Reserve,
                RefusalReason::StaleAuthority,
            ));
        }
        let holder = Holder::new(admission.clone());
        if rights.needs_arbitration() {
            // One arbitrating holder per claim, whoever asks first: the second
            // incompatible request is refused against the first decision
            // instead of obtaining its own writer.
            if claim.writer.is_some() || claim.holders() >= MAX_SHARED_HOLDERS {
                return Err(ReservationError::refuse(
                    AdmissionStage::Reserve,
                    RefusalReason::ConflictingDeclaration,
                ));
            }
            claim.writer = Some(holder);
        } else {
            if claim.holders() >= MAX_SHARED_HOLDERS {
                return Err(ReservationError::refuse(
                    AdmissionStage::Reserve,
                    RefusalReason::LimitExceedsCeiling,
                ));
            }
            claim.readers.insert(key.clone(), holder);
        }
        // Every relationship owns a leg map from admission, so a leg is always
        // addressable and a claim never grows a map for a holder that has
        // already been released.
        claim.legs.entry(key.clone()).or_default();

        // Durable before the handle exists: a crash between the pre-commit and
        // the effect reconciles on retry rather than granting twice. The
        // recorded principal is the arbitrating source identity, so a
        // recovery read can prove the record belongs to this claim.
        match self.store.consume(
            RESERVATION_CELL,
            reservation.reservation_id().as_str(),
            &source_uid.to_canonical_string(),
            CellDurability::OneTime,
        ) {
            Ok(ConsumeDecision::Granted | ConsumeDecision::Reconciled) => {}
            Ok(ConsumeDecision::InProgress) => {
                return Err(ReservationError::refuse(
                    AdmissionStage::Reserve,
                    RefusalReason::UnprovenEffect,
                ));
            }
            Ok(ConsumeDecision::Replayed) => {
                return Err(ReservationError::refuse(
                    AdmissionStage::Reserve,
                    RefusalReason::StaleAuthority,
                ));
            }
            Ok(ConsumeDecision::ForeignPrincipal) => {
                return Err(ReservationError::refuse(
                    AdmissionStage::Reserve,
                    RefusalReason::ConflictingDeclaration,
                ));
            }
            Err(error) => return Err(ReservationError::Record(error)),
        }

        self.slots
            .observe(&key, BindingLifecycleState::Admitted)
            .map_err(|_| {
                ReservationError::refuse(AdmissionStage::Reserve, RefusalReason::ConflictingDeclaration)
            })?;
        let epoch = ReservationEpoch::INITIAL;
        Ok(ReservationHandle {
            reservation,
            binding: key,
            epoch,
        })
    }

    /// Complete the durable claim record once the source-side effect is
    /// observed as prepared.
    pub fn confirm_prepared(&self, handle: &ReservationHandle) -> Result<(), ReservationError> {
        self.store
            .complete(
                RESERVATION_CELL,
                handle.reservation.reservation_id().as_str(),
                &handle.binding.source_uid().to_canonical_string(),
            )
            .map_err(ReservationError::Record)
    }

    /// Record one observation of the consumer relationship.
    ///
    /// Source preparation and consumer completion stay separate, so a
    /// relationship that must exist before its consumer starts never forms a
    /// startup cycle with the observation that the consumer can see it. A
    /// fenced relationship accepts only observations that reduce use: a late
    /// `Prepared` or `Active` observation never restores consumer use under a
    /// revoked binding, and it never widens the fence to a peer that did not
    /// ask to leave.
    pub fn observe_binding(
        &mut self,
        handle: &ReservationHandle,
        observation: BindingObservation,
    ) -> Result<BindingObservation, ReservationError> {
        let key = &handle.binding;
        let observed = observation.state();
        {
            let holder = self.holder_mut(handle)?;
            if holder.fenced && observed.admits_new_use() {
                return Err(ReservationError::refuse(
                    AdmissionStage::Recover,
                    RefusalReason::StaleAuthority,
                ));
            }
            holder.observation = observation;
        }
        self.slots
            .observe(key, observed)
            .map_err(|_| ReservationError::UnexpectedState { observed })?;
        Ok(observation)
    }

    /// The current observation of one relationship.
    pub fn binding_observation(
        &self,
        handle: &ReservationHandle,
    ) -> Result<BindingObservation, ReservationError> {
        Ok(self.holder(handle)?.observation)
    }

    // -- Attenuated realization legs --------------------------------------

    /// Bind one helper to the parent's reservation as an attenuated leg.
    ///
    /// The leg cannot introduce another source, cannot take a right the parent
    /// was not admitted for, and is fenced by the parent's epoch. It holds no
    /// claim of its own: it is a second view of the parent's claim, never a
    /// competing writer or device allocation (AE27).
    #[allow(clippy::too_many_arguments)]
    pub fn attach_leg(
        &mut self,
        handle: &ReservationHandle,
        identity: LegIdentity,
        source: ResourceUid,
        rights: RequestedRights,
        operation: OperationImplementation,
    ) -> Result<LegHandle, ReservationError> {
        let key = handle.binding.clone();
        if source != *key.source_uid() {
            // Another source needs its own admitted binding; a helper leg is
            // never the way to obtain one.
            return Err(ReservationError::refuse(
                AdmissionStage::Prepare,
                RefusalReason::ConflictingDeclaration,
            ));
        }
        if !self.claim(handle)?.admits_new_use() {
            return Err(ReservationError::refuse(
                AdmissionStage::Prepare,
                RefusalReason::StaleAuthority,
            ));
        }
        let (epoch, admitted) = {
            let holder = self.holder(handle)?;
            if !holder.admits_new_use() {
                return Err(ReservationError::refuse(
                    AdmissionStage::Prepare,
                    RefusalReason::StaleAuthority,
                ));
            }
            (holder.fence, holder.admission.rights())
        };
        if !rights_attenuated_under(rights, admitted) {
            return Err(ReservationError::refuse(
                AdmissionStage::Prepare,
                RefusalReason::SourcePolicyRefused,
            ));
        }
        let legs = self.legs_mut_handle(handle)?;
        if legs.contains_key(&identity) || legs.len() >= MAX_RESERVATION_LEGS {
            return Err(ReservationError::refuse(
                AdmissionStage::Prepare,
                RefusalReason::ConflictingDeclaration,
            ));
        }
        legs.insert(
            identity.clone(),
            Leg {
                identity: identity.clone(),
                lane: LegLane::Realization,
                rights: Some(rights),
                operation,
                capabilities: ReservationCapability::CLEANUP_LANE
                    .into_iter()
                    .collect::<BTreeSet<_>>(),
                epoch,
                observation: LegObservation::Missing,
            },
        );
        Ok(LegHandle {
            reservation: handle.reservation.clone(),
            parent: key,
            identity,
            epoch,
        })
    }

    /// Open a cleanup leg for a helper that is gone.
    ///
    /// This is the sole control-lane exception permitting a cleanup child. It
    /// requires a predeclared drain implementation, binds it to the existing
    /// reservation, and grants the control lane only. It never restores
    /// consumer use: the parent relationship is already fenced, so a cleanup
    /// leg cannot serve the consumer, widen rights, or obtain another claim.
    /// With no declared implementation the relationship stays fenced and
    /// unreleased instead of reminting the helper's ordinary leg.
    pub fn open_cleanup_leg(
        &mut self,
        handle: &ReservationHandle,
        identity: LegIdentity,
        source: ResourceUid,
    ) -> Result<LegHandle, ReservationError> {
        let key = handle.binding.clone();
        if source != *key.source_uid() {
            return Err(ReservationError::refuse(
                AdmissionStage::Drain,
                RefusalReason::ConflictingDeclaration,
            ));
        }
        let (epoch, implementation) = {
            let holder = self.holder(handle)?;
            if !holder.fenced {
                return Err(ReservationError::refuse(
                    AdmissionStage::Drain,
                    RefusalReason::StaleAuthority,
                ));
            }
            let implementation = holder.drain_implementation.clone().ok_or_else(|| {
                // No declared drain implementation: the binding stays fenced
                // and unreleased rather than reminting its ordinary helper.
                ReservationError::refuse(AdmissionStage::Drain, RefusalReason::UnprovenEffect)
            })?;
            (holder.fence, implementation)
        };
        let legs = self.legs_mut_handle(handle)?;
        if legs.contains_key(&identity) || legs.len() >= MAX_RESERVATION_LEGS {
            return Err(ReservationError::refuse(
                AdmissionStage::Drain,
                RefusalReason::ConflictingDeclaration,
            ));
        }
        legs.insert(
            identity.clone(),
            Leg {
                identity: identity.clone(),
                lane: LegLane::Cleanup,
                rights: None,
                operation: implementation.implementation().clone(),
                capabilities: implementation.capabilities.clone(),
                epoch,
                observation: LegObservation::Missing,
            },
        );
        Ok(LegHandle {
            reservation: handle.reservation.clone(),
            parent: key,
            identity,
            epoch,
        })
    }

    /// Record one observation of a bound helper.
    pub fn observe_leg(
        &mut self,
        leg: &LegHandle,
        observation: LegObservation,
    ) -> Result<(), ReservationError> {
        let record = self
            .claim_mut_handle(&leg.parent)?
            .legs
            .get_mut(&leg.parent)
            .and_then(|legs| legs.get_mut(&leg.identity))
            .ok_or(ReservationError::UnknownReservation)?;
        if !leg.epoch.is_current(record.epoch) {
            return Err(ReservationError::refuse(
                AdmissionStage::Recover,
                RefusalReason::StaleAuthority,
            ));
        }
        record.observation = observation;
        Ok(())
    }

    /// Authorize one capability for one bound leg.
    ///
    /// The leg's epoch must still be its parent relationship's current fence
    /// and the capability must be in the leg's permitted subset. Advancing
    /// that fence is what retires a realization leg's grant: the parent's
    /// revocation took its right away, and only a cleanup leg minted under the
    /// new fence can drive the control lane on the reservation afterwards. A
    /// cleanup leg holds no right at all, so a grant from it can never serve
    /// the consumer.
    pub fn use_leg(
        &self,
        leg: &LegHandle,
        capability: ReservationCapability,
    ) -> Result<LegGrant, ReservationError> {
        let claim = self.claim_handle(&leg.parent)?;
        let holder = claim
            .holder(&leg.parent)
            .ok_or(ReservationError::UnknownReservation)?;
        let record = claim
            .legs_of(&leg.parent)
            .and_then(|legs| legs.get(&leg.identity))
            .ok_or(ReservationError::UnknownReservation)?;
        if !leg.epoch.is_current(holder.fence) || !leg.epoch.is_current(record.epoch) {
            return Err(ReservationError::refuse(
                AdmissionStage::Revoke,
                RefusalReason::StaleAuthority,
            ));
        }
        if !record.permits(capability) {
            return Err(ReservationError::refuse(
                AdmissionStage::Revoke,
                RefusalReason::PolicySelectionNotAuthorized,
            ));
        }
        Ok(LegGrant {
            reservation: claim.reservation.clone(),
            epoch: holder.fence,
            source: claim.source_uid.clone(),
            lane: record.lane,
            rights: record.rights,
            capability,
        })
    }

    // -- Pre-drain ordering (KTD10) ---------------------------------------

    /// Block new use for one relationship and advance its fence.
    ///
    /// This is the first thing a binding owner does once the manager has
    /// committed its deletion: revocation blocks new use for that
    /// relationship, and the advanced fence is what retires the grants its
    /// already-issued legs hold. A peer still using the same source under a
    /// shared claim is not fenced by someone else's revocation (AE17).
    pub fn fence(
        &mut self,
        handle: &ReservationHandle,
    ) -> Result<ReservationEpoch, ReservationError> {
        let key = handle.binding.clone();
        let fence = {
            let holder = self.holder_mut(handle)?;
            holder.fenced = true;
            holder.fence = holder.fence.next();
            holder.reached = DrainStage::Fenced;
            holder.fence
        };
        // The slot index follows the fence, so a replacement declaration
        // cannot slip a live slot back into the draining relationship.
        let _ = self.slots.observe(&key, BindingLifecycleState::Revoking);
        Ok(fence)
    }

    /// The ordered pre-drain the owner should perform next.
    ///
    /// Cancellation is handled from every state, not only from a normal
    /// `Active` teardown:
    ///
    /// - `Requested` and `Refused` had no reservation or helper to drain, so
    ///   pre-drain is a no-op and nothing is waited for.
    /// - `Admitted`, `Prepared`, and `Degraded` never became `Active`, so a
    ///   consumer-detach observation cannot exist and is not waited for; their
    ///   prepared handles and helpers are closed and the reservation is
    ///   released.
    /// - `Active` requires the consumer-detach observation.
    /// - `Unknown` requires observation or a conservative drain before
    ///   release and never reads as granted use.
    ///
    /// The stage that waits for helpers is deliberately the same stage for
    /// every relationship: a helper that is still live keeps the reservation
    /// held whether the parent reached `Active` or not.
    pub fn plan_drain(&self, handle: &ReservationHandle) -> Result<DrainPlan, ReservationError> {
        let claim = self.claim(handle)?;
        let holder = claim
            .holder(&handle.binding)
            .ok_or(ReservationError::UnknownReservation)?;
        if claim.released || holder.observation.state() == BindingLifecycleState::Released {
            return Ok(DrainPlan {
                reached: DrainStage::ReservationReleased,
                next: None,
            });
        }
        let legs = claim.legs_of(&handle.binding);
        // Nothing was ever reserved or bound: a cancellation here is a no-op
        // pre-drain, not a wait for activity that cannot exist.
        if legs.is_none_or(BTreeMap::is_empty)
            && matches!(
                holder.observation.state(),
                BindingLifecycleState::Requested | BindingLifecycleState::Refused
            )
        {
            return Ok(DrainPlan {
                reached: DrainStage::NotApplicable,
                next: None,
            });
        }
        if !holder.fenced {
            return Ok(DrainPlan {
                reached: DrainStage::NotApplicable,
                next: Some(DrainStage::Fenced),
            });
        }
        if holder.reached < DrainStage::ConsumerDetached {
            return Ok(DrainPlan {
                reached: DrainStage::Fenced,
                next: Some(DrainStage::ConsumerDetached),
            });
        }
        if holder.reached < DrainStage::HelpersFinalized {
            return Ok(DrainPlan {
                reached: DrainStage::ConsumerDetached,
                next: Some(DrainStage::HelpersFinalized),
            });
        }
        Ok(DrainPlan {
            reached: DrainStage::HelpersFinalized,
            next: Some(DrainStage::ReservationReleased),
        })
    }

    /// Whether the current pre-drain stage waits on evidence from outside this
    /// owner.
    ///
    /// A relationship that never became `Active` has no consumer use to
    /// detach, so its detach stage is satisfied by the claim's own state and
    /// never blocks on an observation that can never arrive.
    pub fn detach_requires_evidence(
        &self,
        handle: &ReservationHandle,
    ) -> Result<bool, ReservationError> {
        Ok(self.holder(handle)?.requires_detach_evidence())
    }

    /// Record the evidence for the current pre-drain stage and advance.
    ///
    /// Advancing is idempotent under retry: re-recording the same evidence
    /// lands on the same stage, so a drain retried after a failure neither
    /// skips a stage nor refuses.
    pub fn advance_drain(
        &mut self,
        handle: &ReservationHandle,
        stage: DrainStage,
        evidence: DrainEvidence,
    ) -> Result<DrainStage, ReservationError> {
        let holder_reached = {
            let holder = self.holder_mut(handle)?;
            if holder.reached == DrainStage::ReservationReleased {
                return Ok(holder.reached);
            }
            match stage {
                DrainStage::NotApplicable => {
                    holder.reached = DrainStage::NotApplicable;
                }
                DrainStage::Fenced => {
                    holder.fenced = true;
                    holder.reached = DrainStage::Fenced;
                }
                DrainStage::ConsumerDetached => {
                    if holder.requires_detach_evidence() && !evidence.consumer_detached {
                        return Err(ReservationError::refuse(
                            AdmissionStage::Drain,
                            RefusalReason::UnprovenEffect,
                        ));
                    }
                    holder.reached = DrainStage::ConsumerDetached;
                }
                DrainStage::HelpersFinalized => {
                    holder.reached = DrainStage::HelpersFinalized;
                }
                DrainStage::ReservationReleased => {
                    holder.reached = DrainStage::ReservationReleased;
                }
            }
            holder.reached
        };
        if stage == DrainStage::HelpersFinalized
            && self
                .legs_handle(handle)?
                .values()
                .any(|leg| leg.observation != LegObservation::Finalized)
        {
            // A helper still holds admitted use of the parent's source: the
            // reservation is retained, never released under a live helper.
            return Err(ReservationError::refuse(
                AdmissionStage::Drain,
                RefusalReason::UnprovenEffect,
            ));
        }
        Ok(holder_reached)
    }

    /// Release the reservation, close-before-release.
    ///
    /// Release refuses while a helper is not proven finalized or while an
    /// `Active` consumer's completion is unproven, so no ordering can free a
    /// source a consumer or helper still holds admitted use of. It never
    /// deletes the source itself: a consumer leaving leaves the shared source
    /// and its other holders intact (AE17).
    pub fn release(&mut self, handle: &ReservationHandle) -> Result<ReleaseReport, ReservationError> {
        let key = handle.binding.clone();
        let source_uid = key.source_uid().clone();
        let reservation_id = handle.reservation.reservation_id().clone();
        {
            let holder = self.holder(handle)?;
            if holder.observation.state() == BindingLifecycleState::Released
                || holder.reached == DrainStage::ReservationReleased
            {
                return Ok(ReleaseReport::AlreadyReleased);
            }
            if self
                .legs_handle(handle)?
                .values()
                .any(|leg| leg.observation != LegObservation::Finalized)
            {
                return Err(ReservationError::refuse(
                    AdmissionStage::Release,
                    RefusalReason::UnprovenEffect,
                ));
            }
            if holder.requires_detach_evidence() && holder.reached < DrainStage::ConsumerDetached {
                return Err(ReservationError::refuse(
                    AdmissionStage::Release,
                    RefusalReason::UnprovenEffect,
                ));
            }
        }
        // The durable record is retired only once the in-memory claim is
        // provably free of this holder, so a crash between the two leaves a
        // record recovery reconciles rather than a source nobody holds.
        self.store
            .retire_durable(RESERVATION_CELL, reservation_id.as_str())
            .map_err(ReservationError::Record)?;

        let source_freed = {
            let claim = self
                .claims
                .get_mut(&source_uid)
                .ok_or(ReservationError::UnknownReservation)?;
            if claim
                .writer
                .as_ref()
                .is_some_and(|holder| holder.admission.key() == &key)
            {
                claim.writer = None;
            } else {
                claim.readers.remove(&key);
            }
            claim.legs.remove(&key);
            let freed = claim.writer.is_none() && claim.readers.is_empty();
            claim.released = freed;
            freed
        };
        let _ = self.slots.observe(&key, BindingLifecycleState::Released);
        if source_freed {
            self.claims.remove(&source_uid);
            Ok(ReleaseReport::ReleasedSourceFreed)
        } else {
            Ok(ReleaseReport::ReleasedSourceRetained)
        }
    }

    // -- Stage prerequisites ----------------------------------------------

    /// The stage edges of one claim's activation and drain graphs.
    ///
    /// The edges are read off the claim's own typed records - its bound legs
    /// and their lanes - so there is no second authored dependency list to
    /// drift from them. A realization leg contributes only a
    /// `SourcePrepared` edge to the relationship that owns the reservation:
    /// it needs the reservation, never the parent being `Active`. A cleanup
    /// leg contributes a `SourcePrepared` edge as well, because it is bound to
    /// the same reservation.
    pub fn stage_edges(
        &self,
        handle: &ReservationHandle,
    ) -> Result<Vec<(ReservationStage, StageNode, StageNode)>, ReservationError> {
        let legs = self.legs_handle(handle)?;
        let mut edges = Vec::new();
        for leg in legs.values() {
            let node = StageNode::Leg(leg.identity.clone());
            match leg.lane {
                // The leg's realization waits for the parent relationship's
                // source side, not for the parent's consumer use.
                LegLane::Realization => edges.push((
                    ReservationStage::SourcePrepared,
                    node.clone(),
                    StageNode::Parent,
                )),
                // The parent's release waits for the cleanup leg to finish:
                // release after cleanup, never the other way round.
                LegLane::Cleanup => {
                    edges.push((
                        ReservationStage::SourcePrepared,
                        node.clone(),
                        StageNode::Parent,
                    ));
                    edges.push((ReservationStage::Release, StageNode::Parent, node));
                }
            }
        }
        Ok(edges)
    }

    /// Whether the claim's activation or drain stage graph has a cycle.
    ///
    /// Cycle detection runs on the stage graphs, not on every semantic
    /// relationship. A helper and the relationship that owns its reservation
    /// share one claim by design; treating that sharing as a mutual dependency
    /// inside one stage would refuse a relationship that has no cycle at all.
    /// It is a real self-edge - a leg that claims to be its own parent - that
    /// is refused here.
    pub fn has_stage_cycle(&self, handle: &ReservationHandle) -> Result<bool, ReservationError> {
        let edges = self.stage_edges(handle)?;
        Ok(ReservationStage::ALL.into_iter().any(|stage| {
            let stage_edges = edges
                .iter()
                .filter(|(edge_stage, _, _)| *edge_stage == stage)
                .map(|(_, from, to)| (from.clone(), to.clone()))
                .collect::<BTreeSet<_>>();
            let mut nodes = BTreeSet::new();
            for (from, to) in &stage_edges {
                nodes.insert(from.clone());
                nodes.insert(to.clone());
            }
            let mut visiting = BTreeSet::new();
            let mut visited = BTreeSet::new();
            nodes.iter().any(|node| {
                stage_visit(node, &stage_edges, &mut visiting, &mut visited).is_err()
            })
        }))
    }

    // -- Internals ---------------------------------------------------------

    fn claim(&self, handle: &ReservationHandle) -> Result<&SourceClaim, ReservationError> {
        self.claims
            .get(handle.binding.source_uid())
            .ok_or(ReservationError::UnknownReservation)
    }

    fn claim_mut(&mut self, handle: &ReservationHandle) -> Result<&mut SourceClaim, ReservationError> {
        self.claims
            .get_mut(handle.binding.source_uid())
            .ok_or(ReservationError::UnknownReservation)
    }

    fn claim_handle(&self, key: &BindingKey) -> Result<&SourceClaim, ReservationError> {
        self.claims
            .get(key.source_uid())
            .ok_or(ReservationError::UnknownReservation)
    }

    fn claim_mut_handle(&mut self, key: &BindingKey) -> Result<&mut SourceClaim, ReservationError> {
        self.claims
            .get_mut(key.source_uid())
            .ok_or(ReservationError::UnknownReservation)
    }

    /// The holder that owns one relationship, which is where the fence, the
    /// pre-drain progress, and the predeclared drain implementation live.
    fn holder(&self, handle: &ReservationHandle) -> Result<&Holder, ReservationError> {
        self.claim(handle)?
            .holder(&handle.binding)
            .ok_or(ReservationError::UnknownReservation)
    }

    fn holder_mut(
        &mut self,
        handle: &ReservationHandle,
    ) -> Result<&mut Holder, ReservationError> {
        let key = &handle.binding;
        self.claim_mut(handle)?
            .holder_mut(key)
            .ok_or(ReservationError::UnknownReservation)
    }

    /// The legs bound to one relationship's reservation.
    fn legs_handle(
        &self,
        handle: &ReservationHandle,
    ) -> Result<&BTreeMap<LegIdentity, Leg>, ReservationError> {
        self.claim(handle)?
            .legs_of(&handle.binding)
            .ok_or(ReservationError::UnknownReservation)
    }

    fn legs_mut_handle(
        &mut self,
        handle: &ReservationHandle,
    ) -> Result<&mut BTreeMap<LegIdentity, Leg>, ReservationError> {
        let key = &handle.binding;
        self.claim_mut(handle)?
            .legs
            .get_mut(key)
            .ok_or(ReservationError::UnknownReservation)
    }
}

/// Depth-first cycle detection inside one stage graph.
fn stage_visit(
    node: &StageNode,
    edges: &BTreeSet<(StageNode, StageNode)>,
    visiting: &mut BTreeSet<StageNode>,
    visited: &mut BTreeSet<StageNode>,
) -> Result<(), ReservationError> {
    if visited.contains(node) {
        return Ok(());
    }
    if !visiting.insert(node.clone()) {
        return Err(ReservationError::refuse(
            AdmissionStage::Reserve,
            RefusalReason::ConflictingDeclaration,
        ));
    }
    for (from, to) in edges {
        if from == node {
            stage_visit(to, edges, visiting, visited)?;
        }
    }
    visiting.remove(node);
    visited.insert(node.clone());
    Ok(())
}

/// Whether one right is attenuated under another.
///
/// Attenuation is ordered: `Observe` is under everything, `Consume` is under
/// the writing rights, and the two arbitrating rights are each only under
/// themselves. A leg asking for a right its parent was not admitted for is
/// refused, which is what keeps a helper from widening the relationship it
/// realizes (AE27).
const fn rights_attenuated_under(requested: RequestedRights, admitted: RequestedRights) -> bool {
    match requested {
        RequestedRights::Observe => false,
        RequestedRights::Consume => matches!(
            admitted,
            RequestedRights::Consume | RequestedRights::Mutate | RequestedRights::Exclusive
        ),
        RequestedRights::Mutate => {
            matches!(admitted, RequestedRights::Mutate | RequestedRights::Exclusive)
        }
        RequestedRights::Share => {
            matches!(admitted, RequestedRights::Share | RequestedRights::Exclusive)
        }
        RequestedRights::Exclusive => matches!(admitted, RequestedRights::Exclusive),
    }
}

#[cfg(test)]
mod tests {
    use d2b_contracts_resource::v3::{
        AuditJoin, AuditMode, BoundedText, BoundedToken, BrokerRequirement, BindingAuthorization,
        BindingKind, BindingRealizationFacet, BindingRealizationSupport, BindingSlot,
        DesiredDigest, FreshnessTuple, OperationAudit, OperationAuthority, OperationBounds,
        OperationDomain, OperationFds, OperationSurface, PayloadProvenance, PayloadSchema,
        ResourceRef, SecretAccess, SourceAdmission, StoreIncarnation, admit_binding_request,
    };
    use serde_json::json;
    use std::sync::Arc;

    use super::*;
    use crate::state_cells::CellOutcome;

    const SOURCE_UID: &str = "1b4e28ba-2fa1-41d2-883f-0016d3cca401";
    const OTHER_SOURCE_UID: &str = "1b4e28ba-2fa1-41d2-883f-0016d3cca402";

    fn uid(value: &str) -> ResourceUid {
        ResourceUid::parse(value).expect("uid")
    }

    fn zone() -> ZoneId {
        ZoneId::parse("work").expect("zone")
    }

    fn dependencies() -> Vec<FreshnessTuple> {
        vec![FreshnessTuple::new(
            zone(),
            StoreIncarnation::parse("store-1").expect("incarnation"),
            ResourceRef::parse("Volume/data").expect("source"),
            uid(SOURCE_UID),
            DesiredRevision::INITIAL.try_next().expect("revision"),
            DesiredDigest::of(b"{}"),
        )]
    }

    fn admission(
        key: &BindingKey,
        rights: RequestedRights,
        arbitration: BindingArbitration,
    ) -> BindingAdmission {
        let source = SourceAdmission::new(key.clone(), vec![rights], arbitration).expect("admission");
        let support =
            BindingRealizationSupport::new(vec![BindingRealizationFacet::FilesystemPresentation])
                .expect("support");
        admit_binding_request(
            key,
            rights,
            &[BindingRealizationFacet::FilesystemPresentation],
            &BindingAuthorization::granted(),
            &source,
            &support,
            &dependencies(),
        )
        .expect("admitted")
    }

    fn reservation(token: &str) -> SourceReservation {
        SourceReservation::new(
            zone(),
            uid(SOURCE_UID),
            BoundedToken::parse(token).expect("token"),
        )
    }

    fn fingerprint(seed: &str) -> BindingSpecFingerprint {
        BindingSpecFingerprint::from_request(&json!({ "seed": seed }))
    }

    fn payload() -> PayloadSchema {
        PayloadSchema::parse(json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["target"],
            "properties": { "target": { "type": "string" } }
        }))
        .expect("payload validates")
    }

    fn operation(method: &str) -> CallableOperation {
        CallableOperation::new(
            OperationImplementation::provider_method(
                ResourceRef::parse("Provider/data").expect("provider"),
                BoundedToken::parse("binding").expect("component"),
                BoundedToken::parse(method).expect("method"),
            )
            .expect("declared implementation"),
            payload(),
            None,
            false,
            SecretAccess::ReadWrite,
            OperationAudit::new(
                true,
                AuditMode::Yes,
                vec![BoundedText::parse("target").expect("field")],
                Vec::new(),
                BoundedToken::parse("target").expect("token"),
            )
            .expect("audit facet"),
            Some(AuditJoin::new(vec![BoundedText::parse("target").expect("field")]).expect("join")),
            OperationAuthority::new(
                OperationSurface::Broker,
                OperationDomain::Host,
                BoundedText::parse("host-operator").expect("authority"),
                BrokerRequirement::Yes,
            ),
            OperationFds::default(),
            OperationBounds::default(),
            PayloadProvenance::Request,
        )
        .expect("operation validates")
    }

    fn drain_implementation(method: &str) -> DrainImplementation {
        DrainImplementation::new(
            operation(method),
            ReservationCapability::CLEANUP_LANE,
        )
        .expect("drain implementation")
    }

    fn leg_identity(name: &str, suffix: &str) -> LegIdentity {
        LegIdentity::new(
            BoundedToken::parse(name).expect("name"),
            uid(&format!("1b4e28ba-2fa1-41d2-883f-0016d3cca{suffix}")),
            DesiredRevision::INITIAL,
        )
    }

    fn service() -> BindingReservationService {
        BindingReservationService::new(Arc::new(CellStore::in_memory()))
    }

    fn process_key() -> BindingKey {
        BindingKey::new(
            zone(),
            BindingKind::Volume,
            ResourceRef::parse("Volume/data").expect("source"),
            uid(SOURCE_UID),
            ResourceRef::parse("Process/web").expect("consumer"),
            uid("1b4e28ba-2fa1-41d2-883f-0016d3cca410"),
            BindingSlot::parse("data").expect("slot"),
        )
        .expect("binding key")
    }

    fn guest_key() -> BindingKey {
        BindingKey::new(
            zone(),
            BindingKind::Volume,
            ResourceRef::parse("Volume/data").expect("source"),
            uid(SOURCE_UID),
            ResourceRef::parse("Guest/desktop").expect("consumer"),
            uid("1b4e28ba-2fa1-41d2-883f-0016d3cca411"),
            BindingSlot::parse("root").expect("slot"),
        )
        .expect("binding key")
    }

    fn observe(state: BindingLifecycleState) -> BindingObservation {
        BindingObservation::new(
            state,
            CompletionCondition::Complete,
            CompletionCondition::Complete,
            ReleaseOutcome::Outstanding,
        )
    }

    /// AE5: a Process and a Guest asking for incompatible writable use of one
    /// source are governed by one decision, so the second is refused instead
    /// of obtaining its own writer.
    #[test]
    fn one_arbitration_decision_spans_concurrent_consumers() {
        let mut service = service();
        let process = process_key();
        let guest = guest_key();
        let writer = service
            .admit_binding(
                admission(&process, RequestedRights::Mutate, BindingArbitration::Shared),
                reservation("res-process"),
                &fingerprint("process"),
            )
            .expect("first writer admitted");

        let refused = service.admit_binding(
            admission(&guest, RequestedRights::Mutate, BindingArbitration::Shared),
            reservation("res-guest"),
            &fingerprint("guest"),
        );
        assert_eq!(
            refused.as_ref().err().and_then(ReservationError::refusal),
            Some((AdmissionStage::Reserve, RefusalReason::ConflictingDeclaration))
        );
        assert_eq!(service.claim_state(&writer).unwrap(), BindingLifecycleState::Admitted);

        // A read-only peer of the same source is admitted alongside the writer:
        // the claim is shared, the arbitrating side is not duplicated.
        let reader = guest_key();
        let reader_slot = BindingKey::new(
            zone(),
            BindingKind::Volume,
            ResourceRef::parse("Volume/data").expect("source"),
            uid(SOURCE_UID),
            ResourceRef::parse("Guest/desktop").expect("consumer"),
            uid("1b4e28ba-2fa1-41d2-883f-0016d3cca411"),
            BindingSlot::parse("scratch").expect("slot"),
        )
        .expect("binding key");
        let _ = reader;
        let read_handle = service
            .admit_binding(
                admission(&reader_slot, RequestedRights::Consume, BindingArbitration::Shared),
                reservation("res-read"),
                &fingerprint("read"),
            )
            .expect("a non-arbitrating peer is admitted");
        assert_eq!(
            service.claim_state(&read_handle).unwrap(),
            BindingLifecycleState::Admitted
        );
    }

    /// AE8: an exclusive claim is held until its release is proven, and a
    /// second exclusive request cannot take it in the meantime.
    #[test]
    fn an_exclusive_claim_is_gated_on_release_proof() {
        let mut service = service();
        let first = process_key();
        let handle = service
            .admit_binding(
                admission(&first, RequestedRights::Exclusive, BindingArbitration::Exclusive),
                reservation("res-exclusive"),
                &fingerprint("exclusive"),
            )
            .expect("exclusive admitted");
        service
            .observe_binding(&handle, observe(BindingLifecycleState::Active))
            .unwrap();
        let second = guest_key();
        assert_eq!(
            service
                .admit_binding(
                    admission(&second, RequestedRights::Exclusive, BindingArbitration::Exclusive),
                    reservation("res-second"),
                    &fingerprint("second"),
                )
                .err()
                .and_then(|error| error.refusal()),
            Some((AdmissionStage::Reserve, RefusalReason::ConflictingDeclaration))
        );

        // A drain that cannot prove the consumer detached retains the claim.
        service.fence(&handle).unwrap();
        assert_eq!(
            service
                .advance_drain(
                    &handle,
                    DrainStage::ConsumerDetached,
                    DrainEvidence {
                        consumer_detached: false,
                        helpers_finalized: true,
                    },
                )
                .err()
                .and_then(|error| error.refusal()),
            Some((AdmissionStage::Drain, RefusalReason::UnprovenEffect))
        );
        assert_eq!(
            service
                .release(&handle)
                .err()
                .and_then(|error| error.refusal()),
            Some((AdmissionStage::Release, RefusalReason::UnprovenEffect))
        );
    }

    /// AE27: a helper leg shares the parent's reservation but cannot take a
    /// right the parent was not admitted for and cannot reach another source.
    #[test]
    fn a_helper_leg_shares_the_parent_reservation_without_widening_it() {
        let mut service = service();
        let parent = process_key();
        let handle = service
            .admit_binding(
                admission(&parent, RequestedRights::Mutate, BindingArbitration::Shared),
                reservation("res-parent"),
                &fingerprint("parent"),
            )
            .expect("admitted");
        let implementation = operation("serve-view").implementation().clone();

        assert_eq!(
            service
                .attach_leg(
                    &handle,
                    leg_identity("helper", "420"),
                    uid(SOURCE_UID),
                    RequestedRights::Consume,
                    implementation.clone(),
                )
                .expect("attenuated leg"),
            LegHandle {
                reservation: handle.reservation().clone(),
                parent: parent.clone(),
                identity: leg_identity("helper", "420"),
                epoch: handle.epoch(),
            }
        );

        assert_eq!(
            service
                .attach_leg(
                    &handle,
                    leg_identity("greedy", "421"),
                    uid(SOURCE_UID),
                    RequestedRights::Exclusive,
                    implementation.clone(),
                )
                .err()
                .and_then(|error| error.refusal()),
            Some((AdmissionStage::Prepare, RefusalReason::SourcePolicyRefused))
        );
        assert_eq!(
            service
                .attach_leg(
                    &handle,
                    leg_identity("elsewhere", "422"),
                    uid(OTHER_SOURCE_UID),
                    RequestedRights::Consume,
                    implementation,
                )
                .err()
                .and_then(|error| error.refusal()),
            Some((AdmissionStage::Prepare, RefusalReason::ConflictingDeclaration))
        );
    }

    /// AE17: a consumer's release leaves the shared source and its other
    /// holders intact.
    #[test]
    fn a_consumer_release_does_not_delete_a_shared_source() {
        let mut service = service();
        let first = process_key();
        let second = BindingKey::new(
            zone(),
            BindingKind::Volume,
            ResourceRef::parse("Volume/data").expect("source"),
            uid(SOURCE_UID),
            ResourceRef::parse("Process/other").expect("consumer"),
            uid("1b4e28ba-2fa1-41d2-883f-0016d3cca412"),
            BindingSlot::parse("data").expect("slot"),
        )
        .expect("binding key");
        let first_handle = service
            .admit_binding(
                admission(&first, RequestedRights::Consume, BindingArbitration::Shared),
                reservation("res-first"),
                &fingerprint("first"),
            )
            .expect("first");
        let second_handle = service
            .admit_binding(
                admission(&second, RequestedRights::Consume, BindingArbitration::Shared),
                reservation("res-second"),
                &fingerprint("second"),
            )
            .expect("second");

        service.fence(&first_handle).unwrap();
        assert_eq!(
            service.release(&first_handle).unwrap(),
            ReleaseReport::ReleasedSourceRetained
        );
        assert_eq!(
            service.claim_state(&second_handle).unwrap(),
            BindingLifecycleState::Admitted,
            "the surviving consumer's claim is untouched by the first one's release"
        );
        // The claim's fence advanced with the epoch, so the surviving holder
        // is not fenced by a release that was not its own.
        assert!(!service.is_fenced(&second_handle).unwrap());
    }

    /// KTD10: a live helper holds the reservation through the whole drain; the
    /// source is released only after the helper is finalized, and a failure
    /// anywhere in the order retains the claim.
    #[test]
    fn the_drain_finalizes_helpers_before_it_releases_the_source() {
        let mut service = service();
        let parent = process_key();
        let handle = service
            .admit_binding(
                admission(&parent, RequestedRights::Consume, BindingArbitration::Shared),
                reservation("res-drain"),
                &fingerprint("drain"),
            )
            .expect("admitted");
        service
            .observe_binding(&handle, observe(BindingLifecycleState::Active))
            .unwrap();
        let helper = service
            .attach_leg(
                &handle,
                leg_identity("helper", "430"),
                uid(SOURCE_UID),
                RequestedRights::Consume,
                operation("serve-view").implementation().clone(),
            )
            .expect("helper leg");
        service.observe_leg(&helper, LegObservation::Live).unwrap();

        service.fence(&handle).unwrap();
        assert!(service.is_fenced(&handle).unwrap());
        // The helper holds the parent's right, so the parent's revocation
        // retires that grant: a live helper is observed and finalized as a
        // child, never re-authorized against the fenced claim.
        assert_eq!(
            service
                .use_leg(&helper, ReservationCapability::Observe)
                .err()
                .and_then(|error| error.refusal()),
            Some((AdmissionStage::Revoke, RefusalReason::StaleAuthority))
        );

        // The consumer detaches while the helper is still live.
        assert_eq!(
            service
                .advance_drain(
                    &handle,
                    DrainStage::ConsumerDetached,
                    DrainEvidence {
                        consumer_detached: true,
                        helpers_finalized: false,
                    },
                )
                .unwrap(),
            DrainStage::ConsumerDetached
        );
        assert_eq!(
            service
                .release(&handle)
                .err()
                .and_then(|error| error.refusal()),
            Some((AdmissionStage::Release, RefusalReason::UnprovenEffect)),
            "a live helper retains the reservation"
        );

        service.observe_leg(&helper, LegObservation::Finalized).unwrap();
        assert_eq!(
            service
                .advance_drain(
                    &handle,
                    DrainStage::HelpersFinalized,
                    DrainEvidence {
                        consumer_detached: true,
                        helpers_finalized: true,
                    },
                )
                .unwrap(),
            DrainStage::HelpersFinalized
        );
        assert_eq!(
            service.release(&handle).unwrap(),
            ReleaseReport::ReleasedSourceFreed
        );
        assert!(
            service.recoverable_claims().is_empty(),
            "a released claim retires its durable record"
        );
        // Retiring the last holder frees the claim itself, so a further
        // release addresses a claim that no longer exists rather than
        // re-releasing the same one.
        assert_eq!(
            service.release(&handle).err(),
            Some(ReservationError::UnknownReservation)
        );
    }

    /// A missing helper may only be replaced through a predeclared drain
    /// implementation, and the cleanup leg never restores consumer use.
    #[test]
    fn a_missing_helper_needs_a_declared_cleanup_lane() {
        let mut service = service();
        let parent = process_key();
        let handle = service
            .admit_binding(
                admission(&parent, RequestedRights::Consume, BindingArbitration::Shared),
                reservation("res-missing"),
                &fingerprint("missing"),
            )
            .expect("admitted");
        service
            .observe_binding(&handle, observe(BindingLifecycleState::Active))
            .unwrap();
        let helper = service
            .attach_leg(
                &handle,
                leg_identity("helper", "440"),
                uid(SOURCE_UID),
                RequestedRights::Consume,
                operation("serve-view").implementation().clone(),
            )
            .expect("helper leg");
        service.observe_leg(&helper, LegObservation::Missing).unwrap();
        let ordinary = operation("serve-view").implementation().clone();
        let declared = drain_implementation("drain-view");
        assert!(
            declared.differs_from(&ordinary),
            "the cleanup leg may name a different declared Operation"
        );
        // A second relationship on the same source has no declared drain
        // implementation at all.
        let undeclared = BindingKey::new(
            zone(),
            BindingKind::Volume,
            ResourceRef::parse("Volume/data").expect("source"),
            uid(SOURCE_UID),
            ResourceRef::parse("Process/undeclared").expect("consumer"),
            uid("1b4e28ba-2fa1-41d2-883f-0016d3cca446"),
            BindingSlot::parse("data").expect("slot"),
        )
        .expect("binding key");
        let undeclared_handle = service
            .admit_binding(
                admission(
                    &undeclared,
                    RequestedRights::Consume,
                    BindingArbitration::Shared,
                ),
                reservation("res-undeclared"),
                &fingerprint("undeclared"),
            )
            .expect("admitted");
        service
            .attach_leg(
                &undeclared_handle,
                leg_identity("helper", "442"),
                uid(SOURCE_UID),
                RequestedRights::Consume,
                ordinary.clone(),
            )
            .expect("helper leg");
        service.fence(&undeclared_handle).unwrap();

        // Without a declared drain implementation the relationship stays
        // fenced and unreleased rather than reminting the helper's ordinary
        // leg, and the source stays held.
        assert_eq!(
            service
                .open_cleanup_leg(
                    &undeclared_handle,
                    leg_identity("cleanup", "443"),
                    uid(SOURCE_UID)
                )
                .err()
                .and_then(|error| error.refusal()),
            Some((AdmissionStage::Drain, RefusalReason::UnprovenEffect))
        );
        assert_eq!(
            service
                .attach_leg(
                    &undeclared_handle,
                    leg_identity("helper-again", "444"),
                    uid(SOURCE_UID),
                    RequestedRights::Consume,
                    ordinary.clone(),
                )
                .err()
                .and_then(|error| error.refusal()),
            Some((AdmissionStage::Prepare, RefusalReason::StaleAuthority)),
            "a fenced relationship never remints its ordinary helper"
        );
        assert_eq!(
            service
                .release(&undeclared_handle)
                .err()
                .and_then(|error| error.refusal()),
            Some((AdmissionStage::Release, RefusalReason::UnprovenEffect))
        );

        service
            .declare_drain_implementation(&handle, declared)
            .expect("predeclared while admitted");
        service.fence(&handle).unwrap();

        // A second relationship on the same source proves the declaration is
        // predeclared: a fence refuses to install one afterwards.
        let other = BindingKey::new(
            zone(),
            BindingKind::Volume,
            ResourceRef::parse("Volume/data").expect("source"),
            uid(SOURCE_UID),
            ResourceRef::parse("Process/other").expect("consumer"),
            uid("1b4e28ba-2fa1-41d2-883f-0016d3cca445"),
            BindingSlot::parse("data").expect("slot"),
        )
        .expect("binding key");
        let other_handle = service
            .admit_binding(
                admission(&other, RequestedRights::Consume, BindingArbitration::Shared),
                reservation("res-declared"),
                &fingerprint("declared"),
            )
            .expect("admitted");
        service
            .declare_drain_implementation(
                &other_handle,
                drain_implementation("drain-view"),
            )
            .expect("declared while admitted");
        service.fence(&other_handle).unwrap();
        assert_eq!(
            service
                .declare_drain_implementation(
                    &other_handle,
                    drain_implementation("drain-view"),
                )
                .err()
                .and_then(|error| error.refusal()),
            Some((AdmissionStage::Drain, RefusalReason::StaleAuthority)),
            "a drain implementation is predeclared, never installed during a drain"
        );
        let cleanup = service
            .open_cleanup_leg(&handle, leg_identity("cleanup", "441"), uid(SOURCE_UID))
            .expect("cleanup leg");
        let grant = service
            .use_leg(&cleanup, ReservationCapability::Detach)
            .expect("control lane");
        assert_eq!(grant.lane(), LegLane::Cleanup);
        assert_eq!(grant.rights(), None, "a cleanup leg holds no right");
        assert_eq!(grant.source(), &uid(SOURCE_UID));
        for capability in ReservationCapability::CLEANUP_LANE {
            assert!(service.use_leg(&cleanup, capability).is_ok());
        }

        // The cleanup leg does not restore consumer use under the fence.
        assert_eq!(
            service
                .observe_binding(
                    &handle,
                    observe(BindingLifecycleState::Active)
                )
                .err()
                .and_then(|error| error.refusal()),
            Some((AdmissionStage::Recover, RefusalReason::StaleAuthority))
        );
        assert_eq!(service.claim_state(&handle).unwrap(), BindingLifecycleState::Active);
        assert!(service.is_fenced(&handle).unwrap());
    }

    /// A drain implementation may not smuggle a use-granting capability into
    /// the control lane.
    #[test]
    fn a_drain_implementation_cannot_widen_the_control_lane() {
        assert!(DrainImplementation::new(operation("drain"), []).is_ok());
        assert!(DrainImplementation::new(operation("drain"), ReservationCapability::CLEANUP_LANE).is_ok());
        // Every capability in the closed set is inside the lane, so the
        // refusal is proven by the lane predicate itself rather than by a
        // capability that does not exist.
        for capability in ReservationCapability::CLEANUP_LANE {
            assert!(capability.is_cleanup_lane());
        }
    }

    /// Cancellation is handled from every state: a relationship that never
    /// became active does not wait for a consumer-detach observation that can
    /// never exist, and a request that never reserved anything is a no-op.
    #[test]
    fn cancellation_is_handled_from_every_state() {
        let mut service = service();
        let prepared = process_key();
        let handle = service
            .admit_binding(
                admission(&prepared, RequestedRights::Consume, BindingArbitration::Shared),
                reservation("res-prepared"),
                &fingerprint("prepared"),
            )
            .expect("admitted");
        service
            .observe_binding(
                &handle,
                BindingObservation::new(
                    BindingLifecycleState::Prepared,
                    CompletionCondition::Complete,
                    CompletionCondition::Pending,
                    ReleaseOutcome::Outstanding,
                ),
            )
            .unwrap();
        assert!(!service.detach_requires_evidence(&handle).unwrap());
        service.fence(&handle).unwrap();
        assert_eq!(
            service
                .advance_drain(
                    &handle,
                    DrainStage::ConsumerDetached,
                    DrainEvidence {
                        consumer_detached: false,
                        helpers_finalized: true,
                    },
                )
                .unwrap(),
            DrainStage::ConsumerDetached,
            "a never-active relationship closes its prepared handles without waiting"
        );
        assert_eq!(service.release(&handle).unwrap(), ReleaseReport::ReleasedSourceFreed);

        // An unknown state requires observation: uncertainty is never read as
        // "the consumer never ran".
        let uncertain = process_key();
        let uncertain_handle = service
            .admit_binding(
                admission(&uncertain, RequestedRights::Consume, BindingArbitration::Shared),
                reservation("res-unknown"),
                &fingerprint("unknown"),
            )
            .expect("admitted");
        service
            .observe_binding(&uncertain_handle, observe(BindingLifecycleState::Unknown))
            .unwrap();
        assert!(service.detach_requires_evidence(&uncertain_handle).unwrap());
        service.fence(&uncertain_handle).unwrap();
        assert_eq!(
            service
                .advance_drain(
                    &uncertain_handle,
                    DrainStage::ConsumerDetached,
                    DrainEvidence {
                        consumer_detached: false,
                        helpers_finalized: true,
                    },
                )
                .err()
                .and_then(|error| error.refusal()),
            Some((AdmissionStage::Drain, RefusalReason::UnprovenEffect))
        );
    }

    /// A helper's reference to its own parent reservation is a stage edge, not
    /// a mutual dependency: the activation and drain stage graphs stay
    /// acyclic for a relationship that has no cycle.
    #[test]
    fn a_helper_and_its_parent_share_a_reservation_without_a_stage_cycle() {
        let mut service = service();
        let parent = process_key();
        let handle = service
            .admit_binding(
                admission(&parent, RequestedRights::Consume, BindingArbitration::Shared),
                reservation("res-stage"),
                &fingerprint("stage"),
            )
            .expect("admitted");
        service
            .attach_leg(
                &handle,
                leg_identity("helper", "450"),
                uid(SOURCE_UID),
                RequestedRights::Consume,
                operation("serve-view").implementation().clone(),
            )
            .expect("helper leg");
        assert!(!service.has_stage_cycle(&handle).unwrap());
        let edges = service.stage_edges(&handle).unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].0, ReservationStage::SourcePrepared);
    }

    /// The durable claim record is pre-committed before the handle exists and
    /// is visible to recovery, so a restart reconciles instead of granting
    /// twice.
    #[test]
    fn a_claim_is_durably_recorded_before_its_handle_exists() {
        let store = Arc::new(CellStore::in_memory());
        let mut service = BindingReservationService::new(Arc::clone(&store));
        let key = process_key();
        let handle = service
            .admit_binding(
                admission(&key, RequestedRights::Exclusive, BindingArbitration::Exclusive),
                reservation("res-durable"),
                &fingerprint("durable"),
            )
            .expect("admitted");
        let recovered = store.durable_claims(RESERVATION_CELL);
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].invocation_id, "res-durable");
        assert_eq!(recovered[0].outcome, CellOutcome::Unknown);
        assert_eq!(recovered[0].principal, SOURCE_UID);
        assert_eq!(service.recoverable_claims().len(), 1);

        service.confirm_prepared(&handle).unwrap();
        assert_eq!(
            store.durable_claims(RESERVATION_CELL)[0].outcome,
            CellOutcome::Completed
        );
    }

    /// A relationship is identified by its consumer slot, so a conflicting
    /// second declaration for the same slot is refused at normalization.
    #[test]
    fn a_conflicting_consumer_slot_declaration_is_refused() {
        let mut service = service();
        let key = process_key();
        service
            .admit_binding(
                admission(&key, RequestedRights::Consume, BindingArbitration::Shared),
                reservation("res-slot"),
                &fingerprint("slot-a"),
            )
            .expect("admitted");
        let conflicting = service.admit_binding(
            admission(&key, RequestedRights::Consume, BindingArbitration::Shared),
            reservation("res-slot-2"),
            &fingerprint("slot-b"),
        );
        assert_eq!(
            conflicting.err().and_then(|error| error.refusal()),
            Some((AdmissionStage::Normalize, RefusalReason::ConflictingDeclaration))
        );
    }
}
