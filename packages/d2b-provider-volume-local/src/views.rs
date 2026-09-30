//! Named views, right intersection, and attachment admission.
//!
//! A mount or attachment always selects a named view; it never names a
//! Volume subtree directly. volume-local is the sole Volume writer and
//! the sole admitter of attachments, so the single-writer and
//! shared-write rules are enforced here before any binding is minted.

use d2b_contracts_resource::v3::ResourceRef;
use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::volume::{
    AttachmentAccess, AttachmentSettings, AttachmentTransport, ViewRight, ViewSpec,
    VolumeAttachment, VolumeSpec,
};
use d2b_contracts_resource::v3::{
    AdmissionStage, BindingConsumerKind, BindingKind, BindingRefusal, BindingSlot,
    ChildBindingRequest, ChildRequestDefaults, ChildSupportCeiling, RefusalReason, RequestedRights,
    VolumeBindingRequest, VolumePresentation,
};

use crate::error::VolumeLocalError;

/// Resolve a named view of a Volume.
pub fn resolve_view<'spec>(
    spec: &'spec VolumeSpec,
    view: &BoundedToken,
) -> Result<&'spec ViewSpec, VolumeLocalError> {
    spec.views()
        .get(view.as_str())
        .ok_or(VolumeLocalError::ViewNotFound)
}

/// The rights an access level requires from the selected view.
const fn required_rights(access: AttachmentAccess) -> &'static [ViewRight] {
    match access {
        AttachmentAccess::ReadOnly => &[ViewRight::Read, ViewRight::Traverse],
        AttachmentAccess::ReadWrite | AttachmentAccess::SharedWrite => {
            &[ViewRight::Read, ViewRight::Write, ViewRight::Traverse]
        }
    }
}

/// Check that a view grants every right the requested access needs.
pub fn admit_access(view: &ViewSpec, access: AttachmentAccess) -> Result<(), VolumeLocalError> {
    if required_rights(access)
        .iter()
        .all(|right| view.rights().contains(right))
    {
        Ok(())
    } else {
        Err(VolumeLocalError::ViewRightsInsufficient)
    }
}

/// One admitted virtiofs attachment, ready to become one owned VolumeBinding.
///
/// It carries only typed references and the selected view name. The
/// resolved host path, the serving socket path, and the numeric socket
/// group never appear here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentPlan {
    /// The Host or Guest the Volume is exported to.
    pub execution_ref: ResourceRef,
    /// The selected named view.
    pub view: BoundedToken,
    /// The admitted access level.
    pub access: AttachmentAccess,
    /// The guest-side mount path copied from the base attachment.
    pub mount_path: String,
    /// The typed base mount settings copied from the base attachment.
    pub settings: AttachmentSettings,
}

/// Rejects a view that does not exist, an access level the view's rights
/// do not cover, a second simultaneous writer, `shared-write` when
/// the selected attachment Provider does not declare it, serving settings
/// outside the frozen serving default, and a second virtiofs attachment
/// naming a (guest, mount path) pair that an earlier attachment already
/// claimed (AE5).
///
/// # Errors
///
/// Returns [`VolumeLocalError::ViewNotFound`] when an attachment names an
/// undeclared view, [`VolumeLocalError::ViewRightsInsufficient`] when the
/// requested access exceeds the view's rights,
/// [`VolumeLocalError::SingleWriterConflict`] for a second simultaneous
/// writer, [`VolumeLocalError::SharedWriteUnsupported`] when the Provider
/// does not declare shared write,
/// [`VolumeLocalError::AttachmentSettingsUnsupported`] for non-default
/// virtiofs serving settings, and
/// [`VolumeLocalError::DuplicateMountPath`] when two virtiofs attachments
/// claim the same guest mount path.
pub fn admit_attachments(
    spec: &VolumeSpec,
    supports_shared_write: bool,
) -> Result<Vec<AttachmentPlan>, VolumeLocalError> {
    let mut writers = 0usize;
    let mut plans: Vec<AttachmentPlan> = Vec::with_capacity(spec.attachments().len());
    for attachment in spec.attachments() {
        let view = resolve_view(spec, attachment.view())?;
        admit_access(view, attachment.access())?;
        match attachment.access() {
            AttachmentAccess::ReadWrite => writers += 1,
            AttachmentAccess::SharedWrite if !supports_shared_write => {
                return Err(VolumeLocalError::SharedWriteUnsupported);
            }
            _ => {}
        }
        if writers > 1 {
            return Err(VolumeLocalError::SingleWriterConflict);
        }
        if attachment.transport() == AttachmentTransport::Virtiofs {
            if attachment.settings() != &AttachmentSettings::default() {
                return Err(VolumeLocalError::AttachmentSettingsUnsupported);
            }
            if plans.iter().any(|plan| {
                plan.execution_ref == *attachment.execution_ref()
                    && plan.mount_path == attachment.mount_path()
            }) {
                return Err(VolumeLocalError::DuplicateMountPath);
            }
            plans.push(AttachmentPlan {
                execution_ref: attachment.execution_ref().clone(),
                view: attachment.view().clone(),
                access: attachment.access(),
                mount_path: attachment.mount_path().to_owned(),
                settings: attachment.settings().clone(),
            });
        }
    }
    Ok(plans)
}

/// Whether one attachment is served read-only.
///
/// An attachment is read-only when it declares `read-only` access or when
/// the selected view grants no write right, so a view that never granted
/// write cannot be widened by the attachment.
pub fn is_read_only(view: &ViewSpec, attachment: &VolumeAttachment) -> bool {
    attachment.access() == AttachmentAccess::ReadOnly || !view.rights().contains(&ViewRight::Write)
}

// ---------------------------------------------------------------------------
// The graph-era source-side view admission.
//
// The functions above answer the old question - may this declared attachment
// be served? - and keep serving it, because the unchanged production entry
// point still calls them.  The ones below answer the canonical question for
// the one relationship vocabulary U2 defined: may this consumer's
// `VolumeBindingRequest` be admitted?  A Process mount, an EphemeralProcess
// mount, and a Host or Guest attachment all take this path, so the consumer
// kind is the only thing that differs between them.
//
// Refusals here are `BindingRefusal`: a stage and a reason from the shared
// contract, never a second error vocabulary of this crate's own.
// ---------------------------------------------------------------------------

/// Every consumer kind a Volume binding relationship delivers to.
pub const VOLUME_CONSUMER_KINDS: [BindingConsumerKind; 4] = BindingConsumerKind::ALL;

/// Build one refusal of the shared contract.
const fn refusal(stage: AdmissionStage, reason: RefusalReason) -> BindingRefusal {
    BindingRefusal::new(stage, reason)
}

/// Classify one consumer reference against the Volume binding kind.
///
/// Per-kind consumer eligibility is [`BindingKind::Volume`]'s own decision,
/// read here instead of matched on at the call site, so a family that gains
/// or loses a consumer kind changes this once.
///
/// # Errors
///
/// Returns `Authorize` / `IdentityNotAuthorized` when the reference names no
/// consumer at all, and `Authorize` / `TargetSupportMissing` when the
/// reference names a consumer the Volume binding kind does not deliver to.
pub fn admit_consumer_kind(consumer: &ResourceRef) -> Result<BindingConsumerKind, BindingRefusal> {
    let kind = BindingConsumerKind::from_resource_type(consumer.resource_type().as_str())
        .ok_or_else(|| {
            refusal(
                AdmissionStage::Authorize,
                RefusalReason::IdentityNotAuthorized,
            )
        })?;
    if !BindingKind::Volume.admits_consumer(kind) {
        return Err(refusal(
            AdmissionStage::Authorize,
            RefusalReason::TargetSupportMissing,
        ));
    }
    Ok(kind)
}

/// The exact subdirectory one named view presents inside the Volume.
///
/// This is the declared `ViewSpec` path, never the Volume root substituted
/// for it: a view that names `data` presents `data`.  The empty string is a
/// view that genuinely declares the root, and it is distinguishable from a
/// view that declared a subtree.
///
/// # Errors
///
/// Returns `Admit` / `SourcePolicyRefused` when the Volume declares no such
/// view.  The host path this subtree resolves to is resolved privately at
/// realization time and never appears here.
pub fn view_subdirectory<'spec>(
    spec: &'spec VolumeSpec,
    view: &BoundedToken,
) -> Result<&'spec str, BindingRefusal> {
    resolve_view(spec, view)
        .map(ViewSpec::path)
        .map_err(|_| refusal(AdmissionStage::Admit, RefusalReason::SourcePolicyRefused))
}

/// The rights one declared view grants, in the binding vocabulary.
///
/// `Observe` follows the read right and `Mutate` and `Share` both follow the
/// write right, because a shared writer writes.  Whether this source admits
/// `Share` at all is a separate source-side decision, taken by the admission
/// that carries the profile.
pub fn view_requested_rights(view: &ViewSpec) -> Vec<RequestedRights> {
    let mut rights = Vec::with_capacity(3);
    if view.rights().contains(&ViewRight::Read) {
        rights.push(RequestedRights::Observe);
    }
    if view.rights().contains(&ViewRight::Write) {
        rights.push(RequestedRights::Mutate);
        rights.push(RequestedRights::Share);
    }
    rights
}

/// Admit one canonical request against the exact view it names.
///
/// # Errors
///
/// Returns `Admit` / `SourcePolicyRefused` when the request names an
/// undeclared view or asks for access the view's declared rights do not
/// cover.  A read-only request against a view that grants no write is
/// admitted; a mutating request against that same view is not, because a
/// view that never granted write cannot be widened by the request.
pub fn admit_view_request<'spec>(
    spec: &'spec VolumeSpec,
    request: &VolumeBindingRequest,
) -> Result<&'spec ViewSpec, BindingRefusal> {
    let view = resolve_view(spec, request.view())
        .map_err(|_| refusal(AdmissionStage::Admit, RefusalReason::SourcePolicyRefused))?;
    admit_access(view, request.access())
        .map_err(|_| refusal(AdmissionStage::Admit, RefusalReason::SourcePolicyRefused))?;
    Ok(view)
}

/// Whether two relationships of one consumer collide at their destination.
///
/// Destinations are consumer-local, so this compares only relationships that
/// name the same consumer: two filesystems collide when they name the same
/// path and two block presentations when they claim the same device slot.  A
/// filesystem destination and a device slot never collide with each other.
pub fn destination_collides(
    left: &VolumeBindingRequest,
    right: &VolumeBindingRequest,
) -> bool {
    if left.consumer_ref() != right.consumer_ref() {
        return false;
    }
    match (left.presentation(), right.presentation()) {
        (
            VolumePresentation::Filesystem {
                destination: left_destination,
            },
            VolumePresentation::Filesystem {
                destination: right_destination,
            },
        ) => left_destination == right_destination,
        (
            VolumePresentation::BlockDevice {
                device_slot: left_slot,
            },
            VolumePresentation::BlockDevice {
                device_slot: right_slot,
            },
        ) => left_slot == right_slot,
        _ => false,
    }
}

/// The access level one binding right asks the source for.
///
/// This is the inverse of the contract's own right mapping, used when a
/// parent's classified input carries a right rather than an access level.
/// A right no Volume access level requests has no inverse and returns `None`.
pub const fn access_for_rights(rights: RequestedRights) -> Option<AttachmentAccess> {
    match rights {
        RequestedRights::Observe => Some(AttachmentAccess::ReadOnly),
        RequestedRights::Mutate => Some(AttachmentAccess::ReadWrite),
        RequestedRights::Share => Some(AttachmentAccess::SharedWrite),
        RequestedRights::Consume | RequestedRights::Exclusive => None,
    }
}

/// Normalize one child's draft into the single canonical request the
/// source admits.
///
/// The three meanings a Host or Guest fragment used to carry in one
/// attachment list stay separate here, as U2's classified types: a
/// [`ChildSupportCeiling`] bounds admission and creates nothing, a
/// [`ChildRequestDefaults`] fills an unset field of exactly the child it
/// names, and neither widens the child's own declaration.
///
/// # Errors
///
/// Returns `Authorize` / `ConflictingDeclaration` when the defaults name a
/// different consumer than the draft, `Admit` / `SourcePolicyRefused` when
/// the draft still names no source or no named view after normalization, and
/// `Authorize` / `TargetSupportMissing` when the ceiling does not admit the
/// right the normalized request asks for.
pub fn normalize_consumer_request(
    draft: &ChildBindingRequest,
    ceiling: Option<&ChildSupportCeiling>,
    defaults: Option<&ChildRequestDefaults>,
    slot: BindingSlot,
    presentation: VolumePresentation,
) -> Result<VolumeBindingRequest, BindingRefusal> {
    let normalized = match defaults {
        Some(defaults) => draft.apply_defaults(defaults).map_err(|_| {
            refusal(
                AdmissionStage::Authorize,
                RefusalReason::ConflictingDeclaration,
            )
        })?,
        None => draft.clone(),
    };
    let rights = normalized.rights().ok_or_else(|| {
        refusal(AdmissionStage::Admit, RefusalReason::SourcePolicyRefused)
    })?;
    if let Some(ceiling) = ceiling {
        ceiling
            .admits(BindingKind::Volume, rights)
            .then_some(())
            .ok_or_else(|| {
                refusal(
                    AdmissionStage::Authorize,
                    RefusalReason::TargetSupportMissing,
                )
            })?;
    }
    let source = normalized.source_ref().ok_or_else(|| {
        refusal(AdmissionStage::Admit, RefusalReason::SourcePolicyRefused)
    })?;
    // A Volume relationship always names one declared view: the contract's
    // optional view field is the parent's default slot, not the source's
    // permission to present the Volume root.
    let view = normalized.view().cloned().ok_or_else(|| {
        refusal(AdmissionStage::Admit, RefusalReason::SourcePolicyRefused)
    })?;
    VolumeBindingRequest::new(
        source.clone(),
        normalized.consumer_ref().clone(),
        slot,
        view,
        access_for_rights(rights)
            .ok_or_else(|| refusal(AdmissionStage::Admit, RefusalReason::SourcePolicyRefused))?,
        presentation,
    )
    .map_err(|_| refusal(AdmissionStage::Admit, RefusalReason::SourcePolicyRefused))
}
