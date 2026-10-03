//! Typed relation indexes derived from accepted desired rows (U6, KTD2-KTD4;
//! R2-R4, R16-R18).
//!
//! R3 distinguishes six edge classes, and this module keeps them in six
//! separate indexes rather than one adjacency map. Ownership, consumption,
//! implementation, placement, authorization, and dependency/observation have
//! different admission, allocation, revocation, and cleanup rules, and a reader
//! that could mistake one for another is the bug R3 exists to prevent.
//!
//! # Derived, never authored
//!
//! Every edge is extracted from a row the store already committed.
//! [`RelationIndex::rebuild`] is a pure function of those rows, so an index
//! rebuilt from the committed rows after a restart equals the one the
//! pre-restart process held (F7), and no separately authored dependency list
//! exists anywhere to drift from them.
//!
//! Ownership has exactly one writer - the durable `owner_uid` column - so an
//! [`RelationExtractor`] cannot emit an ownership edge: restating it would give
//! the relationship two sources of truth. R4 keeps a simple observation
//! reference out of the binding lifecycle, so an [`ObservationRelation`] is
//! indexed as its own class and never acquires a reservation, a binding key,
//! or a release obligation.
//!
//! # The runtime stays generic (R2)
//!
//! A row's `spec` bytes are opaque to the manager, so the per-type reading of
//! a canonical declaration is injected through [`RelationExtractor`]: the
//! runtime owns the index, the owning provider owns the projection. The two
//! projections this crate can read without importing any provider crate -
//! [`BindingRequestRelations`] and [`OperationImplementationRelations`] - ship
//! here because the canonical binding-request and Operation schemas live in
//! `d2b-contracts-resource`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use d2b_contracts_resource::v3::{
    BindingContractError, BindingKey, BindingKind, BindingRealizationFacet, BindingSlot,
    BindingSlotDecision, BindingSlotIndex, BindingSpecFingerprint, CallableOperation,
    CredentialBindingRequest, CREDENTIAL_BINDING_RESOURCE_TYPE, DeviceBindingRequest,
    DEVICE_BINDING_RESOURCE_TYPE, EndpointBindingRequest, ENDPOINT_BINDING_RESOURCE_TYPE,
    NetworkBindingRequest, NETWORK_BINDING_RESOURCE_TYPE, OPERATION_RESOURCE_TYPE,
    OperationImplementation, RequestedRights, ResourceRef, ResourceUid, VOLUME_BINDING_RESOURCE_TYPE,
    VolumeBindingSpec, ZoneId,
};

/// The module declared name, asserted by the crate smoke test.
pub const MODULE_NAME: &str = "relations";

/// The five binding ResourceTypes whose canonical request this module reads.
pub const BINDING_RESOURCE_TYPES: [&str; 5] = [
    VOLUME_BINDING_RESOURCE_TYPE,
    DEVICE_BINDING_RESOURCE_TYPE,
    NETWORK_BINDING_RESOURCE_TYPE,
    ENDPOINT_BINDING_RESOURCE_TYPE,
    CREDENTIAL_BINDING_RESOURCE_TYPE,
];

/// The ResourceTypes a binding request may name as its execution parent.
const EXECUTION_TARGET_TYPES: [&str; 4] = ["Host", "Guest", "Process", "EphemeralProcess"];

// ---------------------------------------------------------------------------
// The six relation classes (R3)
// ---------------------------------------------------------------------------

/// Which of the six distinct edge classes one relation belongs to.
///
/// The class is not decoration: it is what keeps an ownership edge, a
/// consumption relationship, and an observation reference from being read as
/// the same thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RelationClass {
    /// The declared lifecycle owner of a row.
    Ownership,
    /// A source-owned binding relationship with independent admission,
    /// allocation, revocation, and cleanup (R4).
    Consumption,
    /// Which trusted implementation answers a declared callable (R9).
    Implementation,
    /// Where a row realizes.
    Placement,
    /// Which Role/RoleBinding grants a subject authority (R29).
    Authorization,
    /// A dependency or observation reference with no binding lifecycle (R4).
    Observation,
}

impl RelationClass {
    /// Every class, in declaration order.
    pub const ALL: [Self; 6] = [
        Self::Ownership,
        Self::Consumption,
        Self::Implementation,
        Self::Placement,
        Self::Authorization,
        Self::Observation,
    ];

    /// The stable label both sides of a comparison render.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ownership => "ownership",
            Self::Consumption => "consumption",
            Self::Implementation => "implementation",
            Self::Placement => "placement",
            Self::Authorization => "authorization",
            Self::Observation => "observation",
        }
    }
}

impl std::fmt::Display for RelationClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One row's declared lifecycle owner.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OwnershipRelation {
    /// The owning row's store-assigned identity.
    pub owner: ResourceUid,
    /// The owned row's store-assigned identity.
    pub child: ResourceUid,
}

/// One declared source-owned binding relationship, indexed by its KTD3 key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConsumptionRelation {
    /// The relationship identity: Zone, source, consumer, kind, and the
    /// stable consumer slot. Rights and destination are deliberately outside
    /// it, so changing them updates this relationship instead of minting a
    /// second one (KTD3).
    pub key: BindingKey,
    /// The right the committed request asks for.
    pub rights: RequestedRights,
    /// The realization facets the committed request depends on.
    pub required_facets: Vec<BindingRealizationFacet>,
    /// The digest of the exact desired bytes this relationship commits.
    pub fingerprint: BindingSpecFingerprint,
}

/// One declaration's trusted implementation identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ImplementationRelation {
    /// The row that declares the callable.
    pub declaration: ResourceRef,
    /// The trusted provider-owned implementation that answers it.
    pub implementation: OperationImplementation,
}

/// Where one row realizes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlacementRelation {
    /// The placed row's store-assigned identity.
    pub resource: ResourceUid,
    /// The placed row's exact reference.
    pub resource_ref: ResourceRef,
    /// The execution target the row declares.
    pub target: ResourceRef,
}

/// One committed RoleBinding's grant shape, indexed without being evaluated.
///
/// Evaluation is the evaluator's job (R8, KTD4): this index records which Role
/// a binding names and which exact subjects it binds, so the accepted graph can
/// be rebuilt from committed rows instead of from a second authored list.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AuthorizationRelation {
    /// The committed RoleBinding row.
    pub binding: ResourceRef,
    /// The Role whose rules it binds.
    pub role: ResourceRef,
    /// The exact subjects it names.
    pub subjects: Vec<ResourceRef>,
}

/// One dependency or observation reference with no binding lifecycle (R4).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObservationRelation {
    pub observer: ResourceUid,
    pub observed: ResourceUid,
}

/// One graph relationship, tagged with the class it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelationEdge {
    Ownership(OwnershipRelation),
    Consumption(ConsumptionRelation),
    Implementation(ImplementationRelation),
    Placement(PlacementRelation),
    Authorization(AuthorizationRelation),
    Observation(ObservationRelation),
}

impl RelationEdge {
    /// Which of the six classes this edge belongs to.
    pub const fn class(&self) -> RelationClass {
        match self {
            Self::Ownership(_) => RelationClass::Ownership,
            Self::Consumption(_) => RelationClass::Consumption,
            Self::Implementation(_) => RelationClass::Implementation,
            Self::Placement(_) => RelationClass::Placement,
            Self::Authorization(_) => RelationClass::Authorization,
            Self::Observation(_) => RelationClass::Observation,
        }
    }
}

// ---------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------

/// One relation-index derivation failure.
///
/// Every variant is field-free and the offending declaration travels beside it
/// in [`UnresolvedRelation`], so a diagnostic never echoes spec bytes, a host
/// path, or caller-supplied text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationError {
    /// Two projections claimed the same ResourceType. One type has one
    /// canonical projection, or the derived index could be read two ways.
    DuplicateProjection,
    /// A projection emitted an ownership edge. Ownership has exactly one
    /// writer - the durable `owner_uid` column - so it cannot be restated.
    OwnershipNotProjectable,
    /// A declared reference does not name a row committed in this Zone.
    UnresolvedReference,
    /// The committed bytes contradict an already derived relationship in one
    /// consumer slot.
    ConflictingSlot,
}

impl core::fmt::Display for RelationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::DuplicateProjection => "relation-projection-duplicate",
            Self::OwnershipNotProjectable => "relation-ownership-not-projectable",
            Self::UnresolvedReference => "relation-reference-unresolved",
            Self::ConflictingSlot => "relation-slot-conflicting",
        })
    }
}

impl std::error::Error for RelationError {}

/// One accepted desired row, as the index reads it.
///
/// It carries exactly what a canonical declaration can contribute: the Zone it
/// was committed in, the row's store-assigned identity, its exact reference, its
/// declared owner, and its desired bytes. An extractor cannot reach the store,
/// the actor, or another row, so a projection cannot become a second authority
#[derive(Debug, Clone)]
pub struct RelationRow<'a> {
    zone: ZoneId,
    uid: ResourceUid,
    resource_ref: ResourceRef,
    owner: Option<ResourceUid>,
    spec: &'a [u8],
}

impl<'a> RelationRow<'a> {
    /// Construct the read-only view one projection sees.
    pub fn new(
        zone: ZoneId,
        uid: ResourceUid,
        resource_ref: ResourceRef,
        owner: Option<ResourceUid>,
        spec: &'a [u8],
    ) -> Self {
        Self { zone, uid, resource_ref, owner, spec }
    }

    /// The Zone this row's Zone-local relationships are keyed in.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// The row's store-assigned identity.
    pub const fn uid(&self) -> &ResourceUid {
        &self.uid
    }

    /// The row's exact reference.
    pub const fn resource_ref(&self) -> &ResourceRef {
        &self.resource_ref
    }

    /// The row's declared lifecycle owner, when it has one.
    pub const fn owner(&self) -> Option<&ResourceUid> {
        self.owner.as_ref()
    }

    /// The row's canonical desired bytes.
    pub const fn spec(&self) -> &'a [u8] {
        self.spec
    }
}

/// Resolve a declared reference against the rows this Zone has committed.
pub struct RelationResolver<'a> {
    identities: &'a BTreeMap<ResourceUid, ResourceRef>,
}

impl RelationResolver<'_> {
    /// The store-assigned identity a committed row holds for one reference.
    pub fn uid_of(&self, reference: &ResourceRef) -> Result<ResourceUid, RelationError> {
        self.identities
            .iter()
            .find(|(_, declared)| *declared == reference)
            .map(|(uid, _)| uid.clone())
            .ok_or(RelationError::UnresolvedReference)
    }
}

/// The per-type reading of one canonical declaration.
///
/// The owning provider registers one projection per ResourceType; the runtime
/// owns the index it fills. Insertion order never matters: [`RelationIndex`] is
/// rebuilt whole and sorts what it derives, so the same committed rows always
/// produce the same index.
pub trait RelationExtractor: Send + Sync + 'static {
    /// The ResourceTypes this projection reads, in a stable order.
    fn resource_types(&self) -> &[&'static str];

    /// Derive this row's edges from its canonical declaration.
    ///
    /// Returning an empty vector is the normal answer for a type whose
    /// declaration names no relationship of a projected class. Refusing with
    /// [`RelationError::UnresolvedReference`] is how a projection reports a
    /// declaration this Zone has not committed yet.
    fn extract(
        &self,
        row: &RelationRow<'_>,
        resolve: &RelationResolver<'_>,
    ) -> Result<Vec<RelationEdge>, RelationError>;
}

/// The registered projections, keyed by ResourceType.
#[derive(Clone, Default)]
pub struct RelationExtractors {
    by_type: BTreeMap<&'static str, Arc<dyn RelationExtractor>>,
}

impl core::fmt::Debug for RelationExtractors {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RelationExtractors")
            .field("types", &self.by_type.keys())
            .finish()
    }
}

impl RelationExtractors {
    /// Construct an empty registry: only ownership is derived.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one projection for every ResourceType it declares.
    ///
    /// A second projection for an already claimed ResourceType is refused
    /// rather than replacing the first, so the derived index cannot change
    /// meaning under a running process.
    pub fn register(&mut self, extractor: Arc<dyn RelationExtractor>) -> Result<(), RelationError> {
        let types = extractor.resource_types().to_vec();
        if types.iter().any(|name| self.by_type.contains_key(name)) {
            return Err(RelationError::DuplicateProjection);
        }
        for type_name in types {
            self.by_type.insert(type_name, Arc::clone(&extractor));
        }
        Ok(())
    }

    /// The projection for one ResourceType, when one is registered.
    pub fn for_type(&self, type_name: &str) -> Option<&Arc<dyn RelationExtractor>> {
        self.by_type.get(type_name)
    }

    /// The ResourceTypes with a registered projection, in index order.
    pub fn projected_types(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.by_type.keys().copied()
    }
}

/// A declared reference no committed row satisfies.
///
/// An accepted row naming a source this Zone has not committed is an
/// unsatisfied declaration, not a graph relationship: it is recorded so the gap
/// is visible, and it never becomes an edge.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct UnresolvedRelation {
    /// The committed row that declared the relationship.
    pub declaration: ResourceRef,
    /// The declaration's own reference, restated so the entry is a complete
    /// description without a back-pointer into the row table.
    pub referenced: ResourceRef,
    /// Which class the unsatisfied declaration would have joined.
    pub class: RelationClass,
}

// ---------------------------------------------------------------------------
// The index
// ---------------------------------------------------------------------------

/// The six derived relation indexes for one Zone.
///
/// Every field is derived from accepted desired rows and nothing else. The
/// whole value is the unit of comparison the restart path checks: rebuilding
/// from committed rows must reproduce it exactly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelationIndex {
    identities: BTreeMap<ResourceUid, ResourceRef>,
    ownership: BTreeMap<ResourceUid, BTreeSet<ResourceUid>>,
    consumption: BTreeMap<BindingKey, ConsumptionRelation>,
    implementation: BTreeMap<ResourceRef, ImplementationRelation>,
    placement: BTreeMap<ResourceUid, PlacementRelation>,
    authorization: BTreeMap<ResourceRef, AuthorizationRelation>,
    observation: BTreeSet<ObservationRelation>,
    /// The KTD3 consumer slot index, derived from the same committed rows.
    slots: BindingSlotIndex,
    unresolved: BTreeSet<UnresolvedRelation>,
}

impl RelationIndex {
    /// Construct an empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Derive the whole index from committed rows.
    ///
    /// This is both the startup path and the restart path (F7), and it is
    /// total: a row whose projection cannot be read still contributes its
    /// ownership edge and joins [`Self::unresolved`], so one unconvertible row
    /// cannot stop a manager from starting.
    pub fn rebuild(rows: &[RelationRow<'_>], extractors: &RelationExtractors) -> Self {
        let mut identities: BTreeMap<ResourceUid, ResourceRef> = BTreeMap::new();
        for row in rows {
            identities.insert(row.uid.clone(), row.resource_ref.clone());
        }
        // Extraction reads the identity table, so every projection runs before
        // the derived indexes start mutating it.
        let resolver = RelationResolver { identities: &identities };
        let derived: Vec<(usize, Result<Vec<RelationEdge>, RelationError>)> = rows
            .iter()
            .enumerate()
            .map(|(position, row)| {
                let edges = match extractors.for_type(row.resource_ref.resource_type().as_str()) {
                    Some(extractor) => extractor.extract(row, &resolver),
                    None => Ok(Vec::new()),
                };
                (position, edges)
            })
            .collect();
        let mut index = Self { identities, ..Self::new() };
        for (position, edges) in derived {
            let row = &rows[position];
            if let Some(owner) = row.owner() {
                index
                    .ownership
                    .entry(owner.clone())
                    .or_default()
                    .insert(row.uid.clone());
            }
            match edges {
                Ok(edges) => {
                    for edge in edges {
                        index.apply(row, edge);
                    }
                }
                Err(_) => {
                    index.note_unresolved(row.resource_ref.clone(), RelationClass::Consumption)
                }
            }
        }
        index
    }

    fn note_unresolved(&mut self, declaration: ResourceRef, class: RelationClass) {
        self.unresolved.insert(UnresolvedRelation {
            referenced: declaration.clone(),
            declaration,
            class,
        });
    }

    fn apply(&mut self, row: &RelationRow<'_>, edge: RelationEdge) {
        match edge {
            // Ownership is derived from the durable `owner_uid` column alone;
            // a projected ownership edge is dropped rather than letting a
            // projection restate it.
            RelationEdge::Ownership(_) => self.note_unresolved(
                row.resource_ref.clone(),
                RelationClass::Ownership,
            ),
            RelationEdge::Consumption(edge) => {
                if self.slots.declare(&edge.key, &edge.fingerprint).is_err() {
                    self.note_unresolved(
                        row.resource_ref.clone(),
                        RelationClass::Consumption,
                    );
                    return;
                }
                self.consumption.insert(edge.key.clone(), edge);
            }
            RelationEdge::Implementation(edge) => {
                self.implementation.insert(edge.declaration.clone(), edge);
            }
            RelationEdge::Placement(edge) => {
                self.placement.insert(edge.resource.clone(), edge);
            }
            RelationEdge::Authorization(edge) => {
                self.authorization.insert(edge.binding.clone(), edge);
            }
            RelationEdge::Observation(edge) => {
                self.observation.insert(edge);
            }
        }
    }

    /// The row's exact reference, when this Zone has committed it.
    pub fn reference_of(&self, uid: &ResourceUid) -> Option<&ResourceRef> {
        self.identities.get(uid)
    }

    /// The store-assigned identity a committed row holds for one reference.
    pub fn uid_of(&self, reference: &ResourceRef) -> Option<&ResourceUid> {
        self.identities
            .iter()
            .find(|(_, declared)| *declared == reference)
            .map(|(uid, _)| uid)
    }

    /// The declared lifecycle owner of one row.
    pub fn owner_of(&self, uid: &ResourceUid) -> Option<&ResourceUid> {
        self.ownership
            .iter()
            .find_map(|(owner, children)| children.contains(uid).then_some(owner))
    }

    /// The owned children of one row, in index order.
    pub fn owned(&self, owner: &ResourceUid) -> Option<&BTreeSet<ResourceUid>> {
        self.ownership.get(owner)
    }

    /// Every derived ownership edge, in index order.
    pub fn ownership_edges(&self) -> impl Iterator<Item = OwnershipRelation> + '_ {
        self.ownership
            .iter()
            .flat_map(|(owner, children)| {
                children.iter().map(move |child| OwnershipRelation {
                    owner: owner.clone(),
                    child: child.clone(),
                })
            })
    }

    /// The consumption relationship for one exact key.
    pub fn consumption(&self, key: &BindingKey) -> Option<&ConsumptionRelation> {
        self.consumption.get(key)
    }

    /// Every derived consumption relationship, in index order.
    pub fn consumption_all(&self) -> impl Iterator<Item = (&BindingKey, &ConsumptionRelation)> + '_ {
        self.consumption.iter()
    }

    /// The consumption relationships one source owns.
    pub fn consumed_by_source<'a>(
        &'a self,
        source: &'a ResourceUid,
    ) -> impl Iterator<Item = (&'a BindingKey, &'a ConsumptionRelation)> + 'a {
        self.consumption
            .iter()
            .filter(move |(_, edge)| edge.key.source_uid() == source)
    }

    /// The derived implementation for one declaration.
    pub fn implementation(&self, declaration: &ResourceRef) -> Option<&ImplementationRelation> {
        self.implementation.get(declaration)
    }

    /// Every derived implementation edge, in index order.
    pub fn implementation_all(&self) -> impl Iterator<Item = (&ResourceRef, &ImplementationRelation)> + '_ {
        self.implementation.iter()
    }

    /// The declared execution target of one row.
    pub fn placement(&self, resource: &ResourceUid) -> Option<&PlacementRelation> {
        self.placement.get(resource)
    }

    /// Every derived placement edge, in index order.
    pub fn placement_all(&self) -> impl Iterator<Item = (&ResourceUid, &PlacementRelation)> + '_ {
        self.placement.iter()
    }

    /// The derived grant shape for one RoleBinding.
    pub fn authorization(&self, binding: &ResourceRef) -> Option<&AuthorizationRelation> {
        self.authorization.get(binding)
    }

    /// Every derived authorization edge, in index order.
    pub fn authorization_all(&self) -> impl Iterator<Item = (&ResourceRef, &AuthorizationRelation)> + '_ {
        self.authorization.iter()
    }

    /// Whether one observation reference is derived.
    pub fn observes(&self, edge: &ObservationRelation) -> bool {
        self.observation.contains(edge)
    }

    /// Every derived observation reference, in index order.
    pub fn observation_all(&self) -> impl Iterator<Item = &ObservationRelation> + '_ {
        self.observation.iter()
    }

    /// The derived KTD3 consumer slot index.
    pub const fn slots(&self) -> &BindingSlotIndex {
        &self.slots
    }

    /// Declared relationships no committed row satisfies.
    pub fn unresolved(&self) -> impl Iterator<Item = &UnresolvedRelation> + '_ {
        self.unresolved.iter()
    }

    /// Check one candidate declaration against the derived consumer slots.
    ///
    /// The candidate is checked, not recorded: the caller commits its row and
    /// rebuilds. KTD3 therefore refuses a second, differently-shaped
    /// declaration for one live consumer slot before any mutation, instead of
    /// letting arrival order decide which relationship is live.
    pub fn check_slot(
        &self,
        key: &BindingKey,
        fingerprint: &BindingSpecFingerprint,
    ) -> Result<BindingSlotDecision, BindingSlotConflict> {
        self.slots
            .clone()
            .declare(key, fingerprint)
            .map_err(BindingSlotConflict)
    }
}

/// One refused consumer-slot declaration, carrying the contract's own reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindingSlotConflict(BindingContractError);

impl core::fmt::Display for BindingSlotConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        core::fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for BindingSlotConflict {}

// ---------------------------------------------------------------------------
// Contract-layer projections
// ---------------------------------------------------------------------------

/// The reserved fields a stored envelope carries beside the closed base spec.
const RESERVED_ROW_ENVELOPE_FIELDS: &[&str] = &["providerRef", "updatePolicy", "provider"];

/// One decoded committed binding row's canonical relationship.
///
/// KTD2 makes the consumer's desired request the canonical place a relationship
/// is authored: the source controller admits it and mints the source-owned
/// binding row. The row is that declaration plus the source provider's own
/// accepted decision, in the one committed encoding, so the relation index and
/// the manager's pre-commit slot check read the same bytes the registered
/// serving driver reconciles and cannot disagree about which relationship a
/// row declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedBindingRequest {
    kind: BindingKind,
    source_ref: ResourceRef,
    consumer_ref: ResourceRef,
    slot: BindingSlot,
    rights: RequestedRights,
    required_facets: Vec<BindingRealizationFacet>,
    fingerprint: BindingSpecFingerprint,
}

impl DecodedBindingRequest {
    /// Decode one committed row's canonical relationship.
    ///
    /// `None` means the row's bytes are not this family's canonical row,
    /// which is the normal answer for a type whose rows have not been
    /// converted to the row form: such a row declares no indexed consumption
    /// relationship rather than a guessed one.
    pub fn decode(type_name: &str, spec: &[u8]) -> Option<Self> {
        macro_rules! decode {
            ($row:ty, $kind:expr) => {{
                let request = serde_json::from_slice::<$row>(spec).ok()?;
                Some(Self {
                    kind: $kind,
                    source_ref: request.source_ref().clone(),
                    consumer_ref: request.consumer_ref().clone(),
                    slot: request.slot().clone(),
                    rights: request.requested_rights(),
                    required_facets: request.required_facets().to_vec(),
                    fingerprint: request.fingerprint(),
                })
            }};
        }
        match type_name {
            // The Volume row is read as the committed [`VolumeBindingSpec`]:
            // the source provider mints it, so the row's own references and
            // slot are the relationship the source admitted, and this is the
            // same encoding the serving driver decodes.
            VOLUME_BINDING_RESOURCE_TYPE => {
                // The reserved envelope fields sit beside the closed base spec
                // and are not part of it, so they are attributed before the
                // row decodes - the same treatment the serving driver's own row
                // reader applies. One committed encoding, read identically by
                // both.
                let mut value: serde_json::Value = serde_json::from_slice(spec).ok()?;
                let object = value.as_object_mut()?;
                for field in RESERVED_ROW_ENVELOPE_FIELDS {
                    object.remove(*field);
                }
                let row: VolumeBindingSpec = serde_json::from_value(value).ok()?;
                Some(Self {
                    kind: BindingKind::Volume,
                    source_ref: row.volume_ref().clone(),
                    consumer_ref: row.execution_ref().clone(),
                    slot: BindingSlot::parse(row.slot().as_str()).ok()?,
                    rights: row.requested_rights(),
                    required_facets: row.required_facets().to_vec(),
                    fingerprint: row.fingerprint(),
                })
            }
            DEVICE_BINDING_RESOURCE_TYPE => decode!(DeviceBindingRequest, BindingKind::Device),
            NETWORK_BINDING_RESOURCE_TYPE => decode!(NetworkBindingRequest, BindingKind::Network),
            ENDPOINT_BINDING_RESOURCE_TYPE => {
                decode!(EndpointBindingRequest, BindingKind::Endpoint)
            }
            CREDENTIAL_BINDING_RESOURCE_TYPE => {
                decode!(CredentialBindingRequest, BindingKind::Credential)
            }
            _ => None,
        }
    }

    /// The binding family this request belongs to.
    pub const fn kind(&self) -> BindingKind {
        self.kind
    }

    /// The source's exact reference.
    pub const fn source_ref(&self) -> &ResourceRef {
        &self.source_ref
    }

    /// The consumer's exact reference.
    pub const fn consumer_ref(&self) -> &ResourceRef {
        &self.consumer_ref
    }

    /// The stable consumer slot.
    pub const fn slot(&self) -> &BindingSlot {
        &self.slot
    }

    /// The right this request asks for.
    pub const fn rights(&self) -> RequestedRights {
        self.rights
    }

    /// The realization facets this request depends on.
    pub fn required_facets(&self) -> &[BindingRealizationFacet] {
        &self.required_facets
    }

    /// The digest of the exact desired bytes this request commits.
    pub const fn fingerprint(&self) -> &BindingSpecFingerprint {
        &self.fingerprint
    }

    /// Derive the relationship's KTD3 key from committed identities.
    ///
    /// # Errors
    ///
    /// Returns [`BindingContractError`] when the committed identities do not
    /// describe a relationship this family admits - a source of another type,
    /// or a consumer this family does not deliver to.
    pub fn key(
        &self,
        zone: ZoneId,
        source_uid: ResourceUid,
        consumer_uid: ResourceUid,
    ) -> Result<BindingKey, BindingContractError> {
        BindingKey::new(
            zone,
            self.kind,
            self.source_ref.clone(),
            source_uid,
            self.consumer_ref.clone(),
            consumer_uid,
            self.slot.clone(),
        )
    }
}

/// Consumption and placement edges from the five canonical binding requests.
pub struct BindingRequestRelations;

impl RelationExtractor for BindingRequestRelations {
    fn resource_types(&self) -> &[&'static str] {
        &BINDING_RESOURCE_TYPES
    }

    fn extract(
        &self,
        row: &RelationRow<'_>,
        resolve: &RelationResolver<'_>,
    ) -> Result<Vec<RelationEdge>, RelationError> {
        // A row only declares a relationship when its own bytes are its
        // family's canonical request. A row in any other shape declares no
        // indexed consumption relationship rather than a guessed one.
        let Some(request) = DecodedBindingRequest::decode(
            row.resource_ref.resource_type().as_str(),
            row.spec(),
        ) else {
            return Ok(Vec::new());
        };
        let source_uid = resolve.uid_of(request.source_ref())?;
        let consumer_uid = resolve.uid_of(request.consumer_ref())?;
        let key = request
            .key(row.zone().clone(), source_uid, consumer_uid)
            .map_err(|_| RelationError::UnresolvedReference)?;
        let mut edges = vec![RelationEdge::Consumption(ConsumptionRelation {
            key,
            rights: request.rights(),
            required_facets: request.required_facets().to_vec(),
            fingerprint: request.fingerprint().clone(),
        })];
        // A consumer that is itself an execution target also declares where the
        // relationship realizes, so placement comes from the same canonical
        // declaration rather than from a second authored list.
        if EXECUTION_TARGET_TYPES
            .contains(&request.consumer_ref().resource_type().as_str())
        {
            edges.push(RelationEdge::Placement(PlacementRelation {
                resource: row.uid().clone(),
                resource_ref: row.resource_ref().clone(),
                target: request.consumer_ref().clone(),
            }));
        }
        Ok(edges)
    }
}

/// The implementation identity one committed `Operation` row declares (R9).
pub struct OperationImplementationRelations;

impl RelationExtractor for OperationImplementationRelations {
    fn resource_types(&self) -> &[&'static str] {
        &[OPERATION_RESOURCE_TYPE]
    }

    fn extract(
        &self,
        row: &RelationRow<'_>,
        _resolve: &RelationResolver<'_>,
    ) -> Result<Vec<RelationEdge>, RelationError> {
        let Ok(operation) = serde_json::from_slice::<CallableOperation>(row.spec()) else {
            return Ok(Vec::new());
        };
        Ok(vec![RelationEdge::Implementation(ImplementationRelation {
            declaration: row.resource_ref().clone(),
            implementation: operation.implementation().clone(),
        })])
    }
}