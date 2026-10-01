//! Provider-owned `NetworkBinding` admission and shared-fabric realization.
//!
//! One Network's host fabric - its bridges, routes, ownership markers,
//! NetworkManager policy, and single ownership-scoped firewall projection - is
//! realized once per `(Network, execution target)` and shared by every consumer
//! holding a membership on it. What stays per consumer is the traffic policy:
//! the ports that consumer may receive and whether it may originate
//! connections. A membership is admitted from a typed `NetworkBindingRequest`
//! against this provider's own source decision, its declared realization
//! support, and the dependency fence the effect is held to, so admitting a
//! second consumer never mints a second fabric (R22, R34).
//!
//! A membership carries typed ports and an egress flag; it never carries a
//! pre-rendered ruleset. The firewall stays the Network's one ownership slot
//! ([`crate::nftables`]), so a request cannot reach the kernel as a
//! caller-authored script. An observation carrying a foreign marker in a
//! trusted slot refuses the mutation and leaves every observed byte untouched
//! (R37), and releasing one membership never removes a fabric another member
//! still uses (R36).
//!
//! The committed `NetworkBinding` row is the other half of that model. This
//! module derives it from the committed `Network` row: one row per attached
//! consumer, each naming the two rows the relationship joins, the
//! presentation the consumer declared, and this provider's own decision
//! about it ([`canonical_binding_rows`]). The derived row states a
//! relationship and a decision; it never restates the traffic policy, which
//! stays the source's admitted membership state.
//!
//! # What a ceiling is not
//!
//! A Host or Guest's child target-support ceiling bounds what a child of that
//! target may request and creates no membership, no reservation, and no
//! access. Only a parent's own consumption becomes a membership here; a
//! parent's defaults shape one named child's request and grant the parent
//! nothing (AE31-AE33, R16).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, LazyLock};

use d2b_contracts_resource::v3::{
    BoundedToken, BindingArbitration, BindingAuthorization, BindingConsumerKind,
    BindingContractError, BindingEvidence, BindingKey, BindingKind, BindingLifecycleState,
    BindingObservation, BindingRealizationFacet, BindingRealizationSupport, BindingRefusal,
    BindingRowError, BindingSlot, BindingSlotAddress, BindingSlotDecision, BindingSlotIndex,
    BindingSourceDecision, BindingSpecFingerprint, ChildSupportCeiling, CompletionCondition,
    FreshnessTuple, IfName, MAX_PORTS, NetworkBindingRequest, NetworkBindingSpec,
    NetworkIfRole, NetworkMembership, NetworkPresentation, NetworkProvenance, NetworkSpec,
    PortProtocol, PortSpec,
    PrimitiveSpecError, RefusalReason, ReleaseOutcome, RequestedRights, ResourceGeneration,
    ResourceRef, ResourceUid, SourceAdmission, SourceReservation, ZoneId, admit_binding_request,
    canonical_json_bytes, derive_network_ifname, network_binding::NetworkExecutionParentInput,
};

use crate::controller::{NetworkAdmissionIntent, NetworkAdmissionProof};
use d2b_resource_runtime::context::{
    ResourceContext, RowLookup, SpecDecoder, WatchCondition, typed_spec_decoder,
};
use d2b_resource_runtime::driver::{
    DynResourceDriver, ReconcileOutcome, RecoveryOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{
    DriverFailure, DriverOp, FailureClass, FailureComparison, FailureDetail, FailureKind,
    FailureKinds,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_types::{AllowedSources, CONVERTED_TYPE_VERBS, DriverDescriptor, WellKnownType};
use crate::nftables::{
    NetworkNftProjection, NftablesError, SharedNftTable, apply_projection, digest_bytes,
};
use crate::observe::HostNetworkOccupancy;

/// The realization facets this provider declares for `NetworkBinding`.
///
/// Both presentations are provider-owned: the shared fabric is realized once
/// per `(Network, target)`, and a namespace presentation is a named interface
/// inside the consumer's own namespace. Nothing else is realizable here, so a
/// request that needs another facet is refused rather than approximated.
const NETWORK_BINDING_FACETS: [BindingRealizationFacet; 2] = [
    BindingRealizationFacet::SharedFabric,
    BindingRealizationFacet::NamespaceInterface,
];

/// How this source arbitrates one network membership.
///
/// The fabric is shared, so a membership is admitted alongside its peers and
/// never as an exclusive claim on the Network. The admission path and the
/// committed row name it here, so a boundary rebuilding the accepted graph
/// from a row cannot read a different arbitration than admission decided.
const NETWORK_MEMBERSHIP_ARBITRATION: BindingArbitration = BindingArbitration::Shared;

/// Lowercase hexadecimal digits one derived row name is spelled from.
const HEX_DIGITS: [u8; 16] = *b"0123456789abcdef";

/// Bytes of the relationship digest a derived row name is spelled from.
const ROW_NAME_DIGEST_BYTES: usize = 12;

/// Bytes of the consumer slot folded into one source-owned reservation token.
///
/// The token is identity evidence, not a capability, and it stays inside the
/// contract's token bound however long a consumer's slot name is.
const RESERVATION_SLOT_BYTES: usize = 32;

/// Bytes of the consumer identity folded into one reservation token.
const RESERVATION_IDENTITY_BYTES: usize = 8;

/// The declared Network binding realization support, resolved once.
static NETWORK_BINDING_SUPPORT: LazyLock<BindingRealizationSupport> = LazyLock::new(|| {
    BindingRealizationSupport::new(NETWORK_BINDING_FACETS.to_vec())
        .expect("the declared Network binding facets are distinct")
});

/// Closed, value-free refusals from the Network binding path.
///
/// Every variant is field-free: a refusal names a class of failure, never the
/// consumer, the interface, or the host bytes it was protecting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkBindingError {
    /// A reference named a ResourceType this path does not realize.
    WrongResourceType,
    /// The accepted graph produced no authorization for this relationship.
    NotAuthorized,
    /// The request's own fields violated a closed contract bound.
    InvalidRequest,
    /// The Network's own policy refused the exact request.
    SourcePolicyRefused,
    /// A different live declaration already occupies the consumer slot.
    SlotOccupied,
    /// The target-support ceiling does not admit this capability.
    TargetSupportMissing,
    /// The request needs a realization facet this provider does not declare.
    UnsupportedFacet,
    /// The admission is no longer fenced against the observed graph.
    StaleAuthority,
    /// A foreign host marker occupies a trusted slot; nothing was mutated.
    ForeignHostState,
    /// The observed lifecycle is not the one the caller assumed.
    UnexpectedState,
}

impl NetworkBindingError {
    /// Return the stable redacted reason code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::WrongResourceType => "network-binding-wrong-resource-type",
            Self::NotAuthorized => "network-binding-identity-not-authorized",
            Self::InvalidRequest => "network-binding-request-invalid",
            Self::SourcePolicyRefused => "network-binding-source-policy-refused",
            Self::SlotOccupied => "network-binding-slot-occupied",
            Self::TargetSupportMissing => "network-binding-target-support-missing",
            Self::UnsupportedFacet => "network-binding-facet-unsupported",
            Self::StaleAuthority => "network-binding-stale-authority",
            Self::ForeignHostState => "network-binding-foreign-host-state",
            Self::UnexpectedState => "network-binding-unexpected-state",
        }
    }
}

impl core::fmt::Display for NetworkBindingError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for NetworkBindingError {}

impl From<BindingContractError> for NetworkBindingError {
    fn from(error: BindingContractError) -> Self {
        match error {
            BindingContractError::WrongResourceType => Self::WrongResourceType,
            BindingContractError::SlotOccupied | BindingContractError::SourceMismatch => {
                Self::SlotOccupied
            }
            BindingContractError::UnexpectedState => Self::UnexpectedState,
            _ => Self::InvalidRequest,
        }
    }
}

impl From<BindingRowError> for NetworkBindingError {
    fn from(error: BindingRowError) -> Self {
        match error {
            BindingRowError::WrongSourceType
            | BindingRowError::WrongConsumerType
            | BindingRowError::ConsumerNotAdmitted => Self::WrongResourceType,
            BindingRowError::InvalidOperations
            | BindingRowError::DuplicateOperation
            | BindingRowError::LifetimeOutOfBounds => Self::InvalidRequest,
        }
    }
}

impl From<BindingRefusal> for NetworkBindingError {
    fn from(refusal: BindingRefusal) -> Self {
        match refusal.reason() {
            RefusalReason::IdentityNotAuthorized => Self::NotAuthorized,
            RefusalReason::SourcePolicyRefused => Self::SourcePolicyRefused,
            RefusalReason::TargetSupportMissing => Self::TargetSupportMissing,
            RefusalReason::MandatoryFacetUnsupported => Self::UnsupportedFacet,
            RefusalReason::ConflictingDeclaration => Self::SlotOccupied,
            RefusalReason::StaleAuthority | RefusalReason::StoreIncarnationMismatch => {
                Self::StaleAuthority
            }
            _ => Self::InvalidRequest,
        }
    }
}

impl From<PrimitiveSpecError> for NetworkBindingError {
    fn from(_: PrimitiveSpecError) -> Self {
        Self::InvalidRequest
    }
}

impl From<NftablesError> for NetworkBindingError {
    fn from(error: NftablesError) -> Self {
        match error {
            NftablesError::ForeignMarkerPreserved
            | NftablesError::AmbiguousOwnership
            | NftablesError::FirewallCoexistenceMismatch => Self::ForeignHostState,
            NftablesError::InvalidRule | NftablesError::InvalidChainLayout => Self::InvalidRequest,
        }
    }
}

/// The realization facets this provider declares for `NetworkBinding`.
pub fn network_binding_support() -> &'static BindingRealizationSupport {
    &NETWORK_BINDING_SUPPORT
}

/// The execution target one shared fabric realization is bound to.
///
/// Fabric is keyed by source and target, so the same Network realized for the
/// Host and for one Guest are two fabrics with two bridge sets, while every
/// consumer on one target shares that target's single realization.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NetworkFabricTarget(ResourceRef);

impl NetworkFabricTarget {
    /// Construct the exact Host or Guest execution target a fabric realizes on.
    ///
    /// # Errors
    ///
    /// Refuses a reference that is not an execution parent: a Process is a
    /// consumer on a target, never a target itself.
    pub fn new(target: ResourceRef) -> Result<Self, NetworkBindingError> {
        match BindingConsumerKind::from_resource_type(target.resource_type().as_str()) {
            Some(kind) if kind.is_execution_parent() => Ok(Self(target)),
            _ => Err(NetworkBindingError::WrongResourceType),
        }
    }

    /// Borrow the exact execution-target reference.
    pub const fn reference(&self) -> &ResourceRef {
        &self.0
    }

    /// Whether this target is the Zone's own Host.
    pub fn is_host(&self) -> bool {
        BindingConsumerKind::from_resource_type(self.0.resource_type().as_str())
            == Some(BindingConsumerKind::Host)
    }
}

impl core::fmt::Debug for NetworkFabricTarget {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("NetworkFabricTarget")
            .field("target", &self.0)
            .finish()
    }
}

/// The shared-fabric identity: one Network realized once on one target.
///
/// The Network generation is part of the key, so a changed Network spec is a
/// new fabric identity rather than an in-place rewrite of the old one, and the
/// derived bridge, route, and marker names stay bound to the identity that
/// authorized them.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NetworkFabricKey {
    zone_uid: ResourceUid,
    network_uid: ResourceUid,
    network_generation: ResourceGeneration,
    target: NetworkFabricTarget,
}

impl NetworkFabricKey {
    /// Construct one fabric identity from committed Network identity.
    pub const fn new(
        zone_uid: ResourceUid,
        network_uid: ResourceUid,
        network_generation: ResourceGeneration,
        target: NetworkFabricTarget,
    ) -> Self {
        Self {
            zone_uid,
            network_uid,
            network_generation,
            target,
        }
    }

    /// Borrow the enclosing Zone identity.
    pub const fn zone_uid(&self) -> &ResourceUid {
        &self.zone_uid
    }

    /// Borrow the Network identity.
    pub const fn network_uid(&self) -> &ResourceUid {
        &self.network_uid
    }

    /// Return the committed Network generation.
    pub const fn network_generation(&self) -> ResourceGeneration {
        self.network_generation
    }

    /// Borrow the execution target this fabric realizes on.
    pub const fn target(&self) -> &NetworkFabricTarget {
        &self.target
    }
}

impl core::fmt::Debug for NetworkFabricKey {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("NetworkFabricKey")
            .field("zone_uid", &self.zone_uid)
            .field("network_uid", &self.network_uid)
            .field("network_generation", &self.network_generation)
            .field("target", &self.target)
            .finish()
    }
}

/// Derive one consumer's interface on the shared fabric.
///
/// The shared fabric is realized once per `(zone, Network, target)`; each
/// admitted membership contributes one interface on it, keyed by that
/// membership's own consumer identity, so no consumer reaches another's
/// interface by naming a different one.
pub fn membership_interface(
    provenance: &NetworkProvenance,
    consumer_uid: &ResourceUid,
) -> Result<IfName, NetworkBindingError> {
    membership_interface_for(provenance.zone_uid(), provenance.network_uid(), consumer_uid)
}

/// Derive one consumer's interface from the fabric's committed identity alone.
///
/// The fabric is identified by the two store-assigned uids the host admission
/// admitted, and that is all the interface name is a function of. A boundary
/// that holds those two uids - a serving pass rebuilding the accepted graph
/// from committed rows - therefore derives exactly the interface
/// [`membership_interface`] derives from the complete provenance, so one
/// consumer cannot reach two interfaces on one fabric.
pub fn membership_interface_for(
    zone_uid: &ResourceUid,
    network_uid: &ResourceUid,
    consumer_uid: &ResourceUid,
) -> Result<IfName, NetworkBindingError> {
    derive_network_ifname(
        zone_uid,
        network_uid,
        NetworkIfRole::WorkloadGuestTap,
        Some(consumer_uid),
    )
    .map_err(|_| NetworkBindingError::InvalidRequest)
}

// ---------------------------------------------------------------------------
// Committed Network row -> committed NetworkBinding rows
// ---------------------------------------------------------------------------

/// One committed `NetworkBinding` relationship the accepted graph admitted onto
/// one Network's shared fabric.
///
/// A committed `Network` row names the execution targets it attaches but
/// cannot carry their traffic policy, so the derivation reads both halves: the
/// row decides which consumers join its fabric, and this record supplies each
/// one's identity and the exact typed request its own committed relationship
/// declared. The record is therefore the single derivation the committed row
/// and the rendered consumer policy are both read from, so they cannot name
/// different consumers, slots, presentations, or traffic policy. What stays
/// out of the derived row is the traffic policy: the firewall is the Network's
/// one ownership slot, so the row states the relationship and the policy stays
/// here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAdmittedConsumer {
    target: NetworkFabricTarget,
    consumer_uid: ResourceUid,
    request: NetworkBindingRequest,
}

impl NetworkAdmittedConsumer {
    /// Record one admitted execution target and the request it declared.
    ///
    /// # Errors
    ///
    /// Refuses a request whose consumer is not the execution target this
    /// relationship names, a consumer kind [`BindingKind::Network`] does not
    /// admit, and a presentation whose required realization facet this
    /// provider does not declare.
    pub fn new(
        target: ResourceRef,
        consumer_uid: ResourceUid,
        request: NetworkBindingRequest,
    ) -> Result<Self, NetworkBindingError> {
        let target = NetworkFabricTarget::new(target)?;
        if request.consumer_ref() != target.reference() {
            return Err(NetworkBindingError::WrongResourceType);
        }
        let kind =
            BindingConsumerKind::from_resource_type(target.reference().resource_type().as_str())
                .ok_or(NetworkBindingError::WrongResourceType)?;
        if !BindingKind::Network.admits_consumer(kind) {
            return Err(NetworkBindingError::WrongResourceType);
        }
        if request
            .required_facets()
            .iter()
            .any(|facet| !network_binding_support().realizes(*facet))
        {
            return Err(NetworkBindingError::UnsupportedFacet);
        }
        Ok(Self {
            target,
            consumer_uid,
            request,
        })
    }

    /// Borrow the execution target that joins the fabric.
    pub const fn target(&self) -> &NetworkFabricTarget {
        &self.target
    }

    /// Borrow the consumer's store-assigned identity.
    pub const fn consumer_uid(&self) -> &ResourceUid {
        &self.consumer_uid
    }

    /// Borrow the consumer's exact committed request.
    pub const fn request(&self) -> &NetworkBindingRequest {
        &self.request
    }

    /// Borrow the consumer-side presentation its own request declared.
    pub const fn presentation(&self) -> &NetworkPresentation {
        self.request.presentation()
    }
}

/// The committed `Network` row one binding derivation reads.
///
/// Every field is a fact the source already holds: the row's own reference
/// and Zone, the two store-assigned uids the root host admission admitted for
/// the Zone and for this Network, the committed base spec whose attachments
/// are the execution targets that join the fabric, and the consumers the
/// accepted graph admitted onto it.
///
/// The identity pair is deliberately narrower than the host effect
/// provenance. The committed row's own facts - its name, its decision, and
/// the interface each consumer holds - are functions of those two uids alone,
/// so a boundary that holds committed identities and nothing else still
/// derives byte-identical rows rather than a second spelling of them.
pub struct NetworkBindingSource<'a> {
    /// The committed Network row's exact reference.
    pub network_ref: &'a ResourceRef,
    /// The Zone the relationships belong to.
    pub zone: &'a ZoneId,
    /// The Zone's store-assigned identity.
    pub zone_uid: &'a ResourceUid,
    /// The Network's store-assigned identity.
    pub network_uid: &'a ResourceUid,
    /// The committed Network base spec, whose attachments are the execution
    /// targets that join the fabric.
    pub spec: &'a NetworkSpec,
    /// The consumers the accepted graph admitted. It may name a consumer the
    /// committed row does not attach: a source row implies no relationship
    /// for a consumer it does not declare, so such a consumer derives no row.
    pub consumers: &'a [NetworkAdmittedConsumer],
}

impl core::fmt::Debug for NetworkBindingSource<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("NetworkBindingSource")
            .field("network_ref", &self.network_ref)
            .field("zone", &self.zone)
            .field("network_uid", &self.network_uid)
            .field("attachment_count", &self.spec.attachments().len())
            .field("admitted_consumer_count", &self.consumers.len())
            .finish()
    }
}

/// One canonical `NetworkBinding` row a committed `Network` row implies.
///
/// The row's spec is the neutral binding contract: the two rows this
/// relationship joins, the presentation the consumer declared, and this
/// provider's own decision about it. The row carries no port, egress, or
/// other per-consumer traffic policy, so the Network's single firewall slot
/// keeps its one owner. The request the row was derived from travels beside
/// those bytes, so the relationship the graph reads back and the consumer
/// policy the render writes are two views of ONE derivation rather than two
/// descriptions that can drift apart.
#[derive(Clone, PartialEq, Eq)]
pub struct NetworkBindingRow {
    name: BoundedToken,
    request: NetworkBindingRequest,
    spec: Vec<u8>,
    fabric_interface: IfName,
}

impl NetworkBindingRow {
    /// Borrow the deterministic row name.
    pub const fn name(&self) -> &BoundedToken {
        &self.name
    }

    /// Borrow the exact request this row was derived from.
    ///
    /// It is the request [`NetworkAdmissionIntent`] resolves the rendered
    /// consumer policy against, so the consumer's declaration the source
    /// admitted and the policy the live config carries cannot name different
    /// consumers, slots, presentations, or traffic policy.
    pub const fn request(&self) -> &NetworkBindingRequest {
        &self.request
    }

    /// Borrow the canonical desired bytes committed as the row's spec.
    pub fn spec(&self) -> &[u8] {
        &self.spec
    }

    /// Borrow the interface this membership holds on the shared fabric.
    pub const fn fabric_interface(&self) -> &IfName {
        &self.fabric_interface
    }
}

impl core::fmt::Debug for NetworkBindingRow {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("NetworkBindingRow")
            .field("name", &self.name)
            .field("request", &self.request)
            .finish_non_exhaustive()
    }
}

/// Derive the canonical `NetworkBinding` rows one committed `Network` row
/// implies.
///
/// One row per attached consumer, named from the KTD3 key the committed
/// identities produce rather than from a declaration position, so the same
/// relationship keeps one identity across restarts and two relationships
/// never collide by ordering. A Network that attaches nothing implies no
/// relationship and derives no row: the empty result is the answer, never a
/// default membership.
///
/// # Errors
///
/// Returns [`NetworkBindingError::NotAuthorized`] when a committed attachment
/// names no admitted consumer, [`NetworkBindingError::SlotOccupied`] when
/// two attachments claim one consumer slot, and any refusal
/// the per-relationship row derivation raises for that relationship.
pub fn canonical_binding_rows(
    source: &NetworkBindingSource<'_>,
) -> Result<Vec<NetworkBindingRow>, NetworkBindingError> {
    let mut rows = Vec::with_capacity(source.spec.attachments().len());
    let mut claimed: BTreeSet<&ResourceRef> = BTreeSet::new();
    for attachment in source.spec.attachments() {
        let consumer = source
            .consumers
            .iter()
            .find(|candidate| candidate.target().reference() == attachment.execution_ref())
            .ok_or(NetworkBindingError::NotAuthorized)?;
        // A committed row's slot is derived from the consumer, so a second
        // attachment for one consumer is a second declaration for one slot
        // rather than a second relationship.
        if !claimed.insert(consumer.target().reference()) {
            return Err(NetworkBindingError::SlotOccupied);
        }
        rows.push(canonical_binding_row(source, consumer)?);
    }
    Ok(rows)
}

/// Derive one canonical `NetworkBinding` row from one admitted relationship.
///
/// The interface this membership holds on the fabric is this family's own
/// derivation ([`membership_interface`]) - the one the host admission
/// reserves the tap with and the serving driver presents - so a relationship
/// whose committed identity does not derive one is refused here instead of
/// being committed as a promise nothing could keep. The row's source
/// decision is the rights this family's own rule admits, the arbitration
/// [`admit_source_membership`] applies, and the facets this provider
/// declares it can realize, so a boundary rebuilding the accepted graph from
/// committed rows alone reads back exactly what admission would decide.
///
/// # Errors
///
/// Returns [`NetworkBindingError::SourcePolicyRefused`] when this family
/// admits no right for a network relationship, [`NetworkBindingError::WrongResourceType`]
/// when the source reference is not a `Network` or the consumer is not one
/// this kind admits, and [`NetworkBindingError::InvalidRequest`] when the
/// presentation names an interface the realization cannot present, the
/// committed identity does not derive the membership's fabric interface, or
/// the derived row name is not a bounded token.
fn canonical_binding_row(
    source: &NetworkBindingSource<'_>,
    consumer: &NetworkAdmittedConsumer,
) -> Result<NetworkBindingRow, NetworkBindingError> {
    let right = network_membership_right()?;
    let fabric_interface =
        membership_interface_for(source.zone_uid, source.network_uid, consumer.consumer_uid())?;
    if let NetworkPresentation::NamespaceInterface { name } = consumer.presentation() {
        // A namespace presentation is realized by presenting the interface
        // the consumer named, so a name the kernel could never present is
        // refused here rather than committed as an unreachable promise.
        IfName::parse(name.as_str()).map_err(|_| NetworkBindingError::InvalidRequest)?;
    }
    let spec = NetworkBindingSpec::new(
        source.network_ref.clone(),
        consumer.target().reference().clone(),
        consumer.presentation().clone(),
        BindingSourceDecision::new(
            vec![right],
            NETWORK_MEMBERSHIP_ARBITRATION,
            NETWORK_BINDING_FACETS.to_vec(),
        )?,
    )?;
    let key = spec.key(
        source.zone.clone(),
        source.network_uid.clone(),
        consumer.consumer_uid().clone(),
    )?;
    Ok(NetworkBindingRow {
        name: binding_row_name(&key)?,
        request: consumer.request().clone(),
        spec: canonical_json_bytes(&spec).map_err(|_| NetworkBindingError::InvalidRequest)?,
        fabric_interface,
    })
}

/// The right this family admits one network relationship under.
///
/// Derived from [`BindingKind::admits_rights`] rather than restated, so a
/// committed row can never claim a right this family would refuse.
fn network_membership_right() -> Result<RequestedRights, NetworkBindingError> {
    RequestedRights::ALL
        .into_iter()
        .find(|right| BindingKind::Network.admits_rights(*right))
        .ok_or(NetworkBindingError::SourcePolicyRefused)
}

/// The deterministic row name this source mints for one relationship.
///
/// The name derives from the relationship's committed identities - Zone, the
/// source reference and identity, the consumer reference and identity, and
/// the stable slot - under this family's own domain, never from a declaration
/// index or an attachment order, so reordering attachments never churns
/// identities and two relationships cannot collide by position.
///
/// # Errors
///
/// Returns [`NetworkBindingError::InvalidRequest`] when the derived name is
/// not a bounded token.
pub fn binding_row_name(key: &BindingKey) -> Result<BoundedToken, NetworkBindingError> {
    let mut input = Vec::new();
    push_digest(&mut input, b"d2b/network/binding-row/v1");
    for part in [
        key.zone().to_canonical_string(),
        key.source_ref().to_canonical_string(),
        key.source_uid().to_canonical_string(),
        key.consumer_ref().to_canonical_string(),
        key.consumer_uid().to_canonical_string(),
        key.slot().as_str().to_owned(),
    ] {
        push_digest(&mut input, part.as_bytes());
    }
    let digest = digest_bytes(&input);
    let mut name = String::with_capacity("net-binding-".len() + ROW_NAME_DIGEST_BYTES * 2);
    name.push_str("net-binding-");
    for byte in digest.iter().copied().take(ROW_NAME_DIGEST_BYTES) {
        name.push(char::from(HEX_DIGITS[usize::from(byte >> 4)]));
        name.push(char::from(HEX_DIGITS[usize::from(byte & 0x0f)]));
    }
    BoundedToken::parse(name).map_err(|_| NetworkBindingError::InvalidRequest)
}

/// One attached consumer's store-assigned identity.
///
/// The committed `Network` row names WHICH execution targets join its fabric;
/// the store is the only place a target's durable identity lives, so this is
/// the one fact a producing pass reads that no spec can carry. It carries no
/// policy: the slot, the presentation, the inbound ports, and the egress flag
/// of every derived membership are read off the Network row itself, so a
/// consumer cannot widen its own relationship by asking for more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkConsumerIdentity {
    target: ResourceRef,
    uid: ResourceUid,
}

impl NetworkConsumerIdentity {
    /// Record one execution target's store-assigned identity.
    ///
    /// # Errors
    ///
    /// Refuses a reference that is not an execution parent: a fabric is
    /// realized on a Host or a Guest, and a Process consumes one of those
    /// rather than joining the fabric itself.
    pub fn new(target: ResourceRef, uid: ResourceUid) -> Result<Self, NetworkBindingError> {
        NetworkFabricTarget::new(target.clone())?;
        Ok(Self { target, uid })
    }

    /// Borrow the execution target that joins the fabric.
    pub const fn target(&self) -> &ResourceRef {
        &self.target
    }

    /// Borrow the target's store-assigned identity.
    pub const fn uid(&self) -> &ResourceUid {
        &self.uid
    }
}

/// Derive the admitted consumers one committed `Network` row implies.
///
/// The set is the row's own attachments: a source row implies a relationship
/// for the execution targets it declares and for no other, so a committed row
/// that attaches nothing admits nothing. Each consumer's exact request is
/// read off that same row rather than supplied by a caller:
///
/// - the source and consumer references are the row's own reference and the
///   attachment's own execution target;
/// - the slot is the committed row contract's own slot, taken from the same
///   key derivation the row name is a function of, so a rebuilt request and
///   a committed row cannot name two relationships for one consumer;
/// - the presentation is the shared fabric, the one facet this provider
///   declares for a consumer that reaches the fabric it reserved for it;
/// - the inbound port set is empty and egress is denied, because the
///   committed Network row's own vocabulary declares no per-consumer inbound
///   port and grants no consumer egress. An undeclared permission stays
///   undeclared rather than becoming a default grant.
///
/// # Errors
///
/// Returns [`NetworkBindingError::NotAuthorized`] when a committed attachment
/// names no store-assigned identity, [`NetworkBindingError::WrongResourceType`]
/// when the row's own reference or a target is not one this kind admits, and
/// any refusal the typed request, decision, or key constructors raise.
/// Derive one attached consumer's admitted relationship.
///
/// This is the single derivation both halves read: the producing half reaches
/// it through [`declared_network_consumers`] for every attachment the
/// committed Network row declares, and the serving half reaches it for every
/// committed relationship row it can prove belongs to this Network. A derived
/// request and a committed row are therefore two views of ONE derivation, so
/// they cannot name different consumers, slots, presentations, or traffic
/// policy.
///
/// # Errors
///
/// Returns [`NetworkBindingError::WrongResourceType`] when the source or the
/// target is not one this kind admits, and any refusal the typed decision,
/// request, or key constructors raise for these identities.
fn derived_consumer(
    zone: &ZoneId,
    network_ref: &ResourceRef,
    network_uid: &ResourceUid,
    target: &ResourceRef,
    consumer_uid: &ResourceUid,
    presentation: &NetworkPresentation,
) -> Result<NetworkAdmittedConsumer, NetworkBindingError> {
    let decision = BindingSourceDecision::new(
        vec![network_membership_right()?],
        NETWORK_MEMBERSHIP_ARBITRATION,
        NETWORK_BINDING_FACETS.to_vec(),
    )?;
    // The consumer slot is read back off the committed row contract's own key
    // derivation, the same derivation the row name is a function of, rather
    // than spelled here: a rebuilt request and a committed row cannot name two
    // relationships for one consumer.
    let slot = NetworkBindingSpec::new(
        network_ref.clone(),
        target.clone(),
        presentation.clone(),
        decision,
    )?
    .key(
        zone.clone(),
        network_uid.clone(),
        consumer_uid.clone(),
    )
    .map_err(|_| NetworkBindingError::InvalidRequest)?
    .slot()
    .clone();
    NetworkAdmittedConsumer::new(
        target.clone(),
        consumer_uid.clone(),
        NetworkBindingRequest::new(
            network_ref.clone(),
            target.clone(),
            slot,
            NetworkMembership::new(Vec::new(), false)?,
            presentation.clone(),
        )?,
    )
}

/// Derive the admitted consumers one committed `Network` row implies.
///
/// The set is the row's own attachments: a source row implies a relationship
/// for the execution targets it declares and for no other, so a committed row
/// that attaches nothing admits nothing.
///
/// # Errors
///
/// Returns [`NetworkBindingError::NotAuthorized`] when a committed attachment
/// names no store-assigned identity, [`NetworkBindingError::SlotOccupied`]
/// when two attachments claim one consumer, and any refusal
/// the per-relationship derivation raises for that relationship.
pub fn declared_network_consumers(
    zone: &ZoneId,
    network_ref: &ResourceRef,
    network_uid: &ResourceUid,
    spec: &NetworkSpec,
    identities: &[NetworkConsumerIdentity],
) -> Result<Vec<NetworkAdmittedConsumer>, NetworkBindingError> {
    let presentation = NetworkPresentation::shared_fabric();
    let mut consumers = Vec::with_capacity(spec.attachments().len());
    let mut claimed: BTreeSet<&ResourceRef> = BTreeSet::new();
    for attachment in spec.attachments() {
        let identity = identities
            .iter()
            .find(|candidate| candidate.target() == attachment.execution_ref())
            .ok_or(NetworkBindingError::NotAuthorized)?;
        if !claimed.insert(attachment.execution_ref()) {
            return Err(NetworkBindingError::SlotOccupied);
        }
        consumers.push(derived_consumer(
            zone,
            network_ref,
            network_uid,
            attachment.execution_ref(),
            identity.uid(),
            &presentation,
        )?);
    }
    Ok(consumers)
}

/// Derive the committed `NetworkBinding` rows one committed `Network` row
/// implies, from that row alone.
///
/// This is the producing half's whole entry point. It admits the consumers the
/// row's own attachments declare, derives their exact requests from that same
/// row, and mints the committed rows through [`canonical_binding_rows`]. The
/// pass is idempotent in both directions: every derived name is a function of
/// committed identities, so re-ensuring unchanged bytes is the manager's own
/// `Unchanged` answer rather than a second row, and a derived set that shrank
/// names no row the source still owns, so the manager retires it.
///
/// # Errors
///
/// Returns every refusal [`declared_network_consumers`] and
/// [`canonical_binding_rows`] name for the relationships this row implies.
pub fn produce_binding_rows(
    zone: &ZoneId,
    zone_uid: &ResourceUid,
    network_ref: &ResourceRef,
    network_uid: &ResourceUid,
    spec: &NetworkSpec,
    identities: &[NetworkConsumerIdentity],
) -> Result<Vec<NetworkBindingRow>, NetworkBindingError> {
    let consumers =
        declared_network_consumers(zone, network_ref, network_uid, spec, identities)?;
    canonical_binding_rows(&NetworkBindingSource {
        network_ref,
        zone,
        zone_uid,
        network_uid,
        spec,
        consumers: &consumers,
    })
}

/// One committed `NetworkBinding` row a boundary reads back.
///
/// The name, the committed desired bytes, and the consumer's store-assigned
/// identity are the three facts a committed row of this type carries. The
/// identity is read from the consumer's own committed row rather than from the
/// relationship row, and [`served_network_consumers`] proves the pairing by
/// re-deriving the row name from it: a mismatched identity yields a different
/// name, and the row is refused rather than served under the wrong consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedNetworkBinding {
    name: BoundedToken,
    spec: Vec<u8>,
    consumer_uid: ResourceUid,
}

impl CommittedNetworkBinding {
    /// Record one committed relationship row and the identity of the consumer
    /// whose own committed row this relationship names.
    pub const fn new(name: BoundedToken, spec: Vec<u8>, consumer_uid: ResourceUid) -> Self {
        Self {
            name,
            spec,
            consumer_uid,
        }
    }

    /// Borrow the committed row name.
    pub const fn name(&self) -> &BoundedToken {
        &self.name
    }

    /// Borrow the committed desired bytes.
    pub fn spec(&self) -> &[u8] {
        &self.spec
    }

    /// Borrow the consumer identity this row was read for.
    pub const fn consumer_uid(&self) -> &ResourceUid {
        &self.consumer_uid
    }
}

/// Rebuild the admitted consumers one committed `Network` row's committed
/// `NetworkBinding` rows state.
///
/// This is the serving half: a boundary that can read the committed rows reads
/// back exactly the relationships the producing half derived, and nothing more.
/// Each committed row is decoded as this family's own row contract, its name is
/// re-derived from the committed identities, and its committed decision is
/// compared against the decision this family's own admission applies. Its
/// consumer request then comes from the same per-relationship derivation
/// the producing half used, so a served relationship and the row it was
/// derived from cannot describe different consumers.
///
/// A row whose committed bytes name a consumer the committed `Network` row does
/// not attach yields no admitted consumer: a source row implies no
/// relationship for a consumer it does not declare.
///
/// # Errors
///
/// Returns [`NetworkBindingError::InvalidRequest`] when a committed row is not
/// canonical `NetworkBinding` bytes or its name is not the name this source
/// derives for the identities it was read with,
/// [`NetworkBindingError::SourcePolicyRefused`] when its committed decision
/// names a right, arbitration, or facet this family would not commit, and
/// [`NetworkBindingError::WrongResourceType`] when the row names a source or a
/// consumer this kind does not admit.
pub fn served_network_consumers(
    zone: &ZoneId,
    network_ref: &ResourceRef,
    network_uid: &ResourceUid,
    spec: &NetworkSpec,
    rows: &[CommittedNetworkBinding],
) -> Result<Vec<NetworkAdmittedConsumer>, NetworkBindingError> {
    let committed_decision = BindingSourceDecision::new(
        vec![network_membership_right()?],
        NETWORK_MEMBERSHIP_ARBITRATION,
        NETWORK_BINDING_FACETS.to_vec(),
    )?;
    let attached: BTreeSet<&ResourceRef> = spec
        .attachments()
        .iter()
        .map(|attachment| attachment.execution_ref())
        .collect();
    let mut consumers = Vec::with_capacity(rows.len());
    let mut seen: BTreeSet<&ResourceUid> = BTreeSet::new();
    for row in rows {
        let binding: NetworkBindingSpec = serde_json::from_slice(row.spec())
            .map_err(|_| NetworkBindingError::InvalidRequest)?;
        if binding.network_ref() != network_ref {
            return Err(NetworkBindingError::WrongResourceType);
        }
        let key = binding
            .key(zone.clone(), network_uid.clone(), row.consumer_uid().clone())
            .map_err(NetworkBindingError::from)?;
        if binding_row_name(&key)?.as_str() != row.name().as_str() {
            return Err(NetworkBindingError::InvalidRequest);
        }
        if binding.source() != &committed_decision {
            return Err(NetworkBindingError::SourcePolicyRefused);
        }
        if !attached.contains(binding.execution_ref()) {
            // The row states a relationship this Network no longer attaches.
            // It stays committed and unserved rather than becoming a
            // membership the source row does not declare.
            continue;
        }
        if !seen.insert(row.consumer_uid()) {
            return Err(NetworkBindingError::SlotOccupied);
        }
        consumers.push(derived_consumer(
            zone,
            network_ref,
            network_uid,
            binding.execution_ref(),
            row.consumer_uid(),
            binding.presentation(),
        )?);
    }
    Ok(consumers)
}


/// The NetworkManager unmanaged state observed for one Zone.
///
/// The unmanaged configuration is a shared host file: a foreign entry there is
/// as binding as a foreign nftables rule, so an observation that carries
/// unmanaged devices without this Network's own marker refuses the mutation
/// instead of rewriting a policy somebody else installed.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct NmUnmanagedObservation {
    devices: Vec<String>,
    markers: Vec<String>,
}

impl NmUnmanagedObservation {
    /// Construct the observation from the unmanaged devices and the ownership
    /// markers the host configuration exposes.
    pub fn new(devices: Vec<String>, markers: Vec<String>) -> Self {
        let mut devices = devices;
        devices.sort();
        devices.dedup();
        let mut markers = markers;
        markers.sort();
        markers.dedup();
        Self { devices, markers }
    }

    /// Borrow the unmanaged device names observed.
    pub fn devices(&self) -> &[String] {
        &self.devices
    }

    /// Borrow the ownership markers observed in that configuration.
    pub fn markers(&self) -> &[String] {
        &self.markers
    }

    /// Whether the unmanaged state is empty and therefore free to claim.
    pub fn is_unclaimed(&self) -> bool {
        self.devices.is_empty()
    }

    /// Whether somebody else already manages devices in that configuration.
    pub fn is_foreign(&self, marker: &str) -> bool {
        !self.devices.is_empty() && !self.markers.iter().any(|observed| observed == marker)
    }
}

impl core::fmt::Debug for NmUnmanagedObservation {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("NmUnmanagedObservation")
            .field("device_count", &self.devices.len())
            .field("marker_count", &self.markers.len())
            .finish()
    }
}

/// The observed host state one fabric realization must find clean.
#[derive(Clone)]
pub struct HostStateObservation {
    firewall: SharedNftTable,
    occupancy: HostNetworkOccupancy,
    unmanaged: NmUnmanagedObservation,
}

impl HostStateObservation {
    /// Construct the observation from the three shared host surfaces a Network
    /// fabric owns a slot in.
    pub const fn new(
        firewall: SharedNftTable,
        occupancy: HostNetworkOccupancy,
        unmanaged: NmUnmanagedObservation,
    ) -> Self {
        Self {
            firewall,
            occupancy,
            unmanaged,
        }
    }

    /// Construct an observation of a host that carries none of this Network's
    /// objects.
    pub fn empty() -> Self {
        Self::new(
            SharedNftTable::new(Vec::new()),
            HostNetworkOccupancy::from_parts(Vec::new(), Vec::new(), Vec::new()),
            NmUnmanagedObservation::default(),
        )
    }

    /// Borrow the observed shared nftables table.
    pub const fn firewall(&self) -> &SharedNftTable {
        &self.firewall
    }

    /// Borrow the observed host link, route, and address occupancy.
    pub const fn occupancy(&self) -> &HostNetworkOccupancy {
        &self.occupancy
    }

    /// Borrow the observed NetworkManager unmanaged state.
    pub const fn unmanaged(&self) -> &NmUnmanagedObservation {
        &self.unmanaged
    }

    /// Verify that every trusted slot this Network owns is free or already
    /// carries this Network's own ownership marker.
    ///
    /// The firewall check runs the same ownership-scoped projection the fabric
    /// realizes, so a foreign marker in the Network's slot is refused by the
    /// existing path rather than by a second interpretation of it. The whole
    /// check borrows the observation: a refusal leaves every observed byte
    /// exactly as it was.
    pub fn verify(&self, intent: &NetworkAdmissionIntent) -> Result<(), NetworkBindingError> {
        apply_projection(
            &self.firewall,
            &NetworkNftProjection::empty(intent.key().network_uid().clone()),
        )?;
        for ifname in intent.interface_names() {
            let Some(expected) = intent.interface_ownership_marker(ifname) else {
                continue;
            };
            if !marker_matches(self.occupancy.interface_ownership_markers(ifname), expected) {
                return Err(NetworkBindingError::ForeignHostState);
            }
        }
        for route in intent.routes() {
            let Some(expected) = intent.route_ownership_marker(route) else {
                continue;
            };
            if !marker_matches(self.occupancy.route_ownership_markers(route), expected) {
                return Err(NetworkBindingError::ForeignHostState);
            }
        }
        if self.unmanaged.is_foreign(intent.ownership_marker()) {
            return Err(NetworkBindingError::ForeignHostState);
        }
        Ok(())
    }
}

impl core::fmt::Debug for HostStateObservation {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("HostStateObservation")
            .field("firewall_entry_count", &self.firewall.entries().len())
            .field("occupancy", &self.occupancy)
            .field("unmanaged", &self.unmanaged)
            .finish()
    }
}

/// Whether an unmarked slot, or one already carrying `expected`, is acceptable.
///
/// An unmarked object stays occupied - it belongs to somebody else - so only an
/// empty marker set or an exact match admits a mutation here.
fn marker_matches(observed: &[String], expected: &str) -> bool {
    observed.is_empty() || observed.iter().any(|value| value == expected)
}

/// The Network's own ceiling on what one consumer may ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkMembershipCeiling {
    allow_egress: bool,
    max_ports: usize,
}

impl NetworkMembershipCeiling {
    /// Construct an explicit ceiling.
    pub const fn new(allow_egress: bool, max_ports: usize) -> Self {
        Self {
            allow_egress,
            max_ports,
        }
    }

    /// Derive the ceiling one Network spec admits.
    ///
    /// A Network with no external attachment has no path off the host, so no
    /// consumer may request egress; the port bound is the contract's own.
    pub fn from_spec(spec: &NetworkSpec) -> Self {
        Self {
            allow_egress: spec.external_attachment().is_some(),
            max_ports: MAX_PORTS,
        }
    }

    /// Whether this Network carries outbound traffic at all.
    pub const fn allow_egress(&self) -> bool {
        self.allow_egress
    }

    /// The most inbound ports one consumer may request.
    pub const fn max_ports(&self) -> usize {
        self.max_ports
    }
}

/// One consumer's admitted traffic policy on the shared fabric.
///
/// The policy is typed data: the ports this consumer may receive and whether it
/// may originate connections. It is never a rendered ruleset, so two consumers
/// on one fabric differ here rather than in a second copy of the bridges,
/// routes, markers, and NetworkManager policy.
#[derive(Clone, PartialEq, Eq)]
pub struct MembershipPolicy {
    consumer_ref: ResourceRef,
    slot: BindingSlot,
    presentation: NetworkPresentation,
    fabric_interface: IfName,
    presented_interface: IfName,
    ports: Vec<PortSpec>,
    allow_egress: bool,
    digest: [u8; 32],
}

impl MembershipPolicy {
    /// Borrow the consumer this policy belongs to.
    pub const fn consumer_ref(&self) -> &ResourceRef {
        &self.consumer_ref
    }

    /// Borrow the stable consumer slot.
    pub const fn slot(&self) -> &BindingSlot {
        &self.slot
    }

    /// Borrow the consumer-side presentation.
    pub const fn presentation(&self) -> &NetworkPresentation {
        &self.presentation
    }

    /// Borrow the interface this membership holds on the shared fabric.
    pub const fn fabric_interface(&self) -> &IfName {
        &self.fabric_interface
    }

    /// Borrow the interface name the consumer sees.
    ///
    /// A namespace presentation names the consumer's own interface; a
    /// shared-fabric presentation sees the fabric interface itself.
    pub const fn presented_interface(&self) -> &IfName {
        &self.presented_interface
    }

    /// Borrow the inbound ports this consumer may receive.
    pub fn ports(&self) -> &[PortSpec] {
        &self.ports
    }

    /// Whether this consumer may originate outbound connections.
    pub const fn allow_egress(&self) -> bool {
        self.allow_egress
    }

    /// Whether this policy admits one inbound `protocol`/`port` pair.
    pub fn admits_inbound(&self, protocol: PortProtocol, port: u16) -> bool {
        self.ports
            .iter()
            .any(|declared| declared.protocol() == protocol && declared.port() == port)
    }

    /// The digest of this consumer's exact policy bytes.
    pub const fn digest(&self) -> [u8; 32] {
        self.digest
    }

    /// Derive one consumer's policy from the committed relationship it names.
    ///
    /// The key's three identity fields are taken directly rather than as a
    /// `BindingKey`, so the admission registry and the host admission intent
    /// derive the same policy from the same relationship instead of two
    /// constructors that could drift.
    pub(crate) fn new(
        consumer_ref: &ResourceRef,
        slot: &BindingSlot,
        consumer_uid: &ResourceUid,
        fabric_provenance: &NetworkProvenance,
        fabric_marker: &str,
        request: &NetworkBindingRequest,
    ) -> Result<Self, NetworkBindingError> {
        let fabric_interface = membership_interface(fabric_provenance, consumer_uid)?;
        let presented_interface = match request.presentation() {
            NetworkPresentation::NamespaceInterface { name } => {
                IfName::parse(name.as_str()).map_err(|_| NetworkBindingError::InvalidRequest)?
            }
            NetworkPresentation::SharedFabric => fabric_interface.clone(),
        };
        let membership = request.membership();
        let egress_part: &[u8] = if membership.allow_egress() {
            b"egress"
        } else {
            b"no-egress"
        };
        let mut digest_input = Vec::new();
        for part in [
            fabric_marker.as_bytes(),
            consumer_uid.as_str().as_bytes(),
            slot.as_str().as_bytes(),
            fabric_interface.as_str().as_bytes(),
            presented_interface.as_str().as_bytes(),
            egress_part,
        ] {
            push_digest(&mut digest_input, part);
        }
        for port in membership.ports() {
            push_digest(&mut digest_input, protocol_token(port.protocol()));
            push_digest(&mut digest_input, &port.port().to_be_bytes());
        }
        Ok(Self {
            consumer_ref: consumer_ref.clone(),
            slot: slot.clone(),
            presentation: request.presentation().clone(),
            fabric_interface,
            presented_interface,
            ports: membership.ports().to_vec(),
            allow_egress: membership.allow_egress(),
            digest: digest_bytes(&digest_input),
        })
    }
}

impl core::fmt::Debug for MembershipPolicy {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("MembershipPolicy")
            .field("consumer_ref", &self.consumer_ref)
            .field("slot", &self.slot)
            .field("port_count", &self.ports.len())
            .field("allow_egress", &self.allow_egress)
            .finish()
    }
}

/// The shared realization one fabric key owns.
///
/// Two consumers admitted on one fabric share this identity: the bridges,
/// routes, ownership markers, NetworkManager policy, and the Network's single
/// firewall projection are realized once, and admitting a second membership
/// does not derive a second copy of them.
#[derive(Clone, PartialEq, Eq)]
pub struct FabricRealization {
    provenance: NetworkProvenance,
    ownership_marker: String,
    digest: [u8; 32],
}

impl FabricRealization {
    /// Borrow the complete immutable Network provenance this fabric is fenced
    /// against.
    pub const fn provenance(&self) -> &NetworkProvenance {
        &self.provenance
    }

    /// Borrow the ownership marker every derived object on this fabric carries.
    pub fn ownership_marker(&self) -> &str {
        &self.ownership_marker
    }

    /// The digest of the shared realization, independent of its members.
    pub const fn digest(&self) -> [u8; 32] {
        self.digest
    }

    fn new(provenance: &NetworkProvenance, intent: &NetworkAdmissionIntent) -> Self {
        let ownership_marker = intent.ownership_marker().to_owned();
        let network_generation = provenance.network_generation().get().to_be_bytes();
        let attachment_generation = provenance.attachment_generation().get().to_be_bytes();
        let mut digest_input = Vec::new();
        for part in [
            provenance.zone_uid().as_str().as_bytes(),
            provenance.network_uid().as_str().as_bytes(),
            network_generation.as_slice(),
            attachment_generation.as_slice(),
            provenance.bundle_generation().as_str().as_bytes(),
            ownership_marker.as_bytes(),
        ] {
            push_digest(&mut digest_input, part);
        }
        for ifname in intent.interface_names() {
            push_digest(&mut digest_input, ifname.as_str().as_bytes());
        }
        for route in intent.routes() {
            let via = route.via().unwrap_or_default().to_owned();
            let device = route.device().unwrap_or_default().to_owned();
            for part in [
                route.destination().as_bytes(),
                via.as_bytes(),
                device.as_bytes(),
                route.table().as_bytes(),
            ] {
                push_digest(&mut digest_input, part);
            }
        }
        Self {
            provenance: provenance.clone(),
            ownership_marker,
            digest: digest_bytes(&digest_input),
        }
    }
}

impl core::fmt::Debug for FabricRealization {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("FabricRealization")
            .field("provenance", &self.provenance)
            .finish()
    }
}

/// One admitted membership: the exact request, the evidence that admitted it,
/// and the provider-owned policy it holds on the shared fabric.
#[derive(Clone, PartialEq, Eq)]
pub struct AdmittedMembership {
    key: BindingKey,
    fabric: NetworkFabricKey,
    policy: MembershipPolicy,
    evidence: BindingEvidence,
}

impl AdmittedMembership {
    /// Borrow the relationship this membership realizes.
    pub const fn key(&self) -> &BindingKey {
        &self.key
    }

    /// Borrow the shared fabric this membership sits on.
    pub const fn fabric(&self) -> &NetworkFabricKey {
        &self.fabric
    }

    /// Borrow this consumer's traffic policy.
    pub const fn policy(&self) -> &MembershipPolicy {
        &self.policy
    }

    /// Borrow the admitted evidence, including the source-owned reservation
    /// identity this relationship was admitted against.
    pub const fn evidence(&self) -> &BindingEvidence {
        &self.evidence
    }

    /// Whether the admission is still fenced against the observed graph.
    pub fn is_current(&self, observed: &[FreshnessTuple]) -> bool {
        self.evidence.is_current(observed)
    }
}

impl core::fmt::Debug for AdmittedMembership {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AdmittedMembership")
            .field("key", &self.key)
            .field("fabric", &self.fabric)
            .field("policy", &self.policy)
            .finish()
    }
}

/// One membership's readiness, with source preparation and consumer-side
/// completion reported separately.
///
/// The two sides stay separate so a relationship that must exist before its
/// consumer starts never forms a startup cycle with the observation that
/// consumer can see it (R39).
#[derive(Clone, PartialEq, Eq)]
pub struct MembershipReadiness {
    key: BindingKey,
    observation: BindingObservation,
}

impl MembershipReadiness {
    /// Borrow the relationship this readiness describes.
    pub const fn key(&self) -> &BindingKey {
        &self.key
    }

    /// Return the observed lifecycle.
    pub const fn state(&self) -> BindingLifecycleState {
        self.observation.state()
    }

    /// Return the source-side preparation condition.
    pub const fn prepare(&self) -> CompletionCondition {
        self.observation.prepare()
    }

    /// Return the consumer-side completion condition.
    pub const fn consumer_completion(&self) -> CompletionCondition {
        self.observation.consumer_completion()
    }

    /// Return the release outcome.
    pub const fn release(&self) -> ReleaseOutcome {
        self.observation.release()
    }

    fn from_record(key: &BindingKey, observation: BindingObservation) -> Self {
        Self {
            key: key.clone(),
            observation,
        }
    }
}

impl core::fmt::Debug for MembershipReadiness {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("MembershipReadiness")
            .field("key", &self.key)
            .field("state", &self.observation.state())
            .field("prepare", &self.observation.prepare())
            .field(
                "consumer_completion",
                &self.observation.consumer_completion(),
            )
            .finish()
    }
}

/// What releasing one membership did to the shared fabric.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FabricRelease {
    remaining_members: usize,
    fabric_retained: bool,
}

impl FabricRelease {
    /// The memberships still using the fabric after this release.
    pub const fn remaining_members(&self) -> usize {
        self.remaining_members
    }

    /// Whether the shared fabric survived this release.
    ///
    /// A live member keeps the fabric, so one consumer leaving never removes
    /// host state another consumer still uses (R36).
    pub const fn fabric_retained(&self) -> bool {
        self.fabric_retained
    }
}

/// What one classified Host or Guest network-attachment input meant.
///
/// Only a parent's own consumption produces a membership. A support ceiling is
/// recorded as an admission constraint and a child default is returned to the
/// caller, so neither creates host state (AE31, AE33).
pub enum ParentInputOutcome {
    /// A child target-support ceiling was recorded for the target.
    Ceiling {
        /// Whether the recorded ceiling admits this binding kind and right.
        admits: bool,
    },
    /// The parent's own consumption was admitted as this membership.
    Membership(Box<AdmittedMembership>),
    /// The defaults shape one named child's request and granted the parent
    /// nothing.
    ChildDefault {
        /// The child these defaults are for.
        child_ref: ResourceRef,
    },
}

impl core::fmt::Debug for ParentInputOutcome {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Ceiling { admits } => formatter
                .debug_struct("ParentInputOutcome::Ceiling")
                .field("admits", admits)
                .finish(),
            Self::Membership(membership) => formatter
                .debug_struct("ParentInputOutcome::Membership")
                .field("membership", membership)
                .finish(),
            Self::ChildDefault { child_ref } => formatter
                .debug_struct("ParentInputOutcome::ChildDefault")
                .field("child_ref", child_ref)
                .finish(),
        }
    }
}

/// Everything the source provider needs to admit one consumer's membership.
///
/// The fields are the root-admitted inputs the effect is held to: the
/// relationship's committed identities, the consumer's exact typed request, the
/// host intent that derived the fabric's names and markers, this Network's own
/// ceiling, the authorization evidence the graph produced, the dependency
/// revisions the admission is fenced against, and the observed host state the
/// realization must find clean. No field carries a host path, a rendered
/// script, or a numerical host principal.
pub struct MembershipAdmission {
    /// The Zone the relationship belongs to.
    pub zone: ZoneId,
    /// The Network's store-assigned identity.
    pub source_uid: ResourceUid,
    /// The consumer's store-assigned identity.
    pub consumer_uid: ResourceUid,
    /// The execution target the fabric realizes on.
    pub target: NetworkFabricTarget,
    /// The consumer's exact typed request.
    pub request: NetworkBindingRequest,
    /// The root-admitted host intent: derived names, ownership markers, the
    /// committed generations, and the installed bundle generation.
    pub host_intent: NetworkAdmissionProof,
    /// This Network's own ceiling on consumer membership.
    pub ceiling: NetworkMembershipCeiling,
    /// The authorization evidence the accepted graph produced.
    pub authorization: BindingAuthorization,
    /// The dependency revisions the admission is fenced against.
    pub dependencies: Vec<FreshnessTuple>,
    /// The observed host state the realization must find clean.
    pub observation: HostStateObservation,
}

impl core::fmt::Debug for MembershipAdmission {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("MembershipAdmission")
            .field("zone", &self.zone)
            .field("source_uid", &self.source_uid)
            .field("consumer_uid", &self.consumer_uid)
            .field("target", &self.target)
            .field("request", &self.request)
            .field("dependency_count", &self.dependencies.len())
            .finish()
    }
}

/// The Network provider's membership registry.
///
/// One entry per shared fabric, keyed by `(zone, Network, Network generation,
/// execution target)`. Memberships live under their fabric and are keyed by
/// the consumer slot address, so two consumers on one Network are two
/// memberships on one realization while one consumer cannot hold two in the
/// same slot.
#[derive(Debug, Default)]
pub struct NetworkBindingRegistry {
    zone: Option<ZoneId>,
    fabrics: BTreeMap<NetworkFabricKey, FabricRecord>,
    slots: BindingSlotIndex,
    ceilings: BTreeMap<NetworkFabricTarget, ChildSupportCeiling>,
}

impl NetworkBindingRegistry {
    /// Construct an empty registry for one Zone.
    ///
    /// A primitive binding is same-Zone, so one registry is scoped to the Zone
    /// its relationships belong to.
    pub fn new(zone: ZoneId) -> Self {
        Self {
            zone: Some(zone),
            ..Self::default()
        }
    }

    /// Borrow the Zone this registry admits relationships in.
    pub const fn zone(&self) -> Option<&ZoneId> {
        self.zone.as_ref()
    }

    /// Every realized fabric identity, in identity order.
    pub fn fabric_keys(&self) -> impl Iterator<Item = &NetworkFabricKey> + '_ {
        self.fabrics.keys()
    }

    /// Borrow one realized fabric, when this registry holds it.
    pub fn fabric(&self, key: &NetworkFabricKey) -> Option<FabricView<'_>> {
        self.fabrics.get(key).map(FabricRecord::view)
    }

    /// How many shared fabrics are realized.
    pub fn fabric_count(&self) -> usize {
        self.fabrics.len()
    }

    /// Borrow the recorded child target-support ceiling for one target.
    pub fn child_ceiling(&self, target: &NetworkFabricTarget) -> Option<&ChildSupportCeiling> {
        self.ceilings.get(target)
    }

    /// Check one candidate declaration against the derived consumer slots.
    ///
    /// The candidate is checked, not recorded: the caller commits its row and
    /// re-derives. A second, differently shaped declaration for one live
    /// consumer slot is refused before any mutation, so arrival order never
    /// decides which relationship is live.
    pub fn check_slot(
        &self,
        key: &BindingKey,
        fingerprint: &BindingSpecFingerprint,
    ) -> Result<BindingSlotDecision, NetworkBindingError> {
        if self.zone.as_ref() != Some(key.zone()) {
            return Err(NetworkBindingError::WrongResourceType);
        }
        self.slots
            .clone()
            .declare(key, fingerprint)
            .map_err(NetworkBindingError::from)
    }

    /// Admit one consumer's membership on the shared fabric.
    ///
    /// Every refusal happens before any host mutation: the target-support
    /// ceiling is consulted first (a ceiling admits, it never creates), then
    /// the trusted host slots are verified, then this provider's own source
    /// decision, the declared support, and the dependency fence are evaluated.
    pub fn admit(
        &mut self,
        admission: MembershipAdmission,
    ) -> Result<AdmittedMembership, NetworkBindingError> {
        if self.zone.as_ref() != Some(&admission.zone) {
            return Err(NetworkBindingError::WrongResourceType);
        }
        if let Some(ceiling) = self.ceilings.get(&admission.target)
            && !ceiling.admits(BindingKind::Network, admission.request.requested_rights())
        {
            return Err(NetworkBindingError::TargetSupportMissing);
        }
        let intent = admission.host_intent.intent();
        admission.observation.verify(intent)?;
        let key = admission.request.key(
            admission.zone.clone(),
            admission.source_uid.clone(),
            admission.consumer_uid.clone(),
        )?;
        let source = admit_source_membership(&key, &admission.request, &admission.ceiling)?;
        let support = network_binding_support();
        if !admission
            .request
            .required_facets()
            .iter()
            .all(|facet| support.realizes(*facet))
        {
            return Err(NetworkBindingError::UnsupportedFacet);
        }
        if !fence_names_both_parties(
            &admission.dependencies,
            &admission.source_uid,
            &admission.consumer_uid,
        ) {
            return Err(NetworkBindingError::StaleAuthority);
        }
        let binding = admit_binding_request(
            &key,
            admission.request.requested_rights(),
            admission.request.required_facets(),
            &admission.authorization,
            &source,
            support,
            &admission.dependencies,
        )?;
        let fingerprint = admission.request.fingerprint();
        self.slots
            .declare(&key, &fingerprint)
            .map_err(NetworkBindingError::from)?;
        let binding_evidence = BindingEvidence::admitted(binding, reservation_for(&key)?);
        let fabric_provenance = intent.key().provenance();
        let provenance = &fabric_provenance;
        let fabric_key = NetworkFabricKey::new(
            provenance.zone_uid().clone(),
            provenance.network_uid().clone(),
            provenance.network_generation(),
            admission.target.clone(),
        );
        // An existing fabric is reused as it stands: the shared realization is
        // never rebuilt for a second member, and the consumer's policy reads
        // the committed one in place rather than a copy of it.
        let (realization, policy) = match self.fabrics.get(&fabric_key) {
            Some(record) => {
                if record.realization.provenance() != provenance {
                    return Err(NetworkBindingError::StaleAuthority);
                }
                let policy = MembershipPolicy::new(
                    key.consumer_ref(),
                    key.slot(),
                    &admission.consumer_uid,
                    record.realization.provenance(),
                    record.realization.ownership_marker(),
                    &admission.request,
                )?;
                (None, policy)
            }
            None => {
                let realization = FabricRealization::new(provenance, intent);
                let policy = MembershipPolicy::new(
                    key.consumer_ref(),
                    key.slot(),
                    &admission.consumer_uid,
                    &realization.provenance,
                    realization.ownership_marker(),
                    &admission.request,
                )?;
                (Some(realization), policy)
            }
        };
        let membership = AdmittedMembership {
            key: key.clone(),
            fabric: fabric_key.clone(),
            policy,
            evidence: binding_evidence,
        };
        let record = match realization {
            Some(realization) => self
                .fabrics
                .entry(fabric_key)
                .or_insert_with(|| FabricRecord::new(realization)),
            None => self
                .fabrics
                .get_mut(&fabric_key)
                .ok_or(NetworkBindingError::StaleAuthority)?,
        };
        record.members.insert(
            key.address(),
            MembershipRecord {
                membership: membership.clone(),
                observation: BindingObservation::new(
                    BindingLifecycleState::Admitted,
                    CompletionCondition::Pending,
                    CompletionCondition::Pending,
                    ReleaseOutcome::Outstanding,
                ),
            },
        );
        self.slots
            .observe(&key, BindingLifecycleState::Admitted)
            .map_err(NetworkBindingError::from)?;
        Ok(membership)
    }

    /// Apply one classified Host or Guest network-attachment input.
    ///
    /// A support ceiling is recorded as the target's admission constraint and
    /// creates nothing; a child default is returned to shape that child's
    /// request and grants the parent nothing; only the parent's own consumption
    /// is admitted as a membership (AE31-AE33).
    pub fn classify_parent_input(
        &mut self,
        target: NetworkFabricTarget,
        input: &NetworkExecutionParentInput,
        admission: Option<MembershipAdmission>,
    ) -> Result<ParentInputOutcome, NetworkBindingError> {
        match input {
            NetworkExecutionParentInput::ChildSupportCeiling(ceiling) => {
                let admits = ceiling.admits(BindingKind::Network, RequestedRights::Consume);
                self.ceilings.insert(target, ceiling.clone());
                Ok(ParentInputOutcome::Ceiling { admits })
            }
            NetworkExecutionParentInput::ChildRequestDefaults(defaults) => {
                Ok(ParentInputOutcome::ChildDefault {
                    child_ref: defaults.child_ref().clone(),
                })
            }
            NetworkExecutionParentInput::ParentUse(request) => {
                let admission = admission.ok_or(NetworkBindingError::InvalidRequest)?;
                if &admission.request != request || admission.target != target {
                    return Err(NetworkBindingError::WrongResourceType);
                }
                self.admit(admission)
                    .map(|membership| ParentInputOutcome::Membership(Box::new(membership)))
            }
        }
    }

    /// Borrow one admitted membership, when this registry holds it.
    pub fn membership(&self, key: &BindingKey) -> Option<&AdmittedMembership> {
        let fabric = self.fabric_key_for(key).ok()?;
        self.fabrics
            .get(&fabric)
            .and_then(|record| record.members.get(&key.address()))
            .map(|record| &record.membership)
    }

    /// Every membership on one shared fabric, in consumer-slot order.
    pub fn memberships(&self, fabric: &NetworkFabricKey) -> Vec<&AdmittedMembership> {
        self.fabrics
            .get(fabric)
            .map(|record| {
                record
                    .members
                    .values()
                    .map(|member| &member.membership)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Whether one shared fabric still has a live member.
    pub fn fabric_in_use(&self, fabric: &NetworkFabricKey) -> bool {
        self.fabrics.get(fabric).is_some_and(|record| {
            record
                .members
                .values()
                .any(|member| !member.observation.state().is_terminal())
        })
    }

    /// Record source-side preparation for one admitted membership.
    ///
    /// Preparation happens against the committed consumer identity before the
    /// consumer runs, so the consumer starts only once its required pre-start
    /// conditions hold (R40).
    pub fn prepare(&mut self, key: &BindingKey) -> Result<MembershipReadiness, NetworkBindingError> {
        self.advance(
            key,
            BindingLifecycleState::Admitted,
            |observation| {
                BindingObservation::new(
                    BindingLifecycleState::Prepared,
                    CompletionCondition::Complete,
                    observation.consumer_completion(),
                    observation.release(),
                )
            },
        )
    }

    /// Record consumer-side completion for one prepared membership.
    pub fn complete(
        &mut self,
        key: &BindingKey,
    ) -> Result<MembershipReadiness, NetworkBindingError> {
        self.advance(
            key,
            BindingLifecycleState::Prepared,
            |observation| {
                BindingObservation::new(
                    BindingLifecycleState::Active,
                    observation.prepare(),
                    CompletionCondition::Complete,
                    observation.release(),
                )
            },
        )
    }

    /// Block new use for one membership ahead of its typed release.
    pub fn revoke(&mut self, key: &BindingKey) -> Result<MembershipReadiness, NetworkBindingError> {
        self.advance(
            key,
            BindingLifecycleState::Active,
            |observation| {
                BindingObservation::new(
                    BindingLifecycleState::Revoking,
                    observation.prepare(),
                    observation.consumer_completion(),
                    ReleaseOutcome::Outstanding,
                )
            },
        )
    }

    /// Drive one membership's outstanding use to the safe state.
    pub fn drain(&mut self, key: &BindingKey) -> Result<MembershipReadiness, NetworkBindingError> {
        self.advance(
            key,
            BindingLifecycleState::Revoking,
            |observation| {
                BindingObservation::new(
                    BindingLifecycleState::Draining,
                    observation.prepare(),
                    CompletionCondition::Pending,
                    ReleaseOutcome::Draining,
                )
            },
        )
    }

    /// Release one membership and report what happened to the shared fabric.
    ///
    /// The fabric is retained while any non-terminal membership remains, so a
    /// consumer leaving never removes host state another consumer still uses
    /// (R36). Only the last membership's release retires the fabric entry; the
    /// Network's own finalizer remains the single owner of tearing host state
    /// down (R38).
    pub fn release(&mut self, key: &BindingKey) -> Result<FabricRelease, NetworkBindingError> {
        let fabric_key = self.fabric_key_for(key)?;
        self.write_observation(
            key,
            BindingLifecycleState::Released,
            ReleaseOutcome::Released,
        )?;
        let live = self
            .fabrics
            .get(&fabric_key)
            .map(|record| {
                record
                    .members
                    .values()
                    .filter(|member| !member.observation.state().is_terminal())
                    .count()
            })
            .unwrap_or(0);
        if live == 0 {
            self.fabrics.remove(&fabric_key);
        }
        Ok(FabricRelease {
            remaining_members: live,
            fabric_retained: live > 0,
        })
    }

    /// Report one membership's readiness with its two sides kept separate.
    pub fn readiness(&self, key: &BindingKey) -> Result<MembershipReadiness, NetworkBindingError> {
        let fabric = self.fabric_key_for(key)?;
        let record = self
            .fabrics
            .get(&fabric)
            .ok_or(NetworkBindingError::UnexpectedState)?;
        let member = record
            .members
            .get(&key.address())
            .ok_or(NetworkBindingError::UnexpectedState)?;
        Ok(MembershipReadiness::from_record(key, member.observation))
    }

    /// Recover one membership's observed state from committed evidence.
    ///
    /// Cached readiness cannot remint access: an admission whose dependency
    /// revisions no longer match is refused rather than reported as granted
    /// use, and a membership that never reached a provable effect reports
    /// degraded rather than active (R41).
    pub fn recover(
        &mut self,
        key: &BindingKey,
        observed: &[FreshnessTuple],
    ) -> Result<MembershipReadiness, NetworkBindingError> {
        let current = self.readiness(key)?;
        let live = self
            .membership(key)
            .ok_or(NetworkBindingError::UnexpectedState)?;
        if !live.is_current(observed) {
            return Err(NetworkBindingError::StaleAuthority);
        }
        let state = if current.state().proves_effect() {
            current.state()
        } else {
            BindingLifecycleState::Degraded
        };
        self.advance(key, current.state(), move |observation| {
            BindingObservation::new(
                state,
                observation.prepare(),
                observation.consumer_completion(),
                observation.release(),
            )
        })
    }

    fn advance(
        &mut self,
        key: &BindingKey,
        required: BindingLifecycleState,
        next: impl FnOnce(BindingObservation) -> BindingObservation,
    ) -> Result<MembershipReadiness, NetworkBindingError> {
        let current = self.readiness(key)?;
        if current.state() != required {
            return Err(NetworkBindingError::UnexpectedState);
        }
        let advanced = next(current.observation);
        self.write_observation_value(key, advanced)?;
        Ok(MembershipReadiness::from_record(key, advanced))
    }

    fn write_observation(
        &mut self,
        key: &BindingKey,
        state: BindingLifecycleState,
        release: ReleaseOutcome,
    ) -> Result<(), NetworkBindingError> {
        let current = self.readiness(key)?;
        self.write_observation_value(key, BindingObservation::new(
            state,
            current.prepare(),
            current.consumer_completion(),
            release,
        ))
    }

    fn write_observation_value(
        &mut self,
        key: &BindingKey,
        observation: BindingObservation,
    ) -> Result<(), NetworkBindingError> {
        let fabric = self.fabric_key_for(key)?;
        let record = self
            .fabrics
            .get_mut(&fabric)
            .ok_or(NetworkBindingError::UnexpectedState)?;
        let member = record
            .members
            .get_mut(&key.address())
            .ok_or(NetworkBindingError::UnexpectedState)?;
        member.observation = observation;
        self.slots
            .observe(key, observation.state())
            .map_err(NetworkBindingError::from)
    }

    fn fabric_key_for(&self, key: &BindingKey) -> Result<NetworkFabricKey, NetworkBindingError> {
        self.fabrics
            .keys()
            .find(|fabric| {
                self.fabrics
                    .get(*fabric)
                    .is_some_and(|record| record.members.contains_key(&key.address()))
            })
            .cloned()
            .ok_or(NetworkBindingError::UnexpectedState)
    }
}

/// One live membership and the observation recorded for it.
#[derive(Clone, PartialEq, Eq)]
struct MembershipRecord {
    membership: AdmittedMembership,
    observation: BindingObservation,
}

impl core::fmt::Debug for MembershipRecord {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("MembershipRecord")
            .field("membership", &self.membership)
            .field("state", &self.observation.state())
            .finish()
    }
}

/// One shared fabric: its realization and the memberships that use it.
#[derive(Clone, PartialEq, Eq, Debug)]
struct FabricRecord {
    realization: FabricRealization,
    members: BTreeMap<BindingSlotAddress, MembershipRecord>,
}

impl FabricRecord {
    fn new(realization: FabricRealization) -> Self {
        Self {
            realization,
            members: BTreeMap::new(),
        }
    }

    fn view(&self) -> FabricView<'_> {
        FabricView {
            realization: &self.realization,
            members: &self.members,
        }
    }
}

/// A read-only view of one realized fabric.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct FabricView<'a> {
    realization: &'a FabricRealization,
    members: &'a BTreeMap<BindingSlotAddress, MembershipRecord>,
}

impl<'a> FabricView<'a> {
    /// Borrow the shared realization every member sits on.
    pub const fn realization(&self) -> &'a FabricRealization {
        self.realization
    }

    /// How many memberships currently sit on this fabric.
    pub fn member_count(&self) -> usize {
        self.members.len()
    }

    /// Every membership on this fabric, in consumer-slot order.
    pub fn members(&self) -> impl Iterator<Item = &'a AdmittedMembership> + 'a {
        self.members
            .values()
            .map(|record| &record.membership)
    }
}

impl core::fmt::Debug for FabricView<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("FabricView")
            .field("realization", &self.realization)
            .field("member_count", &self.member_count())
            .finish()
    }
}

/// The Network provider's own decision on one exact request.
///
/// Fabric is shared, so the source admits a membership alongside its peers
/// rather than arbitrating an exclusive claim. What the source does narrow is
/// the traffic policy: more ports than the Network's ceiling, a repeated
/// `protocol`/`port` pair, and egress the Network has no path to carry are each
/// refused rather than approximated.
fn admit_source_membership(
    key: &BindingKey,
    request: &NetworkBindingRequest,
    ceiling: &NetworkMembershipCeiling,
) -> Result<SourceAdmission, NetworkBindingError> {
    let membership = request.membership();
    if membership.ports().len() > ceiling.max_ports() {
        return Err(NetworkBindingError::SourcePolicyRefused);
    }
    if membership.allow_egress() && !ceiling.allow_egress() {
        return Err(NetworkBindingError::SourcePolicyRefused);
    }
    let mut declared: BTreeSet<(PortProtocol, u16)> = BTreeSet::new();
    for port in membership.ports() {
        if !declared.insert((port.protocol(), port.port())) {
            return Err(NetworkBindingError::SourcePolicyRefused);
        }
    }
    SourceAdmission::new(
        key.clone(),
        vec![request.requested_rights()],
        NETWORK_MEMBERSHIP_ARBITRATION,
    )
    .map_err(NetworkBindingError::from)
}

/// Whether the dependency fence names both the source and the consumer row.
///
/// A fence that omits either side would keep an admission alive across a change
/// to the very row that carried it, which is exactly the cached-readiness
/// failure R41 forbids.
fn fence_names_both_parties(
    dependencies: &[FreshnessTuple],
    source_uid: &ResourceUid,
    consumer_uid: &ResourceUid,
) -> bool {
    let names =
        |candidate: &ResourceUid| dependencies.iter().any(|row| row.resource_uid() == candidate);
    names(source_uid) && names(consumer_uid)
}

/// The source-owned reservation identity one admitted relationship holds.
fn reservation_for(key: &BindingKey) -> Result<SourceReservation, NetworkBindingError> {
    let reservation_id = BoundedToken::parse(format!(
        "net-{}-{}",
        bounded_slot(key.slot().as_str()),
        bounded_identity(key.consumer_uid().as_str())
    ))
    .map_err(NetworkBindingError::from)?;
    Ok(SourceReservation::new(
        key.zone().clone(),
        key.source_uid().clone(),
        reservation_id,
    ))
}

/// The consumer slot, bounded so the reservation token stays in contract.
fn bounded_slot(slot: &str) -> &str {
    if slot.len() <= RESERVATION_SLOT_BYTES {
        return slot;
    }
    slot[..RESERVATION_SLOT_BYTES].trim_end_matches('-')
}

/// The consumer identity, reduced to a bounded deterministic suffix.
fn bounded_identity(identity: &str) -> String {
    identity
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(RESERVATION_IDENTITY_BYTES)
        .collect()
}

/// The closed transport token one declared port folds into a policy digest.
fn protocol_token(protocol: PortProtocol) -> &'static [u8] {
    match protocol {
        PortProtocol::Tcp => b"tcp",
        PortProtocol::Udp => b"udp",
        PortProtocol::Sctp => b"sctp",
    }
}

/// Length-prefix one part so two concatenations cannot be confused.
fn push_digest(input: &mut Vec<u8>, part: &[u8]) {
    input.extend_from_slice(&(part.len() as u64).to_be_bytes());
    input.extend_from_slice(part);
}

// ---------------------------------------------------------------------------
// Serving one committed `NetworkBinding` row
// ---------------------------------------------------------------------------

/// The `NetworkBinding` ResourceType one committed relationship commits as.
///
/// The name is the contract's own constant rather than a second hand-written
/// spelling, so the child type the producing pass ensures and the type this
/// driver decodes cannot drift.
const NETWORK_BINDING_TYPE: &str =
    d2b_contracts_resource::v3::network_binding::NETWORK_BINDING_RESOURCE_TYPE;

/// The `Network` ResourceType whose committed row is this relationship's
/// source.
const NETWORK_RESOURCE_TYPE: &str = d2b_contracts_resource::v3::network::NETWORK_RESOURCE_TYPE;

/// The `Zone` ResourceType whose committed self row carries the uid the
/// derived fabric identity is a function of.
const ZONE_RESOURCE_TYPE: &str = "Zone";

/// The execution domains the `NetworkBinding` type can be reconciled in.
///
/// A fabric membership is a host-side host effect realized once per `(Network,
/// execution target)`, so the relationship is reconciled on the Host domain
/// whichever Zone declares it.
const NETWORK_BINDING_EXECUTION_DOMAINS: &[&str] = &["host"];

/// The resource types the `NetworkBinding` realization reads while reconciling:
/// the owning `Network` row whose attachments decide which consumers join the
/// fabric, the committed consumer row it delivers to, and the `Zone` self row
/// whose uid the derived fabric identity is a function of.
const NETWORK_BINDING_READS: &[WellKnownType] = &[
    WellKnownType::NETWORK,
    WellKnownType::HOST,
    WellKnownType::GUEST,
    WellKnownType::ZONE,
];

/// Closed, field-free classifications of a serving failure on this row.
///
/// Every variant names a condition, never a material: a refusal carries no
/// host path, no interface name, and no numerical principal, so it reads the
/// same in a status, an audit record, and a log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetworkBindingDriverErrorKind {
    /// The durable spec did not decode as the closed `NetworkBinding`
    /// contract, or its committed name and decision are not this source's.
    SpecInvalid,
    /// The row's owner is not the `Network` row the relationship names.
    OwnerMismatch,
    /// The owning `Network` row, the named consumer row, or the `Zone` self row
    /// is absent or unreadable, or the owning row no longer attaches this
    /// consumer.
    ParentUnavailable,
}

impl NetworkBindingDriverErrorKind {
    /// The registered failure kind this classification reports.
    const fn failure_kind(self) -> FailureKind {
        match self {
            Self::SpecInvalid => FailureKinds::BINDING_SPEC_INVALID,
            Self::OwnerMismatch => FailureKinds::BINDING_OWNER_MISMATCH,
            Self::ParentUnavailable => FailureKinds::BINDING_PARENT_UNAVAILABLE,
        }
    }

    /// How the plane should classify a failure of this kind.
    const fn failure_class(self) -> FailureClass {
        match self {
            Self::SpecInvalid | Self::OwnerMismatch => FailureClass::Terminal,
            Self::ParentUnavailable => FailureClass::Retryable,
        }
    }
}

/// Typed serving failure for one committed `NetworkBinding` row.
#[derive(Debug, Clone)]
pub struct NetworkBindingDriverError {
    kind: NetworkBindingDriverErrorKind,
    op: DriverOp,
    detail: FailureDetail,
}

impl NetworkBindingDriverError {
    fn new(kind: NetworkBindingDriverErrorKind, op: DriverOp) -> Self {
        Self {
            kind,
            op,
            detail: FailureDetail::new(),
        }
    }

    fn with_detail(mut self, detail: FailureDetail) -> Self {
        self.detail = detail;
        self
    }
}

impl core::fmt::Display for NetworkBindingDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self.kind {
            NetworkBindingDriverErrorKind::SpecInvalid => "network-binding-spec-invalid",
            NetworkBindingDriverErrorKind::OwnerMismatch => "network-binding-owner-mismatch",
            NetworkBindingDriverErrorKind::ParentUnavailable => {
                "network-binding-parent-unavailable"
            }
        })
    }
}

impl std::error::Error for NetworkBindingDriverError {}

/// The spec-store envelope for one `NetworkBinding` row, exactly as persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NetworkBindingSpecEnvelope {
    base: d2b_contracts_resource::v3::CanonicalJsonObject,
}

/// The manager-wired decode hook for `NetworkBinding` rows.
///
/// The wire decoder is the row contract's own, so a stored row that is not
/// canonical `NetworkBinding` bytes - an unknown field, a consumer this kind
/// does not admit, a decision the family never commits - is refused here
/// rather than half-read.
pub fn network_binding_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| {
        serde_json::from_slice::<d2b_contracts_resource::v3::ResourceSpec>(bytes).map(|spec| {
            NetworkBindingSpecEnvelope {
                base: spec.base().clone(),
            }
        })
    })
}

/// Everything the plane must construct to instantiate the `NetworkBinding`
/// driver factory for one zone.
pub struct NetworkBindingDriverArgs {
    /// The zone this driver's rows live in.
    pub zone: ZoneId,
}

/// [`ResourceDriverFactory`] for the `NetworkBinding` resource type.
/// Construction is infallible by contract (R3).
pub struct NetworkBindingDriverFactory {
    types: [ResourceTypeName; 1],
    args: NetworkBindingDriverArgs,
}

impl NetworkBindingDriverFactory {
    /// Build the factory for one zone's plane.
    pub fn new(args: NetworkBindingDriverArgs) -> Self {
        Self {
            types: [ResourceTypeName::new(NETWORK_BINDING_TYPE)],
            args,
        }
    }
}

#[async_trait::async_trait]
impl ResourceDriverFactory for NetworkBindingDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(NetworkBindingDriver {
            zone: self.args.zone.clone(),
            watched: Vec::new(),
        })
    }
}

/// One committed relationship a serving pass proved.
struct NetworkBindingRelationship {
    binding: NetworkBindingSpec,
    fabric_interface: IfName,
}

/// One committed `NetworkBinding` row's driver.
///
/// The shared fabric is realized once per `(Network, execution target)` by the
/// owning `Network` row, so this driver owns no host state of its own and holds
/// no port, path, or numerical principal. What it owns is the relationship: it
/// re-derives the committed row from the owning Network row and the consumer's
/// own committed row, refuses a row that no longer matches them, and publishes
/// the interface this consumer holds on the fabric. A boundary that reads the
/// committed rows therefore learns the same relationship this source admitted.
pub struct NetworkBindingDriver {
    zone: ZoneId,
    /// Rows this driver already registered a dependency watch on (R12/R17).
    /// Runtime-only (R6/R11): one registration per target, so the dependency
    /// edge that wakes this actor does not accumulate manager watch entries.
    watched: Vec<ResourceKey>,
}

impl NetworkBindingDriver {
    fn error(
        &self,
        kind: NetworkBindingDriverErrorKind,
        op: DriverOp,
    ) -> NetworkBindingDriverError {
        NetworkBindingDriverError::new(kind, op)
    }

    /// Decode the stored envelope into the strict `NetworkBinding` contract.
    fn decoded_binding(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<NetworkBindingSpec, NetworkBindingDriverError> {
        let envelope = ctx
            .spec::<NetworkBindingSpecEnvelope>()
            .map_err(|_| self.error(NetworkBindingDriverErrorKind::SpecInvalid, op))?;
        serde_json::from_slice::<NetworkBindingSpec>(&envelope.base.to_canonical_bytes())
            .map_err(|_| self.error(NetworkBindingDriverErrorKind::SpecInvalid, op))
    }

    /// The row's own name must be the name this source derives.
    ///
    /// [`binding_row_name`] is a deterministic function of the identities the
    /// row itself carries, so a committed row whose name is anything else was
    /// not minted by this source's producing pass. Checking it here is what
    /// makes the row a boundary reads back and the derivation that minted it
    /// two views of ONE relationship.
    fn check_row_name(
        &self,
        ctx: &ResourceContext,
        binding: &NetworkBindingSpec,
        network_uid: &ResourceUid,
        consumer_uid: &ResourceUid,
        op: DriverOp,
    ) -> Result<(), NetworkBindingDriverError> {
        let key = binding
            .key(self.zone.clone(), network_uid.clone(), consumer_uid.clone())
            .map_err(|_| self.error(NetworkBindingDriverErrorKind::SpecInvalid, op))?;
        let derived = binding_row_name(&key)
            .map_err(|_| self.error(NetworkBindingDriverErrorKind::SpecInvalid, op))?;
        if derived.as_str() != ctx.key().name {
            return Err(self
                .error(NetworkBindingDriverErrorKind::SpecInvalid, op)
                .with_detail(FailureDetail::at("spec/rowName").comparison(
                    FailureComparison::new("binding.rowName", derived.as_str(), &ctx.key().name),
                )));
        }
        Ok(())
    }

    /// The committed `BindingSourceDecision` must be the decision this family
    /// commits.
    ///
    /// The row records what the source admitted when it minted it; this is the
    /// second, independent half, derived from this family's own admission
    /// rather than read back, so a row claiming a wider right, a wider
    /// arbitration, or a facet this provider does not realize is refused.
    fn check_committed_decision(
        &self,
        binding: &NetworkBindingSpec,
        op: DriverOp,
    ) -> Result<(), NetworkBindingDriverError> {
        let refused = |field: &'static str| {
            self.error(NetworkBindingDriverErrorKind::SpecInvalid, op).with_detail(
                FailureDetail::at("spec/decision")
                    .comparison(FailureComparison::new(field, "committed", "committed")),
            )
        };
        let expected = BindingSourceDecision::new(
            vec![network_membership_right().map_err(|_| {
                self.error(NetworkBindingDriverErrorKind::SpecInvalid, op)
            })?],
            NETWORK_MEMBERSHIP_ARBITRATION,
            NETWORK_BINDING_FACETS.to_vec(),
        )
        .map_err(|_| self.error(NetworkBindingDriverErrorKind::SpecInvalid, op))?;
        if binding.source().admitted_rights() != expected.admitted_rights() {
            return Err(refused("source.admittedRights"));
        }
        if binding.source().arbitration() != expected.arbitration() {
            return Err(refused("source.arbitration"));
        }
        if binding.source().realized_facets() != expected.realized_facets() {
            return Err(refused("source.realizedFacets"));
        }
        Ok(())
    }

    /// The owning `Network` row, behind the owner fence.
    ///
    /// The relationship's source is what mints it, so the row the manager
    /// reports as this resource's owner is the row whose attachments decide
    /// whether this consumer still joins the fabric.
    async fn parent_network(
        &self,
        ctx: &mut ResourceContext,
        binding: &NetworkBindingSpec,
        op: DriverOp,
    ) -> Result<(NetworkSpec, ResourceUid), NetworkBindingDriverError> {
        let key = self.network_key(binding);
        let row = match ctx.lookup(&key).await {
            RowLookup::Present { row, .. } => row,
            _ => {
                return Err(self
                    .error(NetworkBindingDriverErrorKind::ParentUnavailable, op)
                    .with_detail(FailureDetail::at("parent/lookup").comparison(
                        FailureComparison::new("parent.networkRef", "present", "absent"),
                    )));
            }
        };
        if let Some(owner) = ctx.owner()
            && owner != &row.uid
        {
            return Err(self
                .error(NetworkBindingDriverErrorKind::OwnerMismatch, op)
                .with_detail(FailureDetail::at("parent/owner").comparison(
                    FailureComparison::new("parent.ownerUid", "this row's owner", "another row"),
                )));
        }
        let spec = decode_network_spec(&row.spec).ok_or_else(|| {
            self.error(NetworkBindingDriverErrorKind::ParentUnavailable, op).with_detail(
                FailureDetail::at("parent/decode").comparison(FailureComparison::new(
                    "parent.spec",
                    "a canonical Network row",
                    "decode failed",
                )),
            )
        })?;
        let uid = ResourceUid::from_bytes(&row.uid)
            .map_err(|_| self.error(NetworkBindingDriverErrorKind::ParentUnavailable, op))?;
        Ok((spec, uid))
    }

    /// The Zone's own store-assigned identity, from its committed self row.
    ///
    /// The fabric's interface names are a function of the Zone uid, and the
    /// store is the only place that identity lives, so a Zone whose self row is
    /// absent defers the pass instead of answering with a number nothing
    /// verified.
    async fn zone_uid(
        &self,
        ctx: &mut ResourceContext,
        op: DriverOp,
    ) -> Result<ResourceUid, NetworkBindingDriverError> {
        let key = ResourceKey::new(self.zone.as_str(), ZONE_RESOURCE_TYPE, self.zone.as_str());
        match ctx.lookup(&key).await {
            RowLookup::Present { row, .. } => ResourceUid::from_bytes(&row.uid)
                .map_err(|_| self.error(NetworkBindingDriverErrorKind::ParentUnavailable, op)),
            _ => Err(self
                .error(NetworkBindingDriverErrorKind::ParentUnavailable, op)
                .with_detail(FailureDetail::at("zone/lookup").comparison(
                    FailureComparison::new("zone.uid", "the committed Zone self row", "absent"),
                ))),
        }
    }

    /// The consumer's own store-assigned identity.
    ///
    /// The identity is part of the relationship's key, so a consumer replaced
    /// under the same name is a different relationship rather than a silent
    /// continuation of the old one.
    async fn consumer_uid(
        &self,
        ctx: &mut ResourceContext,
        binding: &NetworkBindingSpec,
        op: DriverOp,
    ) -> Result<ResourceUid, NetworkBindingDriverError> {
        match ctx.lookup(&self.consumer_key(binding)).await {
            RowLookup::Present { row, .. } => ResourceUid::from_bytes(&row.uid)
                .map_err(|_| self.error(NetworkBindingDriverErrorKind::ParentUnavailable, op)),
            _ => Err(self
                .error(NetworkBindingDriverErrorKind::ParentUnavailable, op)
                .with_detail(FailureDetail::at("consumer/lookup").comparison(
                    FailureComparison::new("consumer.executionRef", "present", "absent"),
                ))),
        }
    }

    /// Every check one serving pass runs: the wire decode, the derived row
    /// name, the committed decision, the owning `Network` row behind its owner
    /// fence, that row's own attachments at their current generation, and the
    /// named consumer row.
    ///
    /// The owning row's attachments are the second, independent half of the
    /// check. A relationship the source row no longer attaches is refused here
    /// rather than served from a stale decision, which is what keeps an
    /// unattached consumer's membership out of the rendered policy.
    async fn structural(
        &self,
        ctx: &mut ResourceContext,
        op: DriverOp,
    ) -> Result<NetworkBindingRelationship, NetworkBindingDriverError> {
        let binding = self.decoded_binding(ctx, op)?;
        self.check_committed_decision(&binding, op)?;
        let (spec, network_uid) = self.parent_network(ctx, &binding, op).await?;
        if !spec
            .attachments()
            .iter()
            .any(|attachment| attachment.execution_ref() == binding.execution_ref())
        {
            return Err(self
                .error(NetworkBindingDriverErrorKind::ParentUnavailable, op)
                .with_detail(FailureDetail::at("parent/attachments").comparison(
                    FailureComparison::new(
                        "network.attachments",
                        binding.execution_ref().to_canonical_string(),
                        "not attached",
                    ),
                )));
        }
        let consumer_uid = self.consumer_uid(ctx, &binding, op).await?;
        self.check_row_name(ctx, &binding, &network_uid, &consumer_uid, op)?;
        let zone_uid = self.zone_uid(ctx, op).await?;
        let fabric_interface =
            membership_interface_for(&zone_uid, &network_uid, &consumer_uid).map_err(|_| {
                self.error(NetworkBindingDriverErrorKind::SpecInvalid, op)
            })?;
        Ok(NetworkBindingRelationship {
            binding,
            fabric_interface,
        })
    }

    /// Register one dependency watch, at most once per target (R12/R17).
    async fn watch_once(&mut self, ctx: &mut ResourceContext, target: ResourceKey) {
        if self.watched.contains(&target) {
            return;
        }
        if ctx
            .watch(target.clone(), WatchCondition::Ready)
            .await
            .is_ok()
        {
            self.watched.push(target);
        }
    }

    /// The consumer row's key, for the dependency edge.
    fn consumer_key(&self, binding: &NetworkBindingSpec) -> ResourceKey {
        ResourceKey::new(
            self.zone.as_str(),
            binding.execution_ref().resource_type().as_str(),
            binding.execution_ref().name().as_str(),
        )
    }

    /// The owning `Network` row's key, for the dependency edge.
    fn network_key(&self, binding: &NetworkBindingSpec) -> ResourceKey {
        ResourceKey::new(
            self.zone.as_str(),
            NETWORK_RESOURCE_TYPE,
            binding.network_ref().name().as_str(),
        )
    }
}

#[async_trait::async_trait]
impl ResourceDriver for NetworkBindingDriver {
    type Error = NetworkBindingDriverError;

    fn classify_error(&self, error: &NetworkBindingDriverError) -> DriverFailure {
        DriverFailure::error(
            error.op,
            error.kind.failure_kind(),
            error.kind.failure_class(),
        )
        .with_detail(error.detail.clone())
    }

    /// Structural validation: the wire decode, the derived row name, the
    /// committed decision, the owning `Network` row behind its owner fence,
    /// that row's own attachments, and the named consumer row.
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        self.structural(ctx, DriverOp::Validate).await?;
        Ok(())
    }

    /// Adoption of the pre-restart incarnation (F2).
    ///
    /// A membership's realization is a tap on a fabric the owning `Network` row
    /// creates and this driver does not pin, so it holds no durable record of
    /// which interface it presented and cannot prove the pre-restart membership
    /// still answers for the fabric that is there. A restart therefore reports
    /// `Missing` and the next reconcile pass re-derives the relationship against
    /// the committed rows as they are now, rather than adopting a membership
    /// whose fabric nothing in this process witnessed.
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        self.structural(ctx, DriverOp::Recover).await?;
        Ok(RecoveryOutcome::Missing)
    }

    /// One reconcile pass: re-derive the committed relationship against the
    /// rows as they are now, then publish the interface this consumer holds.
    ///
    /// The pass re-derives rather than caches. The row name is a function of the
    /// committed identities, the committed decision is compared against the
    /// decision this family's own admission applies, and the owning Network
    /// row's attachments are read at their current generation, so a consumer the
    /// source row no longer attaches is refused rather than reported as a
    /// membership nothing realizes (R41).
    async fn reconcile(
        &mut self,
        ctx: &mut ResourceContext,
    ) -> Result<ReconcileOutcome, Self::Error> {
        let op = DriverOp::Reconcile;
        let relationship = self.structural(ctx, op).await?;
        // Dependency edges (R12/R17): the owning Network row and the consumer
        // row both wake this actor when they change.
        self.watch_once(ctx, self.network_key(&relationship.binding))
            .await;
        self.watch_once(ctx, self.consumer_key(&relationship.binding))
            .await;
        ctx.set_status_projection(serde_json::json!({
            "consumerRef": relationship.binding.execution_ref().to_canonical_string(),
            "presentation": &relationship.binding.presentation(),
            "fabricInterface": relationship.fabric_interface.as_str(),
        }));
        // `Satisfied` is this driver's own convergence: the committed row was
        // proved against the rows it is derived from and the relationship this
        // consumer holds was published. The fabric itself is realized once by
        // the owning `Network` row, so there is no second host effect here for
        // the pass to be waiting on.
        Ok(ReconcileOutcome::Satisfied)
    }

    /// Retiring a relationship retires the row; the fabric belongs to the
    /// owning `Network` row, which owns its own teardown.
    async fn finalize(&mut self, _ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        Ok(())
    }

    /// The consumer's membership is a row on a fabric the owning `Network` row
    /// owns, so retiring this relationship touches no host state of its own.
    async fn delete(&mut self, _ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// The `NetworkBinding` type's driver declaration.
///
/// `NetworkBinding` is `BUILTIN | STARTUP` (no RUNTIME bit): the plane cannot
/// serve a committed fabric membership without it, so it must be registered
/// before the plane opens. The type is not exportable: `ResourceExport` admits
/// only qualified `*.d2bus.org.*Service` types, so a relationship can never be
/// an export subject. The driver serves no broker operations, mints no
/// children, contributes no startup steps, and declares no hosted effects
/// service: a relationship joins one consumer to one fabric and owns nothing
/// else, and a `ServiceDecl` with no host behind it would be a surface nothing
/// can reach.
pub fn network_binding_descriptor(args: NetworkBindingDriverArgs) -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::NETWORK_BINDING,
        allowed_sources: AllowedSources::BUILTIN | AllowedSources::STARTUP,
        verbs: CONVERTED_TYPE_VERBS,
        execution: NETWORK_BINDING_EXECUTION_DOMAINS,
        exportable: false,
        reads: NETWORK_BINDING_READS,
        operations: &[],
        creations: &[],
        startup: &[],
        services: &[],
        decoder: network_binding_spec_decoder(),
        factory: Arc::new(NetworkBindingDriverFactory::new(args)),
    }
}

/// The typed `Network` contract of one committed row's stored spec.
///
/// The universal desired-state layer carries the Provider selection, so the
/// row's base object is the typed Network contract; a row whose base is not
/// canonical `Network` bytes decodes to nothing rather than to a partial spec.
fn decode_network_spec(spec: &[u8]) -> Option<NetworkSpec> {
    let envelope = serde_json::from_slice::<d2b_contracts_resource::v3::ResourceSpec>(spec).ok()?;
    serde_json::from_slice::<NetworkSpec>(&envelope.base().to_canonical_bytes()).ok()
}
