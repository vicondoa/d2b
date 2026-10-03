//! Controller-created runtime Volume specification.

use d2b_contracts_resource::v3::{
    BindingSlot, BoundedToken, ResourceRef, volume::AttachmentAccess,
    volume_binding::{VolumeBindingRequest, VolumePresentation},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::types::runtime_volume_name;

/// Runtime Volume finalizer.
pub const RUNTIME_VOLUME_FINALIZER: &str = "runtime-qemu-media.d2bus.org/runtime-volume";

/// The stable consumer slot the runner's runtime-volume use occupies.
pub const RUNTIME_VOLUME_SLOT: &str = "runtime";

/// The named view the runner reads its QMP and serial sockets through.
pub const RUNTIME_VOLUME_RUNNER_VIEW: &str = "runner";

/// The consumer-side mount path the runner's runtime volume is presented at.
pub const RUNTIME_VOLUME_MOUNT_PATH: &str = "/run/qemu";

/// Runtime Volume layout entry type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VolumeLayoutType {
    /// Runtime directory.
    Directory,
    /// QMP or serial socket.
    UnixSocket,
}

/// Runtime Volume layout entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LayoutEntry {
    /// Relative layout path.
    pub path: String,
    /// Entry type.
    pub entry_type: VolumeLayoutType,
    /// Required mode.
    pub mode: String,
    /// Cleanup policy.
    pub cleanup_policy: String,
    /// Adoption policy.
    pub adoption_policy: String,
    /// Restart policy.
    pub restart_policy: String,
}

/// Runtime Volume named view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeVolumeView {
    /// Relative view path.
    pub path: String,
    /// View rights.
    pub rights: Vec<String>,
}

/// Runtime Volume hard quota.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VolumeQuota {
    /// Byte limit.
    pub max_bytes: u64,
    /// Inode limit.
    pub max_inodes: u32,
    /// Enforcement mode.
    pub enforcement: String,
}

/// Controller-created runtime tmpfs Volume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeVolumeSpec {
    /// Deterministic Volume name.
    pub name: String,
    /// Zone containing the Volume.
    pub zone: String,
    /// Owning Guest.
    pub owner_ref: ResourceRef,
    /// Provider reference.
    pub provider_ref: ResourceRef,
    /// Source kind.
    pub source_kind: String,
    /// Source policy identifier.
    pub source_policy_id: String,
    /// Layout entries.
    pub layout: Vec<LayoutEntry>,
    /// Named views.
    pub views: Vec<(String, RuntimeVolumeView)>,
    /// Hard quota.
    pub quota: VolumeQuota,
    /// Finalizer.
    pub finalizer: String,
}

impl RuntimeVolumeSpec {
    /// Build the canonical per-Guest runtime Volume.
    pub fn new(
        guest_ref: ResourceRef,
        zone: impl Into<String>,
        quota_bytes: u64,
        quota_inodes: u32,
    ) -> Result<Self, VolumeSpecError> {
        Self::new_with_provider(
            guest_ref,
            zone,
            ResourceRef::parse("Provider/volume-local").expect("frozen Volume Provider ref"),
            quota_bytes,
            quota_inodes,
        )
    }

    /// Build the runtime Volume with an explicit typed Volume Provider.
    pub fn new_with_provider(
        guest_ref: ResourceRef,
        zone: impl Into<String>,
        provider_ref: ResourceRef,
        quota_bytes: u64,
        quota_inodes: u32,
    ) -> Result<Self, VolumeSpecError> {
        if guest_ref.resource_type().as_str() != "Guest"
            || provider_ref.resource_type().as_str() != "Provider"
            || !(1024 * 1024..=256 * 1024 * 1024).contains(&quota_bytes)
            || !(64..=65_536).contains(&quota_inodes)
        {
            return Err(VolumeSpecError::Invalid);
        }
        let zone = zone.into();
        if BoundedToken::parse(zone.as_str()).is_err() {
            return Err(VolumeSpecError::Invalid);
        }
        let name = runtime_volume_name(&short_guest_key(
            &guest_ref.to_canonical_string(),
        ));
        Ok(Self {
            name,
            zone,
            owner_ref: guest_ref,
            provider_ref,
            source_kind: "tmpfs".to_owned(),
            source_policy_id: "runtime-qemu-media-runtime-tmpfs".to_owned(),
            layout: canonical_layout(),
            views: canonical_views(),
            quota: VolumeQuota {
                max_bytes: quota_bytes,
                max_inodes: quota_inodes,
                enforcement: "hard".to_owned(),
            },
            finalizer: RUNTIME_VOLUME_FINALIZER.to_owned(),
        })
    }

    /// Return the owner reference.
    pub const fn owner_ref(&self) -> &ResourceRef {
        &self.owner_ref
    }

    /// Return the required cleanup policy.
    pub fn cleanup_policy(&self) -> &str {
        self.layout
            .first()
            .map(|entry| entry.cleanup_policy.as_str())
            .unwrap_or("")
    }

    /// Validate the canonical runtime Volume shape.
    pub fn validate(&self) -> Result<(), VolumeSpecError> {
        let expected_name = runtime_volume_name(&short_guest_key(
            &self.owner_ref.to_canonical_string(),
        ));
        if self.owner_ref.resource_type().as_str() != "Guest"
            || self.provider_ref.resource_type().as_str() != "Provider"
            || BoundedToken::parse(self.zone.as_str()).is_err()
            || self.name != expected_name
            || self.source_kind != "tmpfs"
            || self.source_policy_id != "runtime-qemu-media-runtime-tmpfs"
            || self.finalizer != RUNTIME_VOLUME_FINALIZER
            || self.layout != canonical_layout()
            || self.views != canonical_views()
            || !(1024 * 1024..=256 * 1024 * 1024).contains(&self.quota.max_bytes)
            || !(64..=65_536).contains(&self.quota.max_inodes)
            || self.quota.enforcement != "hard"
        {
            return Err(VolumeSpecError::Invalid);
        }
        Ok(())
    }

    /// Derive the runner's own storage request from this row.
    ///
    /// The row's store name is a digest of the owning Guest, so it is a store
    /// identity rather than a reference name: the committed `Volume` reference
    /// the graph admitted is supplied by the caller, while the view, the
    /// access, and the destination all come from this row's own declaration.
    ///
    /// The runtime Volume is an ordinary source, so the runner's use of it is
    /// an ordinary `VolumeBinding` request: the runner's named view, at the
    /// runner's consumer slot, presented as a filesystem at the declared
    /// destination. The QMP and serial sockets therefore reach the runner
    /// through the same admitted relationship as every other storage use
    /// rather than through a mount the Process spec hard-coded (R17-R20,
    /// R34).
    ///
    /// # Errors
    ///
    /// Returns [`VolumeSpecError::Invalid`] when the row's own declared
    /// values cannot form a typed request, which is a shape error rather than
    /// an admission one.
    pub fn runner_request(
        &self,
        volume_ref: ResourceRef,
        consumer_ref: ResourceRef,
    ) -> Result<VolumeBindingRequest, VolumeSpecError> {
        if volume_ref.resource_type().as_str() != "Volume" {
            return Err(VolumeSpecError::Invalid);
        }
        let slot = BindingSlot::parse(RUNTIME_VOLUME_SLOT)
            .map_err(|_| VolumeSpecError::Invalid)?;
        let view = BoundedToken::parse(RUNTIME_VOLUME_RUNNER_VIEW)
            .map_err(|_| VolumeSpecError::Invalid)?;
        let presentation = VolumePresentation::filesystem(RUNTIME_VOLUME_MOUNT_PATH)
            .map_err(|_| VolumeSpecError::Invalid)?;
        VolumeBindingRequest::new(
            volume_ref,
            consumer_ref,
            slot,
            view,
            AttachmentAccess::ReadWrite,
            presentation,
        )
        .map_err(|_| VolumeSpecError::Invalid)
    }
}

/// Runtime Volume specification failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeSpecError {
    /// The shape or bound is invalid.
    Invalid,
}

fn layout(
    path: &str,
    entry_type: VolumeLayoutType,
    mode: &str,
    restart_policy: &str,
) -> LayoutEntry {
    LayoutEntry {
        path: path.to_owned(),
        entry_type,
        mode: mode.to_owned(),
        cleanup_policy: "vm-stop-with-proof".to_owned(),
        adoption_policy: "quarantine-on-ambiguity".to_owned(),
        restart_policy: restart_policy.to_owned(),
    }
}

/// The canonical runtime Volume layout.
fn canonical_layout() -> Vec<LayoutEntry> {
    vec![
        layout(
            "",
            VolumeLayoutType::Directory,
            "0700",
            "preserve-across-controller-restart",
        ),
        layout(
            "qmp.sock",
            VolumeLayoutType::UnixSocket,
            "0600",
            "clear-on-runner-restart",
        ),
        layout(
            "serial.sock",
            VolumeLayoutType::UnixSocket,
            "0600",
            "clear-on-runner-restart",
        ),
    ]
}

/// The canonical runtime Volume views.
fn canonical_views() -> Vec<(String, RuntimeVolumeView)> {
    vec![
        (
            "runner".to_owned(),
            RuntimeVolumeView {
                path: String::new(),
                rights: vec![
                    "read".to_owned(),
                    "write".to_owned(),
                    "create".to_owned(),
                    "delete".to_owned(),
                    "traverse".to_owned(),
                ],
            },
        ),
        (
            "controller-observe".to_owned(),
            RuntimeVolumeView {
                path: String::new(),
                rights: vec!["read".to_owned(), "traverse".to_owned()],
            },
        ),
    ]
}

fn short_guest_key(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
