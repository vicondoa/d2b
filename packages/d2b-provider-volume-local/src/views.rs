//! Named views, right intersection, and attachment admission.
//!
//! A mount or attachment always selects a named view; it never names a
//! Volume subtree directly. volume-local is the sole Volume writer and
//! the sole admitter of attachments, so the single-writer and
//! shared-write rules are enforced here before any Export is requested.

use d2b_contracts::v3::ResourceRef;
use d2b_contracts::v3::execution_policy::BoundedToken;
use d2b_contracts::v3::volume::{
    AttachmentAccess, AttachmentTransport, ViewRight, ViewSpec, VolumeAttachment, VolumeSpec,
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

/// One admitted virtiofs attachment, ready to become one owned Export.
///
/// It carries only typed references and the selected view name. The
/// resolved host path, the export socket path, and the numeric socket
/// group never appear here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentPlan {
    /// The Host or Guest the Volume is exported to.
    pub execution_ref: ResourceRef,
    /// The selected named view.
    pub view: BoundedToken,
    /// The admitted access level.
    pub access: AttachmentAccess,
}

/// Admit every declared attachment of a Volume.
///
/// Rejects a view that does not exist, an access level the view's rights
/// do not cover, a second simultaneous writer, and `shared-write` when
/// the selected attachment Provider does not declare it.
pub fn admit_attachments(
    spec: &VolumeSpec,
    supports_shared_write: bool,
) -> Result<Vec<AttachmentPlan>, VolumeLocalError> {
    let mut writers = 0usize;
    let mut plans = Vec::with_capacity(spec.attachments().len());
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
            plans.push(AttachmentPlan {
                execution_ref: attachment.execution_ref().clone(),
                view: attachment.view().clone(),
                access: attachment.access(),
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
