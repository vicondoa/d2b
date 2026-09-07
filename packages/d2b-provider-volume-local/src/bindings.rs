//! Deterministic `VolumeBinding` intents produced from Volume attachments.
//!
//! volume-local owns this translation.  The shared runner mints one owned
//! binding child per intent, while the local Provider itself never imports
//! or calls the virtiofs Provider crate.  Attachments stay validated input
//! only: nothing besides the binding relationship is described here.

use d2b_contracts_resource::v3::ResourceRef;
use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::volume::{
    AttachmentAccess, AttachmentSettings, AttachmentTransport, VolumeSpec,
};
use sha2::{Digest, Sha256};

use crate::error::VolumeLocalError;
use crate::views::admit_attachments;

/// One desired Volume-side-created `VolumeBinding` resource.
#[derive(Clone, PartialEq, Eq)]
pub struct BindingIntent {
    name: BoundedToken,
    owner_ref: ResourceRef,
    volume_ref: ResourceRef,
    execution_ref: ResourceRef,
    view: BoundedToken,
    access: AttachmentAccess,
    mount_path: String,
    settings: AttachmentSettings,
}

impl BindingIntent {
    /// Borrow the deterministic binding name.
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

impl core::fmt::Debug for BindingIntent {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BindingIntent")
            .field("access", &self.access)
            .finish_non_exhaustive()
    }
}

/// Translate every virtiofs attachment into one deterministic binding.
///
/// The binding name derives from the Volume, execution target, named view,
/// and guest mount path — never from the attachment index — so reordering
/// declared attachments never churns identities.
pub fn desired_binding_intents(
    volume_ref: ResourceRef,
    spec: &VolumeSpec,
    supports_shared_write: bool,
) -> Result<Vec<BindingIntent>, VolumeLocalError> {
    let admitted = admit_attachments(spec, supports_shared_write)?;
    let mut intents = Vec::with_capacity(admitted.len());
    for attachment in spec.attachments().iter() {
        if attachment.transport() != AttachmentTransport::Virtiofs {
            continue;
        }
        let name = derive_binding_name(&volume_ref, attachment)?;
        intents.push(BindingIntent {
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

fn derive_binding_name(
    volume_ref: &ResourceRef,
    attachment: &d2b_contracts_resource::v3::volume::VolumeAttachment,
) -> Result<BoundedToken, VolumeLocalError> {
    let mut hasher = Sha256::new();
    hasher.update(b"d2b/volume-local/binding/v1");
    hasher.update([0]);
    hasher.update(volume_ref.to_canonical_string().as_bytes());
    hasher.update([0]);
    hasher.update(attachment.execution_ref().to_canonical_string().as_bytes());
    hasher.update([0]);
    hasher.update(attachment.view().as_str().as_bytes());
    hasher.update([0]);
    hasher.update(attachment.mount_path().as_bytes());
    let digest = hasher.finalize();
    let mut suffix = String::with_capacity(24);
    for byte in digest[..12].iter().copied() {
        suffix.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        suffix.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }
    BoundedToken::parse(format!("vol-binding-{suffix}")).map_err(|_| VolumeLocalError::InvalidSpec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::fixtures;

    fn two_attachment_volume() -> VolumeSpec {
        serde_json::from_value(serde_json::json!({
            "source": {
                "executionRef": "Host/host-system",
                "settings": { "kind": "local-path", "sourcePolicyId": "state-root" },
            },
            "kind": "state",
            "layout": [],
            "views": {
                "controller": {
                    "path": "",
                    "rights": ["read", "write", "create", "delete", "traverse"],
                },
                "reader": { "path": "", "rights": ["read", "traverse"] },
            },
            "attachments": [
                {
                    "executionRef": "Guest/work-vm",
                    "transport": "virtiofs",
                    "view": "controller",
                    "access": "read-write",
                    "mountPath": "/state",
                },
                {
                    "executionRef": "Guest/other-vm",
                    "transport": "virtiofs",
                    "view": "reader",
                    "access": "read-only",
                    "mountPath": "/data",
                },
            ],
        }))
        .expect("conformant fixture Volume spec")
    }

    #[test]
    fn every_virtiofs_attachment_becomes_a_stable_owned_intent() {
        let volume = ResourceRef::parse("Volume/work-state").unwrap();
        let intents =
            desired_binding_intents(volume.clone(), &fixtures::attached_state_volume(), false)
                .expect("intent");
        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].owner_ref(), &volume);
        assert_eq!(intents[0].mount_path(), "/state");
        assert!(intents[0].name().as_str().starts_with("vol-binding-"));
        assert_eq!(
            intents[0].name(),
            desired_binding_intents(volume, &fixtures::attached_state_volume(), false,).unwrap()[0]
                .name()
        );
    }

    #[test]
    fn reordering_attachments_never_churns_binding_names() {
        let volume = ResourceRef::parse("Volume/work-state").unwrap();
        let spec = two_attachment_volume();
        let forward = desired_binding_intents(volume.clone(), &spec, false).expect("intents");
        let mut reordered = serde_json::to_value(&spec).unwrap();
        let attachments = reordered["attachments"].as_array().unwrap().clone();
        let swapped: Vec<_> = attachments.into_iter().rev().collect();
        reordered["attachments"] = serde_json::Value::Array(swapped);
        let backward_spec: VolumeSpec = serde_json::from_value(reordered).unwrap();
        let backward = desired_binding_intents(volume, &backward_spec, false).expect("intents");

        assert_eq!(forward.len(), backward.len());
        for intent in &forward {
            assert!(backward.iter().any(|other| other.name() == intent.name()));
        }
    }

    #[test]
    fn the_named_view_is_part_of_the_binding_identity() {
        let volume = ResourceRef::parse("Volume/work-state").unwrap();
        let mut value = serde_json::to_value(fixtures::attached_state_volume()).unwrap();
        let spec: VolumeSpec = serde_json::from_value(value.clone()).unwrap();
        let controller = desired_binding_intents(volume.clone(), &spec, false).unwrap();
        value["attachments"][0]["view"] = serde_json::json!("reader");
        value["attachments"][0]["access"] = serde_json::json!("read-only");
        let reader_spec: VolumeSpec = serde_json::from_value(value).unwrap();
        let reader = desired_binding_intents(volume, &reader_spec, false).unwrap();

        assert_eq!(controller[0].execution_ref(), reader[0].execution_ref());
        assert_eq!(controller[0].mount_path(), reader[0].mount_path());
        assert_ne!(controller[0].name(), reader[0].name());
    }

    #[test]
    fn virtio_blk_attachments_do_not_create_filesystem_bindings() {
        let mut value = serde_json::to_value(fixtures::state_volume()).unwrap();
        value["source"]["settings"]["kind"] = serde_json::json!("block-image");
        value["source"]["settings"]["sourcePolicyId"] = serde_json::json!("disk-root");
        value["quota"] = serde_json::json!({ "maxBytes": 4096, "enforcement": "none" });
        value["attachments"] = serde_json::json!([{
            "executionRef": "Guest/work-vm",
            "transport": "virtio-blk",
            "view": "controller",
            "access": "read-only",
            "mountPath": "/disk"
        }]);
        let spec: VolumeSpec = serde_json::from_value(value).unwrap();
        assert!(
            desired_binding_intents(
                ResourceRef::parse("Volume/work-state").unwrap(),
                &spec,
                false,
            )
            .unwrap()
            .is_empty()
        );
    }
}
