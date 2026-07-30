//! TPM Volume mode: never silently re-provision guest TPM state.
//!
//! The per-Guest swtpm state directory holds the guest's TPM NVRAM and EK
//! seed. Wiping it looks like device tampering to any identity provider,
//! so a directory that is absent *after* its provisioning marker was
//! written is a hard failure, never a fresh empty TPM.

use d2b_contracts::v3::ResourceUid;
use d2b_contracts::v3::volume::{
    CreatePolicy, EntryType, RepairPolicy, SensitivityClass, VolumeKind, VolumeSpec,
};

use crate::error::VolumeLocalError;
use crate::identity::MarkerState;
use crate::layout::EntryRequest;

/// What a reconcile pass may do to an swtpm state root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SwtpmDisposition {
    /// No marker and no directory: provision once and write the marker.
    Provision,
    /// Marker and directory agree: reconcile posture in place.
    ReconcileInPlace,
}

/// Decide what to do with an swtpm state root.
///
/// A marker with no directory is `previously-provisioned-state-missing`
/// and fails closed; the controller never re-creates the directory.
pub fn evaluate_swtpm_state(
    marker: MarkerState,
    directory_present: bool,
) -> Result<SwtpmDisposition, VolumeLocalError> {
    match (marker, directory_present) {
        (MarkerState::Provisioned, false) => {
            Err(VolumeLocalError::PreviouslyProvisionedStateMissing)
        }
        (MarkerState::Provisioned, true) => Ok(SwtpmDisposition::ReconcileInPlace),
        (MarkerState::NeverProvisioned, _) => Ok(SwtpmDisposition::Provision),
    }
}

/// Check that a Volume declares the fail-closed TPM state posture.
///
/// The root entry must be a `state`-kind directory that is created only
/// when it was never provisioned, is never repaired by chowning existing
/// NVRAM, and is marked secret so its path cannot reach public status.
pub fn assert_swtpm_volume(
    volume_uid: &ResourceUid,
    spec: &VolumeSpec,
) -> Result<(), VolumeLocalError> {
    if spec.kind() != VolumeKind::State {
        return Err(VolumeLocalError::InvalidSpec);
    }
    let root = spec
        .layout()
        .iter()
        .find(|entry| entry.path().is_empty())
        .ok_or(VolumeLocalError::InvalidSpec)?;
    if root.entry_type() != EntryType::Directory {
        return Err(VolumeLocalError::InvalidSpec);
    }
    let rendered = serde_json::to_value(root).map_err(|_| VolumeLocalError::InvalidSpec)?;
    let sensitivity: SensitivityClass = rendered
        .get("sensitivity")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .ok_or(VolumeLocalError::InvalidSpec)?;
    if sensitivity != SensitivityClass::Secret {
        return Err(VolumeLocalError::InvalidSpec);
    }
    let root = EntryRequest::resolve(volume_uid, root)?;
    if root.create_policy() != CreatePolicy::CreateIfNeverProvisioned {
        return Err(VolumeLocalError::InvalidSpec);
    }
    // The contract's repair set has no fail-closed member, so the
    // strictest available posture is the mode-only repair that never
    // rewrites the owner of existing NVRAM.
    if root.repair_policy() != RepairPolicy::ExactMode {
        return Err(VolumeLocalError::InvalidSpec);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marker_without_state_fails_closed_instead_of_re_provisioning() {
        assert_eq!(
            evaluate_swtpm_state(MarkerState::Provisioned, false),
            Err(VolumeLocalError::PreviouslyProvisionedStateMissing)
        );
        assert_eq!(
            evaluate_swtpm_state(MarkerState::Provisioned, true),
            Ok(SwtpmDisposition::ReconcileInPlace)
        );
        assert_eq!(
            evaluate_swtpm_state(MarkerState::NeverProvisioned, false),
            Ok(SwtpmDisposition::Provision)
        );
    }
}
