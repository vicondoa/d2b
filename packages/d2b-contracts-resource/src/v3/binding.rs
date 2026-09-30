//! Shared machinery for the five primitive binding relationships.
//!
//! KTD14 keeps the five kinds typed while sharing their lifecycle and
//! evidence structures, so this module owns what genuinely is common: the
//! closed [`BindingKind`] and [`BindingConsumerKind`] vocabularies, the
//! stable consumer [`BindingSlot`], the requested-rights vocabulary, the
//! observed [`BindingLifecycleState`], the KTD3 [`BindingKey`], and the
//! admitted [`BindingEvidence`] every kind produces. Each kind's own desired
//! schema lives in its own module and is closed over that shared core.
//!
//! # A desired request is not admitted use
//!
//! The consumer's desired request is the canonical place a relationship is
//! authored; the source provider admits it and owns the resulting binding.
//! Nothing here converts a request into authority by itself. Admitted
//! evidence is minted by [`admit_binding_request`], which needs the
//! authorization grant, the source's own decision, the selected realization's
//! declared support, and the freshness evidence every dependency is fenced
//! against. [`BindingAdmission`] and [`BindingEvidence`] have no public
//! constructor reachable from desired fields alone.
//!
//! # What a desired schema may not carry
//!
//! No binding request names a raw host source path, a numerical host
//! principal, secret material, or a free-form command line. The source side
//! is always an exact typed reference plus its own named view or function, the
//! consumer side is an exact typed reference plus a bounded destination, and
//! every wire mirror denies unknown fields so a retired shape cannot arrive as
//! an unparsed extra.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    ResourceRef,
    authority::{AdmissionStage, FreshnessTuple, RefusalReason},
    execution_policy::{
        BoundedToken, PrimitiveSpecError, parsed_deserialize, redacted_debug, string_schema,
    },
    identity::{ResourceUid, ZoneId},
    resource_schema::{canonical_json_bytes, framed_canonical_digest, is_canonical_digest},
    volume_binding::VOLUME_BINDING_RESOURCE_TYPE,
};
use d2b_contracts::wire_deserialize;

/// Maximum bytes in one stable consumer slot token.
pub const MAX_BINDING_SLOT_BYTES: usize = 63;
/// Maximum admitted dependencies one binding is fenced against.
pub const MAX_BINDING_DEPENDENCIES: usize = 16;
/// Maximum entries in one child target-support ceiling.
pub const MAX_BINDING_SUPPORT_ENTRIES: usize = 16;
/// Highest consumer device slot a block presentation may name.
pub const MAX_CONSUMER_DEVICE_SLOT: u16 = 255;

/// One `binding` contract rejection.
///
/// Every variant is field-free, so a rejection never echoes a path, a resource
/// identity, or caller-supplied text. The relationship and the enforcing stage
/// travel beside the rejection, never inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingContractError {
    /// A reference named the wrong ResourceType for its role.
    WrongResourceType,
    /// The consumer kind is not one this binding kind admits.
    UnsupportedConsumerKind,
    /// The requested right is not one this binding kind admits.
    UnsupportedRight,
    /// A bounded token or path field was empty, over bound, or malformed.
    InvalidField,
    /// A numeric field was outside its frozen bound.
    OutOfRange,
    /// A collection was empty, over bound, or carried a duplicate.
    InvalidCollection,
    /// A conditionally required field was absent.
    MissingRequiredField,
    /// The observed lifecycle is not the one the caller assumed.
    UnexpectedState,
    /// A different declaration already occupies the live consumer slot.
    SlotOccupied,
    /// The caller named a different source than the live slot occupant.
    SourceMismatch,
    /// A parent's default was applied to a different consumer.
    WrongConsumer,
}

impl core::fmt::Display for BindingContractError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::WrongResourceType => f.write_str("reference names the wrong ResourceType"),
            Self::UnsupportedConsumerKind => f.write_str("consumer kind is not admitted for this binding"),
            Self::UnsupportedRight => f.write_str("requested right is not admitted for this binding"),
            Self::InvalidField => f.write_str("invalid bounded field"),
            Self::OutOfRange => f.write_str("value is outside its frozen bound"),
            Self::InvalidCollection => f.write_str("collection is empty, over bound, or duplicated"),
            Self::MissingRequiredField => f.write_str("conditionally required field is absent"),
            Self::UnexpectedState => f.write_str("observed lifecycle is not the assumed one"),
            Self::SlotOccupied => f.write_str("a different declaration occupies the live slot"),
            Self::SourceMismatch => f.write_str("request names a different source than the slot occupant"),
            Self::WrongConsumer => f.write_str("default applies to a different consumer"),
        }
    }
}

impl std::error::Error for BindingContractError {}

impl From<PrimitiveSpecError> for BindingContractError {
    fn from(error: PrimitiveSpecError) -> Self {
        match error {
            PrimitiveSpecError::WrongResourceType => Self::WrongResourceType,
            PrimitiveSpecError::TooManyEntries | PrimitiveSpecError::DuplicateEntry => {
                Self::InvalidCollection
            }
            PrimitiveSpecError::OutOfRange => Self::OutOfRange,
            PrimitiveSpecError::MissingRequiredField => Self::MissingRequiredField,
            PrimitiveSpecError::ConflictingFields => Self::InvalidField,
            PrimitiveSpecError::CanonicalJson(_) | PrimitiveSpecError::InvalidMode => Self::InvalidField,
            _ => Self::InvalidField,
        }
    }
}

/// One admission refusal, carrying the stage that enforced it.
///
/// The stage and reason vocabularies are U1's, restated nowhere: this is the
/// shape a refusal travels in, not a second reason list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingRefusal {
    /// The stage that refused.
    stage: AdmissionStage,
    /// The typed reason it refused.
    reason: RefusalReason,
}

impl BindingRefusal {
    /// Construct a refusal.
    pub const fn new(stage: AdmissionStage, reason: RefusalReason) -> Self {
        Self { stage, reason }
    }

    /// Borrow the enforcing stage.
    pub const fn stage(&self) -> AdmissionStage {
        self.stage
    }

    /// Borrow the refusal reason.
    pub const fn reason(&self) -> RefusalReason {
        self.reason
    }
}

impl core::fmt::Display for BindingRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?} refused at {:?}", self.reason, self.stage)
    }
}

impl std::error::Error for BindingRefusal {}

/// The primitive relationship families.
///
/// Each family keeps its own desired schema and its own admission and
/// revocation semantics; this enum is what keeps them from collapsing into one
/// untyped permission bag.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum BindingKind {
    /// Volume or named-view use by a consumer.
    Volume,
    /// Shared or exclusive use of a device capability.
    Device,
    /// Network membership and traffic policy for a consumer.
    Network,
    /// Connection or attachment to one exact endpoint.
    Endpoint,
    /// Authorized delivery of one credential to a consumer.
    Credential,
}

impl BindingKind {
    /// Every binding kind, in contract order.
    pub const ALL: [Self; 5] = [
        Self::Volume,
        Self::Device,
        Self::Network,
        Self::Endpoint,
        Self::Credential,
    ];

    /// The ResourceType of the binding row this kind materializes.
    pub const fn resource_type(self) -> &'static str {
        match self {
            Self::Volume => VOLUME_BINDING_RESOURCE_TYPE,
            Self::Device => super::device_binding::DEVICE_BINDING_RESOURCE_TYPE,
            Self::Network => super::network_binding::NETWORK_BINDING_RESOURCE_TYPE,
            Self::Endpoint => super::endpoint_binding::ENDPOINT_BINDING_RESOURCE_TYPE,
            Self::Credential => super::credential_binding::CREDENTIAL_BINDING_RESOURCE_TYPE,
        }
    }

    /// The ResourceType this kind consumes as its source.
    pub const fn source_resource_type(self) -> &'static str {
        match self {
            Self::Volume => "Volume",
            Self::Device => "Device",
            Self::Network => "Network",
            Self::Endpoint => "Endpoint",
            Self::Credential => "Credential",
        }
    }

    /// Resolve the kind from the ResourceType of a source reference.
    pub const fn from_source_resource_type(value: &str) -> Option<Self> {
        match value.as_bytes() {
            b"Volume" => Some(Self::Volume),
            b"Device" => Some(Self::Device),
            b"Network" => Some(Self::Network),
            b"Endpoint" => Some(Self::Endpoint),
            b"Credential" => Some(Self::Credential),
            _ => None,
        }
    }

    /// Whether this kind admits `consumer` as a consumer.
    ///
    /// A `Host` consumes a Volume, a Device, and a Network - those are the
    /// parents whose own attachment inputs are converted into a binding with
    /// that parent as consumer. An Endpoint or a Credential is delivered to a
    /// consumer identity instead: a host-level need there is a child
    /// target-support ceiling, and a provider's host-side delivery is an
    /// admitted realization leg of the binding whose consumer is that helper,
    /// never a binding whose consumer is the Host.
    pub const fn admits_consumer(self, consumer: BindingConsumerKind) -> bool {
        match self {
            Self::Volume | Self::Device | Self::Network => true,
            Self::Endpoint | Self::Credential => !matches!(consumer, BindingConsumerKind::Host),
        }
    }

    /// Whether this kind admits `rights` as a requested right.
    pub const fn admits_rights(self, rights: RequestedRights) -> bool {
        match self {
            Self::Volume => matches!(
                rights,
                RequestedRights::Observe | RequestedRights::Mutate | RequestedRights::Share
            ),
            Self::Device => matches!(
                rights,
                RequestedRights::Share | RequestedRights::Exclusive
            ),
            Self::Network | Self::Credential => matches!(rights, RequestedRights::Consume),
            Self::Endpoint => matches!(
                rights,
                RequestedRights::Observe | RequestedRights::Consume
            ),
        }
    }

    /// The right a defaulted request takes when the child declared none.
    pub(crate) const fn default_right(self) -> RequestedRights {
        match self {
            Self::Volume | Self::Device => RequestedRights::Observe,
            Self::Network | Self::Endpoint | Self::Credential => RequestedRights::Consume,
        }
    }
}

/// The consumer a binding relationship delivers to.
///
/// The vocabulary is closed and identical for every kind; which of the four a
/// kind accepts is [`BindingKind::admits_consumer`], never a string match at
/// the call site.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum BindingConsumerKind {
    /// A long-running `Process`.
    Process,
    /// A run-to-completion `EphemeralProcess`.
    EphemeralProcess,
    /// A `Host` consuming a capability as its own parent.
    Host,
    /// A `Guest` consuming a capability as its own parent.
    Guest,
}

impl BindingConsumerKind {
    /// Every consumer kind, in contract order.
    pub const ALL: [Self; 4] = [
        Self::Process,
        Self::EphemeralProcess,
        Self::Host,
        Self::Guest,
    ];

    /// The ResourceType a consumer of this kind is.
    pub const fn resource_type(self) -> &'static str {
        match self {
            Self::Process => "Process",
            Self::EphemeralProcess => "EphemeralProcess",
            Self::Host => "Host",
            Self::Guest => "Guest",
        }
    }

    /// Resolve the consumer kind from a consumer reference's ResourceType.
    pub const fn from_resource_type(value: &str) -> Option<Self> {
        match value.as_bytes() {
            b"Process" => Some(Self::Process),
            b"EphemeralProcess" => Some(Self::EphemeralProcess),
            b"Host" => Some(Self::Host),
            b"Guest" => Some(Self::Guest),
            _ => None,
        }
    }

    /// Whether this consumer is an execution parent rather than an instance.
    ///
    /// A parent is where AE31-AE33's three input meanings are distinguished:
    /// a support ceiling bounds children, a parent use is a binding whose
    /// consumer is the parent, and a child default shapes one child's request.
    pub const fn is_execution_parent(self) -> bool {
        matches!(self, Self::Host | Self::Guest)
    }
}

/// The right one request asks for.
///
/// This is the shared requested-rights vocabulary. Each kind admits only the
/// subset its own semantics define, so a device claim, a storage writer, a
/// network membership, and a credential delivery cannot be exchanged for one
/// another by spelling a different variant.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum RequestedRights {
    /// Observe the exact named capability without changing it.
    Observe,
    /// Use the exact named capability for its own effect.
    Consume,
    /// Take a mutating claim the source arbitrates.
    Mutate,
    /// Share the source with peer consumers under one source-side decision.
    Share,
    /// Hold the source exclusively while this relationship is live.
    Exclusive,
}

impl RequestedRights {
    /// Every requested right, in contract order.
    pub const ALL: [Self; 5] = [
        Self::Observe,
        Self::Consume,
        Self::Mutate,
        Self::Share,
        Self::Exclusive,
    ];

    /// Whether taking this right needs source-side arbitration.
    pub const fn needs_arbitration(self) -> bool {
        matches!(self, Self::Mutate | Self::Share | Self::Exclusive)
    }
}

/// How the source arbitrates one admitted relationship.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum BindingArbitration {
    /// The source admits this consumer alongside its peers.
    Shared,
    /// The source admits this consumer alone until it releases.
    Exclusive,
}

/// The stable consumer slot a request occupies.
///
/// The slot is the consumer's own local name for the relationship. It is what
/// makes a rights or destination update the same relationship rather than a
/// second one, so it identifies; rights, destination, and every other mutable
/// payload field never do.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct BindingSlot(BoundedToken);

impl BindingSlot {
    /// Parse a `^[a-z][a-z0-9-]*$` slot token.
    pub fn parse(value: impl Into<String>) -> Result<Self, PrimitiveSpecError> {
        BoundedToken::parse(value).map(Self)
    }

    /// Borrow the canonical slot token.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

redacted_debug!(BindingSlot);
parsed_deserialize!(BindingSlot);
string_schema!(BindingSlot, 1, MAX_BINDING_SLOT_BYTES);

/// The observed lifecycle of one binding relationship.
///
/// This is a status vocabulary, never a desired field: a request cannot say it
/// is `Active`, and an uncertain observation stays `Unknown` or `Degraded`
/// rather than being reported as granted use.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum BindingLifecycleState {
    /// Identities are declared but access is not authorized yet.
    Requested,
    /// The exact requested relationship is authorized against current evidence.
    Admitted,
    /// Source-side access and delivery prerequisites exist.
    Prepared,
    /// The consumer is using the prepared relationship.
    Active,
    /// New use is blocked ahead of typed release.
    Revoking,
    /// Outstanding use is being driven to the kind's safe state.
    Draining,
    /// No outstanding use or lease remains for this relationship.
    Released,
    /// Admission failed for one typed reason.
    Refused,
    /// The relationship exists but an effect cannot be proven effective.
    Degraded,
    /// Completion could not be proven either way.
    Unknown,
}

impl BindingLifecycleState {
    /// Whether this observed state still admits new use.
    pub const fn admits_new_use(self) -> bool {
        matches!(
            self,
            Self::Requested | Self::Admitted | Self::Prepared | Self::Active
        )
    }

    /// Whether this observed state is finished for good.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Released | Self::Refused)
    }

    /// Whether this observed state proves an effective result.
    ///
    /// `Degraded` and `Unknown` are uncertainty, not success: recovery has to
    /// prove adoption or report the refusal rather than read either as
    /// granted access.
    pub const fn proves_effect(self) -> bool {
        matches!(self, Self::Prepared | Self::Active | Self::Released)
    }
}

/// One side of a binding relationship being ready.
///
/// Source preparation and consumer-side completion stay separate so a
/// relationship that must exist before its consumer starts never forms a
/// startup cycle with the observation that consumer can see it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum CompletionCondition {
    /// Not established yet.
    Pending,
    /// Established under this relationship's own identity.
    Complete,
    /// Not established, for one typed reason.
    Failed(RefusalReason),
}

impl CompletionCondition {
    /// Whether this condition is established.
    pub const fn is_complete(self) -> bool {
        matches!(self, Self::Complete)
    }
}

/// What happened to the relationship's use.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ReleaseOutcome {
    /// Use is outstanding or has not started.
    Outstanding,
    /// New use is blocked and existing use is being driven closed.
    Draining,
    /// No outstanding use remains; the shared source itself is untouched.
    Released,
}

/// The realization facet a requested presentation depends on.
///
/// A presentation the selected backend cannot enforce is refused rather than
/// skipped: an unapplied mount policy, an unclaimed device, or an unproven
/// endpoint is not success.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum BindingRealizationFacet {
    /// A private mount tree carrying the exact named view at the destination.
    FilesystemPresentation,
    /// A block device in a consumer device slot.
    ConsumerDeviceSlot,
    /// A verified device descriptor or mediated attachment.
    DeviceAttachment,
    /// An inherited interface in the consumer's own network namespace.
    NamespaceInterface,
    /// Membership in the provider-owned shared fabric realization.
    SharedFabric,
    /// A verified connected or listening descriptor for the exact endpoint.
    EndpointDescriptor,
    /// A private binding of the exact socket where the backend needs a name.
    EndpointPathname,
    /// Credential material delivered inside an admitted delivery session.
    CredentialDelivery,
}

/// What one binding implementation declares it can realize.
#[derive(Clone, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BindingRealizationSupport {
    facets: Vec<BindingRealizationFacet>,
}

impl BindingRealizationSupport {
    /// Construct a support set, requiring unique entries.
    pub fn new(facets: Vec<BindingRealizationFacet>) -> Result<Self, BindingContractError> {
        let mut sorted = facets.clone();
        sorted.sort_unstable();
        sorted.dedup();
        if sorted.len() != facets.len() {
            return Err(BindingContractError::InvalidCollection);
        }
        Ok(Self { facets })
    }

    /// Whether the implementation realizes `facet`.
    pub fn realizes(&self, facet: BindingRealizationFacet) -> bool {
        self.facets.contains(&facet)
    }

    /// Borrow the declared facets.
    pub fn facets(&self) -> &[BindingRealizationFacet] {
        &self.facets
    }
}

redacted_debug!(BindingRealizationSupport);

/// Authorization evidence for one binding request.
///
/// The Role and RoleBinding evaluation produces this. A well-formed request
/// is not authorization: without this evidence the request is refused, and a
/// candidate that would introduce its own grant is never evaluated against
/// itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindingAuthorization {
    granted: bool,
}

impl BindingAuthorization {
    /// Authorization evidence for a permitted request.
    pub const fn granted() -> Self {
        Self { granted: true }
    }

    /// The absence of authorization evidence.
    pub const fn absent() -> Self {
        Self { granted: false }
    }

    /// Whether the request is authorized.
    pub const fn is_granted(&self) -> bool {
        self.granted
    }
}

/// The KTD3 identity of one binding relationship.
///
/// The key is derived from Zone, source, consumer, binding kind, and the
/// stable slot. Rights, destination, presentation, and every other mutable
/// payload field are deliberately outside it: changing them updates the
/// relationship instead of minting a second one.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BindingKey {
    zone: ZoneId,
    kind: BindingKind,
    source_ref: ResourceRef,
    source_uid: ResourceUid,
    consumer_ref: ResourceRef,
    consumer_uid: ResourceUid,
    slot: BindingSlot,
}

impl BindingKey {
    /// Derive one relationship's key from exact identities.
    ///
    /// The source must be the kind's own source ResourceType, and the
    /// consumer must be a kind this binding admits. Both halves are in the
    /// same Zone by construction: a primitive binding is same-Zone, and a
    /// cross-Zone use is consumed as an admitted semantic projection under the
    /// exporting Zone's policy, ceiling, and lease.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        zone: ZoneId,
        kind: BindingKind,
        source_ref: ResourceRef,
        source_uid: ResourceUid,
        consumer_ref: ResourceRef,
        consumer_uid: ResourceUid,
        slot: BindingSlot,
    ) -> Result<Self, BindingContractError> {
        if source_ref.resource_type().as_str() != kind.source_resource_type() {
            return Err(BindingContractError::WrongResourceType);
        }
        let consumer = BindingConsumerKind::from_resource_type(consumer_ref.resource_type().as_str())
            .ok_or(BindingContractError::WrongResourceType)?;
        if !kind.admits_consumer(consumer) {
            return Err(BindingContractError::UnsupportedConsumerKind);
        }
        Ok(Self {
            zone,
            kind,
            source_ref,
            source_uid,
            consumer_ref,
            consumer_uid,
            slot,
        })
    }

    /// Borrow the Zone the relationship belongs to.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Return the binding kind.
    pub const fn kind(&self) -> BindingKind {
        self.kind
    }

    /// Borrow the exact source reference.
    pub const fn source_ref(&self) -> &ResourceRef {
        &self.source_ref
    }

    /// Borrow the source's store-assigned identity.
    pub const fn source_uid(&self) -> &ResourceUid {
        &self.source_uid
    }

    /// Borrow the exact consumer reference.
    pub const fn consumer_ref(&self) -> &ResourceRef {
        &self.consumer_ref
    }

    /// Borrow the consumer's store-assigned identity.
    pub const fn consumer_uid(&self) -> &ResourceUid {
        &self.consumer_uid
    }

    /// Borrow the stable consumer slot.
    pub const fn slot(&self) -> &BindingSlot {
        &self.slot
    }

    /// The consumer slot index entry this relationship occupies.
    ///
    /// The index is keyed independently of the source-owned key, so a
    /// conflicting simultaneous declaration is caught even when it names a
    /// different source.
    pub fn address(&self) -> BindingSlotAddress {
        BindingSlotAddress {
            zone: self.zone.clone(),
            consumer_uid: self.consumer_uid.clone(),
            kind: self.kind,
            slot: self.slot.clone(),
        }
    }
}

redacted_debug!(BindingKey);

wire_deserialize!(
    BindingKey,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        zone: ZoneId,
        kind: BindingKind,
        source_ref: ResourceRef,
        source_uid: ResourceUid,
        consumer_ref: ResourceRef,
        consumer_uid: ResourceUid,
        slot: BindingSlot,
    },
    wire,
    BindingKey::new(
        wire.zone,
        wire.kind,
        wire.source_ref,
        wire.source_uid,
        wire.consumer_ref,
        wire.consumer_uid,
        wire.slot,
    )
    .map_err(serde::de::Error::custom)
);

/// The consumer slot index key: Zone, consumer identity, kind, and slot.
///
/// It deliberately excludes the source. Two declarations that agree on the
/// source but not on their payload are one relationship; two declarations
/// that disagree on the source are a replacement, and both are refused while
/// the slot is live.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BindingSlotAddress {
    zone: ZoneId,
    consumer_uid: ResourceUid,
    kind: BindingKind,
    slot: BindingSlot,
}

impl BindingSlotAddress {
    /// Borrow the Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the consumer identity.
    pub const fn consumer_uid(&self) -> &ResourceUid {
        &self.consumer_uid
    }

    /// Return the binding kind.
    pub const fn kind(&self) -> BindingKind {
        self.kind
    }

    /// Borrow the stable slot token.
    pub const fn slot(&self) -> &BindingSlot {
        &self.slot
    }
}

redacted_debug!(BindingSlotAddress);

/// The digest of the exact desired bytes one declaration carries.
///
/// Two declarations of the same relationship coalesce only when their
/// canonical bytes match, which is what makes a rights or destination change
/// visible as an update instead of silently reusing the earlier request.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BindingSpecFingerprint(String);

impl BindingSpecFingerprint {
    /// The domain tag framing one binding-request digest.
    pub const DOMAIN_TAG: &'static str = "d2b:v3:binding-request";

    /// Frame the digest of one desired request.
    pub fn from_request<T: Serialize>(request: &T) -> Self {
        let bytes = canonical_json_bytes(request)
            .expect("a typed binding request always renders as canonical bytes");
        Self(framed_canonical_digest(Self::DOMAIN_TAG, &bytes))
    }

    /// Parse a framed request digest.
    pub fn parse(value: impl Into<String>) -> Result<Self, BindingContractError> {
        let value = value.into();
        if is_canonical_digest(&value) {
            Ok(Self(value))
        } else {
            Err(BindingContractError::InvalidField)
        }
    }

    /// Borrow the framed digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

redacted_debug!(BindingSpecFingerprint);

/// What normalizing one declaration against the consumer slot index did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingSlotDecision {
    /// The slot was free and this declaration now occupies it.
    Claimed,
    /// An identical declaration already occupies the slot, so the two coalesce.
    Coalesced,
    /// The previous occupant released and this declaration is its successor.
    SuccessorClaimed,
    /// The same relationship's payload changed while its old use was closed.
    PayloadUpdated,
}

/// One observed declaration occupying a consumer slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingSlotEntry {
    source_ref: ResourceRef,
    source_uid: ResourceUid,
    fingerprint: BindingSpecFingerprint,
    state: BindingLifecycleState,
}

impl BindingSlotEntry {
    /// Borrow the occupying source reference.
    pub const fn source_ref(&self) -> &ResourceRef {
        &self.source_ref
    }

    /// Borrow the occupying source identity.
    pub const fn source_uid(&self) -> &ResourceUid {
        &self.source_uid
    }

    /// Borrow the occupying declaration's digest.
    pub const fn fingerprint(&self) -> &BindingSpecFingerprint {
        &self.fingerprint
    }

    /// Return the observed lifecycle.
    pub const fn state(&self) -> BindingLifecycleState {
        self.state
    }
}

/// The consumer slot index for one Zone's declared relationships.
///
/// At most one binding occupies a live slot, which is what lets a source
/// replacement retire the old source-owned binding before its successor is
/// admitted without ever changing an existing binding's owner.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BindingSlotIndex {
    entries: BTreeMap<BindingSlotAddress, BindingSlotEntry>,
}

impl BindingSlotIndex {
    /// Construct an empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Borrow the declaration occupying one slot, when there is one.
    pub fn occupant(&self, address: &BindingSlotAddress) -> Option<&BindingSlotEntry> {
        self.entries.get(address)
    }

    /// Borrow every live slot, in index order.
    pub fn entries(&self) -> impl Iterator<Item = (&BindingSlotAddress, &BindingSlotEntry)> {
        self.entries.iter()
    }

    /// Normalize one declaration for its consumer slot.
    ///
    /// An identical declaration coalesces. A different declaration for the
    /// same source is refused before any mutation while the slot is live,
    /// because changing its rights or destination in place is what would
    /// activate old and new access together; once the occupant has released,
    /// the declaration updates it. A declaration naming a different source
    /// waits for the same release and then becomes the successor, so an
    /// existing binding never changes owner.
    pub fn declare(
        &mut self,
        key: &BindingKey,
        fingerprint: &BindingSpecFingerprint,
    ) -> Result<BindingSlotDecision, BindingContractError> {
        let address = key.address();
        match self.entries.get_mut(&address) {
            None => {
                self.entries.insert(
                    address,
                    BindingSlotEntry {
                        source_ref: key.source_ref().clone(),
                        source_uid: key.source_uid().clone(),
                        fingerprint: fingerprint.clone(),
                        state: BindingLifecycleState::Requested,
                    },
                );
                Ok(BindingSlotDecision::Claimed)
            }
            Some(entry) => {
                if entry.source_uid() != key.source_uid() {
                    return if entry.state == BindingLifecycleState::Released {
                        entry.source_ref = key.source_ref().clone();
                        entry.source_uid = key.source_uid().clone();
                        entry.fingerprint = fingerprint.clone();
                        Ok(BindingSlotDecision::SuccessorClaimed)
                    } else {
                        Err(BindingContractError::SourceMismatch)
                    };
                }
                if entry.fingerprint() == fingerprint {
                    return Ok(BindingSlotDecision::Coalesced);
                }
                if entry.state != BindingLifecycleState::Released {
                    return Err(BindingContractError::SlotOccupied);
                }
                entry.fingerprint = fingerprint.clone();
                Ok(BindingSlotDecision::PayloadUpdated)
            }
        }
    }

    /// Apply a rights or destination change to an existing relationship.
    ///
    /// The change is admitted only once the old use is blocked or closed, so
    /// the previous access and the new one are never both live. The slot keeps
    /// its owner across the change.
    pub fn change_payload(
        &mut self,
        key: &BindingKey,
        fingerprint: &BindingSpecFingerprint,
    ) -> Result<BindingSlotDecision, BindingContractError> {
        let entry = self
            .entries
            .get_mut(&key.address())
            .ok_or(BindingContractError::UnexpectedState)?;
        if entry.source_uid() != key.source_uid() {
            return Err(BindingContractError::SourceMismatch);
        }
        if entry.fingerprint() == fingerprint {
            return Ok(BindingSlotDecision::Coalesced);
        }
        if matches!(
            entry.state,
            BindingLifecycleState::Revoking
                | BindingLifecycleState::Draining
                | BindingLifecycleState::Released
        ) {
            entry.fingerprint = fingerprint.clone();
            Ok(BindingSlotDecision::PayloadUpdated)
        } else {
            Err(BindingContractError::SlotOccupied)
        }
    }

    /// Record the observed lifecycle for a declared slot.
    pub fn observe(
        &mut self,
        key: &BindingKey,
        state: BindingLifecycleState,
    ) -> Result<(), BindingContractError> {
        let entry = self
            .entries
            .get_mut(&key.address())
            .ok_or(BindingContractError::UnexpectedState)?;
        if entry.source_uid() != key.source_uid() {
            return Err(BindingContractError::SourceMismatch);
        }
        entry.state = state;
        Ok(())
    }
}


/// The source provider's own decision on one exact request.
///
/// The decision is scoped to the relationship it was made for, so it cannot be
/// carried to another source, consumer, or slot.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceAdmission {
    binding: BindingKey,
    admitted_rights: Vec<RequestedRights>,
    arbitration: BindingArbitration,
}

impl SourceAdmission {
    /// Construct the source's decision after checking its bounds.
    pub fn new(
        binding: BindingKey,
        admitted_rights: Vec<RequestedRights>,
        arbitration: BindingArbitration,
    ) -> Result<Self, BindingContractError> {
        if admitted_rights.is_empty() {
            return Err(BindingContractError::MissingRequiredField);
        }
        let mut sorted = admitted_rights.clone();
        sorted.sort_unstable();
        sorted.dedup();
        if sorted.len() != admitted_rights.len() {
            return Err(BindingContractError::InvalidCollection);
        }
        Ok(Self {
            binding,
            admitted_rights,
            arbitration,
        })
    }

    /// Borrow the exact relationship this decision was made for.
    pub const fn binding(&self) -> &BindingKey {
        &self.binding
    }

    /// Whether the source admits `rights` for this relationship right now.
    pub fn admits(&self, rights: RequestedRights) -> bool {
        self.admitted_rights.contains(&rights)
    }

    /// Borrow the rights the source admits.
    pub fn admitted_rights(&self) -> &[RequestedRights] {
        &self.admitted_rights
    }

    /// Return how the source arbitrates this relationship.
    pub const fn arbitration(&self) -> BindingArbitration {
        self.arbitration
    }
}

redacted_debug!(SourceAdmission);

/// One admitted binding request.
///
/// This is the fence, not the grant: it names the exact dependency revisions
/// the admission was evaluated against, so an ownership, view, consumer,
/// provider-assignment, or policy change that does not advance a spec
/// generation still invalidates the earlier use.
#[derive(Clone, PartialEq, Eq)]
pub struct BindingAdmission {
    key: BindingKey,
    rights: RequestedRights,
    arbitration: BindingArbitration,
    dependencies: Vec<FreshnessTuple>,
}

impl BindingAdmission {
    /// Borrow the admitted relationship.
    pub const fn key(&self) -> &BindingKey {
        &self.key
    }

    /// Return the admitted right.
    pub const fn rights(&self) -> RequestedRights {
        self.rights
    }

    /// Return the source-side arbitration this admission holds.
    pub const fn arbitration(&self) -> BindingArbitration {
        self.arbitration
    }

    /// Borrow the dependency versions this admission is fenced against.
    pub fn dependencies(&self) -> &[FreshnessTuple] {
        &self.dependencies
    }

    /// Whether every admitted dependency still matches observed evidence.
    ///
    /// A dependency that is absent from `observed`, changed revision, or
    /// changed digest fails the check: cached readiness cannot remint access.
    pub fn is_current(&self, observed: &[FreshnessTuple]) -> bool {
        self.dependencies.iter().all(|admitted| {
            observed.iter().any(|current| {
                current.store_incarnation() == admitted.store_incarnation()
                    && current.resource_uid() == admitted.resource_uid()
                    && current.desired_revision() == admitted.desired_revision()
                    && current.desired_digest() == admitted.desired_digest()
            })
        })
    }
}

redacted_debug!(BindingAdmission);

/// Admit one binding request against its grant, source decision, and support.
///
/// Evaluation is deterministic and stops at the first refusal, so a caller
/// always sees one enforcing stage. Nothing here widens the request: a right
/// the kind does not admit was already refused by the constructor, a right the
/// source does not admit, an exclusive claim the source is not arbitrating, a
/// presentation the selected realization cannot enforce, and an unfenced
/// admission are each refused rather than approximated.
pub fn admit_binding_request(
    key: &BindingKey,
    requested_rights: RequestedRights,
    required_facets: &[BindingRealizationFacet],
    authorization: &BindingAuthorization,
    source: &SourceAdmission,
    support: &BindingRealizationSupport,
    dependencies: &[FreshnessTuple],
) -> Result<BindingAdmission, BindingRefusal> {
    if !authorization.is_granted() {
        return Err(BindingRefusal::new(
            AdmissionStage::Authorize,
            RefusalReason::IdentityNotAuthorized,
        ));
    }
    if source.binding() != key {
        return Err(BindingRefusal::new(
            AdmissionStage::Admit,
            RefusalReason::SourcePolicyRefused,
        ));
    }
    if !source.admits(requested_rights)
        || (requested_rights == RequestedRights::Exclusive
            && source.arbitration() != BindingArbitration::Exclusive)
    {
        return Err(BindingRefusal::new(
            AdmissionStage::Admit,
            RefusalReason::SourcePolicyRefused,
        ));
    }
    if required_facets
        .iter()
        .any(|facet| !support.realizes(*facet))
    {
        return Err(BindingRefusal::new(
            AdmissionStage::Prepare,
            RefusalReason::MandatoryFacetUnsupported,
        ));
    }
    if dependencies.len() > MAX_BINDING_DEPENDENCIES {
        return Err(BindingRefusal::new(
            AdmissionStage::Authorize,
            RefusalReason::LimitExceedsCeiling,
        ));
    }
    if dependencies.is_empty()
        || dependencies
            .iter()
            .any(|dependency| dependency.zone() != key.zone())
    {
        return Err(BindingRefusal::new(
            AdmissionStage::Reserve,
            RefusalReason::UnprovenEffect,
        ));
    }
    Ok(BindingAdmission {
        key: key.clone(),
        rights: requested_rights,
        arbitration: source.arbitration(),
        dependencies: dependencies.to_vec(),
    })
}

/// The source-owned reservation identity behind one admitted relationship.
///
/// This is identity evidence, not the capability itself. The broker-minted
/// opaque handle that realizes the delivery stays private to its reservation
/// owner and is deliberately absent from this contract, so nothing here can be
/// replayed as access.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceReservation {
    zone: ZoneId,
    source_uid: ResourceUid,
    reservation_id: BoundedToken,
}

impl SourceReservation {
    /// Construct the reservation identity the source owns.
    pub const fn new(zone: ZoneId, source_uid: ResourceUid, reservation_id: BoundedToken) -> Self {
        Self {
            zone,
            source_uid,
            reservation_id,
        }
    }

    /// Borrow the Zone the reservation belongs to.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the reserving source identity.
    pub const fn source_uid(&self) -> &ResourceUid {
        &self.source_uid
    }

    /// Borrow the source's own reservation token.
    pub const fn reservation_id(&self) -> &BoundedToken {
        &self.reservation_id
    }
}

redacted_debug!(SourceReservation);

/// One observation of an admitted relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingObservation {
    state: BindingLifecycleState,
    prepare: CompletionCondition,
    consumer_completion: CompletionCondition,
    release: ReleaseOutcome,
}

impl BindingObservation {
    /// Construct one observation.
    pub const fn new(
        state: BindingLifecycleState,
        prepare: CompletionCondition,
        consumer_completion: CompletionCondition,
        release: ReleaseOutcome,
    ) -> Self {
        Self {
            state,
            prepare,
            consumer_completion,
            release,
        }
    }

    /// Return the observed lifecycle.
    pub const fn state(&self) -> BindingLifecycleState {
        self.state
    }

    /// Return the source-side preparation condition.
    pub const fn prepare(&self) -> CompletionCondition {
        self.prepare
    }

    /// Return the consumer-side completion condition.
    pub const fn consumer_completion(&self) -> CompletionCondition {
        self.consumer_completion
    }

    /// Return the release outcome.
    pub const fn release(&self) -> ReleaseOutcome {
        self.release
    }
}

/// The observed state of one admitted relationship.
///
/// The only constructor takes an admission produced by
/// [`admit_binding_request`], so a desired request cannot be turned into
/// admitted evidence by naming itself, and no secret material, host path, or
/// numerical principal is reachable from here.
#[derive(Clone, PartialEq, Eq)]
pub struct BindingEvidence {
    admission: BindingAdmission,
    reservation: SourceReservation,
    observation: BindingObservation,
}

impl BindingEvidence {
    /// Mint admitted evidence from one admission and its reservation.
    pub fn admitted(admission: BindingAdmission, reservation: SourceReservation) -> Self {
        Self {
            admission,
            reservation,
            observation: BindingObservation::new(
                BindingLifecycleState::Admitted,
                CompletionCondition::Pending,
                CompletionCondition::Pending,
                ReleaseOutcome::Outstanding,
            ),
        }
    }

    /// Record the next observation of this relationship.
    pub fn observed(mut self, observation: BindingObservation) -> Self {
        self.observation = observation;
        self
    }

    /// Borrow the relationship's identity.
    pub const fn key(&self) -> &BindingKey {
        self.admission.key()
    }

    /// Borrow the admission this evidence was minted from.
    pub const fn admission(&self) -> &BindingAdmission {
        &self.admission
    }

    /// Borrow the source-owned reservation identity.
    pub const fn reservation(&self) -> &SourceReservation {
        &self.reservation
    }

    /// Borrow the latest observation.
    pub const fn observation(&self) -> &BindingObservation {
        &self.observation
    }

    /// Return the observed lifecycle.
    pub const fn state(&self) -> BindingLifecycleState {
        self.observation.state
    }

    /// Whether this evidence is still fenced against the observed graph.
    pub fn is_current(&self, observed: &[FreshnessTuple]) -> bool {
        self.admission.is_current(observed)
    }
}

redacted_debug!(BindingEvidence);

/// One capability a child target admits its children to request.
///
/// A support ceiling is the conversion of a host target-support input. It
/// bounds admission and creates no binding, no reservation, and no access.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingSupportEntry {
    kind: BindingKind,
    rights: Vec<RequestedRights>,
}

impl BindingSupportEntry {
    /// Construct one entry after checking its bounds.
    pub fn new(kind: BindingKind, rights: Vec<RequestedRights>) -> Result<Self, BindingContractError> {
        if rights.is_empty() {
            return Err(BindingContractError::MissingRequiredField);
        }
        if rights.iter().any(|right| !kind.admits_rights(*right)) {
            return Err(BindingContractError::UnsupportedRight);
        }
        let mut sorted = rights.clone();
        sorted.sort_unstable();
        sorted.dedup();
        if sorted.len() != rights.len() {
            return Err(BindingContractError::InvalidCollection);
        }
        Ok(Self { kind, rights })
    }

    /// Return the binding kind this entry covers.
    pub const fn kind(&self) -> BindingKind {
        self.kind
    }

    /// Borrow the rights a child may request.
    pub fn rights(&self) -> &[RequestedRights] {
        &self.rights
    }

    /// Whether this entry admits one child request.
    pub fn admits(&self, kind: BindingKind, rights: RequestedRights) -> bool {
        self.kind == kind && self.rights.contains(&rights)
    }
}

redacted_debug!(BindingSupportEntry);

/// The child target-support ceiling carried by one execution parent.
#[derive(Clone, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChildSupportCeiling {
    entries: Vec<BindingSupportEntry>,
}

impl ChildSupportCeiling {
    /// Construct a ceiling after checking its bound and uniqueness.
    pub fn new(entries: Vec<BindingSupportEntry>) -> Result<Self, BindingContractError> {
        if entries.len() > MAX_BINDING_SUPPORT_ENTRIES {
            return Err(BindingContractError::InvalidCollection);
        }
        let mut sorted = entries.clone();
        sorted.sort_by_key(BindingSupportEntry::kind);
        sorted.dedup_by_key(|entry| entry.kind());
        if sorted.len() != entries.len() {
            return Err(BindingContractError::InvalidCollection);
        }
        Ok(Self { entries })
    }

    /// Borrow the covered kinds.
    pub fn entries(&self) -> &[BindingSupportEntry] {
        &self.entries
    }

    /// Whether this ceiling admits one child request.
    pub fn admits(&self, kind: BindingKind, rights: RequestedRights) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.admits(kind, rights))
    }
}

redacted_debug!(ChildSupportCeiling);

/// The source one parent's default names for a child request.
///
/// A default names a typed source and, where the kind has one, its named
/// view. It is never a host path and never a grant.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DefaultedSource {
    kind: BindingKind,
    source_ref: ResourceRef,
    view: Option<BoundedToken>,
}

impl DefaultedSource {
    /// Construct the default after checking the source's ResourceType.
    pub fn new(
        kind: BindingKind,
        source_ref: ResourceRef,
        view: Option<BoundedToken>,
    ) -> Result<Self, BindingContractError> {
        if source_ref.resource_type().as_str() != kind.source_resource_type() {
            return Err(BindingContractError::WrongResourceType);
        }
        Ok(Self {
            kind,
            source_ref,
            view,
        })
    }

    /// Return the defaulted binding kind.
    pub const fn kind(&self) -> BindingKind {
        self.kind
    }

    /// Borrow the defaulted source reference.
    pub const fn source_ref(&self) -> &ResourceRef {
        &self.source_ref
    }

    /// Borrow the defaulted named view, when the kind has one.
    pub const fn view(&self) -> Option<&BoundedToken> {
        self.view.as_ref()
    }
}

redacted_debug!(DefaultedSource);

/// One default an execution parent supplies to shape a single child's request.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChildRequestDefaults {
    child_ref: ResourceRef,
    source: DefaultedSource,
}

impl ChildRequestDefaults {
    /// Construct the defaults for the child they name.
    pub fn new(child_ref: ResourceRef, source: DefaultedSource) -> Result<Self, BindingContractError> {
        match BindingConsumerKind::from_resource_type(child_ref.resource_type().as_str()) {
            Some(consumer) if !consumer.is_execution_parent() => {}
            Some(_) => return Err(BindingContractError::UnsupportedConsumerKind),
            None => return Err(BindingContractError::WrongResourceType),
        }
        Ok(Self { child_ref, source })
    }

    /// Borrow the child these defaults are for.
    pub const fn child_ref(&self) -> &ResourceRef {
        &self.child_ref
    }

    /// Borrow the defaulted source.
    pub const fn source(&self) -> &DefaultedSource {
        &self.source
    }
}

redacted_debug!(ChildRequestDefaults);

/// One child's binding request as authored, before it becomes a desired spec.
///
/// The draft is what a parent's defaults are applied to. A default fills only
/// a field the child left unset and only the child it names, so inheritance
/// cannot widen a request or move it onto another consumer.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChildBindingRequest {
    consumer_ref: ResourceRef,
    kind: BindingKind,
    source_ref: Option<ResourceRef>,
    view: Option<BoundedToken>,
    rights: Option<RequestedRights>,
}

impl ChildBindingRequest {
    /// Construct an empty draft for one consumer.
    pub fn new(consumer_ref: ResourceRef, kind: BindingKind) -> Result<Self, BindingContractError> {
        let consumer = BindingConsumerKind::from_resource_type(consumer_ref.resource_type().as_str())
            .ok_or(BindingContractError::WrongResourceType)?;
        if !kind.admits_consumer(consumer) {
            return Err(BindingContractError::UnsupportedConsumerKind);
        }
        Ok(Self {
            consumer_ref,
            kind,
            source_ref: None,
            view: None,
            rights: None,
        })
    }

    /// Record the child's own declaration, which a default can never override.
    pub fn declaring(
        mut self,
        source_ref: ResourceRef,
        view: Option<BoundedToken>,
        rights: RequestedRights,
    ) -> Result<Self, BindingContractError> {
        if source_ref.resource_type().as_str() != self.kind.source_resource_type() {
            return Err(BindingContractError::WrongResourceType);
        }
        if !self.kind.admits_rights(rights) {
            return Err(BindingContractError::UnsupportedRight);
        }
        self.source_ref = Some(source_ref);
        self.view = view;
        self.rights = Some(rights);
        Ok(self)
    }

    /// Borrow the consumer this request belongs to.
    pub const fn consumer_ref(&self) -> &ResourceRef {
        &self.consumer_ref
    }

    /// Return the requested binding kind.
    pub const fn kind(&self) -> BindingKind {
        self.kind
    }

    /// Borrow the declared source, when the child declared one.
    pub const fn source_ref(&self) -> Option<&ResourceRef> {
        self.source_ref.as_ref()
    }

    /// Borrow the declared named view, when the child declared one.
    pub const fn view(&self) -> Option<&BoundedToken> {
        self.view.as_ref()
    }

    /// Return the declared right, when the child declared one.
    pub const fn rights(&self) -> Option<RequestedRights> {
        self.rights
    }

    /// Apply one parent's defaults to this child's request.
    ///
    /// A default belongs to exactly one child: applying it to another
    /// consumer's request is refused, and a field the child already declared
    /// is left alone.
    pub fn apply_defaults(
        &self,
        defaults: &ChildRequestDefaults,
    ) -> Result<Self, BindingContractError> {
        if self.consumer_ref != defaults.child_ref {
            return Err(BindingContractError::WrongConsumer);
        }
        let source = defaults.source();
        if source.kind() != self.kind {
            return Err(BindingContractError::WrongResourceType);
        }
        let mut applied = self.clone();
        if applied.source_ref.is_none() {
            applied.source_ref = Some(source.source_ref().clone());
            applied.view = source.view().cloned();
        }
        if applied.rights.is_none() {
            applied.rights = Some(self.kind.default_right());
        }
        Ok(applied)
    }
}

/// What one classified Host or Guest input means after conversion.
///
/// The old flattened execution-parent fragment mixed three meanings in one
/// attachment list. They stay separate here: a support ceiling bounds child
/// admission and creates nothing, a parent use is a binding whose consumer is
/// that parent, and a child default shapes one named child's request and
/// grants the parent nothing.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ExecutionParentInput<R> {
    /// A ceiling on what a child of this target may request.
    ChildSupportCeiling(ChildSupportCeiling),
    /// The parent's own consumption: a desired request whose consumer is the
    /// parent.
    ParentUse(R),
    /// Defaults applied to one named child's request only.
    ChildRequestDefaults(ChildRequestDefaults),
}

/// The classification an execution-parent input carries.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutionParentInputClass {
    /// A child target-support ceiling.
    ChildSupportCeiling,
    /// The parent's own consumption.
    ParentUse,
    /// Defaults for one child's request.
    ChildRequestDefaults,
}

impl<R> ExecutionParentInput<R> {
    /// Return this input's classification.
    pub const fn class(&self) -> ExecutionParentInputClass {
        match self {
            Self::ChildSupportCeiling(_) => ExecutionParentInputClass::ChildSupportCeiling,
            Self::ParentUse(_) => ExecutionParentInputClass::ParentUse,
            Self::ChildRequestDefaults(_) => ExecutionParentInputClass::ChildRequestDefaults,
        }
    }

    /// Borrow the child support ceiling, when this input is one.
    pub const fn support_ceiling(&self) -> Option<&ChildSupportCeiling> {
        match self {
            Self::ChildSupportCeiling(ceiling) => Some(ceiling),
            _ => None,
        }
    }

    /// Borrow the parent's own desired request, when this input is one.
    pub const fn parent_use(&self) -> Option<&R> {
        match self {
            Self::ParentUse(request) => Some(request),
            _ => None,
        }
    }

    /// Borrow the child defaults, when this input is one.
    pub const fn child_defaults(&self) -> Option<&ChildRequestDefaults> {
        match self {
            Self::ChildRequestDefaults(defaults) => Some(defaults),
            _ => None,
        }
    }

    /// Whether this input produces a binding relationship at all.
    ///
    /// Only a parent's own consumption does. A support ceiling and a child
    /// default are inputs to someone else's request, never relationships of
    /// their own.
    pub const fn yields_binding(&self) -> bool {
        matches!(self, Self::ParentUse(_))
    }

    /// Whether this input admits a child's binding request.
    ///
    /// Only a support ceiling can, and only for the capabilities it lists.
    pub fn admits_child_request(
        &self,
        kind: BindingKind,
        rights: RequestedRights,
    ) -> Result<(), BindingRefusal> {
        match self {
            Self::ChildSupportCeiling(ceiling) if ceiling.admits(kind, rights) => Ok(()),
            _ => Err(BindingRefusal::new(
                AdmissionStage::Authorize,
                RefusalReason::TargetSupportMissing,
            )),
        }
    }
}