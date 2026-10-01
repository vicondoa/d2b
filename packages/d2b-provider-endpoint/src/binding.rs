//! Provider-owned `EndpointBinding` admission and exact endpoint delivery
//! (U18, R23).
//!
//! An `EndpointBinding` grants one consumer the ONE exact endpoint its request
//! named - and the containing host directory is not part of the grant. That is
//! the whole point of the family, so this module never hands a consumer a
//! locator, a directory, or an environment value: it hands a consumer an
//! [`EndpointDelivery`] over the inode the endpoint owner already resolved
//! privately, and it refuses any delivery payload that would reach past that
//! inode.
//!
//! # What the consumer may name
//!
//! An [`EndpointBindingRequest`] names a source `Endpoint`, a consumer, a
//! stable slot, an attachment kind, and a bounded purpose. It has no path
//! field, so there is nothing in the desired request that could be rewritten
//! into a different socket. The exact endpoint itself arrives as an
//! [`EndpointProvenance`] built from the committed `Endpoint` row, and the
//! host's view of it arrives as an [`EndpointAccessObservation`] carrying the
//! pinned `(dev, ino)` the effect adapter resolved - never a caller-supplied
//! pathname. [`fence_delivery_payload`] is the last check on the way out: a
//! consumer payload naming an absolute host path, the socket's containing
//! directory, or a destination this relationship does not own is refused
//! rather than honoured (AE7).
//!
//! # Readiness is effective access, not ACL presence
//!
//! A POSIX ACL mask is recomputed from a file's group bits by any later
//! `chmod`, which silently nullifies a named entry that is still listed. The
//! recorded solution is
//! `docs/solutions/infrastructure/posix-acl-mask-nullified-by-chmod-on-mode-0700-directories.md`,
//! and this module takes its shape from it: [`EndpointAccessObservation`]
//! reports the bits the KERNEL applies - the named entry already ANDed with
//! the mask, folded across every ancestor's effective traverse bit - and a
//! relationship whose effective access is short of the admitted right is
//! refused, never reported prepared (AE19).
//!
//! # Teardown order is a state machine, not a convention
//!
//! A consumer detaches, THEN the endpoint is torn down, THEN the producer or
//! helper that owned it is retired. [`EndpointBindingRegistry::teardown_endpoint`]
//! refuses while any consumer is still attached and
//! [`EndpointBindingRegistry::retire_producer`] refuses while the endpoint has
//! not been torn down, so a helper cannot be removed underneath a consumer
//! that still holds a live endpoint (R36, R38).
//!
//! # What a ceiling is not
//!
//! A Host or Guest's child target-support ceiling bounds what a child of that
//! target may request and creates no binding, no reservation, and no access.
//! Only a parent's own consumption becomes a relationship here; a parent's
//! defaults shape one named child's request and grant the parent nothing
//! (R16, AE31-AE33).
//!
//! # What a source row derives
//!
//! An `EndpointBinding` is a committed row of its own type, and the endpoint
//! owner is what mints it: [`canonical_binding_rows`] turns one committed
//! `Endpoint` row plus the deliveries it declares into exactly the rows the
//! endpoint's own declaration admits, each carrying the source's own
//! [`BindingSourceDecision`](d2b_contracts_resource::v3::BindingSourceDecision).
//! The realized facets are the attachment kind's own
//! [`required_facets`](EndpointAttachmentKind::required_facets) rather than a
//! fixed pair, so a connect or a listen commits the endpoint descriptor alone
//! while an attach commits the descriptor and the private presentation. A
//! source row that declares no delivery derives no row.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use d2b_contracts_resource::v3::{
    AdmissionStage, BindingArbitration, BindingAuthorization, BindingConsumerKind,
    BindingContractError, BindingEvidence, BindingKey, BindingKind, BindingLifecycleState,
    BindingObservation, BindingRealizationFacet, BindingRealizationSupport, BindingRefusal,
    BindingSlot, BindingSlotAddress, BindingSlotDecision, BindingSlotIndex,
    BindingSourceDecision, BindingSpecFingerprint, BindingSupportEntry, BoundedToken,
    ChildSupportCeiling, CompletionCondition, EndpointAttachmentKind, EndpointBindingRequest,
    FreshnessTuple, PrimitiveSpecError, RefusalReason, ReleaseOutcome, RequestedRights,
    ResourceGeneration, ResourceRef, ResourceUid, SourceAdmission, SourceReservation, ZoneId,
    admit_binding_request, canonical_json_bytes, redacted_debug,
};

use d2b_contracts_resource::v3::endpoint_binding::{
    EndpointBindingSpec, EndpointExecutionParentInput,
};

use crate::endpoint::{
    EndpointClass, EndpointConsumerPolicy, EndpointLocality, EndpointOperation, EndpointSpec,
    EndpointTransport,
};

/// The ResourceType of the exact endpoint a relationship's source is.
const ENDPOINT_RESOURCE_TYPE: &str = "Endpoint";

/// The realization facets this provider declares for `EndpointBinding`.
///
/// Both facets are exact-endpoint facets: a verified descriptor for the one
/// admitted inode, or a private binding of that same inode where the backend
/// requires a name. Nothing here realizes "access to the directory that
/// happens to contain the socket", so a request that needs any other facet is
/// refused rather than approximated.
const ENDPOINT_BINDING_FACETS: [BindingRealizationFacet; 2] = [
    BindingRealizationFacet::EndpointDescriptor,
    BindingRealizationFacet::EndpointPathname,
];

/// The declared `EndpointBinding` realization support, resolved once.
static ENDPOINT_BINDING_SUPPORT: LazyLock<BindingRealizationSupport> = LazyLock::new(|| {
    BindingRealizationSupport::new(ENDPOINT_BINDING_FACETS.to_vec())
        .expect("the endpoint binding facet set is fixed and duplicate-free")
});

/// The realization facets this provider declares for `EndpointBinding`.
pub fn endpoint_binding_support() -> &'static BindingRealizationSupport {
    &ENDPOINT_BINDING_SUPPORT
}

/// Bytes of the consumer slot folded into one source-owned reservation token.
///
/// The token is identity evidence, not a capability, and it stays inside the
/// contract's token bound however long a consumer's slot name is.
const RESERVATION_SLOT_BYTES: usize = 32;

/// Bytes of the consumer identity folded into one reservation token.
const RESERVATION_IDENTITY_BYTES: usize = 8;

/// The POSIX permission bit for read.
const PERM_READ: u32 = 0o4;
/// The POSIX permission bit for write.
const PERM_WRITE: u32 = 0o2;
/// The POSIX permission bit for execute (traverse, on a directory).
const PERM_EXECUTE: u32 = 0o1;

/// Closed, value-free refusals from the Endpoint binding path.
///
/// Every variant is field-free: a refusal names a class of failure, never the
/// socket, the consumer, or the host bytes it was protecting. The
/// relationship and the enforcing stage travel beside the rejection in the
/// [`BindingRefusal`] a contract refusal carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointBindingError {
    /// The request names a relationship this registry's Zone or a declared
    /// endpoint does not own.
    WrongResourceType,
    /// The request's own shape is not one this provider admits.
    InvalidRequest,
    /// The endpoint's own consumer policy does not admit this consumer.
    ConsumerNotAllowed,
    /// The endpoint's own consumer policy does not admit this provider
    /// component.
    ComponentNotAllowed,
    /// The endpoint's own consumer policy does not admit the operation the
    /// requested attachment kind performs.
    OperationNotAllowed,
    /// The endpoint does not support attachments, or its own simultaneous
    /// attachment ceiling is already full.
    AttachmentRefused,
    /// The execution target's child target-support ceiling does not admit the
    /// requested endpoint relationship.
    TargetSupportMissing,
    /// The selected realization cannot enforce a facet the request requires.
    UnsupportedFacet,
    /// The dependency fence does not name both committed rows, or an admitted
    /// revision no longer matches the observed graph.
    StaleAuthority,
    /// The lifecycle step was attempted from a state it does not follow.
    UnexpectedState,
    /// The exact endpoint the observation describes is not the endpoint this
    /// relationship was admitted against.
    ForeignEndpointIdentity,
    /// The exact endpoint is not accepting connections or attachments right
    /// now, so the delivery is not ready.
    EndpointNotAccepting,
    /// The bits the kernel actually applies are short of the admitted right,
    /// or an ancestor's effective traverse bit is missing.
    EffectiveAccessMissing,
    /// The exact endpoint's inode was replaced since preparation, so the
    /// prepared delivery no longer names the endpoint that was admitted.
    EndpointIdentityReplaced,
    /// A delivery payload named something outside this relationship's own
    /// destination.
    DeliveryFenceRefused,
    /// A consumer is still attached to the endpoint, so tearing it down would
    /// pull it out from under live use.
    ConsumerStillAttached,
    /// The endpoint has not been torn down yet, so its producer or helper
    /// must stay.
    EndpointNotTornDown,
    /// The contract refused the request.
    Contract(BindingRefusal),
}

impl EndpointBindingError {
    /// The enforcing stage this refusal belongs to.
    ///
    /// The stage travels beside the reason (R42) so a diagnostic can name
    /// where the endpoint owner stopped without echoing anything it was
    /// protecting.
    pub const fn stage(&self) -> AdmissionStage {
        match self {
            Self::ConsumerNotAllowed | Self::ComponentNotAllowed | Self::OperationNotAllowed => {
                AdmissionStage::Authorize
            }
            Self::InvalidRequest
            | Self::WrongResourceType
            | Self::AttachmentRefused
            | Self::TargetSupportMissing => AdmissionStage::Admit,
            Self::UnsupportedFacet => AdmissionStage::Prepare,
            Self::Contract(refusal) => refusal.stage(),
            Self::StaleAuthority
            | Self::ForeignEndpointIdentity
            | Self::EndpointIdentityReplaced
            | Self::EffectiveAccessMissing
            | Self::EndpointNotAccepting => AdmissionStage::Reserve,
            Self::UnexpectedState | Self::DeliveryFenceRefused => AdmissionStage::Activate,
            Self::ConsumerStillAttached | Self::EndpointNotTornDown => AdmissionStage::Drain,
        }
    }
}

impl core::fmt::Display for EndpointBindingError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::WrongResourceType => f.write_str("the request names an endpoint this Zone does not own"),
            Self::InvalidRequest => f.write_str("the endpoint request is not one this provider admits"),
            Self::ConsumerNotAllowed => f.write_str("the endpoint's own consumer policy does not admit this consumer"),
            Self::ComponentNotAllowed => f.write_str("the endpoint's own consumer policy does not admit this provider component"),
            Self::OperationNotAllowed => f.write_str("the endpoint's own consumer policy does not admit this operation"),
            Self::AttachmentRefused => f.write_str("the endpoint does not admit another attachment"),
            Self::TargetSupportMissing => f.write_str("the execution target's child support ceiling does not admit this endpoint relationship"),
            Self::UnsupportedFacet => f.write_str("the selected realization cannot enforce a required endpoint facet"),
            Self::StaleAuthority => f.write_str("the admitted dependency revisions no longer match the observed graph"),
            Self::UnexpectedState => f.write_str("the lifecycle step does not follow the observed state"),
            Self::ForeignEndpointIdentity => f.write_str("the observation describes a different endpoint than the one admitted"),
            Self::EndpointNotAccepting => f.write_str("the exact endpoint is not accepting right now"),
            Self::EffectiveAccessMissing => f.write_str("the effective access the kernel applies is short of the admitted right"),
            Self::EndpointIdentityReplaced => f.write_str("the exact endpoint's inode was replaced since preparation"),
            Self::DeliveryFenceRefused => f.write_str("the delivery payload names something outside this relationship's own destination"),
            Self::ConsumerStillAttached => f.write_str("a consumer is still attached to the endpoint"),
            Self::EndpointNotTornDown => f.write_str("the endpoint has not been torn down yet"),
            Self::Contract(refusal) => write!(f, "{refusal}"),
        }
    }
}

impl std::error::Error for EndpointBindingError {}

impl From<BindingContractError> for EndpointBindingError {
    fn from(error: BindingContractError) -> Self {
        // The contract already reduced the rejection to a field-free class,
        // so a contract refusal keeps its own stage/reason pair rather than
        // being re-labelled with a provider one.
        Self::Contract(BindingRefusal::new(AdmissionStage::Admit, contract_reason(error)))
    }
}

impl From<BindingRefusal> for EndpointBindingError {
    fn from(refusal: BindingRefusal) -> Self {
        Self::Contract(refusal)
    }
}

impl From<DeliveryFenceViolation> for EndpointBindingError {
    fn from(_: DeliveryFenceViolation) -> Self {
        Self::DeliveryFenceRefused
    }
}

impl From<PrimitiveSpecError> for EndpointBindingError {
    fn from(_: PrimitiveSpecError) -> Self {
        Self::InvalidRequest
    }
}

/// The contract's typed reason for one field-free rejection.
fn contract_reason(error: BindingContractError) -> RefusalReason {
    match error {
        BindingContractError::WrongResourceType | BindingContractError::UnsupportedConsumerKind => {
            RefusalReason::IdentityNotAuthorized
        }
        BindingContractError::UnsupportedRight => RefusalReason::SourcePolicyRefused,
        BindingContractError::InvalidField
        | BindingContractError::OutOfRange
        | BindingContractError::InvalidCollection
        | BindingContractError::MissingRequiredField => RefusalReason::SourcePolicyRefused,
        BindingContractError::UnexpectedState => RefusalReason::UnprovenEffect,
        BindingContractError::SlotOccupied | BindingContractError::SourceMismatch => {
            RefusalReason::ConflictingDeclaration
        }
        BindingContractError::WrongConsumer => RefusalReason::IdentityNotAuthorized,
        // This row contract cannot carry the presentation the source declared,
        // so the source policy refuses it rather than the runtime guessing.
        BindingContractError::UnsupportedPresentation => RefusalReason::SourcePolicyRefused,
    }
}

/// The pinned identity of the exact endpoint inode on the host target.
///
/// This is the locator the endpoint owner resolved privately. It is identity,
/// not a path: nothing in this crate can re-open the endpoint from it, and the
/// `Debug` rendering carries the device only, so a log line built from it
/// cannot leak the socket's host location.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EndpointSocketIdentity {
    device: u64,
    inode: u64,
}

impl EndpointSocketIdentity {
    /// Bind one resolved inode identity.
    pub const fn new(device: u64, inode: u64) -> Self {
        Self { device, inode }
    }

    /// The host device the exact endpoint lives on.
    pub const fn device(&self) -> u64 {
        self.device
    }

    /// The inode the exact endpoint resolved to.
    pub const fn inode(&self) -> u64 {
        self.inode
    }
}

impl core::fmt::Debug for EndpointSocketIdentity {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            formatter,
            "EndpointSocketIdentity(dev={}, ino={})",
            self.device, self.inode
        )
    }
}

/// The exact endpoint one admitted relationship delivers.
///
/// The provenance is derived from the committed `Endpoint` row plus the
/// store-assigned identity of its producer, so a relationship is bound to one
/// endpoint at one generation. A changed endpoint spec is a new provenance and
/// therefore a new relationship identity, never an in-place rewrite of the
/// one a consumer already holds.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EndpointProvenance {
    zone_uid: ZoneId,
    endpoint_ref: ResourceRef,
    endpoint_uid: ResourceUid,
    endpoint_generation: ResourceGeneration,
    producer_ref: ResourceRef,
    producer_uid: ResourceUid,
    endpoint_class: EndpointClass,
    transport: EndpointTransport,
    locality: EndpointLocality,
    purpose: BoundedToken,
}

impl EndpointProvenance {
    /// Derive the exact endpoint's identity from its committed row.
    ///
    /// `spec` is the admitted `Endpoint` spec the owner published and
    /// `endpoint_ref` is the store-assigned NAME of that row - the reference
    /// a consumer's request has to name to reach it. Both come from the
    /// committed graph, never from the consumer: the endpoint's own locator
    /// is not an input here and has no field to arrive in.
    ///
    /// # Errors
    ///
    /// Refuses a row reference that does not name an `Endpoint`.
    pub fn new(
        spec: &EndpointSpec,
        endpoint_ref: ResourceRef,
        zone_uid: ZoneId,
        endpoint_uid: ResourceUid,
        endpoint_generation: ResourceGeneration,
        producer_uid: ResourceUid,
    ) -> Result<Self, EndpointBindingError> {
        if endpoint_ref.resource_type().as_str() != ENDPOINT_RESOURCE_TYPE {
            return Err(EndpointBindingError::WrongResourceType);
        }
        Ok(Self {
            zone_uid,
            endpoint_ref,
            endpoint_uid,
            endpoint_generation,
            producer_ref: spec.producer_ref().clone(),
            producer_uid,
            endpoint_class: spec.endpoint_class(),
            transport: spec.transport(),
            locality: spec.locality(),
            purpose: spec.purpose().clone(),
        })
    }

    /// Borrow the Zone the endpoint belongs to.
    pub const fn zone_uid(&self) -> &ZoneId {
        &self.zone_uid
    }

    /// Borrow the exact source `Endpoint` reference.
    pub const fn endpoint_ref(&self) -> &ResourceRef {
        &self.endpoint_ref
    }

    /// Borrow the store-assigned endpoint identity.
    pub const fn endpoint_uid(&self) -> &ResourceUid {
        &self.endpoint_uid
    }

    /// Return the endpoint generation this provenance was derived from.
    pub const fn endpoint_generation(&self) -> ResourceGeneration {
        self.endpoint_generation
    }

    /// Borrow the exact producing resource.
    pub const fn producer_ref(&self) -> &ResourceRef {
        &self.producer_ref
    }

    /// Borrow the store-assigned producer identity.
    pub const fn producer_uid(&self) -> &ResourceUid {
        &self.producer_uid
    }

    /// Return the endpoint class.
    pub const fn endpoint_class(&self) -> EndpointClass {
        self.endpoint_class
    }

    /// Return the transport the endpoint owner resolved it through.
    pub const fn transport(&self) -> EndpointTransport {
        self.transport
    }

    /// Return the endpoint locality.
    pub const fn locality(&self) -> EndpointLocality {
        self.locality
    }

    /// Borrow the bounded purpose.
    pub const fn purpose(&self) -> &BoundedToken {
        &self.purpose
    }
}

impl core::fmt::Debug for EndpointProvenance {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("EndpointProvenance")
            .field("endpoint_uid", &self.endpoint_uid)
            .field("endpoint_generation", &self.endpoint_generation)
            .field("endpoint_class", &self.endpoint_class)
            .field("transport", &self.transport)
            .finish_non_exhaustive()
    }
}

/// What the host actually applies to one consumer principal for one exact
/// endpoint.
///
/// Every field is an EFFECTIVE value, observed through the inode the effect
/// adapter pinned. `effective_rights` is the named ACL entry already ANDed
/// with the ACL mask (or the inode's mode class when it carries no extended
/// ACL); `effective_traverse` is the AND across every ancestor directory's own
/// effective traverse bit. A named entry the mask has nullified contributes
/// nothing, which is the failure a mode reconciliation introduces and the one
/// a presence check cannot see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointAccessObservation {
    socket: EndpointSocketIdentity,
    effective_rights: u32,
    effective_traverse: u32,
    parent_listable: bool,
    accepting: bool,
}

impl EndpointAccessObservation {
    /// Record what the kernel applies for the consumer principal.
    ///
    /// The caller is the declared host facet, which is the only layer that
    /// reads filesystem ACL state; this constructor takes the effective
    /// values it computed, so no caller-supplied string can stand in for an
    /// access check.
    pub const fn new(
        socket: EndpointSocketIdentity,
        effective_rights: u32,
        effective_traverse: u32,
        parent_listable: bool,
        accepting: bool,
    ) -> Self {
        Self {
            socket,
            effective_rights: effective_rights & 0o7,
            effective_traverse: effective_traverse & 0o7,
            parent_listable,
            accepting,
        }
    }

    /// Borrow the pinned identity of the exact endpoint.
    pub const fn socket(&self) -> EndpointSocketIdentity {
        self.socket
    }

    /// The bits the kernel applies to the consumer principal on the socket.
    pub const fn effective_rights(&self) -> u32 {
        self.effective_rights
    }

    /// The effective traverse bit every ancestor directory applies.
    pub const fn effective_traverse(&self) -> u32 {
        self.effective_traverse
    }

    /// Whether the consumer principal may enumerate the socket's parent.
    pub const fn parent_listable(&self) -> bool {
        self.parent_listable
    }

    /// Whether the exact endpoint is accepting connections or attachments.
    pub const fn accepting(&self) -> bool {
        self.accepting
    }

    /// Whether the effective access covers `required` on the socket itself.
    pub const fn grants(&self, required: u32) -> bool {
        self.effective_rights & required == required
    }

    /// Whether every ancestor still applies an effective traverse bit.
    pub const fn traversable(&self) -> bool {
        self.effective_traverse & PERM_EXECUTE == PERM_EXECUTE
    }
}

/// The POSIX bits one admitted right requires of the exact endpoint.
///
/// A consumer that connects reads the socket's identity to reach the endpoint,
/// so a consume right needs read; an accepted connection carries a stream, so
/// the `listen` right additionally needs write. Traverse is checked separately
/// because it belongs to the ancestors, not to the socket.
pub const fn required_right_bits(rights: RequestedRights) -> u32 {
    match rights {
        RequestedRights::Observe => PERM_READ,
        _ => PERM_READ | PERM_WRITE,
    }
}

/// How one prepared relationship receives the exact endpoint.
///
/// Both forms deliver the SAME inode the endpoint owner resolved. Neither
/// form carries a host path, and neither names the directory the socket
/// happens to sit in - a descriptor is a capability and a private
/// presentation is a bind of the admitted inode at a bounded destination
/// inside the launch's own tree.
#[derive(Clone, PartialEq, Eq)]
pub enum EndpointDelivery {
    /// A verified connected or listening descriptor for the exact endpoint.
    Descriptor {
        /// The launch's pre-opened descriptor slot the endpoint is delivered
        /// in. A descriptor delivery has no destination, so a payload that
        /// names one is refused rather than honoured.
        fd_slot: u16,
    },
    /// A private exact-socket presentation: the admitted inode bound at the
    /// consumer's own bounded destination inside the launch's private tree.
    PrivateSocketPresentation {
        /// The single destination component this relationship owns inside
        /// the launch's private root. It is a slot, never a path.
        destination_slot: BoundedToken,
    },
}

redacted_debug!(EndpointDelivery);

impl EndpointDelivery {
    /// The realization facets this delivery form depends on.
    pub const fn required_facets(&self) -> &'static [BindingRealizationFacet] {
        match self {
            Self::Descriptor { .. } => &[BindingRealizationFacet::EndpointDescriptor],
            Self::PrivateSocketPresentation { .. } => &[
                BindingRealizationFacet::EndpointDescriptor,
                BindingRealizationFacet::EndpointPathname,
            ],
        }
    }

    /// Whether this delivery names a destination at all.
    pub const fn has_destination(&self) -> bool {
        matches!(self, Self::PrivateSocketPresentation { .. })
    }
}

/// The closed delivery form one attachment kind declares.
///
/// A connect or listen reaches the endpoint through a verified descriptor
/// wherever the backend allows it; an attach addresses a display or stream by
/// name, so it additionally needs the private exact-socket presentation.
pub const fn declared_delivery_form(attachment: EndpointAttachmentKind) -> DeliveryForm {
    match attachment {
        EndpointAttachmentKind::Attach => DeliveryForm::PrivateSocketPresentation,
        EndpointAttachmentKind::Connect | EndpointAttachmentKind::Listen => {
            DeliveryForm::Descriptor
        }
    }
}

/// The closed set of delivery forms this provider declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeliveryForm {
    /// A verified descriptor for the exact endpoint.
    Descriptor,
    /// A private exact-socket presentation of the exact endpoint.
    PrivateSocketPresentation,
}

impl DeliveryForm {
    /// Whether this form is what `attachment` declares.
    pub const fn satisfies(self, attachment: EndpointAttachmentKind) -> bool {
        matches!(
            (self, attachment),
            (
                Self::Descriptor,
                EndpointAttachmentKind::Connect | EndpointAttachmentKind::Listen
            ) | (Self::PrivateSocketPresentation, EndpointAttachmentKind::Attach)
        )
    }
}

/// Why one delivery payload was refused.
///
/// Every variant is a refusal. There is no variant meaning "deliver it
/// anyway" and none meaning "a different socket is close enough", so a payload
/// that reaches past the admitted endpoint has no success path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryFenceViolation {
    /// The value names an absolute host path. A consumer is never given a
    /// host path: it receives the exact endpoint at its own destination, so
    /// an environment value naming another socket, a sibling socket, or a
    /// runtime directory is refused here rather than composed into a grant.
    AbsoluteHostPath,
    /// The value names the directory that contains the socket. The admitted
    /// relationship is the endpoint, not its container.
    ContainerDirectory,
    /// The value escapes its own destination through a relative component.
    PathEscape,
    /// The value names a destination this relationship does not own, or any
    /// destination at all for a descriptor delivery.
    ForeignDestination,
}

impl core::fmt::Display for DeliveryFenceViolation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::AbsoluteHostPath => {
                f.write_str("the delivery payload names an absolute host path")
            }
            Self::ContainerDirectory => f.write_str(
                "the delivery payload names the directory that contains the endpoint",
            ),
            Self::PathEscape => {
                f.write_str("the delivery payload escapes its own destination")
            }
            Self::ForeignDestination => {
                f.write_str("the delivery payload names a destination this relationship does not own")
            }
        }
    }
}

impl std::error::Error for DeliveryFenceViolation {}

/// Fence one caller-supplied delivery payload value against the exact
/// delivery it will ride on.
///
/// This is the last check on the way out, and it exists because a path
/// string is the one thing a consumer can construct by itself. An absolute
/// spelling would escape the launch's private tree and reach whatever the
/// producer or attacker placed at that path; a relative component would walk
/// out of the single destination this relationship owns; and for a descriptor
/// delivery there is no destination at all, so no pathname can be honoured.
/// The rejected value never appears in the returned reason (R42).
pub fn fence_delivery_payload(
    value: &str,
    delivery: &EndpointDelivery,
) -> Result<(), DeliveryFenceViolation> {
    if value.contains('\0') {
        return Err(DeliveryFenceViolation::ForeignDestination);
    }
    if value.starts_with('/') {
        return Err(DeliveryFenceViolation::AbsoluteHostPath);
    }
    if value.split('/').any(|component| component == "..") {
        return Err(DeliveryFenceViolation::PathEscape);
    }
    let EndpointDelivery::PrivateSocketPresentation { destination_slot } = delivery else {
        return Err(DeliveryFenceViolation::ForeignDestination);
    };
    if value.contains('/') {
        return Err(DeliveryFenceViolation::ForeignDestination);
    }
    if value == destination_slot.as_str() {
        Ok(())
    } else {
        Err(DeliveryFenceViolation::ForeignDestination)
    }
}

/// Fence a set of proposed endpoint locators against one exact delivery.
///
/// This is the rule an effect adapter applies to every value it is about to
/// hand the consumer as an endpoint locator but cannot positively identify as
/// the delivery: the set is the whole of the values it proposes, and each one
/// must be the single destination this relationship owns.
pub fn fence_delivery_payload_all<'a, I>(
    values: I,
    delivery: &EndpointDelivery,
) -> Result<(), DeliveryFenceViolation>
where
    I: IntoIterator<Item = &'a str>,
{
    for value in values {
        fence_delivery_payload(value, delivery)?;
    }
    Ok(())
}

/// Fence one `KEY=VALUE` environment entry against one exact delivery.
///
/// The split is STRUCTURAL - the launch payload's own `KEY=VALUE` shape, the
/// same shape the broker's launch preflight already validates - and nothing
/// here reads a variable's meaning or recognises a name. An undeclared
/// variable naming an alternate socket is therefore fenced exactly as a
/// familiar one is, which is what makes a name this crate has never heard of
/// no different from a known one (AE7).
pub fn fence_delivery_environment(
    entry: &str,
    delivery: &EndpointDelivery,
) -> Result<(), DeliveryFenceViolation> {
    let Some((_key, value)) = entry.split_once('=') else {
        return fence_delivery_payload(entry, delivery);
    };
    if value.is_empty() {
        return Err(DeliveryFenceViolation::ForeignDestination);
    }
    fence_delivery_payload(value, delivery)
}

/// Fence a whole environment block against one exact delivery.
pub fn fence_delivery_environment_all<'a, I>(
    entries: I,
    delivery: &EndpointDelivery,
) -> Result<(), DeliveryFenceViolation>
where
    I: IntoIterator<Item = &'a str>,
{
    for entry in entries {
        fence_delivery_environment(entry, delivery)?;
    }
    Ok(())
}

/// The child target-support ceiling one execution parent offers for endpoint
/// relationships.
///
/// A ceiling bounds what a child of that target may request and creates no
/// binding, no reservation, and no access (R16, AE31). The rights it carries
/// are derived from the endpoint's OWN declaration: an endpoint whose
/// consumer policy names `observe` offers the observing right as well as the
/// consuming one. The ceiling never widens past what the endpoint itself
/// declared, so a parent cannot turn a connect-only endpoint into a listener
/// by restating it as a support ceiling.
pub fn endpoint_binding_support_ceiling(
    spec: &EndpointSpec,
) -> Result<ChildSupportCeiling, EndpointBindingError> {
    let mut rights = vec![RequestedRights::Consume];
    if endpoint_grants_observe(spec) {
        rights.push(RequestedRights::Observe);
    }
    ChildSupportCeiling::new(vec![BindingSupportEntry::new(BindingKind::Endpoint, rights)?])
        .map_err(EndpointBindingError::from)
}

/// Whether one endpoint's own declaration offers the observing right.
///
/// An unconstrained operation list admits every operation the endpoint's
/// attachment vocabulary defines, so the observing right is offered; a
/// narrowed list offers it only when `observe` is one of the named
/// operations.
pub fn endpoint_grants_observe(spec: &EndpointSpec) -> bool {
    spec.consumer_policy()
        .admits_operation(EndpointOperation::Observe)
}

/// What one classified Host or Guest endpoint-attachment input meant.
///
/// Only a parent's own consumption produces a relationship. A support ceiling
/// is recorded as the target's admission constraint and a child default is
/// returned to shape that child's request, so neither creates host state
/// (AE31, AE33).
pub enum ParentInputOutcome {
    /// The target's child support ceiling was recorded; it admits this kind
    /// and right, or it does not.
    Ceiling {
        /// Whether the recorded ceiling admits an endpoint `consume`.
        admits: bool,
    },
    /// The default is handed back to shape one named child's request and
    /// grants the parent nothing.
    ChildDefault {
        /// The child the defaults apply to.
        child_ref: ResourceRef,
    },
    /// The parent's own consumption was admitted as a relationship.
    Relationship(Box<AdmittedEndpointBinding>),
}

impl core::fmt::Debug for ParentInputOutcome {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Ceiling { admits } => formatter
                .debug_struct("Ceiling")
                .field("admits", admits)
                .finish(),
            Self::ChildDefault { child_ref } => formatter
                .debug_struct("ChildDefault")
                .field("child_ref", child_ref)
                .finish(),
            Self::Relationship(binding) => formatter
                .debug_struct("Relationship")
                .field("binding", binding)
                .finish(),
        }
    }
}

/// Everything the source provider needs to admit one consumer's exact
/// endpoint relationship.
///
/// The fields are the root-admitted inputs the effect is held to: the
/// relationship's committed identities, the consumer's exact typed request,
/// the exact endpoint the owner resolved, the endpoint's own declared policy
/// and ceiling, the consumer's signed provider component, the authorization
/// evidence the graph produced, and the dependency revisions the admission is
/// fenced against. No field carries a host path, a socket name, or a
/// numerical host principal.
pub struct EndpointBindingAdmission {
    zone: ZoneId,
    source_uid: ResourceUid,
    consumer_uid: ResourceUid,
    request: EndpointBindingRequest,
    provenance: EndpointProvenance,
    spec: EndpointSpec,
    component: Option<BoundedToken>,
    target: EndpointConsumerTarget,
    authorization: BindingAuthorization,
    dependencies: Vec<FreshnessTuple>,
}

impl EndpointBindingAdmission {
    /// Assemble one admission from the root-admitted inputs.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        zone: ZoneId,
        source_uid: ResourceUid,
        consumer_uid: ResourceUid,
        request: EndpointBindingRequest,
        provenance: EndpointProvenance,
        spec: EndpointSpec,
        component: Option<BoundedToken>,
        target: EndpointConsumerTarget,
        authorization: BindingAuthorization,
        dependencies: Vec<FreshnessTuple>,
    ) -> Self {
        Self {
            zone,
            source_uid,
            consumer_uid,
            request,
            provenance,
            spec,
            component,
            target,
            authorization,
            dependencies,
        }
    }
}

impl core::fmt::Debug for EndpointBindingAdmission {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("EndpointBindingAdmission")
            .field("target", &self.target)
            .field("component", &self.component)
            .field("provenance", &self.provenance)
            .field("dependency_count", &self.dependencies.len())
            .finish_non_exhaustive()
    }
}

/// The execution target one consumer runs on.
///
/// A target is here for the child target-support ceiling and the ceiling
/// lookup only. It is never a host path and never a runtime directory: the
/// endpoint's own locator stays with the owner.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EndpointConsumerTarget(ResourceRef);

impl EndpointConsumerTarget {
    /// Name one typed execution target.
    pub fn new(target: ResourceRef) -> Result<Self, EndpointBindingError> {
        let kind = target.resource_type().as_str();
        if !matches!(kind, "Host" | "Guest" | "Process" | "EphemeralProcess") {
            return Err(EndpointBindingError::WrongResourceType);
        }
        Ok(Self(target))
    }

    /// Borrow the exact typed target.
    pub const fn as_ref(&self) -> &ResourceRef {
        &self.0
    }
}

impl core::fmt::Debug for EndpointConsumerTarget {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_tuple("EndpointConsumerTarget")
            .field(&self.0)
            .finish()
    }
}

/// The source-owned identity of one declared exact endpoint.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EndpointSourceKey {
    zone_uid: ZoneId,
    endpoint_uid: ResourceUid,
    endpoint_generation: ResourceGeneration,
}

impl EndpointSourceKey {
    fn of(provenance: &EndpointProvenance) -> Self {
        Self {
            zone_uid: provenance.zone_uid.clone(),
            endpoint_uid: provenance.endpoint_uid.clone(),
            endpoint_generation: provenance.endpoint_generation,
        }
    }
}

impl core::fmt::Debug for EndpointSourceKey {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("EndpointSourceKey")
            .field("endpoint_uid", &self.endpoint_uid)
            .field("endpoint_generation", &self.endpoint_generation)
            .finish()
    }
}

/// One admitted endpoint relationship: the exact request, the evidence that
/// admitted it, the pinned endpoint identity, and the delivery form it
/// declared.
#[derive(Clone, PartialEq, Eq)]
pub struct AdmittedEndpointBinding {
    key: BindingKey,
    provenance: EndpointProvenance,
    attachment: EndpointAttachmentKind,
    component: Option<BoundedToken>,
    evidence: BindingEvidence,
    socket: Option<EndpointSocketIdentity>,
    delivery: Option<EndpointDelivery>,
}

impl AdmittedEndpointBinding {
    /// Borrow the relationship's identity.
    pub const fn key(&self) -> &BindingKey {
        &self.key
    }

    /// Borrow the exact endpoint this relationship delivers.
    pub const fn provenance(&self) -> &EndpointProvenance {
        &self.provenance
    }

    /// Return the attachment kind the consumer requested.
    pub const fn attachment(&self) -> EndpointAttachmentKind {
        self.attachment
    }

    /// Borrow the consumer's signed provider component, when it named one.
    pub fn component(&self) -> Option<&BoundedToken> {
        self.component.as_ref()
    }

    /// The pinned identity of the exact endpoint, once preparation has
    /// established one.
    pub const fn socket(&self) -> Option<EndpointSocketIdentity> {
        self.socket
    }

    /// The delivery this prepared relationship hands to the launch.
    pub fn delivery(&self) -> Option<&EndpointDelivery> {
        self.delivery.as_ref()
    }

    /// The delivery form this relationship declared from its attachment kind.
    pub const fn declared_form(&self) -> DeliveryForm {
        declared_delivery_form(self.attachment)
    }

    /// Whether the admission this evidence was minted from is still fenced
    /// against the observed graph.
    pub fn is_current(&self, observed: &[FreshnessTuple]) -> bool {
        self.evidence.is_current(observed)
    }

    /// Borrow the admitted evidence.
    pub const fn evidence(&self) -> &BindingEvidence {
        &self.evidence
    }
}

impl core::fmt::Debug for AdmittedEndpointBinding {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AdmittedEndpointBinding")
            .field("key", &self.key)
            .field("attachment", &self.attachment)
            .field("component", &self.component)
            .field("socket", &self.socket)
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// Source row -> committed `EndpointBinding` rows
// ---------------------------------------------------------------------------

/// The arbitration every admitted endpoint relationship carries.
///
/// An endpoint delivers the same exact inode to every consumer its own
/// declaration names, so the source admits each of them alongside its peers
/// and never alone: one endpoint is not a resource a second consumer
/// displaces the first from. The endpoint's own attachment ceiling, not the
/// arbitration, is what bounds how many may hold it at once.
const ENDPOINT_BINDING_ARBITRATION: BindingArbitration = BindingArbitration::Shared;

/// The domain tag framing one derived `EndpointBinding` row name.
const BINDING_ROW_NAME_DOMAIN: &str = "d2b:v3:endpoint-binding-row";

/// Bytes of the framed digest one derived row name carries.
const BINDING_ROW_NAME_BYTES: usize = 12;

/// One delivery a committed `Endpoint` row declares for one consumer.
///
/// A delivery names the consumer, the consumer's own stable slot, and how
/// that consumer reaches the exact endpoint. The endpoint reference and the
/// bounded purpose are NOT its own: both come from the committed `Endpoint`
/// row it is declared against, so a delivery cannot reach a different
/// endpoint or invent a purpose the endpoint never published.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeclaredEndpointBinding {
    consumer: EndpointConsumerTarget,
    slot: BindingSlot,
    attachment: EndpointAttachmentKind,
}

impl DeclaredEndpointBinding {
    /// Declare one delivery of the exact endpoint to one consumer.
    pub const fn new(
        consumer: EndpointConsumerTarget,
        slot: BindingSlot,
        attachment: EndpointAttachmentKind,
    ) -> Self {
        Self {
            consumer,
            slot,
            attachment,
        }
    }

    /// Borrow the consumer this delivery is declared for.
    pub const fn consumer(&self) -> &EndpointConsumerTarget {
        &self.consumer
    }

    /// Borrow the consumer's own stable slot for the relationship.
    pub const fn slot(&self) -> &BindingSlot {
        &self.slot
    }

    /// Return how this consumer reaches the exact endpoint.
    pub const fn attachment(&self) -> EndpointAttachmentKind {
        self.attachment
    }
}

impl core::fmt::Debug for DeclaredEndpointBinding {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DeclaredEndpointBinding")
            .field("consumer", &self.consumer)
            .field("slot", &self.slot)
            .field("attachment", &self.attachment)
            .finish()
    }
}

/// One source-owned `EndpointBinding` row the source mints for one admitted
/// relationship.
///
/// The row's spec is the canonical `EndpointBindingSpec` bytes: the exact
/// endpoint, the consumer, the consumer's slot, the attachment kind, and the
/// source's own decision about the relationship. The request the row was
/// derived from travels beside those bytes, so the admission that mints the
/// relationship and the row a boundary reads back are two views of ONE
/// derivation rather than two descriptions that can drift apart.
#[derive(Clone, PartialEq, Eq)]
pub struct EndpointBindingRow {
    name: BoundedToken,
    request: EndpointBindingRequest,
    spec: Vec<u8>,
}

impl EndpointBindingRow {
    /// Borrow the deterministic row name.
    pub const fn name(&self) -> &BoundedToken {
        &self.name
    }

    /// Borrow the exact request this row was derived from.
    ///
    /// It is the request [`EndpointBindingRegistry::admit`] is evaluated
    /// against, so the consumer's declaration the source admitted and the row
    /// the graph reads back cannot name different consumers, slots, or
    /// attachment kinds.
    pub const fn request(&self) -> &EndpointBindingRequest {
        &self.request
    }

    /// Borrow the canonical desired bytes committed as the row's spec.
    pub fn spec(&self) -> &[u8] {
        &self.spec
    }
}

impl core::fmt::Debug for EndpointBindingRow {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("EndpointBindingRow")
            .field("name", &self.name)
            .field("request", &self.request)
            .finish_non_exhaustive()
    }
}

/// Derive the committed `EndpointBinding` rows one source row implies.
///
/// Every declared delivery becomes exactly one row, named from the
/// relationship's KTD3 slot address rather than from a declaration position,
/// so the same delivery keeps one identity across restarts and two deliveries
/// never collide by ordering. A source row that declares no delivery derives
/// NO row: there is no default relationship to commit, and an absent fact
/// stays absent.
///
/// # Errors
///
/// Returns [`EndpointBindingError::InvalidRequest`] when two deliveries claim
/// the one consumer slot: the slot address IS the relationship, so a second
/// claim on it is not a second relationship. Every other refusal is the one
/// [`canonical_binding_row`] names for the delivery itself.
pub fn canonical_binding_rows(
    zone: &ZoneId,
    spec: &EndpointSpec,
    endpoint_ref: &ResourceRef,
    deliveries: &[DeclaredEndpointBinding],
) -> Result<Vec<EndpointBindingRow>, EndpointBindingError> {
    let mut rows = Vec::with_capacity(deliveries.len());
    let mut minted: BTreeSet<BoundedToken> = BTreeSet::new();
    for delivery in deliveries {
        let row = canonical_binding_row(zone, spec, endpoint_ref, delivery)?;
        // The row name IS the relationship identity, so a second delivery
        // claiming it is the same relationship declared twice - refused here
        // rather than committed as two rows over one slot.
        if !minted.insert(row.name().clone()) {
            return Err(EndpointBindingError::InvalidRequest);
        }
        rows.push(row);
    }
    Ok(rows)
}

/// The committed `EndpointBinding` row one delivery implies.
///
/// The endpoint's own declaration decides, in the same order
/// [`EndpointBindingRegistry::admit`] consults it: the consumer must be one
/// this binding kind admits at all (a `Host` never is, because the host-side
/// delivery is an admitted realization leg of the binding whose consumer is
/// that helper), then the endpoint's subject allowlist, then the operation
/// the attachment kind performs, then the endpoint's own attachment capacity.
/// The delivery then rides on the facets its own kind requires - a connect or
/// a listen through the verified descriptor alone, an attach through the
/// descriptor and the private presentation - and the committed decision
/// records the right that kind requests, the shared arbitration, and exactly
/// those facets.
///
/// No host path, socket name, or numerical host principal is added: the
/// exact endpoint's locator stays with the owner that resolved it.
///
/// # Errors
///
/// Returns [`EndpointBindingError::WrongResourceType`] for a consumer that is
/// not a binding consumer at all,
/// [`EndpointBindingError::InvalidRequest`] for a consumer this kind does not
/// admit, [`EndpointBindingError::ConsumerNotAllowed`] for a consumer the
/// endpoint's own policy does not name,
/// [`EndpointBindingError::OperationNotAllowed`] for an operation that policy
/// does not admit, [`EndpointBindingError::AttachmentRefused`] for an `attach`
/// the endpoint declares no capacity for, and
/// [`EndpointBindingError::UnsupportedFacet`] for a facet this family does not
/// declare it can realize. The typed request and decision constructors keep
/// their own refusals, so a bound capability that is not an `Endpoint` and a
/// decision whose right or facets are not a valid set surface as
/// [`EndpointBindingError::Contract`].
pub fn canonical_binding_row(
    zone: &ZoneId,
    spec: &EndpointSpec,
    endpoint_ref: &ResourceRef,
    delivery: &DeclaredEndpointBinding,
) -> Result<EndpointBindingRow, EndpointBindingError> {
    let attachment = delivery.attachment();
    let consumer = delivery.consumer().as_ref();
    let consumer_kind =
        BindingConsumerKind::from_resource_type(consumer.resource_type().as_str())
            .ok_or(EndpointBindingError::WrongResourceType)?;
    // The kind's own admitted set is the rule rather than a per-family list,
    // so a `Process` or an `EphemeralProcess` helper is derivable and a
    // `Host` never is.
    if !BindingKind::Endpoint.admits_consumer(consumer_kind) {
        return Err(EndpointBindingError::InvalidRequest);
    }
    let policy = spec.consumer_policy();
    if !policy.admits_subject(consumer) {
        return Err(EndpointBindingError::ConsumerNotAllowed);
    }
    if !policy.admits_operation(EndpointConsumerPolicy::operation_for(attachment)) {
        return Err(EndpointBindingError::OperationNotAllowed);
    }
    if attachment == EndpointAttachmentKind::Attach
        && !spec.attachment_policy().admits_attachment(0)
    {
        return Err(EndpointBindingError::AttachmentRefused);
    }
    // The decision commits what the delivery rides on, so a facet the
    // family cannot realize is refused here rather than committed as a fact
    // no implementation here could honour.
    ensure_realizable(attachment.required_facets())?;
    let request = EndpointBindingRequest::new(
        endpoint_ref.clone(),
        consumer.clone(),
        delivery.slot().clone(),
        attachment,
        spec.purpose().clone(),
    )?;
    let decision = BindingSourceDecision::new(
        vec![request.requested_rights()],
        ENDPOINT_BINDING_ARBITRATION,
        attachment.required_facets().to_vec(),
    )?;
    // Both references are already settled above - the request refused a bound
    // capability that is not an `Endpoint`, and the kind refused a consumer
    // it does not admit - so the only failure this check can still report is
    // the wrong bound capability, and the refusal stays field-free.
    let row = EndpointBindingSpec::new(
        endpoint_ref.clone(),
        consumer.clone(),
        attachment,
        BoundedToken::parse(delivery.slot().as_str())?,
        decision,
    )
    .map_err(|_| EndpointBindingError::WrongResourceType)?;
    Ok(EndpointBindingRow {
        name: binding_row_name(zone, endpoint_ref, consumer, delivery.slot())?,
        request,
        // A typed specification over canonical references always renders; a
        // failure here is a programming error in this crate, never a
        // consumer's declaration, so it is refused rather than committed.
        spec: canonical_json_bytes(&row).map_err(|_| EndpointBindingError::InvalidRequest)?,
    })
}

/// Refuse a facet set this family does not declare it can realize.
///
/// The Endpoint family realizes a relationship through a verified descriptor
/// for the one admitted inode, and through a private presentation of that
/// same inode where a backend requires a name. Nothing here realizes access
/// to the directory that happens to contain the socket, so a decision naming
/// any other facet is refused rather than committed: a committed facet is
/// read back as something the source admitted through, and this crate can
/// carry only the two it declares.
///
/// # Errors
///
/// Returns [`EndpointBindingError::UnsupportedFacet`] when any named facet is
/// outside [`endpoint_binding_support`].
pub fn ensure_realizable(facets: &[BindingRealizationFacet]) -> Result<(), EndpointBindingError> {
    let support = endpoint_binding_support();
    if facets.iter().all(|facet| support.realizes(*facet)) {
        Ok(())
    } else {
        Err(EndpointBindingError::UnsupportedFacet)
    }
}

/// The deterministic row name one relationship mints.
///
/// The name derives from the KTD3 slot address - the bound endpoint, the
/// consumer, and the consumer's own stable slot - and never from a
/// declaration index or an attachment order, so reordering declarations
/// never churns row identities and two relationships never collide by
/// position. The attachment kind is deliberately not part of it: a consumer
/// that reaches the same endpoint differently in the same slot is the SAME
/// relationship, which is what makes such a change one relationship rather
/// than a second one.
///
/// # Errors
///
/// Returns [`BindingContractError::InvalidField`] when the derived row name is
/// not a bounded token.
pub fn binding_row_name(
    zone: &ZoneId,
    endpoint_ref: &ResourceRef,
    execution_ref: &ResourceRef,
    slot: &BindingSlot,
) -> Result<BoundedToken, BindingContractError> {
    let mut digest = framed_digest(BINDING_ROW_NAME_DOMAIN.as_bytes());
    for part in [
        zone.as_str().to_owned(),
        endpoint_ref.to_canonical_string(),
        execution_ref.to_canonical_string(),
        slot.as_str().to_owned(),
    ] {
        let next = framed_digest(part.as_bytes());
        for (byte, part_byte) in digest.iter_mut().zip(next) {
            *byte ^= part_byte;
        }
    }
    BoundedToken::parse(format!(
        "endpoint-binding-{}",
        digest_into_hex(digest, BINDING_ROW_NAME_BYTES)
    ))
    .map_err(|_| BindingContractError::InvalidField)
}

/// One relationship's readiness, with source preparation and consumer-side
/// completion reported separately.
///
/// The two sides stay separate so a relationship that must exist before its
/// consumer starts never forms a startup cycle with the observation that
/// consumer can see it (R39, R40).
#[derive(Clone, PartialEq, Eq)]
pub struct BindingReadiness {
    state: BindingLifecycleState,
    observation: BindingObservation,
    socket: Option<EndpointSocketIdentity>,
    attached: bool,
}

impl BindingReadiness {
    fn from_record(record: &BindingRecord) -> Self {
        Self {
            state: record.observation.state(),
            observation: record.observation,
            socket: record.binding.socket,
            attached: record.attached,
        }
    }

    /// Return the observed lifecycle.
    pub const fn state(&self) -> BindingLifecycleState {
        self.state
    }

    /// Borrow the two-sided observation.
    pub const fn observation(&self) -> &BindingObservation {
        &self.observation
    }

    /// The pinned identity of the exact endpoint this relationship prepared.
    pub const fn socket(&self) -> Option<EndpointSocketIdentity> {
        self.socket
    }

    /// Whether the consumer is still attached to the delivered endpoint.
    pub const fn attached(&self) -> bool {
        self.attached
    }

    /// Whether the exact endpoint's effective access still holds.
    ///
    /// A replaced inode and a nullified ACL mask both land here: the
    /// relationship stops reporting a usable delivery instead of continuing
    /// to claim the access it admitted.
    pub const fn proves_effect(&self) -> bool {
        self.state.proves_effect() && self.socket.is_some() && self.observation.prepare().is_complete()
    }
}

impl core::fmt::Debug for BindingReadiness {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BindingReadiness")
            .field("state", &self.state)
            .field("socket", &self.socket)
            .field("attached", &self.attached)
            .finish_non_exhaustive()
    }
}

/// What tearing one exact endpoint down did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointTeardown {
    /// The relationships that had already detached.
    detached_consumers: u16,
}

impl EndpointTeardown {
    /// How many relationships had already detached when the endpoint went.
    ///
    /// The count is what the teardown proved, not what it assumed: a step
    /// that reached here found every live relationship on this endpoint
    /// already detached, and the count is the evidence for that.
    pub const fn detached_consumers(&self) -> u16 {
        self.detached_consumers
    }
}

/// The Endpoint provider's `EndpointBinding` registry.
///
/// One entry per declared exact endpoint, keyed by `(Zone, Endpoint,
/// Endpoint generation)`. Relationships live under their endpoint and are
/// keyed by the consumer slot address, so two consumers on one compositor are
/// two relationships over one endpoint while one consumer cannot hold two in
/// the same slot.
#[derive(Debug, Default)]
pub struct EndpointBindingRegistry {
    zone: Option<ZoneId>,
    sources: BTreeMap<EndpointSourceKey, SourceRecord>,
    slots: BindingSlotIndex,
    ceilings: BTreeMap<EndpointConsumerTarget, ChildSupportCeiling>,
}

impl EndpointBindingRegistry {
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

    /// Declare one exact endpoint the owner resolved.
    ///
    /// The endpoint is registered with its own committed policy and
    /// attachment capacity, so every later admission is judged against the
    /// declaration rather than against a consumer's claim. Re-declaring the
    /// SAME endpoint generation with a different policy is refused: an
    /// endpoint's own rules cannot be widened underneath a live consumer.
    pub fn declare_endpoint(
        &mut self,
        provenance: EndpointProvenance,
        spec: EndpointSpec,
    ) -> Result<(), EndpointBindingError> {
        if self.zone.as_ref() != Some(provenance.zone_uid()) {
            return Err(EndpointBindingError::WrongResourceType);
        }
        let key = EndpointSourceKey::of(&provenance);
        if let Some(existing) = self.sources.get(&key) {
            if existing.spec != spec {
                return Err(EndpointBindingError::UnexpectedState);
            }
            return Ok(());
        }
        self.sources.insert(
            key,
            SourceRecord {
                provenance,
                spec,
                bindings: BTreeMap::new(),
                torn_down: false,
            },
        );
        Ok(())
    }

    /// Whether this registry has declared the exact endpoint.
    pub fn has_endpoint(&self, provenance: &EndpointProvenance) -> bool {
        self.sources.contains_key(&EndpointSourceKey::of(provenance))
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
    ) -> Result<BindingSlotDecision, EndpointBindingError> {
        if self.zone.as_ref() != Some(key.zone()) {
            return Err(EndpointBindingError::WrongResourceType);
        }
        self.slots
            .clone()
            .declare(key, fingerprint)
            .map_err(EndpointBindingError::from)
    }

    /// Record one execution target's child target-support ceiling.
    pub fn record_ceiling(
        &mut self,
        target: EndpointConsumerTarget,
        ceiling: ChildSupportCeiling,
    ) {
        self.ceilings.insert(target, ceiling);
    }

    /// Borrow the recorded child target-support ceiling for one target.
    pub fn child_ceiling(&self, target: &EndpointConsumerTarget) -> Option<&ChildSupportCeiling> {
        self.ceilings.get(target)
    }

    /// Apply one classified Host or Guest endpoint-attachment input.
    pub fn classify_parent_input(
        &mut self,
        target: EndpointConsumerTarget,
        input: &EndpointExecutionParentInput,
        admission: Option<EndpointBindingAdmission>,
    ) -> Result<ParentInputOutcome, EndpointBindingError> {
        use EndpointExecutionParentInput as Input;
        match input {
            Input::ChildSupportCeiling(ceiling) => {
                let admits = ceiling.admits(
                    BindingKind::Endpoint,
                    RequestedRights::Consume,
                );
                self.ceilings.insert(target, ceiling.clone());
                Ok(ParentInputOutcome::Ceiling { admits })
            }
            Input::ChildRequestDefaults(defaults) => Ok(ParentInputOutcome::ChildDefault {
                child_ref: defaults.child_ref().clone(),
            }),
            Input::ParentUse(request) => {
                let admission = admission.ok_or(EndpointBindingError::InvalidRequest)?;
                if &admission.request != request || admission.target != target {
                    return Err(EndpointBindingError::WrongResourceType);
                }
                self.admit(admission)
                    .map(|binding| ParentInputOutcome::Relationship(Box::new(binding)))
            }
        }
    }

    /// Admit one consumer's exact endpoint relationship.
    ///
    /// Every refusal happens before any host mutation: the target-support
    /// ceiling is consulted first (a ceiling admits, it never creates), then
    /// the endpoint's own declaration, then the dependency fence, and only
    /// then the shared contract admission is evaluated. The exact endpoint is
    /// never resolved from anything the consumer supplied, because nothing the
    /// consumer supplies names one.
    pub fn admit(
        &mut self,
        admission: EndpointBindingAdmission,
    ) -> Result<AdmittedEndpointBinding, EndpointBindingError> {
        if self.zone.as_ref() != Some(&admission.zone) {
            return Err(EndpointBindingError::WrongResourceType);
        }
        if admission.provenance.zone_uid() != &admission.zone {
            return Err(EndpointBindingError::WrongResourceType);
        }
        if admission.request.source_ref().resource_type().as_str() != ENDPOINT_RESOURCE_TYPE {
            return Err(EndpointBindingError::WrongResourceType);
        }
        // A target with no recorded ceiling bounds nothing, and a ceiling that
        // omits this kind bounds this kind not at all. Either way the child
        // request is refused: there is no "unbounded" reading of R16.
        let admitted_by_ceiling = self.ceilings.get(&admission.target).is_some_and(|ceiling| {
            ceiling.admits(BindingKind::Endpoint, admission.request.requested_rights())
        });
        if !admitted_by_ceiling {
            return Err(EndpointBindingError::TargetSupportMissing);
        }
        let source_key = EndpointSourceKey::of(&admission.provenance);
        let declared = self
            .sources
            .get(&source_key)
            .ok_or(EndpointBindingError::ForeignEndpointIdentity)?;
        if declared.provenance != admission.provenance || declared.spec != admission.spec {
            return Err(EndpointBindingError::ForeignEndpointIdentity);
        }
        let live_attachments = declared
            .bindings
            .values()
            .filter(|record| {
                record.binding.attachment() == EndpointAttachmentKind::Attach
                    && !record.observation.state().is_terminal()
            })
            .count();
        let spec = &admission.spec;
        let policy = spec.consumer_policy();
        if !policy.admits_subject(admission.request.consumer_ref()) {
            return Err(EndpointBindingError::ConsumerNotAllowed);
        }
        if let Some(component) = &admission.component
            && !policy.admits_provider_component(component)
        {
            return Err(EndpointBindingError::ComponentNotAllowed);
        }
        if !policy.admits_operation(EndpointConsumerPolicy::operation_for(
            admission.request.attachment(),
        )) {
            return Err(EndpointBindingError::OperationNotAllowed);
        }
        if admission.request.attachment() == EndpointAttachmentKind::Attach
            && !spec
                .attachment_policy()
                .admits_attachment(u16::try_from(live_attachments).unwrap_or(u16::MAX))
        {
            return Err(EndpointBindingError::AttachmentRefused);
        }
        let key = admission.request.key(
            admission.zone.clone(),
            admission.source_uid.clone(),
            admission.consumer_uid.clone(),
        )?;
        let source = admit_source_endpoint(&key, &admission)?;
        let support = endpoint_binding_support();
        ensure_realizable(admission.request.required_facets())?;
        if !fence_names_both_parties(
            &admission.dependencies,
            &admission.source_uid,
            &admission.consumer_uid,
        ) {
            return Err(EndpointBindingError::StaleAuthority);
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
            .map_err(EndpointBindingError::from)?;
        let evidence = BindingEvidence::admitted(binding, reservation_for(&key)?);
        let admitted = AdmittedEndpointBinding {
            key: key.clone(),
            provenance: admission.provenance.clone(),
            attachment: admission.request.attachment(),
            component: admission.component.clone(),
            evidence,
            socket: None,
            delivery: None,
        };
        self.sources
            .get_mut(&source_key)
            .ok_or(EndpointBindingError::ForeignEndpointIdentity)?
            .bindings
            .insert(
                key.address(),
                BindingRecord {
                    binding: admitted.clone(),
                    observation: BindingObservation::new(
                        BindingLifecycleState::Admitted,
                        CompletionCondition::Pending,
                        CompletionCondition::Pending,
                        ReleaseOutcome::Outstanding,
                    ),
                    attached: false,
                },
            );
        self.slots
            .observe(&key, BindingLifecycleState::Admitted)
            .map_err(EndpointBindingError::from)?;
        Ok(admitted)
    }

    /// Borrow one admitted relationship, when this registry holds it.
    pub fn binding(&self, key: &BindingKey) -> Option<&AdmittedEndpointBinding> {
        self.sources
            .values()
            .find_map(|source| source.bindings.get(&key.address()))
            .map(|record| &record.binding)
    }

    /// Every relationship on one exact endpoint, in consumer-slot order.
    pub fn bindings(&self, provenance: &EndpointProvenance) -> Vec<&AdmittedEndpointBinding> {
        self.sources
            .get(&EndpointSourceKey::of(provenance))
            .map(|source| {
                source
                    .bindings
                    .values()
                    .map(|record| &record.binding)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// How many relationships still hold the exact endpoint.
    pub fn live_bindings(&self, provenance: &EndpointProvenance) -> usize {
        self.sources
            .get(&EndpointSourceKey::of(provenance))
            .map(|source| {
                source
                    .bindings
                    .values()
                    .filter(|record| !record.observation.state().is_terminal())
                    .count()
            })
            .unwrap_or(0)
    }

    /// Prepare one relationship's delivery against the exact endpoint.
    ///
    /// This is where the endpoint owner checks EFFECTIVE access rather than
    /// ACL presence, and where the inode is pinned. A relationship whose
    /// observation describes a different endpoint, whose ancestors no longer
    /// apply a traverse bit, or whose exact socket no longer grants the
    /// admitted right is refused before any delivery is constructed, so a
    /// prepared relationship always names an inode the consumer can actually
    /// reach and no other.
    pub fn prepare(
        &mut self,
        key: &BindingKey,
        observation: &EndpointAccessObservation,
        delivery: EndpointDelivery,
    ) -> Result<BindingReadiness, EndpointBindingError> {
        let (source_key, address) = self.locate(key)?;
        let record = self
            .sources
            .get(&source_key)
            .and_then(|source| source.bindings.get(&address))
            .ok_or(EndpointBindingError::UnexpectedState)?;
        if record.binding.declared_form() != delivery_form(&delivery) {
            return Err(EndpointBindingError::UnsupportedFacet);
        }
        if record.observation.state() != BindingLifecycleState::Admitted {
            return Err(EndpointBindingError::UnexpectedState);
        }
        // Preparation is where the pinned identity is ESTABLISHED, so there
        // is nothing here to compare it against yet: the endpoint the
        // observation describes was already matched against the declared
        // endpoint and its generation at admission, and a prepared
        // relationship that later finds a different inode is invalidated by
        // [`EndpointBindingRegistry::observe`], not re-checked here.
        let rights = record
            .binding
            .evidence
            .admission()
            .rights();
        if !observation.grants(required_right_bits(rights)) || !observation.traversable() {
            return Err(EndpointBindingError::EffectiveAccessMissing);
        }
        if record.binding.attachment() == EndpointAttachmentKind::Attach
            && !observation.accepting()
        {
            return Err(EndpointBindingError::EndpointNotAccepting);
        }
        if observation.parent_listable() {
            // Listing the containing directory is a capability the graph never
            // admitted. It is refused here rather than relied on the host
            // posture for, because "the directory happens to be traversable"
            // is exactly the authority R23 removed.
            return Err(EndpointBindingError::EffectiveAccessMissing);
        }
        // A descriptor delivery emits no pathname, so there is nothing to
        // fence on it here; the fence is applied by the effect adapter to the
        // one value it is about to hand the consumer as a locator.
        if let Some(slot) = destination_slot(&delivery) {
            fence_delivery_payload(slot, &delivery)?;
        }
        let socket = observation.socket();
        let source = self
            .sources
            .get_mut(&source_key)
            .ok_or(EndpointBindingError::UnexpectedState)?;
        let record = source
            .bindings
            .get_mut(&address)
            .ok_or(EndpointBindingError::UnexpectedState)?;
        record.binding.socket = Some(socket);
        record.binding.delivery = Some(delivery);
        let next = BindingObservation::new(
            BindingLifecycleState::Prepared,
            CompletionCondition::Complete,
            record.observation.consumer_completion(),
            record.observation.release(),
        );
        record.observation = next;
        record.attached = true;
        self.slots
            .observe(key, BindingLifecycleState::Prepared)
            .map_err(EndpointBindingError::from)?;
        Ok(BindingReadiness::from_record(record))
    }

    /// Re-check one prepared relationship against the current host state.
    ///
    /// A replaced inode invalidates the prepared delivery: the relationship
    /// stops reporting an effect and the caller has to prepare the NEW exact
    /// endpoint. Cached readiness cannot survive an inode swap (R41), and the
    /// same check catches an ACL mask a later `chmod` nullified (AE19).
    pub fn observe(
        &mut self,
        key: &BindingKey,
        observation: &EndpointAccessObservation,
    ) -> Result<BindingReadiness, EndpointBindingError> {
        let (source_key, address) = self.locate(key)?;
        let source = self
            .sources
            .get_mut(&source_key)
            .ok_or(EndpointBindingError::UnexpectedState)?;
        let record = source
            .bindings
            .get_mut(&address)
            .ok_or(EndpointBindingError::UnexpectedState)?;
        let Some(pinned) = record.binding.socket else {
            return Err(EndpointBindingError::UnexpectedState);
        };
        if observation.socket() != pinned {
            self.invalidate(source_key, address, RefusalReason::StaleAuthority);
            return Err(EndpointBindingError::EndpointIdentityReplaced);
        }
        let rights = record.binding.evidence.admission().rights();
        if !observation.grants(required_right_bits(rights)) || !observation.traversable() {
            self.invalidate(source_key, address, RefusalReason::UnprovenEffect);
            return Err(EndpointBindingError::EffectiveAccessMissing);
        }
        Ok(BindingReadiness::from_record(record))
    }

    /// Drop one relationship's prepared delivery and record that its effect
    /// can no longer be proven.
    ///
    /// This is the state a replaced inode and a nullified ACL mask both land
    /// in. The pinned identity and the delivery go with it, so nothing can
    /// keep handing a consumer the endpoint that was there before, and the
    /// relationship reports `Degraded` until the NEW exact endpoint is
    /// prepared against a fresh admission.
    fn invalidate(
        &mut self,
        source_key: EndpointSourceKey,
        address: BindingSlotAddress,
        reason: RefusalReason,
    ) {
        let Some(source) = self.sources.get_mut(&source_key) else {
            return;
        };
        let Some(record) = source.bindings.get_mut(&address) else {
            return;
        };
        record.binding.socket = None;
        record.binding.delivery = None;
        record.attached = false;
        record.observation = BindingObservation::new(
            BindingLifecycleState::Degraded,
            CompletionCondition::Failed(reason),
            record.observation.consumer_completion(),
            ReleaseOutcome::Outstanding,
        );
    }

    /// Record consumer-side completion for one prepared relationship.
    pub fn complete(&mut self, key: &BindingKey) -> Result<BindingReadiness, EndpointBindingError> {
        self.advance(
            key,
            BindingLifecycleState::Prepared,
            |record| {
                BindingObservation::new(
                    BindingLifecycleState::Active,
                    record.observation.prepare(),
                    CompletionCondition::Complete,
                    record.observation.release(),
                )
            },
        )
    }

    /// Block new use for one relationship ahead of its typed release.
    pub fn revoke(&mut self, key: &BindingKey) -> Result<BindingReadiness, EndpointBindingError> {
        self.advance(
            key,
            BindingLifecycleState::Active,
            |record| {
                BindingObservation::new(
                    BindingLifecycleState::Revoking,
                    record.observation.prepare(),
                    record.observation.consumer_completion(),
                    ReleaseOutcome::Outstanding,
                )
            },
        )
    }

    /// Drive one relationship's outstanding use to the safe state.
    pub fn drain(&mut self, key: &BindingKey) -> Result<BindingReadiness, EndpointBindingError> {
        self.advance(
            key,
            BindingLifecycleState::Revoking,
            |record| {
                BindingObservation::new(
                    BindingLifecycleState::Draining,
                    record.observation.prepare(),
                    CompletionCondition::Pending,
                    ReleaseOutcome::Draining,
                )
            },
        )
    }

    /// Record that the consumer has detached from the delivered endpoint.
    ///
    /// Detachment is what releases the endpoint for teardown, and it is
    /// separate from the relationship's own release so the endpoint owner can
    /// still see WHICH relationship released the endpoint while the
    /// relationship itself is finishing its typed drain.
    pub fn detach(&mut self, key: &BindingKey) -> Result<BindingReadiness, EndpointBindingError> {
        let (source_key, address) = self.locate(key)?;
        let source = self
            .sources
            .get_mut(&source_key)
            .ok_or(EndpointBindingError::UnexpectedState)?;
        let record = source
            .bindings
            .get_mut(&address)
            .ok_or(EndpointBindingError::UnexpectedState)?;
        if record.observation.state() != BindingLifecycleState::Draining {
            return Err(EndpointBindingError::UnexpectedState);
        }
        record.attached = false;
        // The delivery is gone with the consumer: an endpoint whose consumer
        // detached must not keep handing out the inode it was realized for.
        record.binding.delivery = None;
        record.binding.socket = None;
        let next = BindingObservation::new(
            BindingLifecycleState::Draining,
            record.observation.prepare(),
            CompletionCondition::Failed(RefusalReason::UnprovenEffect),
            ReleaseOutcome::Released,
        );
        record.observation = next;
        Ok(BindingReadiness::from_record(record))
    }

    /// Release one relationship after its consumer detached.
    pub fn release(&mut self, key: &BindingKey) -> Result<BindingReadiness, EndpointBindingError> {
        let (source_key, address) = self.locate(key)?;
        let source = self
            .sources
            .get_mut(&source_key)
            .ok_or(EndpointBindingError::UnexpectedState)?;
        let record = source
            .bindings
            .get_mut(&address)
            .ok_or(EndpointBindingError::UnexpectedState)?;
        if record.observation.state() != BindingLifecycleState::Draining {
            return Err(EndpointBindingError::UnexpectedState);
        }
        if record.attached {
            return Err(EndpointBindingError::ConsumerStillAttached);
        }
        let next = BindingObservation::new(
            BindingLifecycleState::Released,
            record.observation.prepare(),
            record.observation.consumer_completion(),
            ReleaseOutcome::Released,
        );
        record.observation = next;
        self.slots
            .observe(key, BindingLifecycleState::Released)
            .map_err(EndpointBindingError::from)?;
        Ok(BindingReadiness::from_record(record))
    }

    /// Tear one exact endpoint down, once every consumer has detached.
    ///
    /// This is the endpoint-first leg of the preserved teardown order: the
    /// endpoint goes BEFORE the producer or helper that owns it, and a
    /// still-attached consumer refuses the step rather than losing its
    /// delivery under it (R36, R38).
    pub fn teardown_endpoint(
        &mut self,
        provenance: &EndpointProvenance,
    ) -> Result<EndpointTeardown, EndpointBindingError> {
        let source_key = EndpointSourceKey::of(provenance);
        let source = self
            .sources
            .get_mut(&source_key)
            .ok_or(EndpointBindingError::ForeignEndpointIdentity)?;
        if source.provenance != *provenance {
            return Err(EndpointBindingError::ForeignEndpointIdentity);
        }
        if source.torn_down {
            return Ok(EndpointTeardown { detached_consumers: 0 });
        }
        let mut detached = 0_u16;
        for record in source.bindings.values() {
            if record.attached {
                return Err(EndpointBindingError::ConsumerStillAttached);
            }
            if !record.observation.state().is_terminal() {
                return Err(EndpointBindingError::UnexpectedState);
            }
            detached = detached.saturating_add(1);
        }
        source.torn_down = true;
        for record in source.bindings.values_mut() {
            record.binding.socket = None;
            record.binding.delivery = None;
        }
        Ok(EndpointTeardown {
            detached_consumers: detached,
        })
    }

    /// Whether one exact endpoint has already been torn down.
    pub fn endpoint_torn_down(&self, provenance: &EndpointProvenance) -> bool {
        self.sources
            .get(&EndpointSourceKey::of(provenance))
            .is_some_and(|source| source.torn_down)
    }

    /// Retire the producer or helper that owned one exact endpoint.
    ///
    /// The endpoint must already be torn down and every relationship must be
    /// terminal, so a worker Process is never removed while a consumer still
    /// holds - or is still finishing - the endpoint it serves. This is the
    /// single responsible owner for that ordering (R38).
    pub fn retire_producer(
        &mut self,
        provenance: &EndpointProvenance,
    ) -> Result<(), EndpointBindingError> {
        let source_key = EndpointSourceKey::of(provenance);
        let source = self
            .sources
            .get(&source_key)
            .ok_or(EndpointBindingError::ForeignEndpointIdentity)?;
        if !source.torn_down {
            return Err(EndpointBindingError::EndpointNotTornDown);
        }
        if source
            .bindings
            .values()
            .any(|record| !record.observation.state().is_terminal() || record.attached)
        {
            return Err(EndpointBindingError::ConsumerStillAttached);
        }
        self.sources.remove(&source_key);
        Ok(())
    }

    /// Report one relationship's readiness with its two sides separate.
    pub fn readiness(&self, key: &BindingKey) -> Result<BindingReadiness, EndpointBindingError> {
        let (source_key, address) = self.locate(key)?;
        let record = self
            .sources
            .get(&source_key)
            .and_then(|source| source.bindings.get(&address))
            .ok_or(EndpointBindingError::UnexpectedState)?;
        Ok(BindingReadiness::from_record(record))
    }

    /// Recover one relationship's observed state from committed evidence.
    ///
    /// Cached readiness cannot remint access: an admission whose dependency
    /// revisions no longer match is refused rather than reported as granted
    /// use, and a relationship that never reached a provable effect reports
    /// degraded rather than active (R41).
    pub fn recover(
        &mut self,
        key: &BindingKey,
        observed: &[FreshnessTuple],
    ) -> Result<BindingReadiness, EndpointBindingError> {
        let current = self.readiness(key)?;
        let live = self
            .binding(key)
            .ok_or(EndpointBindingError::UnexpectedState)?;
        if !live.is_current(observed) {
            return Err(EndpointBindingError::StaleAuthority);
        }
        let state = if current.proves_effect() {
            current.state
        } else {
            BindingLifecycleState::Degraded
        };
        self.advance(key, current.state(), move |record| {
            BindingObservation::new(
                state,
                record.observation.prepare(),
                record.observation.consumer_completion(),
                record.observation.release(),
            )
        })
    }

    fn locate(&self, key: &BindingKey) -> Result<(EndpointSourceKey, BindingSlotAddress), EndpointBindingError> {
        self.sources
            .iter()
            .find(|(_, source)| source.bindings.contains_key(&key.address()))
            .map(|(source_key, _)| (source_key.clone(), key.address()))
            .ok_or(EndpointBindingError::UnexpectedState)
    }

    fn advance(
        &mut self,
        key: &BindingKey,
        required: BindingLifecycleState,
        next: impl FnOnce(&BindingRecord) -> BindingObservation,
    ) -> Result<BindingReadiness, EndpointBindingError> {
        let (source_key, address) = self.locate(key)?;
        let source = self
            .sources
            .get_mut(&source_key)
            .ok_or(EndpointBindingError::UnexpectedState)?;
        let record = source
            .bindings
            .get_mut(&address)
            .ok_or(EndpointBindingError::UnexpectedState)?;
        if record.observation.state() != required {
            return Err(EndpointBindingError::UnexpectedState);
        }
        record.observation = next(record);
        let advanced = record.observation;
        self.slots
            .observe(key, advanced.state())
            .map_err(EndpointBindingError::from)?;
        Ok(BindingReadiness::from_record(record))
    }
}

/// One live relationship and the observation recorded for it.
#[derive(Clone, PartialEq, Eq)]
struct BindingRecord {
    binding: AdmittedEndpointBinding,
    observation: BindingObservation,
    attached: bool,
}

/// One declared exact endpoint and the relationships that use it.
#[derive(Clone, PartialEq, Eq)]
struct SourceRecord {
    provenance: EndpointProvenance,
    spec: EndpointSpec,
    bindings: BTreeMap<BindingSlotAddress, BindingRecord>,
    torn_down: bool,
}

impl core::fmt::Debug for SourceRecord {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("SourceRecord")
            .field("provenance", &self.provenance)
            .field("binding_count", &self.bindings.len())
            .field("torn_down", &self.torn_down)
            .finish_non_exhaustive()
    }
}

/// The delivery form one concrete delivery is.
fn delivery_form(delivery: &EndpointDelivery) -> DeliveryForm {
    match delivery {
        EndpointDelivery::Descriptor { .. } => DeliveryForm::Descriptor,
        EndpointDelivery::PrivateSocketPresentation { .. } => DeliveryForm::PrivateSocketPresentation,
    }
}

/// The single destination component one delivery owns, when it owns one.
///
/// A descriptor delivery owns no pathname, which is exactly why it cannot be
/// widened into one.
fn destination_slot(delivery: &EndpointDelivery) -> Option<&str> {
    match delivery {
        EndpointDelivery::Descriptor { .. } => None,
        EndpointDelivery::PrivateSocketPresentation { destination_slot } => {
            Some(destination_slot.as_str())
        }
    }
}

/// The Endpoint provider's own decision on one exact request.
///
/// The endpoint's own declaration decides: the consumer must be on the
/// endpoint's subject allowlist, the provider component on its component
/// allowlist, the attachment kind's operation on its operation allowlist, and
/// an attach needs attachment capacity. An empty allowlist is the
/// deliberately unconstrained policy, so it admits rather than denies; what it
/// never does is admit a consumer the graph did not authorize, which
/// `admit_binding_request` checks separately.
fn admit_source_endpoint(
    key: &BindingKey,
    admission: &EndpointBindingAdmission,
) -> Result<SourceAdmission, EndpointBindingError> {
    let request = &admission.request;
    let spec = &admission.spec;
    let policy = spec.consumer_policy();
    if request.consumer_ref() != key.consumer_ref()
        || request.source_ref() != key.source_ref()
        || request.slot() != key.slot()
    {
        return Err(EndpointBindingError::WrongResourceType);
    }
    if request.source_ref() != &admission.provenance.endpoint_ref {
        return Err(EndpointBindingError::ForeignEndpointIdentity);
    }
    if !policy.admits_subject(request.consumer_ref()) {
        return Err(EndpointBindingError::ConsumerNotAllowed);
    }
    if let Some(component) = &admission.component
        && !policy.admits_provider_component(component)
    {
        return Err(EndpointBindingError::ComponentNotAllowed);
    }
    if !policy.admits_operation(EndpointConsumerPolicy::operation_for(request.attachment())) {
        return Err(EndpointBindingError::OperationNotAllowed);
    }
    if request.attachment() == EndpointAttachmentKind::Attach
        && !spec.attachment_policy().supported
    {
        return Err(EndpointBindingError::AttachmentRefused);
    }
    if !spec
        .attachment_policy()
        .admits_attachment(0)
        && request.attachment() == EndpointAttachmentKind::Attach
    {
        return Err(EndpointBindingError::AttachmentRefused);
    }
    SourceAdmission::new(
        key.clone(),
        vec![request.requested_rights()],
        BindingArbitration::Shared,
    )
    .map_err(EndpointBindingError::from)
}

/// Whether the dependency fence names both the endpoint row and the consumer
/// row.
///
/// A fence that omits either side would keep an admission alive across a
/// change to the very row that carried it, which is exactly the
/// cached-readiness failure R41 forbids.
fn fence_names_both_parties(
    dependencies: &[FreshnessTuple],
    source_uid: &ResourceUid,
    consumer_uid: &ResourceUid,
) -> bool {
    dependencies
        .iter()
        .any(|dependency| dependency.resource_uid() == source_uid)
        && dependencies
            .iter()
            .any(|dependency| dependency.resource_uid() == consumer_uid)
}

/// The source-owned reservation identity one admitted relationship holds.
///
/// The token is a deterministic function of the zone, the source identity, the
/// consumer identity, and the consumer slot, framed so two concatenations
/// cannot collide. It is IDENTITY EVIDENCE and not a capability: nothing in
/// this crate or the broker treats it as an access path, and the opaque handle
/// that actually realizes the delivery stays private to its reservation owner.
fn reservation_for(key: &BindingKey) -> Result<SourceReservation, EndpointBindingError> {
    let slot_digest = digest_into_hex(
        framed_digest(key.slot().as_str().as_bytes()),
        RESERVATION_SLOT_BYTES,
    );
    let identity_digest = digest_into_hex(
        framed_digest(key.consumer_uid().as_str().as_bytes()),
        RESERVATION_IDENTITY_BYTES,
    );
    // The bounded-token grammar starts with a letter and admits lowercase
    // hex, so the fixed prefix keeps the rendering in contract however the
    // digests come out.
    let reservation_id = BoundedToken::parse(format!("e{slot_digest}-{identity_digest}-{}", digest_into_hex(framed_digest(key.zone().as_str().as_bytes()), RESERVATION_ZONE_BYTES)))
        .map_err(EndpointBindingError::from)?;
    Ok(SourceReservation::new(
        key.zone().clone(),
        key.source_uid().clone(),
        reservation_id,
    ))
}

/// The bounded-token grammar admits at most 63 bytes, and the three framed
/// parts below are sized to leave room for the prefix and separators.
const RESERVATION_ZONE_BYTES: usize = 4;

/// The framed 128-bit digest of one part, without an external dependency.
///
/// A 128-bit FNV-1a over a length-prefixed input keeps the reservation token a
/// deterministic function of the committed identities without pulling a hash
/// crate into this provider package. The token is identity EVIDENCE, not a
/// capability: it is never an access path, so a non-cryptographic digest
/// cannot be mistaken for one.
fn framed_digest(input: &[u8]) -> [u8; 16] {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET ^ (input.len() as u64);
    for byte in input.iter().copied() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    let mixed = hash ^ (hash >> 33).wrapping_mul(0xff51_afd7_ed55_8ccd);
    let mut out = [0_u8; 16];
    out[..8].copy_from_slice(&mixed.to_be_bytes());
    out[8..].copy_from_slice(&(mixed.rotate_left(32)).to_be_bytes());
    out
}

/// Render the first `bytes` of a digest as lowercase hex.
fn digest_into_hex(digest: [u8; 16], bytes: usize) -> String {
    let mut out = String::with_capacity(bytes * 2);
    for byte in &digest[..bytes.min(digest.len())] {
        out.push(HEX[usize::from(byte >> 4)]);
        out.push(HEX[usize::from(byte & 0x0f)]);
    }
    out
}

/// The lowercase hex alphabet, so the token is built without a formatter.
const HEX: [char; 16] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f',
];
