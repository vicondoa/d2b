//! Neutral `VolumeBinding` ResourceType contracts.
//!
//! One `VolumeBinding` is the durable neutral record of one Volume /
//! execution-target / named-view relationship.  It is a standard unqualified
//! core type: the Volume side mints it and owns the relationship, and the
//! serving Provider only observes it and writes its fenced status
//! projection.  The spec never carries a host path, socket path,
//! shared-directory path, argv, or numeric identity.
//!
//! [`VolumeBindingSpec`] is the committed row's one encoding.  The manager's
//! relation index, the source-side authority rebuild, and the registered
//! serving driver all read these exact bytes, so a row can never be indexed
//! as one relationship and served as another.  [`VolumeBindingRequest`] is
//! the consumer-side declaration the source admits; the committed row is
//! that declaration plus the source provider's own accepted decision.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    ResourceRef, ResourceUid,
    binding::{
        BindingConsumerKind, BindingContractError, BindingKey, BindingKind, BindingRealizationFacet,
        BindingSlot, BindingSourceDecision, BindingSpecFingerprint, ExecutionParentInput,
        MAX_CONSUMER_DEVICE_SLOT,
        RequestedRights,
    },
    execution_policy::{BoundedToken, PrimitiveSpecError, redacted_debug, require_resource_type},
    identity::{ResourceGeneration, ZoneId, ZoneRevision},
    resource_status::StatusCode,
    volume::{AttachmentAccess, validate_mount_path},
};
use d2b_contracts::wire_deserialize;

/// Canonical standard VolumeBinding ResourceType.
pub const VOLUME_BINDING_RESOURCE_TYPE: &str = "VolumeBinding";

/// Strict base VolumeBinding specification.
///
/// The row names the source Volume, the consumer, the named Volume view it
/// selects, the access level it was admitted at, the consumer-side
/// presentation that view takes, the stable consumer slot the relationship
/// occupies, and the source provider's accepted decision.  The presentation
/// is what makes a block-device attachment a representable row rather than a
/// filesystem destination invented for it, so the field is the closed
/// [`VolumePresentation`] vocabulary and never a bare path.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VolumeBindingSpec {
    volume_ref: ResourceRef,
    execution_ref: ResourceRef,
    view: BoundedToken,
    access: AttachmentAccess,
    presentation: VolumePresentation,
    slot: BoundedToken,
    source: BindingSourceDecision,
}

impl VolumeBindingSpec {
    /// Construct a strict binding specification from typed references.
    ///
    /// # Errors
    ///
    /// Refuses a source that is not a `Volume` (or a consumer this binding
    /// kind does not deliver to), a presentation whose own fields are not
    /// internally consistent, and a slot or view that is not a bounded token.
    pub fn new(
        volume_ref: ResourceRef,
        execution_ref: ResourceRef,
        view: impl Into<String>,
        access: AttachmentAccess,
        presentation: VolumePresentation,
        slot: impl Into<String>,
        source: BindingSourceDecision,
    ) -> Result<Self, PrimitiveSpecError> {
        // The consumer is whatever the binding kind admits, derived from the
        // kind's own rule rather than a per-family list. `BindingKind::Volume`
        // admits every consumer kind, so a Process or Host consumer is
        // legitimate here; hard-bounding this to `Guest` would refuse a
        // relationship the admission path already admits.
        super::binding::admit_binding_row_refs(
            super::binding::BindingKind::Volume,
            &volume_ref,
            &execution_ref,
        )
        .map_err(|_| PrimitiveSpecError::WrongResourceType)?;
        // The presentation arrives as a typed value whose variants are public,
        // so its own rule is enforced here rather than assumed: a row is
        // never committed over a destination or device slot the closed
        // vocabulary refuses.
        presentation
            .validate()
            .map_err(|error| match error {
                BindingContractError::OutOfRange => PrimitiveSpecError::OutOfRange,
                _ => PrimitiveSpecError::InvalidPath,
            })?;
        Ok(Self {
            volume_ref,
            execution_ref,
            view: BoundedToken::parse(view.into())?,
            access,
            presentation,
            slot: BoundedToken::parse(slot.into())?,
            source,
        })
    }

    /// Return the standard ResourceType name.
    pub const fn resource_type(&self) -> &'static str {
        VOLUME_BINDING_RESOURCE_TYPE
    }

    /// Borrow the bound Volume.
    pub const fn volume_ref(&self) -> &ResourceRef {
        &self.volume_ref
    }

    /// Borrow the Guest execution target.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }

    /// Borrow the named Volume view.
    pub const fn view(&self) -> &BoundedToken {
        &self.view
    }

    /// Return the requested access mode.
    pub const fn access(&self) -> AttachmentAccess {
        self.access
    }

    /// Borrow the consumer-side presentation this view takes.
    pub const fn presentation(&self) -> &VolumePresentation {
        &self.presentation
    }

    /// Borrow the stable consumer slot this attachment occupies.
    pub const fn slot(&self) -> &BoundedToken {
        &self.slot
    }

    /// Borrow the source provider's accepted decision for this relationship.
    pub const fn source(&self) -> &BindingSourceDecision {
        &self.source
    }

    /// The right this committed relationship asks the source for.
    pub const fn requested_rights(&self) -> RequestedRights {
        volume_access_rights(self.access)
    }

    /// The realization facets this committed presentation depends on.
    pub const fn required_facets(&self) -> &'static [BindingRealizationFacet] {
        self.presentation.required_facets()
    }

    /// The digest of this row's exact desired bytes.
    pub fn fingerprint(&self) -> BindingSpecFingerprint {
        BindingSpecFingerprint::from_request(self)
    }

    /// The KTD3 consumer slot this committed row occupies.
    fn slot_key(&self) -> Result<BindingSlot, BindingContractError> {
        BindingSlot::parse(self.slot.as_str()).map_err(|_| BindingContractError::InvalidCollection)
    }

    /// Derive this committed relationship's KTD3 key from its identities.
    ///
    /// The same derivation the source-side request performs, over the row's own
    /// committed references and slot, so a boundary evaluating a committed row
    /// reaches exactly the key the source admitted.
    pub fn key(
        &self,
        zone: ZoneId,
        source_uid: ResourceUid,
        consumer_uid: ResourceUid,
    ) -> Result<BindingKey, BindingContractError> {
        BindingKey::new(
            zone,
            BindingKind::Volume,
            self.volume_ref.clone(),
            source_uid,
            self.execution_ref.clone(),
            consumer_uid,
            self.slot_key()?,
        )
    }

    /// The consumer-side declaration this committed row carries.
    ///
    /// The row is that declaration plus the source provider's own accepted
    /// decision, so a reader that wants the relationship rather than the
    /// decision reads it back out of the same bytes instead of keeping a
    /// second encoding of the relationship.
    ///
    /// # Errors
    ///
    /// Returns [`BindingContractError`] when the committed identities are not
    /// one this binding family admits.
    pub fn request(&self) -> Result<VolumeBindingRequest, BindingContractError> {
        VolumeBindingRequest::new(
            self.volume_ref.clone(),
            self.execution_ref.clone(),
            self.slot_key()?,
            self.view.clone(),
            self.access,
            self.presentation.clone(),
        )
    }

    /// Admit one binding create or update against its owner reference.
    ///
    /// Every binding mutation must be owned by an existing Volume: a create
    /// or update without a Volume owner reference is a direct external
    /// create and is rejected.
    pub fn admit_owner_ref(owner_ref: Option<&ResourceRef>) -> Result<(), PrimitiveSpecError> {
        match owner_ref {
            Some(owner) if owner.resource_type().as_str() == "Volume" => Ok(()),
            Some(_) => Err(PrimitiveSpecError::WrongResourceType),
            None => Err(PrimitiveSpecError::MissingRequiredField),
        }
    }
}

impl core::fmt::Debug for VolumeBindingSpec {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("VolumeBindingSpec(<redacted>)")
    }
}

wire_deserialize!(
    VolumeBindingSpec,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        volume_ref: ResourceRef,
        execution_ref: ResourceRef,
        view: String,
        access: AttachmentAccess,
        presentation: VolumePresentation,
        slot: String,
        source: BindingSourceDecision,
    },
    wire,
    Self::new(
        wire.volume_ref,
        wire.execution_ref,
        wire.view,
        wire.access,
        wire.presentation,
        wire.slot,
        wire.source,
    )
    .map_err(serde::de::Error::custom)
);

// ---------------------------------------------------------------------------
// The graph-era VolumeBinding request.
//
// `VolumeBindingRequest` is the canonical desired request KTD2 places in the
// consumer's own spec. It is the one authoritative declaration of one
// Volume/view/consumer relationship: a Process mount, an EphemeralProcess
// mount, and a Host or Guest attachment all normalize into this shape
// instead of authoring a second list. It names the source Volume by exact
// reference, never a raw host path, and it names the consumer by exact
// reference, never a numerical principal. It is what the consumer authors;
// the committed row above is what the source mints from it, so the two
// carry the same relationship in the two roles it has.
// ---------------------------------------------------------------------------

/// The consumer-side presentation one admitted Volume view takes.
///
/// The destination is a location inside the consumer. It is never the source:
/// a host source path is resolved privately from the admitted source and its
/// named view, never authored here.
// `rename_all_fields` is what makes the serialized form match the wire form
// this enum's decoder accepts.  Without it the block variant emits
// `device_slot` while `wire_deserialize!` reads `deviceSlot`, so a committed
// canonical request carrying a block presentation could not be read back and
// its row would declare no relationship at all.  The `schemars` attribute
// repeats the casing on the variant because schemars 0.8 reads `rename_all`
// on a variant but not the container-level `rename_all_fields`.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "presentation"
)]
pub enum VolumePresentation {
    /// The exact named view presented at a destination inside the consumer.
    #[serde(rename = "filesystem")]
    Filesystem {
        /// The consumer-side destination path.
        destination: String,
    },
    /// The exact named view presented as a block device in a consumer slot.
    #[serde(rename = "block-device")]
    #[schemars(rename_all = "camelCase")]
    BlockDevice {
        /// The consumer-side device slot.
        device_slot: u16,
    },
}

impl VolumePresentation {
    /// Construct a filesystem presentation after validating its destination.
    pub fn filesystem(destination: impl Into<String>) -> Result<Self, BindingContractError> {
        let presentation = Self::Filesystem { destination: destination.into() };
        presentation.validate()?;
        Ok(presentation)
    }

    /// Construct a block presentation after checking its slot bound.
    pub fn block_device(device_slot: u16) -> Result<Self, BindingContractError> {
        let presentation = Self::BlockDevice { device_slot };
        presentation.validate()?;
        Ok(presentation)
    }

    /// Whether this presentation's own fields are internally consistent.
    ///
    /// The variants are public, so a presentation built by field rather than
    /// by constructor is still checked here: a committed row is never written
    /// over a destination or device slot the closed vocabulary refuses.
    pub fn validate(&self) -> Result<(), BindingContractError> {
        match self {
            Self::Filesystem { destination } => {
                if validate_mount_path(destination) {
                    Ok(())
                } else {
                    Err(BindingContractError::InvalidField)
                }
            }
            Self::BlockDevice { device_slot } => {
                if *device_slot > MAX_CONSUMER_DEVICE_SLOT {
                    Err(BindingContractError::OutOfRange)
                } else {
                    Ok(())
                }
            }
        }
    }

    /// The realization facet this presentation requires.
    pub const fn required_facets(&self) -> &'static [BindingRealizationFacet] {
        match self {
            Self::Filesystem { .. } => &[BindingRealizationFacet::FilesystemPresentation],
            Self::BlockDevice { .. } => &[BindingRealizationFacet::ConsumerDeviceSlot],
        }
    }

    /// Borrow the consumer-side destination of a filesystem presentation.
    pub const fn destination(&self) -> Option<&str> {
        match self {
            Self::Filesystem { destination } => Some(destination.as_str()),
            Self::BlockDevice { .. } => None,
        }
    }

    /// The consumer-side device slot of a block presentation.
    pub const fn device_slot(&self) -> Option<u16> {
        match self {
            Self::BlockDevice { device_slot } => Some(*device_slot),
            Self::Filesystem { .. } => None,
        }
    }
}

redacted_debug!(VolumePresentation);

wire_deserialize!(
    VolumePresentation,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        presentation: String,
        destination: Option<String>,
        device_slot: Option<u16>,
    },
    wire,
    match (wire.presentation.as_str(), wire.destination, wire.device_slot) {
        ("filesystem", Some(destination), None) => Self::filesystem(destination),
        ("block-device", None, Some(device_slot)) => Self::block_device(device_slot),
        _ => Err(BindingContractError::InvalidField),
    }
    .map_err(serde::de::Error::custom)
);

/// The desired request for one Volume view used by one consumer.
///
/// This is the canonical declaration site under KTD2: the consumer's own
/// desired spec states the relationship here, the source Volume admits it,
/// and the admitted binding is the source-owned row. The source owns writer
/// arbitration across every realization of the view, so a Process mount and a
/// Guest attachment cannot each obtain a writer independently.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VolumeBindingRequest {
    source_ref: ResourceRef,
    consumer_ref: ResourceRef,
    slot: BindingSlot,
    view: BoundedToken,
    access: AttachmentAccess,
    presentation: VolumePresentation,
}

impl VolumeBindingRequest {
    /// Construct one request from typed references.
    ///
    /// # Errors
    ///
    /// Refuses a source that is not a `Volume`, a consumer that is not one of
    /// the four admitted consumer kinds, and an access level this binding
    /// kind does not admit.
    pub fn new(
        source: ResourceRef,
        consumer: ResourceRef,
        slot: BindingSlot,
        view: BoundedToken,
        access: AttachmentAccess,
        presentation: VolumePresentation,
    ) -> Result<Self, BindingContractError> {
        require_resource_type(&source, BindingKind::Volume.source_resource_type())?;
        let consumer_kind = BindingConsumerKind::from_resource_type(consumer.resource_type().as_str())
            .ok_or(BindingContractError::WrongResourceType)?;
        if !BindingKind::Volume.admits_consumer(consumer_kind) {
            return Err(BindingContractError::UnsupportedConsumerKind);
        }
        if !BindingKind::Volume.admits_rights(volume_access_rights(access)) {
            return Err(BindingContractError::UnsupportedRight);
        }
        Ok(Self {
            source_ref: source,
            consumer_ref: consumer,
            slot,
            view,
            access,
            presentation,
        })
    }

    /// The binding kind this request belongs to.
    pub const fn kind(&self) -> BindingKind {
        BindingKind::Volume
    }

    /// Borrow the exact source Volume.
    pub const fn source_ref(&self) -> &ResourceRef {
        &self.source_ref
    }

    /// Borrow the exact consumer.
    pub const fn consumer_ref(&self) -> &ResourceRef {
        &self.consumer_ref
    }

    /// Borrow the stable consumer slot.
    pub const fn slot(&self) -> &BindingSlot {
        &self.slot
    }

    /// Borrow the named Volume view.
    pub const fn view(&self) -> &BoundedToken {
        &self.view
    }

    /// Return the requested access level.
    pub const fn access(&self) -> AttachmentAccess {
        self.access
    }

    /// Borrow the consumer-side presentation.
    pub const fn presentation(&self) -> &VolumePresentation {
        &self.presentation
    }

    /// The right this request asks the source to admit.
    pub const fn requested_rights(&self) -> RequestedRights {
        volume_access_rights(self.access)
    }

    /// The realization facets this request depends on.
    pub const fn required_facets(&self) -> &'static [BindingRealizationFacet] {
        self.presentation.required_facets()
    }

    /// Derive this relationship's KTD3 key from its committed identities.
    pub fn key(
        &self,
        zone: ZoneId,
        source_uid: ResourceUid,
        consumer_uid: ResourceUid,
    ) -> Result<BindingKey, BindingContractError> {
        BindingKey::new(
            zone,
            self.kind(),
            self.source_ref.clone(),
            source_uid,
            self.consumer_ref.clone(),
            consumer_uid,
            self.slot.clone(),
        )
    }

    /// The digest of this request's exact desired bytes.
    pub fn fingerprint(&self) -> BindingSpecFingerprint {
        BindingSpecFingerprint::from_request(self)
    }
}

redacted_debug!(VolumeBindingRequest);

wire_deserialize!(
    VolumeBindingRequest,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        source_ref: ResourceRef,
        consumer_ref: ResourceRef,
        slot: BindingSlot,
        view: BoundedToken,
        access: AttachmentAccess,
        presentation: VolumePresentation,
    },
    wire,
    Self::new(
        wire.source_ref,
        wire.consumer_ref,
        wire.slot,
        wire.view,
        wire.access,
        wire.presentation,
    )
    .map_err(serde::de::Error::custom)
);

/// A Host or Guest volume attachment input, classified.
///
/// The old flattened fragment carried one overloaded attachment list. This
/// alias carries the conversion's result instead: a child support ceiling, a
/// parent use, or defaults for one named child.
pub type VolumeExecutionParentInput = ExecutionParentInput<VolumeBindingRequest>;

/// The shared right one Volume access level requests.
const fn volume_access_rights(access: AttachmentAccess) -> RequestedRights {
    match access {
        AttachmentAccess::ReadOnly => RequestedRights::Observe,
        AttachmentAccess::ReadWrite => RequestedRights::Mutate,
        AttachmentAccess::SharedWrite => RequestedRights::Share,
    }
}


/// The UID / generation / revision fence on readiness evidence.
///
/// A readiness report is only current when the UID and generation match the
/// binding's own identity and the fence revision is at most the stored
/// revision.  Every store mutation (including the status write carrying the
/// report itself) advances the stored revision past the observed one, so an
/// exact-revision rule could never latch; the UID pins reassignment and the
/// generation pins spec changes, which together bound report staleness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VolumeBindingReadinessFence {
    /// The binding UID the evidence was observed under.
    pub uid: ResourceUid,
    /// The binding spec generation the evidence was observed under.
    pub generation: ResourceGeneration,
    /// The Zone-store revision the evidence was observed under.
    pub revision: ZoneRevision,
}

impl VolumeBindingReadinessFence {
    /// Whether this fence still matches the binding's current identity.
    pub fn matches(
        &self,
        uid: &ResourceUid,
        generation: ResourceGeneration,
        revision: ZoneRevision,
    ) -> bool {
        self.uid == *uid && self.generation == generation && self.revision <= revision
    }
}

/// Public VolumeBinding status resource projection.
///
/// It exposes only fenced readiness and a stable safe failure code.  The
/// worker, endpoint, socket, resolved paths, argv, and numeric identities
/// stay provider-private and never appear here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VolumeBindingStatusResource {
    /// Whether the serving side reports the relationship ready.
    pub ready: bool,
    /// The fence the readiness evidence was observed under.
    pub fence: VolumeBindingReadinessFence,
    /// The stable safe failure code, when not ready.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<StatusCode>,
}

impl VolumeBindingStatusResource {
    /// Whether this projection reports ready under the current fence.
    ///
    /// Readiness without a matching UID and generation is never current
    /// (fail-closed).  The fence revision only needs to precede the stored
    /// revision: the status write carrying the report advances the store
    /// past the observed commit.
    pub fn readiness_is_current(
        &self,
        uid: &ResourceUid,
        generation: ResourceGeneration,
        revision: ZoneRevision,
    ) -> bool {
        self.ready && self.fence.matches(uid, generation, revision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v3::binding::BindingArbitration;

    fn volume_ref() -> ResourceRef {
        ResourceRef::parse("Volume/work-state").expect("valid fixture ref")
    }

    fn guest_ref() -> ResourceRef {
        ResourceRef::parse("Guest/work-vm").expect("valid fixture ref")
    }

    /// The source decision these fixtures commit: a read-only view a Guest may
    /// observe. Every construction site below states the same decision, so the
    /// tests differ only in what they are testing.
    fn decision() -> BindingSourceDecision {
        BindingSourceDecision::new(
            vec![RequestedRights::Observe],
            BindingArbitration::Shared,
            vec![BindingRealizationFacet::FilesystemPresentation],
        )
        .expect("decision validates")
    }

    fn spec() -> VolumeBindingSpec {
        VolumeBindingSpec::new(
            volume_ref(),
            guest_ref(),
            "ro-store",
            AttachmentAccess::ReadOnly,
            VolumePresentation::filesystem("/nix/.ro-store").expect("valid destination"),
            "store",
            decision(),
        )
        .expect("valid fixture spec")
    }

    fn uid() -> ResourceUid {
        ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("valid fixture uid")
    }

    fn fence() -> VolumeBindingReadinessFence {
        VolumeBindingReadinessFence {
            uid: uid(),
            generation: ResourceGeneration::new(1).expect("nonzero generation"),
            revision: ZoneRevision::new(9),
        }
    }

    #[test]
    fn spec_serializes_and_round_trips_strictly() {
        let value = serde_json::to_value(spec()).expect("spec serializes");
        assert_eq!(
            value,
            serde_json::json!({
                "volumeRef": "Volume/work-state",
                "executionRef": "Guest/work-vm",
                "view": "ro-store",
                "access": "read-only",
                "presentation": {
                    "presentation": "filesystem",
                    "destination": "/nix/.ro-store",
                },
                "slot": "store",
                "source": {
                    "admittedRights": ["observe"],
                    "arbitration": "shared",
                    "realizedFacets": ["filesystem-presentation"],
                },
            })
        );
        let parsed: VolumeBindingSpec =
            serde_json::from_value(value).expect("spec round trips");
        assert_eq!(parsed, spec());
    }

    /// The committed row is the declaration the consumer authored plus the
    /// source provider's accepted decision, so reading it back as the
    /// consumer's request must reproduce that declaration exactly.
    #[test]
    fn a_committed_row_reads_back_as_the_declaration_it_was_minted_from() {
        let row = spec();
        let request = row.request().expect("the row declares an admitted relationship");
        assert_eq!(request.source_ref(), row.volume_ref());
        assert_eq!(request.consumer_ref(), row.execution_ref());
        assert_eq!(request.slot().as_str(), row.slot().as_str());
        assert_eq!(request.view(), row.view());
        assert_eq!(request.access(), row.access());
        assert_eq!(request.presentation(), row.presentation());
        assert_eq!(request.requested_rights(), row.requested_rights());
        assert_eq!(request.required_facets(), row.required_facets());
    }

    /// A block-device attachment is a row with its own presentation, not a
    /// filesystem destination invented for it: the committed row carries the
    /// device slot and reads back with no destination at all.
    #[test]
    fn a_block_device_attachment_commits_its_device_slot_and_no_destination() {
        let row = VolumeBindingSpec::new(
            volume_ref(),
            ResourceRef::parse("Process/worker").expect("valid fixture ref"),
            "raw-store",
            AttachmentAccess::ReadOnly,
            VolumePresentation::block_device(1).expect("bounded device slot"),
            "store",
            decision(),
        )
        .expect("a block presentation is an admitted row");
        assert_eq!(row.presentation().device_slot(), Some(1));
        assert_eq!(row.presentation().destination(), None);
        assert_eq!(row.requested_rights(), RequestedRights::Observe);
        assert_eq!(row.required_facets(), &[BindingRealizationFacet::ConsumerDeviceSlot]);
        let request = row.request().expect("the row declares an admitted relationship");
        assert_eq!(request.presentation().device_slot(), Some(1));
        assert_eq!(request.presentation().destination(), None);

        let value = serde_json::to_value(&row).expect("spec serializes");
        assert_eq!(
            value["presentation"],
            serde_json::json!({ "presentation": "block-device", "deviceSlot": 1 })
        );
        let parsed: VolumeBindingSpec = serde_json::from_value(value).expect("spec round trips");
        assert_eq!(parsed, row);
    }

    #[test]
    fn spec_rejects_unknown_fields_and_wrong_references() {
        let mut unknown = serde_json::to_value(spec()).expect("spec serializes");
        unknown["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<VolumeBindingSpec>(unknown).is_err());

        let wrong_volume = VolumeBindingSpec::new(
            ResourceRef::parse("Guest/work-vm").expect("valid fixture ref"),
            guest_ref(),
            "ro-store",
            AttachmentAccess::ReadOnly,
            VolumePresentation::filesystem("/nix/.ro-store").expect("valid destination"),
            "store",
            decision(),
        );
        assert_eq!(wrong_volume.unwrap_err(), PrimitiveSpecError::WrongResourceType);

        // The presentation is validated at the row boundary too, so a value
        // built by field rather than by constructor cannot be committed.
        let invalid_path = VolumeBindingSpec::new(
            volume_ref(),
            guest_ref(),
            "ro-store",
            AttachmentAccess::ReadOnly,
            VolumePresentation::Filesystem { destination: "relative/path".to_owned() },
            "store",
            decision(),
        );
        assert_eq!(invalid_path.unwrap_err(), PrimitiveSpecError::InvalidPath);

        let invalid_slot = VolumeBindingSpec::new(
            volume_ref(),
            guest_ref(),
            "ro-store",
            AttachmentAccess::ReadOnly,
            VolumePresentation::filesystem("/nix/.ro-store").expect("valid destination"),
            "Not A Slot",
            decision(),
        );
        assert_eq!(invalid_slot.unwrap_err(), PrimitiveSpecError::InvalidToken);
    }

    #[test]
    fn admission_rejects_direct_external_creates_without_a_volume_owner() {
        assert!(VolumeBindingSpec::admit_owner_ref(Some(&volume_ref())).is_ok());
        assert_eq!(
            VolumeBindingSpec::admit_owner_ref(None).unwrap_err(),
            PrimitiveSpecError::MissingRequiredField,
            "owner-less binding create must be rejected"
        );
        assert_eq!(
            VolumeBindingSpec::admit_owner_ref(Some(&guest_ref())).unwrap_err(),
            PrimitiveSpecError::WrongResourceType,
            "a non-Volume owner reference must be rejected"
        );
    }

    #[test]
    fn fence_structural_validity_is_enforced_by_typed_fields() {
        let value = serde_json::to_value(fence()).expect("fence serializes");
        assert_eq!(
            value,
            serde_json::json!({
                "uid": "123e4567-e89b-42d3-a456-426614174000",
                "generation": 1,
                "revision": 9,
            })
        );
        assert!(
            serde_json::from_value::<VolumeBindingReadinessFence>(value).is_ok(),
            "a structurally valid fence parses"
        );

        let mut malformed = serde_json::to_value(fence()).expect("fence serializes");
        malformed["uid"] = serde_json::json!("not-a-uid");
        assert!(
            serde_json::from_value::<VolumeBindingReadinessFence>(malformed).is_err(),
            "a fence with a malformed UID must be rejected"
        );

        let mut zeroed = serde_json::to_value(fence()).expect("fence serializes");
        zeroed["generation"] = serde_json::json!(0);
        assert!(
            serde_json::from_value::<VolumeBindingReadinessFence>(zeroed).is_err(),
            "a fence with a zero generation must be rejected"
        );

        let mut unknown = serde_json::to_value(fence()).expect("fence serializes");
        unknown["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<VolumeBindingReadinessFence>(unknown).is_err());
    }

    #[test]
    fn stale_or_reassigned_readiness_never_reports_current() {
        let status = VolumeBindingStatusResource {
            ready: true,
            fence: fence(),
            reason: None,
        };
        assert!(status.readiness_is_current(
            &uid(),
            ResourceGeneration::new(1).expect("nonzero generation"),
            ZoneRevision::new(9),
        ));

        // Wrong UID: a reassigned binding name must not report ready.
        let reassigned = ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001")
            .expect("valid fixture uid");
        assert!(!status.readiness_is_current(
            &reassigned,
            ResourceGeneration::new(1).expect("nonzero generation"),
            ZoneRevision::new(9),
        ));

        // Older generation: a superseded spec must not report ready.
        assert!(!status.readiness_is_current(
            &uid(),
            ResourceGeneration::new(2).expect("nonzero generation"),
            ZoneRevision::new(9),
        ));

        // Newer stored revision: later store commits (including the status
        // write carrying this report) do not invalidate the evidence, so
        // the report stays current while UID and generation match.
        assert!(status.readiness_is_current(
            &uid(),
            ResourceGeneration::new(1).expect("nonzero generation"),
            ZoneRevision::new(10),
        ));

        // Future-dated fence: evidence claiming a commit newer than the
        // stored resource is forged or corrupt, never current.
        assert!(!status.readiness_is_current(
            &uid(),
            ResourceGeneration::new(1).expect("nonzero generation"),
            ZoneRevision::new(8),
        ));

        // Not ready stays not ready under any fence.
        let pending = VolumeBindingStatusResource {
            ready: false,
            fence: fence(),
            reason: Some(StatusCode::parse("view-not-found").expect("valid code")),
        };
        assert!(!pending.readiness_is_current(
            &uid(),
            ResourceGeneration::new(1).expect("nonzero generation"),
            ZoneRevision::new(9),
        ));
    }

    #[test]
    fn status_serialization_exposes_no_paths_sockets_argv_or_numeric_identities() {
        let status = VolumeBindingStatusResource {
            ready: true,
            fence: fence(),
            reason: None,
        };
        let rendered = serde_json::to_string(&status).expect("status serializes");
        let banned = [
            "mountPath",
            "/nix",
            "socket",
            "Socket",
            "argv",
            "Argv",
            "path",
            "Path",
            "sharedDir",
            "endpoint",
            "worker",
            "Worker",
            // Numeric host identities never appear; the only UID is the
            // binding's own opaque fence evidence below.
            "uid=",
            "gid=",
            "processRef",
        ];
        for entry in banned {
            assert!(
                !rendered.contains(entry),
                "status must not expose {entry}: {rendered}"
            );
        }
        // The fence UID is opaque evidence, not a numeric identity, but it
        // is the only identity-shaped field and must appear exactly once as
        // the fence value.
        let value = serde_json::to_value(&status).expect("status serializes");
        assert_eq!(
            value["fence"]["uid"],
            serde_json::json!("123e4567-e89b-42d3-a456-426614174000")
        );
        assert_eq!(value["ready"], serde_json::json!(true));
        assert!(value.get("reason").is_none());

        let parsed: VolumeBindingStatusResource =
            serde_json::from_value(serde_json::to_value(&status).expect("serializes"))
                .expect("status round trips");
        assert_eq!(parsed, status);
    }
    #[test]
    fn status_rejects_unknown_fields() {
        let mut unknown = serde_json::to_value(VolumeBindingStatusResource {
            ready: false,
            fence: fence(),
            reason: None,
        })
        .expect("status serializes");
        unknown["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<VolumeBindingStatusResource>(unknown).is_err());
    }
}
