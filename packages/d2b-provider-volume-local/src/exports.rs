//! Provider-neutral Export intents produced from Volume attachments.
//!
//! volume-local owns this translation.  The volume-virtiofs implementation
//! consumes the resulting resource shape, while the local Provider itself
//! never imports or calls the other Provider crate.

use d2b_contracts::v3::ResourceRef;
use d2b_contracts::v3::execution_policy::BoundedToken;
use d2b_contracts::v3::volume::{
    AttachmentAccess, AttachmentSettings, AttachmentTransport, VolumeSpec,
};
use sha2::{Digest, Sha256};

use crate::error::VolumeLocalError;
use crate::views::admit_attachments;

/// The qualified ResourceType emitted by this translation.
pub const EXPORT_RESOURCE_TYPE: &str = "virtiofs.d2bus.org.Export";

/// One desired controller-created Export resource.
#[derive(Clone, PartialEq, Eq)]
pub struct ExportIntent {
    name: BoundedToken,
    owner_ref: ResourceRef,
    volume_ref: ResourceRef,
    execution_ref: ResourceRef,
    view: BoundedToken,
    access: AttachmentAccess,
    mount_path: String,
    settings: AttachmentSettings,
}

impl ExportIntent {
    /// Borrow the deterministic Export name.
    pub const fn name(&self) -> &BoundedToken {
        &self.name
    }

    /// Borrow the Volume owner reference.
    pub const fn owner_ref(&self) -> &ResourceRef {
        &self.owner_ref
    }

    /// Borrow the referenced Volume.
    pub const fn volume_ref(&self) -> &ResourceRef {
        &self.volume_ref
    }

    /// Borrow the execution target.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }

    /// Borrow the selected named View.
    pub const fn view(&self) -> &BoundedToken {
        &self.view
    }

    /// Return the admitted access class.
    pub const fn access(&self) -> AttachmentAccess {
        self.access
    }

    /// Borrow the guest-side mount path.
    pub fn mount_path(&self) -> &str {
        &self.mount_path
    }

    /// Borrow the typed base attachment settings.
    pub const fn settings(&self) -> &AttachmentSettings {
        &self.settings
    }
}

impl core::fmt::Debug for ExportIntent {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ExportIntent")
            .field("access", &self.access)
            .finish_non_exhaustive()
    }
}

/// Translate every virtiofs attachment into one deterministic Export.
pub fn desired_export_intents(
    volume_ref: ResourceRef,
    spec: &VolumeSpec,
    supports_shared_write: bool,
) -> Result<Vec<ExportIntent>, VolumeLocalError> {
    let admitted = admit_attachments(spec, supports_shared_write)?;
    let mut intents = Vec::with_capacity(admitted.len());
    for (index, attachment) in spec.attachments().iter().enumerate() {
        if attachment.transport() != AttachmentTransport::Virtiofs {
            continue;
        }
        let name = derive_export_name(&volume_ref, attachment, index)?;
        intents.push(ExportIntent {
            name,
            owner_ref: volume_ref.clone(),
            volume_ref: volume_ref.clone(),
            execution_ref: attachment.execution_ref().clone(),
            view: attachment.view().clone(),
            access: attachment.access(),
            mount_path: attachment.mount_path().to_owned(),
            settings: attachment.settings().clone(),
        });
    }
    debug_assert_eq!(intents.len(), admitted.len());
    Ok(intents)
}

fn derive_export_name(
    volume_ref: &ResourceRef,
    attachment: &d2b_contracts::v3::volume::VolumeAttachment,
    index: usize,
) -> Result<BoundedToken, VolumeLocalError> {
    let mut hasher = Sha256::new();
    hasher.update(b"d2b/volume-local/export/v1");
    hasher.update([0]);
    hasher.update(volume_ref.to_canonical_string().as_bytes());
    hasher.update([0]);
    hasher.update(attachment.execution_ref().to_canonical_string().as_bytes());
    hasher.update([0]);
    hasher.update(attachment.mount_path().as_bytes());
    hasher.update([0]);
    hasher.update(index.to_be_bytes());
    let digest = hasher.finalize();
    let mut suffix = String::with_capacity(24);
    for byte in digest[..12].iter().copied() {
        suffix.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        suffix.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }
    BoundedToken::parse(format!("vol-export-{suffix}")).map_err(|_| VolumeLocalError::InvalidSpec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::fixtures;

    #[test]
    fn every_virtiofs_attachment_becomes_a_stable_owned_intent() {
        let volume = ResourceRef::parse("Volume/work-state").unwrap();
        let intents =
            desired_export_intents(volume.clone(), &fixtures::attached_state_volume(), false)
                .expect("intent");
        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].owner_ref(), &volume);
        assert_eq!(intents[0].mount_path(), "/state");
        assert!(intents[0].name().as_str().starts_with("vol-export-"));
        assert_eq!(
            intents[0].name(),
            desired_export_intents(volume, &fixtures::attached_state_volume(), false,).unwrap()[0]
                .name()
        );
    }

    #[test]
    fn virtio_blk_attachments_do_not_create_filesystem_exports() {
        let mut value = serde_json::to_value(fixtures::state_volume()).unwrap();
        value["attachments"] = serde_json::json!([{
            "executionRef": "Guest/work-vm",
            "transport": "virtio-blk",
            "view": "controller",
            "access": "read-only",
            "mountPath": "/disk"
        }]);
        let spec: VolumeSpec = serde_json::from_value(value).unwrap();
        assert!(
            desired_export_intents(
                ResourceRef::parse("Volume/work-state").unwrap(),
                &spec,
                false,
            )
            .unwrap()
            .is_empty()
        );
    }
}
