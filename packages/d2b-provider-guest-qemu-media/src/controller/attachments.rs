//! QEMU's private descriptor slots, projected from admitted relationships.
//!
//! The runner's descriptor list is a REALIZATION of the Guest's own graph
//! relationships, not a second attachment list. Every private slot names the
//! `BindingKey` that admitted it, the source's own `SourceReservation`, the
//! right the source admitted, and the arbitration that right was granted
//! under, so no private slot can select a source, a consumer, or an access
//! mode the graph did not admit (R16-R24, R34).
//!
//! Admission is re-evaluated here through the same pure evaluator every other
//! composition step uses. A request whose source decision, realization
//! support, dependency fence, or observed lifecycle does not admit it produces
//! no slot at all: the refusal is the result, and it is produced before any
//! descriptor, leg, or process state exists (R35, R40, R41).
//!
//! Nothing here resolves a host path, opens a device node, reads a launch
//! argument, or names a socket. A slot names a relationship; the effect
//! adapter holding the broker-minted reservation handle is the only thing that
//! turns one into a descriptor (R34, R37).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use d2b_contracts_resource::v3::{
    BindingArbitration, BindingAuthorization, BindingContractError, BindingEvidence, BindingKey,
    BindingKind, BindingLifecycleState, BindingObservation, BindingRealizationFacet,
    BindingRealizationSupport, BindingRefusal, BindingSlot, BoundedToken, CompletionCondition,
    FreshnessTuple, RefusalReason, ReleaseOutcome, RequestedRights, ResourceRef, ResourceUid,
    SourceAdmission, SourceReservation, admit_binding_request,
    device_binding::{DeviceAttachmentMode, DeviceBindingRequest},
    endpoint_binding::{EndpointAttachmentKind, EndpointBindingRequest},
    network_binding::NetworkBindingRequest,
    volume::AttachmentAccess,
    volume_binding::{VolumeBindingRequest, VolumePresentation},
    ZoneId,
};
use sha2::{Digest, Sha256};

use crate::controller::process_builder::AttachmentKind;
use crate::types::MAX_REMOVABLE_VOLUMES;

/// The private slot label the KVM acceleration descriptor occupies.
pub const KVM_SLOT: &str = "kvm";
/// The private slot label the network tap descriptor occupies.
pub const TAP_SLOT: &str = "tap-0";
/// The private slot label the display descriptor occupies.
pub const DISPLAY_SLOT: &str = "display";
/// The prefix a media block descriptor's slot label carries.
pub const MEDIA_SLOT_PREFIX: &str = "media-";
/// The device function this Provider requires of a KVM acceleration claim.
pub const KVM_FUNCTION: &str = "kvm-acceleration";
/// The endpoint purpose this Provider requires of a display attachment.
///
/// The purpose is a declared, provider-owned token rather than a hostname
/// convention: an endpoint admitted for a compositor session, a pipe, or a
/// telemetry collector is not a display just because its socket exists
/// (R23, AE7).
pub const DISPLAY_PURPOSE: &str = "guest-display";

/// The realization facets this Provider declares it can realize.
///
/// A request whose presentation needs a facet outside this set is refused at
/// the prepare stage by the shared evaluator, not skipped: an unapplied
/// attachment is not a smaller success (R19-R23, R42).
pub const QEMU_MEDIA_REALIZATION_FACETS: &[BindingRealizationFacet] = &[
    BindingRealizationFacet::ConsumerDeviceSlot,
    BindingRealizationFacet::DeviceAttachment,
    BindingRealizationFacet::EndpointDescriptor,
    BindingRealizationFacet::EndpointPathname,
    BindingRealizationFacet::NamespaceInterface,
    BindingRealizationFacet::SharedFabric,
];

/// The realization support this Provider declares.
pub fn qemu_media_realization_support() -> BindingRealizationSupport {
    BindingRealizationSupport::new(QEMU_MEDIA_REALIZATION_FACETS.to_vec())
        .expect("the declared facet set is duplicate-free")
}

// ---------------------------------------------------------------------------
// Declared requests
// ---------------------------------------------------------------------------

/// One attachment input this Guest declared, in its canonical typed form.
///
/// The four variants are the four relationship kinds a QEMU media Guest
/// realizes as a private descriptor. The request is the desired declaration
/// the graph admitted; nothing here carries a host path, a device node, a
/// socket pathname, or a command line (R16, R21-R24).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaRequest {
    /// The acceleration Device whose descriptor opens `/dev/kvm`.
    Kvm(DeviceBindingRequest),
    /// The Network whose tap descriptor becomes the Guest's NIC.
    Tap(NetworkBindingRequest),
    /// One removable or boot media Volume presented as a block descriptor.
    Media(VolumeBindingRequest),
    /// The display Endpoint whose descriptor connects the Guest's output.
    Display(EndpointBindingRequest),
}

impl MediaRequest {
    /// Return the binding kind this request belongs to.
    pub const fn kind(&self) -> BindingKind {
        match self {
            Self::Kvm(_) => BindingKind::Device,
            Self::Tap(_) => BindingKind::Network,
            Self::Media(_) => BindingKind::Volume,
            Self::Display(_) => BindingKind::Endpoint,
        }
    }

    /// Borrow the exact source this request names.
    pub const fn source_ref(&self) -> &ResourceRef {
        match self {
            Self::Kvm(request) => request.source_ref(),
            Self::Tap(request) => request.source_ref(),
            Self::Media(request) => request.source_ref(),
            Self::Display(request) => request.source_ref(),
        }
    }

    /// Borrow the exact consumer this request names.
    pub const fn consumer_ref(&self) -> &ResourceRef {
        match self {
            Self::Kvm(request) => request.consumer_ref(),
            Self::Tap(request) => request.consumer_ref(),
            Self::Media(request) => request.consumer_ref(),
            Self::Display(request) => request.consumer_ref(),
        }
    }

    /// Borrow the stable consumer slot this request occupies.
    pub const fn slot(&self) -> &BindingSlot {
        match self {
            Self::Kvm(request) => request.slot(),
            Self::Tap(request) => request.slot(),
            Self::Media(request) => request.slot(),
            Self::Display(request) => request.slot(),
        }
    }

    /// Return the right this request asks the source to admit.
    pub const fn requested_rights(&self) -> RequestedRights {
        match self {
            Self::Kvm(request) => request.requested_rights(),
            Self::Tap(request) => request.requested_rights(),
            Self::Media(request) => request.requested_rights(),
            Self::Display(request) => request.requested_rights(),
        }
    }

    /// Return the realization facets this request depends on.
    pub const fn required_facets(&self) -> &'static [BindingRealizationFacet] {
        match self {
            Self::Kvm(request) => request.required_facets(),
            Self::Tap(request) => request.required_facets(),
            Self::Media(request) => request.required_facets(),
            Self::Display(request) => request.required_facets(),
        }
    }

    /// Derive this relationship's KTD3 key from its committed identities.
    ///
    /// # Errors
    ///
    /// Returns [`MediaAdmissionError::InvalidRequest`] when the typed request
    /// cannot name a key for the given identities.
    pub fn key(
        &self,
        zone: ZoneId,
        source_uid: ResourceUid,
        consumer_uid: ResourceUid,
    ) -> Result<BindingKey, MediaAdmissionError> {
        let key = match self {
            Self::Kvm(request) => request.key(zone, source_uid, consumer_uid),
            Self::Tap(request) => request.key(zone, source_uid, consumer_uid),
            Self::Media(request) => request.key(zone, source_uid, consumer_uid),
            Self::Display(request) => request.key(zone, source_uid, consumer_uid),
        };
        key.map_err(MediaAdmissionError::InvalidRequest)
    }

    /// Return the private slot label this request occupies.
    ///
    /// # Errors
    ///
    /// Returns [`MediaAdmissionError::UnsupportedAccess`] when the request's
    /// presentation does not become a private descriptor at all: a mediated
    /// Device has no descriptor to hand the runner, and a filesystem Volume
    /// presentation is a mount rather than a block device.
    pub fn slot_label(&self) -> Result<String, MediaAdmissionError> {
        match self {
            Self::Kvm(_) => Ok(KVM_SLOT.to_owned()),
            Self::Tap(_) => Ok(TAP_SLOT.to_owned()),
            Self::Media(request) => match request.presentation() {
                VolumePresentation::BlockDevice { device_slot } => {
                    Ok(format!("{MEDIA_SLOT_PREFIX}{device_slot}"))
                }
                VolumePresentation::Filesystem { .. } => {
                    Err(MediaAdmissionError::UnsupportedAccess)
                }
            },
            Self::Display(_) => Ok(DISPLAY_SLOT.to_owned()),
        }
    }

    /// Return the consumer-side device slot a block presentation occupies.
    pub const fn device_slot(&self) -> Option<u16> {
        match self {
            Self::Media(request) => match request.presentation() {
                VolumePresentation::BlockDevice { device_slot } => Some(*device_slot),
                VolumePresentation::Filesystem { .. } => None,
            },
            _ => None,
        }
    }

    /// Check the access mode this Provider realizes as a private descriptor.
    ///
    /// The rules are closed and provider-owned:
    ///
    /// - the KVM Device must be claimed for the declared acceleration
    ///   function and delivered as a descriptor, because a mediated
    ///   attachment is realized by its own provider and has nothing for the
    ///   runner to open;
    /// - a media Volume must be a block presentation, and must not be a
    ///   shared write, because a QEMU block descriptor is a single writer and
    ///   a shared-write claim names simultaneous writers the descriptor
    ///   cannot be;
    /// - the display Endpoint must carry this Provider's declared purpose and
    ///   must be reached in the consume direction, because a listening
    ///   attachment makes the runner a server for a display it was given.
    fn check_access_mode(&self) -> Result<(), MediaAdmissionError> {
        match self {
            Self::Kvm(request) => {
                if request.attachment() != DeviceAttachmentMode::Descriptor
                    || request.function().as_str() != KVM_FUNCTION
                {
                    return Err(MediaAdmissionError::UnsupportedAccess);
                }
            }
            Self::Tap(_) => {}
            Self::Media(request) => {
                if request.presentation().device_slot().is_none()
                    || request.access() == AttachmentAccess::SharedWrite
                {
                    return Err(MediaAdmissionError::UnsupportedAccess);
                }
            }
            Self::Display(request) => {
                if request.purpose().as_str() != DISPLAY_PURPOSE
                    || request.attachment() == EndpointAttachmentKind::Listen
                {
                    return Err(MediaAdmissionError::UnsupportedAccess);
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Declared request plus the source's own decision
// ---------------------------------------------------------------------------

/// One relationship the Guest declared, with the source's own decision.
///
/// The source provider is the only party that admits a request, so its
/// decision arrives with the request rather than being re-derived here. The
/// dependency tuples are the exact rows the decision was evaluated against and
/// the observed tuples are the current rows, so a source view, consumer, or
/// provider-assignment change that never advanced a spec generation still
/// invalidates the earlier use (R35, AE16).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedRelationship {
    request: MediaRequest,
    authorization: BindingAuthorization,
    source: SourceAdmission,
    support: BindingRealizationSupport,
    reservation: SourceReservation,
    dependencies: Vec<FreshnessTuple>,
    observed: Vec<FreshnessTuple>,
    state: BindingLifecycleState,
    observation: BindingObservation,
}

/// The inputs one relationship's evidence is assembled from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationshipEvidence {
    /// Whether the graph authorized this consumer for this relationship.
    pub authorization: BindingAuthorization,
    /// The source provider's own decision.
    pub source: SourceAdmission,
    /// The realization facets the admitting implementation declared.
    pub support: BindingRealizationSupport,
    /// The source-owned reservation identity.
    pub reservation: SourceReservation,
    /// The rows the decision was evaluated against.
    pub dependencies: Vec<FreshnessTuple>,
    /// The rows currently committed.
    pub observed: Vec<FreshnessTuple>,
    /// The observed lifecycle of the relationship.
    pub state: BindingLifecycleState,
}

impl AdmittedRelationship {
    /// Pair one declared request with the source's decision about it.
    pub fn new(request: MediaRequest, evidence: RelationshipEvidence) -> Self {
        let observation = BindingObservation::new(
            evidence.state,
            CompletionCondition::Pending,
            CompletionCondition::Pending,
            ReleaseOutcome::Outstanding,
        );
        Self {
            request,
            authorization: evidence.authorization,
            source: evidence.source,
            support: evidence.support,
            reservation: evidence.reservation,
            dependencies: evidence.dependencies,
            observed: evidence.observed,
            state: evidence.state,
            observation,
        }
    }

    /// Record the consumer-side completion the source observed.
    #[must_use]
    pub fn observed(mut self, observation: BindingObservation) -> Self {
        self.observation = observation;
        self
    }

    /// Borrow the declared request.
    pub const fn request(&self) -> &MediaRequest {
        &self.request
    }

    /// Return the observed lifecycle of the relationship.
    pub const fn state(&self) -> BindingLifecycleState {

        self.state
    }

    /// Borrow the latest observation of the relationship.
    pub const fn observation(&self) -> &BindingObservation {
        &self.observation
    }

    /// Evaluate the admission and return the evidence the relationship holds.
    ///
    /// # Errors
    ///
    /// Returns the first refusal in order: an access mode this Provider does
    /// not realize, a key the request cannot derive, a Zone outside the
    /// Guest's, a consumer that is not this Guest, a source decision made for
    /// another source or slot, an unauthorized subject, a right or a facet
    /// the source did not admit, a dependency fence that no longer matches,
    /// and finally a lifecycle that does not admit new use.
    fn admit(
        &self,
        zone: &ZoneId,
        guest_ref: &ResourceRef,
        guest_uid: &ResourceUid,
    ) -> Result<BindingEvidence, MediaAdmissionError> {
        self.request.check_access_mode()?;
        let key = self.source.binding();
        if key.zone() != zone {
            return Err(MediaAdmissionError::ForeignZone);
        }
        if key.consumer_ref() != guest_ref || key.consumer_uid() != guest_uid {
            return Err(MediaAdmissionError::ForeignConsumer);
        }
        let expected = self.request.key(
            zone.clone(),
            key.source_uid().clone(),
            guest_uid.clone(),
        )?;
        if expected != *key {
            return Err(MediaAdmissionError::UnprovenSource);
        }
        let admission = admit_binding_request(
            key,
            self.request.requested_rights(),
            self.request.required_facets(),
            &self.authorization,
            &self.source,
            &self.support,
            &self.dependencies,
        )
        .map_err(MediaAdmissionError::Admission)?;
        if !admission.is_current(&self.observed) {
            return Err(MediaAdmissionError::StaleAuthority);
        }
        if !self.state.admits_new_use() {
            return Err(MediaAdmissionError::StaleAuthority);
        }
        let evidence = BindingEvidence::admitted(admission, self.reservation.clone())
            .observed(self.observation);
        if evidence.state() != self.state {
            return Err(MediaAdmissionError::StaleAuthority);
        }
        Ok(evidence)
    }
}

// ---------------------------------------------------------------------------
// The Guest's realized relationship set
// ---------------------------------------------------------------------------

/// Every relationship one Guest declared, with each source's own decision.
///
/// The set is keyed by the Guest's committed identity, so a relationship
/// admitted for another consumer is not merely unused here - it is refused,
/// and a replaced consumer identity cannot inherit the previous one (R16,
/// R35, R41).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GuestMediaBindings {
    zone: Option<ZoneId>,
    guest_uid: Option<ResourceUid>,
    relationships: Vec<AdmittedRelationship>,
}

impl GuestMediaBindings {
    /// Construct the set for one Guest.
    pub fn new(
        zone: ZoneId,
        guest_uid: ResourceUid,
        relationships: impl IntoIterator<Item = AdmittedRelationship>,
    ) -> Self {
        Self {
            zone: Some(zone),
            guest_uid: Some(guest_uid),
            relationships: relationships.into_iter().collect(),
        }
    }

    /// Return whether the set names a Guest at all.
    pub const fn is_empty(&self) -> bool {
        self.zone.is_none() || self.relationships.is_empty()
    }

    /// Borrow every declared relationship.
    pub fn relationships(&self) -> &[AdmittedRelationship] {
        &self.relationships
    }

    /// Project the set onto the private descriptor slots this Guest's own spec
    /// requires.
    ///
    /// # Errors
    ///
    /// Returns the first refusal any relationship produces, and returns
    /// [`MediaAdmissionError::MissingBinding`] when a required relationship
    /// is absent. The projection is total: it either returns a complete
    /// descriptor list, or it returns nothing at all, so a partially prepared
    /// Guest never reaches an effect (R34, R40).
    pub fn project(
        &self,
        guest_ref: &ResourceRef,
        requirements: SlotRequirements,
    ) -> Result<AdmittedAttachments, MediaAdmissionError> {
        let (Some(zone), Some(guest_uid)) = (&self.zone, &self.guest_uid) else {
            return Err(MediaAdmissionError::MissingBinding);
        };
        let mut kvm = None;
        let mut tap = None;
        let mut media = Vec::new();
        let mut display = None;
        for relationship in &self.relationships {
            let evidence = relationship.admit(zone, guest_ref, guest_uid)?;
            let attachment = AdmittedAttachment::new(relationship, evidence)?;
            match attachment.kind() {
                AttachmentKind::Kvm => {
                    if kvm.replace(attachment).is_some() {
                        return Err(MediaAdmissionError::DuplicateSlot);
                    }
                }
                AttachmentKind::Tap => {
                    if tap.replace(attachment).is_some() {
                        return Err(MediaAdmissionError::DuplicateSlot);
                    }
                }
                AttachmentKind::Media => media.push(attachment),
                _ => {
                    if display.replace(attachment).is_some() {
                        return Err(MediaAdmissionError::DuplicateSlot);
                    }
                }
            }
        }
        if media.len() > MAX_REMOVABLE_VOLUMES {
            return Err(MediaAdmissionError::LimitExceeded);
        }
        media.sort_by_key(AdmittedAttachment::device_slot);
        for (index, attachment) in media.iter().enumerate() {
            if attachment.device_slot() != Some(index as u16) {
                // A media descriptor's private label is its own consumer
                // device slot, so a gap or a duplicate in that numbering is
                // refused rather than renumbered: the runner's `-drive`
                // indices are the admitted ones, not a re-packaging of them.
                return Err(MediaAdmissionError::UnprovenSource);
            }
        }
        if requirements.kvm && kvm.is_none() {
            return Err(MediaAdmissionError::MissingBinding);
        }
        if requirements.tap && tap.is_none() {
            return Err(MediaAdmissionError::MissingBinding);
        }
        if media.len() < requirements.media {
            return Err(MediaAdmissionError::MissingBinding);
        }
        if requirements.display && display.is_none() {
            return Err(MediaAdmissionError::MissingBinding);
        }
        let mut attachments = Vec::with_capacity(
            3 + media.len() + usize::from(kvm.is_some()) + usize::from(tap.is_some())
                + usize::from(display.is_some()),
        );
        attachments.extend(kvm);
        attachments.extend(tap);
        attachments.extend(media);
        attachments.extend(display);
        Ok(AdmittedAttachments {
            guest_ref: guest_ref.clone(),
            attachments,
        })
    }
}

/// What the Guest's own spec requires its private descriptor list to hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SlotRequirements {
    /// The acceleration Device must be present.
    pub kvm: bool,
    /// The network tap must be present.
    pub tap: bool,
    /// At least this many media block descriptors must be present.
    pub media: usize,
    /// The display descriptor must be present.
    pub display: bool,
}

impl SlotRequirements {
    /// Derive the requirements from the runner's own Process contract and the
    /// Guest's provider settings.
    ///
    /// The requirements come from the same two declared specs that used to
    /// name the attachment list directly, so the private list is now a
    /// projection of the spec rather than a second copy of it (R15, R16).
    pub fn derive(
        network_declared: bool,
        media_count: usize,
        display_window: bool,
    ) -> Self {
        Self {
            kvm: true,
            tap: network_declared,
            media: media_count,
            display: display_window,
        }
    }
}

// ---------------------------------------------------------------------------
// Projected private slots
// ---------------------------------------------------------------------------

/// One private descriptor and the admitted relationship that authorizes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedAttachment {
    label: String,
    kind: AttachmentKind,
    key: BindingKey,
    reservation: SourceReservation,
    right: RequestedRights,
    arbitration: BindingArbitration,
    device_slot: Option<u16>,
    state: BindingLifecycleState,
    evidence: BindingEvidence,
}

impl AdmittedAttachment {
    fn new(
        relationship: &AdmittedRelationship,
        evidence: BindingEvidence,
    ) -> Result<Self, MediaAdmissionError> {
        let label = relationship.request().slot_label()?;
        let admission = evidence.admission();
        Ok(Self {
            label,
            kind: attachment_kind(relationship.request()),
            key: admission.key().clone(),
            reservation: evidence.reservation().clone(),
            right: admission.rights(),
            arbitration: admission.arbitration(),
            device_slot: relationship.request().device_slot(),
            state: evidence.state(),
            evidence,
        })
    }

    /// Borrow the private slot label.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Return the descriptor class this slot delivers.
    pub const fn kind(&self) -> AttachmentKind {
        self.kind
    }

    /// Borrow the admitted relationship this slot realizes.
    pub const fn key(&self) -> &BindingKey {
        &self.key
    }

    /// Borrow the source's own reservation for that relationship.
    pub const fn reservation(&self) -> &SourceReservation {
        &self.reservation
    }

    /// Return the right the source admitted.
    pub const fn right(&self) -> RequestedRights {
        self.right
    }

    /// Return how the source arbitrates that right.
    pub const fn arbitration(&self) -> BindingArbitration {
        self.arbitration
    }

    /// Return the consumer-side device slot a media descriptor occupies.
    pub const fn device_slot(&self) -> Option<u16> {
        self.device_slot
    }

    /// Return the observed lifecycle of the relationship.
    pub const fn state(&self) -> BindingLifecycleState {
        self.state
    }

    /// Borrow the admitted evidence behind this slot.
    pub const fn evidence(&self) -> &BindingEvidence {
        &self.evidence
    }

    /// Derive the runner's realization leg on this slot's own reservation.
    ///
    /// The leg is the Guest's relationship realized by its helper. It is
    /// never a second claim: the source, the reservation, and the relationship
    /// identity are all the parent's, and the only thing the helper adds is
    /// its own identity on that reservation (AE27).
    ///
    /// # Errors
    ///
    /// Returns [`MediaAdmissionError::UnattenuatedRight`] when the parent
    /// holds a right that has no attenuated helper form.
    pub fn leg(&self) -> Result<Option<ImplementationLeg>, MediaAdmissionError> {
        match self.right() {
            // An observation has no helper form: the runner reads the source
            // through the parent's own descriptor and takes no leg of its own.
            RequestedRights::Observe => Ok(None),
            _ => ImplementationLeg::derive(self).map(Some),
        }
    }
}

const fn attachment_kind(request: &MediaRequest) -> AttachmentKind {
    match request {
        MediaRequest::Kvm(_) => AttachmentKind::Kvm,
        MediaRequest::Tap(_) => AttachmentKind::Tap,
        MediaRequest::Media(_) => AttachmentKind::Media,
        MediaRequest::Display(_) => AttachmentKind::Display,
    }
}

/// The complete private descriptor list a Guest's runner is launched with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedAttachments {
    guest_ref: ResourceRef,
    attachments: Vec<AdmittedAttachment>,
}

impl AdmittedAttachments {
    /// Borrow the Guest these descriptors belong to.
    pub const fn guest_ref(&self) -> &ResourceRef {
        &self.guest_ref
    }

    /// Borrow every projected descriptor, in launch order.
    pub fn attachments(&self) -> &[AdmittedAttachment] {
        &self.attachments
    }

    /// Borrow the acceleration descriptor, when one was admitted.
    pub fn kvm(&self) -> Option<&AdmittedAttachment> {
        self.find(AttachmentKind::Kvm)
    }

    /// Borrow the tap descriptor, when one was admitted.
    pub fn tap(&self) -> Option<&AdmittedAttachment> {
        self.find(AttachmentKind::Tap)
    }

    /// Borrow the display descriptor, when one was admitted.
    pub fn display(&self) -> Option<&AdmittedAttachment> {
        self.find(AttachmentKind::Display)
    }

    /// Borrow every media descriptor, in device-slot order.
    pub fn media(&self) -> impl Iterator<Item = &AdmittedAttachment> {
        self.attachments
            .iter()
            .filter(|attachment| attachment.kind == AttachmentKind::Media)
    }

    /// Return every private slot label, in launch order.
    pub fn labels(&self) -> Vec<&str> {
        self.attachments
            .iter()
            .map(AdmittedAttachment::label)
            .collect()
    }

    /// Derive one realization leg per descriptor that admits a helper right.
    pub fn legs(&self) -> Result<Vec<ImplementationLeg>, MediaAdmissionError> {
        self.attachments
            .iter()
            .map(ImplementationLeg::derive)
            .collect()
    }

    fn find(&self, kind: AttachmentKind) -> Option<&AdmittedAttachment> {
        self.attachments
            .iter()
            .find(|attachment| attachment.kind == kind)
    }
}

// ---------------------------------------------------------------------------
// The runner's realization leg
// ---------------------------------------------------------------------------

/// One helper's attenuated realization of a Guest's own reservation.
///
/// The VMM runner is a helper: it opens the descriptors the Guest's bindings
/// already reserved. It therefore holds a LEG of that reservation, never a
/// second reservation, a competing writer, or a second device allocation
/// (R5, R21, R38-R40, AE27).
///
/// `rights_attenuated_under` mirrors the reservation service's own ordering:
/// observation has no helper form, and each writing or arbitrating right is
/// only under itself. A leg asking for more than its parent was admitted for
/// is refused here before it is ever named on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImplementationLeg {
    /// The relationship the leg realizes. This is the Guest's own binding.
    pub parent: BindingKey,
    /// This helper's identity on the parent's reservation.
    pub identity: BoundedToken,
    /// The exact source, copied from the parent and never supplied by the
    /// helper.
    pub source_uid: ResourceUid,
    /// The parent's own source-owned reservation identity.
    pub reservation: SourceReservation,
    /// The permitted operation subset, never wider than the parent's right.
    pub rights: RequestedRights,
}

impl ImplementationLeg {
    /// Derive the leg this Provider's runner holds on one admitted slot.
    ///
    /// # Errors
    ///
    /// Returns [`MediaAdmissionError::UnattenuatedRight`] when the parent
    /// holds an observation right, which has no helper form.
    pub fn derive(parent: &AdmittedAttachment) -> Result<Self, MediaAdmissionError> {
        Self::with_rights(parent, parent.right())
    }

    /// Derive the leg with an explicit permitted operation subset.
    ///
    /// # Errors
    ///
    /// Returns [`MediaAdmissionError::UnattenuatedRight`] when `rights` is
    /// not attenuated under the parent's admitted right.
    pub fn with_rights(
        parent: &AdmittedAttachment,
        rights: RequestedRights,
    ) -> Result<Self, MediaAdmissionError> {
        if !rights_attenuated_under(rights, parent.right()) {
            return Err(MediaAdmissionError::UnattenuatedRight);
        }
        let identity = leg_identity(parent.key())?;
        Ok(Self {
            parent: parent.key().clone(),
            identity,
            source_uid: parent.key().source_uid().clone(),
            reservation: parent.reservation().clone(),
            rights,
        })
    }
}

/// Whether one right is attenuated under another.
///
/// This is the same ordering the reservation service enforces on a helper leg,
/// restated here so the Provider refuses an unattenuated leg at the point it
/// builds one instead of discovering it on the wire (AE27).
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

/// Derive one helper's identity on a parent relationship's reservation.
///
/// The identity is derived from the relationship's own key, so a helper that
/// restarts re-derives the same identity and a leg cannot be bound to a
/// second relationship by choosing a different name.
fn leg_identity(parent: &BindingKey) -> Result<BoundedToken, MediaAdmissionError> {
    let mut hasher = Sha256::new();
    hasher.update(b"d2b/qemu-media/realization-leg/v3");
    hasher.update(parent.zone().as_str().as_bytes());
    hasher.update([0_u8]);
    hasher.update(parent.kind().resource_type().as_bytes());
    hasher.update([0_u8]);
    hasher.update(parent.source_uid().as_str().as_bytes());
    hasher.update([0_u8]);
    hasher.update(parent.consumer_uid().as_str().as_bytes());
    hasher.update([0_u8]);
    hasher.update(parent.slot().as_str().as_bytes());
    let digest = hasher.finalize();
    let label = format!(
        "qemu-media-vmm-{}",
        digest[..6].iter().map(|byte| format!("{byte:02x}")).collect::<String>()
    );
    BoundedToken::parse(label).map_err(|_| {
        MediaAdmissionError::InvalidRequest(BindingContractError::InvalidField)
    })
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

/// A relationship the projection refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaAdmissionError {
    /// A typed request could not name a key for the given identities.
    InvalidRequest(BindingContractError),
    /// The relationship belongs to another Zone.
    ForeignZone,
    /// The relationship names another consumer.
    ForeignConsumer,
    /// The source's decision was made for another source, consumer, or slot.
    UnprovenSource,
    /// The access mode this Provider realizes does not cover the request.
    UnsupportedAccess,
    /// More than one relationship claims the same private slot.
    DuplicateSlot,
    /// The relationship set exceeds the declared media ceiling.
    LimitExceeded,
    /// A relationship the Guest's own spec requires is absent.
    MissingBinding,
    /// The admitted fence no longer matches the committed rows.
    StaleAuthority,
    /// A helper leg asked for a right its parent was not admitted for.
    UnattenuatedRight,
    /// The shared evaluator refused the request.
    Admission(BindingRefusal),
}

impl MediaAdmissionError {
    /// Return the stable Provider error code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest(_) => "runtime-qemu-media-invalid-binding-request",
            Self::ForeignZone => "runtime-qemu-media-foreign-zone",
            Self::ForeignConsumer => "runtime-qemu-media-foreign-consumer",
            Self::UnprovenSource => "runtime-qemu-media-unproven-source",
            Self::UnsupportedAccess => "runtime-qemu-media-unsupported-access-mode",
            Self::DuplicateSlot => "runtime-qemu-media-duplicate-slot",
            Self::LimitExceeded => "runtime-qemu-media-limit-exceeds-ceiling",
            Self::MissingBinding => "dependency-not-ready",
            Self::StaleAuthority => "runtime-qemu-media-stale-authority",
            Self::UnattenuatedRight => "runtime-qemu-media-unattenuated-leg",
            Self::Admission(refusal) => match refusal.reason() {
                RefusalReason::IdentityNotAuthorized => "runtime-qemu-media-identity-not-authorized",
                RefusalReason::MandatoryFacetUnsupported => "mandatory-facet-unsupported",
                RefusalReason::LimitExceedsCeiling => "limit-exceeds-ceiling",
                RefusalReason::SourcePolicyRefused => "source-policy-refused",
                RefusalReason::ConflictingDeclaration => "conflicting-declaration",
                RefusalReason::StaleAuthority => "runtime-qemu-media-stale-authority",
                RefusalReason::UnprovenEffect => "unproven-effect",
                _ => "runtime-qemu-media-binding-refused",
            },
        }
    }

    /// Return the enforcing stage, when the shared evaluator named one.
    pub const fn stage(&self) -> Option<d2b_contracts_resource::v3::AdmissionStage> {
        match self {
            Self::Admission(refusal) => Some(refusal.stage()),
            _ => None,
        }
    }
}

impl core::fmt::Display for MediaAdmissionError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for MediaAdmissionError {}

impl From<BindingContractError> for MediaAdmissionError {
    fn from(error: BindingContractError) -> Self {
        Self::InvalidRequest(error)
    }
}
impl AdmittedAttachments {
    /// Return the distinct private slot labels, refusing a collision.
    ///
    /// # Errors
    ///
    /// Returns [`MediaAdmissionError::DuplicateSlot`] when two descriptors
    /// claim the same label.
    pub fn validate_labels(&self) -> Result<BTreeSet<&str>, MediaAdmissionError> {
        let mut labels = BTreeSet::new();
        for attachment in &self.attachments {
            if !labels.insert(attachment.label.as_str()) {
                return Err(MediaAdmissionError::DuplicateSlot);
            }
        }
        Ok(labels)
    }
}

// ---------------------------------------------------------------------------
// Hermetic admission evidence
// ---------------------------------------------------------------------------

/// Hermetic admission evidence for this crate's own tests.
///
/// The source provider's decision, its reservation, and the committed rows
/// that decision was evaluated against are owned by other layers, so a test
/// cannot produce them by naming a request. These builders mint the same
/// evidence a real source would admit, which is what lets the tests exercise
/// the refusals instead of asserting that a constructor returned `Ok`.
///
/// This is test support, not a Provider API: it is `doc(hidden)`, it is the
/// same shape as the crate's existing `for_test` identity helper, and it is
/// never reachable from a production effect path, which takes its
/// relationships from the watch path instead.
#[doc(hidden)]
pub mod test_fixtures {
    use d2b_contracts_resource::v3::{
        BindingArbitration, BindingAuthorization, BindingSlot, BindingLifecycleState,
        DesiredDigest, DesiredRevision, FreshnessTuple, SourceAdmission, SourceReservation,
        StoreIncarnation, device_binding::{
            DeviceAttachmentMode, DeviceBindingRequest, DeviceClaimRequest, DeviceFunction,
        },
        endpoint_binding::{EndpointAttachmentKind, EndpointBindingRequest},
        network_binding::{NetworkBindingRequest, NetworkMembership, NetworkPresentation},
        volume::AttachmentAccess,
        volume_binding::{VolumeBindingRequest, VolumePresentation},
        RequestedRights,
    };

    use super::{DISPLAY_PURPOSE, KVM_FUNCTION, qemu_media_realization_support};

    /// The Zone every fixture relationship belongs to.
    pub const FIXTURE_ZONE: &str = "corp";

    /// Build the fixture Zone.
    ///
    /// # Panics
    ///
    /// Panics when the fixture Zone name is rejected, which a constant cannot
    /// be.
    pub fn zone() -> d2b_contracts_resource::v3::ZoneId {
        d2b_contracts_resource::v3::ZoneId::parse(FIXTURE_ZONE).expect("fixture Zone id")
    }

    /// Build a deterministic row identity from a one-digit seed.
    ///
    /// # Panics
    ///
    /// Panics when the seed is outside `0..=9`, which a test constant is not.
    pub fn uid(seed: u8) -> d2b_contracts_resource::v3::ResourceUid {
        assert!(seed <= 9, "the fixture uid seed is one digit");
        d2b_contracts_resource::v3::ResourceUid::parse(format!(
            "0000000{seed}-0000-4000-8000-000000000000"
        ))
        .expect("a canonical UUIDv4")
    }

    /// Build the fixture Guest reference.
    ///
    /// # Panics
    ///
    /// Panics when the fixture Guest name is rejected, which a constant
    /// cannot be.
    pub fn guest_ref() -> d2b_contracts_resource::v3::ResourceRef {
        d2b_contracts_resource::v3::ResourceRef::parse("Guest/media-vm").expect("fixture Guest ref")
    }

    /// The fixture Guest's committed identity.
    pub fn guest_uid() -> d2b_contracts_resource::v3::ResourceUid {
        uid(1)
    }

    /// Build one committed row's freshness tuple.
    ///
    /// # Panics
    ///
    /// Panics when the Zone or resource name is rejected, which a test
    /// constant is not.
    pub fn freshness(
        resource_ref: &d2b_contracts_resource::v3::ResourceRef,
        uid: &d2b_contracts_resource::v3::ResourceUid,
    ) -> FreshnessTuple {
        FreshnessTuple::new(
            zone(),
            StoreIncarnation::parse("store-1").expect("fixture store incarnation"),
            resource_ref.clone(),
            uid.clone(),
            DesiredRevision::INITIAL,
            DesiredDigest::of(resource_ref.to_canonical_string().as_bytes()),
        )
    }

    fn slot(value: &str) -> BindingSlot {
        BindingSlot::parse(value).expect("a canonical consumer slot")
    }

    /// The acceleration Device request a qemu-media Guest declares.
    ///
    /// # Panics
    ///
    /// Panics when the fixture Device request is rejected, which a constant
    /// cannot be.
    pub fn kvm_request(consumer: &d2b_contracts_resource::v3::ResourceRef) -> super::MediaRequest {
        let request = DeviceBindingRequest::new(
            d2b_contracts_resource::v3::ResourceRef::parse("Device/host-kvm")
                .expect("fixture Device ref"),
            consumer.clone(),
            slot("acceleration"),
            DeviceFunction::parse(KVM_FUNCTION).expect("fixture device function"),
            DeviceClaimRequest::Shared,
            DeviceAttachmentMode::Descriptor,
        )
        .expect("a well-formed Device request");
        super::MediaRequest::Kvm(request)
    }

    /// The tap Network request a qemu-media Guest declares.
    ///
    /// # Panics
    ///
    /// Panics when the fixture Network request is rejected, which a constant
    /// cannot be.
    pub fn tap_request(consumer: &d2b_contracts_resource::v3::ResourceRef) -> super::MediaRequest {
        let request = NetworkBindingRequest::new(
            d2b_contracts_resource::v3::ResourceRef::parse("Network/corp-net")
                .expect("fixture Network ref"),
            consumer.clone(),
            slot("primary"),
            NetworkMembership::new(Vec::new(), true).expect("an empty inbound port set"),
            NetworkPresentation::namespace_interface("corp0").expect("a canonical interface name"),
        )
        .expect("a well-formed Network request");
        super::MediaRequest::Tap(request)
    }

    /// One media Volume request, presented in the named consumer device slot.
    ///
    /// # Panics
    ///
    /// Panics when the fixture Volume request is rejected, which a constant
    /// cannot be.
    pub fn media_request(
        consumer: &d2b_contracts_resource::v3::ResourceRef,
        name: &str,
        device_slot: u16,
    ) -> super::MediaRequest {
        let request = VolumeBindingRequest::new(
            d2b_contracts_resource::v3::ResourceRef::parse(format!("Volume/{name}").as_str())
                .expect("fixture Volume ref"),
            consumer.clone(),
            slot(name),
            d2b_contracts_resource::v3::BoundedToken::parse("root").expect("a canonical view name"),
            AttachmentAccess::ReadWrite,
            VolumePresentation::block_device(device_slot).expect("a bounded device slot"),
        )
        .expect("a well-formed Volume request");
        super::MediaRequest::Media(request)
    }

    /// The display Endpoint request a qemu-media Guest declares.
    ///
    /// # Panics
    ///
    /// Panics when the fixture Endpoint request is rejected, which a constant
    /// cannot be.
    pub fn display_request(
        consumer: &d2b_contracts_resource::v3::ResourceRef,
    ) -> super::MediaRequest {
        let request = EndpointBindingRequest::new(
            d2b_contracts_resource::v3::ResourceRef::parse("Endpoint/display")
                .expect("fixture Endpoint ref"),
            consumer.clone(),
            slot("display"),
            EndpointAttachmentKind::Connect,
            d2b_contracts_resource::v3::BoundedToken::parse(DISPLAY_PURPOSE)
                .expect("the declared display purpose"),
        )
        .expect("a well-formed Endpoint request");
        super::MediaRequest::Display(request)
    }

    /// The source's admitted evidence for one request.
    ///
    /// # Panics
    ///
    /// Panics when the fixture source decision is rejected, which a constant
    /// cannot be.
    pub fn admitted(
        request: super::MediaRequest,
        source_uid: &d2b_contracts_resource::v3::ResourceUid,
        consumer_uid: &d2b_contracts_resource::v3::ResourceUid,
    ) -> super::AdmittedRelationship {
        admitted_with(
            request,
            source_uid,
            consumer_uid,
            None,
            BindingLifecycleState::Admitted,
        )
    }

    /// The source's evidence for one request with an explicit decision.
    ///
    /// `rights` overrides which rights the source admits, so a test can prove
    /// that a right the source did not admit produces no descriptor.
    ///
    /// # Panics
    ///
    /// Panics when the fixture source decision is rejected, which a constant
    /// cannot be.
    pub fn admitted_with(
        request: super::MediaRequest,
        source_uid: &d2b_contracts_resource::v3::ResourceUid,
        consumer_uid: &d2b_contracts_resource::v3::ResourceUid,
        rights: Option<Vec<RequestedRights>>,
        state: BindingLifecycleState,
    ) -> super::AdmittedRelationship {
        let zone = zone();
        let key = request
            .key(zone.clone(), source_uid.clone(), consumer_uid.clone())
            .expect("the fixture request names a key");
        let source = SourceAdmission::new(
            key.clone(),
            rights.unwrap_or_else(|| vec![request.requested_rights()]),
            match request.requested_rights() {
                RequestedRights::Exclusive => BindingArbitration::Exclusive,
                _ => BindingArbitration::Shared,
            },
        )
        .expect("a well-formed source decision");
        let rows = vec![freshness(key.source_ref(), source_uid)];
        let reservation = SourceReservation::new(
            zone,
            source_uid.clone(),
            d2b_contracts_resource::v3::BoundedToken::parse("fixture-reservation")
                .expect("a canonical reservation id"),
        );
        super::AdmittedRelationship::new(
            request,
            super::RelationshipEvidence {
                authorization: BindingAuthorization::granted(),
                source,
                support: qemu_media_realization_support(),
                reservation,
                dependencies: rows.clone(),
                observed: rows,
                state,
            },
        )
    }
}

// ---------------------------------------------------------------------------
// The pre-graph declared list
// ---------------------------------------------------------------------------

/// The pre-graph declared descriptor list, retained until the daemon
/// composition supplies admitted evidence.
///
/// This is the shape `LaunchTicket` carried before the conversion: slots built
/// from the Guest's own declared attachment references, with no admitted
/// relationship behind any of them. It is kept ONLY so the unchanged daemon
/// composition keeps running; `LaunchTicket::declared` is the whole of it, and
/// U34 deletes this type together with the fields that feed it (R15, R49-R54).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttachmentSlot {
    /// Slot label.
    pub slot: String,
    /// Slot kind.
    pub kind: AttachmentKind,
    /// The reference the Guest's own spec declared.
    pub source_ref: ResourceRef,
}

/// The declared refs one pre-graph Guest carries.
///
/// These are the Guest's own desired attachment references, with no admitted
/// relationship behind any of them. They exist only so the unchanged daemon
/// composition keeps running; U34 deletes this type together with the snapshot
/// fields that feed it (R15, R49-R54).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeclaredAttachments {
    /// Declared media Volume refs.
    pub media_refs: Vec<ResourceRef>,
    /// Declared display Endpoint ref.
    pub display_ref: Option<ResourceRef>,
}

impl DeclaredAttachments {
    /// Project the declared refs onto the same private slot labels the
    /// admitted model produces, taking the acceleration Device and the tap
    /// Network from the runner's own Process contract.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessSpecError`](crate::ProcessSpecError) when a declared
    /// ref is not the ResourceType its slot delivers, when more than the
    /// declared media ceiling is declared, or when a media ref is duplicated.
    pub fn project(
        self,
        process: &d2b_contracts_resource::v3::ProcessSpec,
    ) -> Result<Vec<AttachmentSlot>, crate::ProcessSpecError> {
        use crate::ProcessSpecError;

        let Self {
            media_refs,
            display_ref,
        } = self;
        if media_refs.len() > crate::types::MAX_REMOVABLE_VOLUMES
            || media_refs
                .iter()
                .any(|reference| reference.resource_type().as_str() != "Volume")
            || {
                let mut seen = BTreeSet::new();
                media_refs.iter().any(|reference| !seen.insert(reference))
            }
        {
            return Err(ProcessSpecError::InvalidReference);
        }
        let mut slots = Vec::with_capacity(media_refs.len() + 3);
        if let Some(device_ref) = process.execution().device_usage().first() {
            slots.push(AttachmentSlot {
                slot: KVM_SLOT.to_owned(),
                kind: AttachmentKind::Kvm,
                source_ref: device_ref.device_ref().clone(),
            });
        }
        if let Some(network_ref) = process
            .execution()
            .network_usage()
            .and_then(|usage| usage.network_ref())
        {
            slots.push(AttachmentSlot {
                slot: TAP_SLOT.to_owned(),
                kind: AttachmentKind::Tap,
                source_ref: network_ref.clone(),
            });
        }
        for (index, reference) in media_refs.into_iter().enumerate() {
            slots.push(AttachmentSlot {
                slot: format!("{MEDIA_SLOT_PREFIX}{index}"),
                kind: AttachmentKind::Media,
                source_ref: reference,
            });
        }
        if let Some(reference) = display_ref {
            if reference.resource_type().as_str() != "Endpoint" {
                return Err(ProcessSpecError::InvalidReference);
            }
            slots.push(AttachmentSlot {
                slot: DISPLAY_SLOT.to_owned(),
                kind: AttachmentKind::Display,
                source_ref: reference,
            });
        }
        Ok(slots)
    }
}

/// The descriptor authorization one launch ticket carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchAttachments {
    /// Descriptors projected from admitted graph relationships.
    ///
    /// Every slot names the `BindingKey` that admitted it, so no private slot
    /// can select a source, a consumer, or an access mode the graph did not
    /// admit (R16, R34).
    Admitted(AdmittedAttachments),
    /// The pre-graph declared list, retained until the daemon composition is
    /// rewired. U34 deletes this variant.
    Declared(Vec<AttachmentSlot>),
}

impl LaunchAttachments {
    /// Return the private slot labels, in launch order.
    pub fn labels(&self) -> Vec<&str> {
        match self {
            Self::Admitted(attachments) => attachments.labels(),
            Self::Declared(slots) => slots.iter().map(|slot| slot.slot.as_str()).collect(),
        }
    }
}
