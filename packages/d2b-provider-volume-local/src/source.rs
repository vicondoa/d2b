//! Volume source-policy and source-kind validation.
//!
//! The Provider receives only the opaque policy identifier from the Volume
//! contract.  This module validates the semantic relationship between that
//! identifier, the source kind, the Volume kind, and the attachment
//! transport.  Resolution of an identifier to a host path remains an
//! effect-port responsibility.

use std::collections::{BTreeMap, BTreeSet};

use d2b_contracts::v3::execution_policy::BoundedToken;
use d2b_contracts::v3::volume::{
    AttachmentTransport, BlockImageFormat, CreatePolicy, EntryRestartPolicy, SourceKind,
    VolumeKind, VolumeSpec,
};

use crate::error::VolumeLocalError;

/// One private source-policy class advertised by a Provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePolicy {
    id: BoundedToken,
    class: SourceKind,
    volume_kinds: BTreeSet<VolumeKind>,
}

impl SourcePolicy {
    /// Construct a policy entry without carrying its private root.
    pub fn new(
        id: impl Into<String>,
        class: SourceKind,
        volume_kinds: impl IntoIterator<Item = VolumeKind>,
    ) -> Result<Self, VolumeLocalError> {
        let id = BoundedToken::parse(id.into()).map_err(|_| VolumeLocalError::InvalidSpec)?;
        let volume_kinds: BTreeSet<VolumeKind> = volume_kinds.into_iter().collect();
        if volume_kinds.is_empty() {
            return Err(VolumeLocalError::InvalidSpec);
        }
        Ok(Self {
            id,
            class,
            volume_kinds,
        })
    }

    /// Borrow the opaque policy identifier.
    pub const fn id(&self) -> &BoundedToken {
        &self.id
    }

    /// Return the source class guarded by this policy.
    pub const fn class(&self) -> SourceKind {
        self.class
    }

    /// Return whether the policy permits a Volume kind.
    pub fn permits_kind(&self, kind: VolumeKind) -> bool {
        self.volume_kinds.contains(&kind)
    }
}

/// A bounded source-policy catalog.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct SourcePolicyCatalog {
    policies: BTreeMap<String, SourcePolicy>,
}

impl core::fmt::Debug for SourcePolicyCatalog {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("SourcePolicyCatalog(<redacted>)")
    }
}

impl SourcePolicyCatalog {
    /// Build a catalog and reject duplicate opaque IDs.
    pub fn new(policies: impl IntoIterator<Item = SourcePolicy>) -> Result<Self, VolumeLocalError> {
        let mut catalog = Self::default();
        for policy in policies {
            if catalog
                .policies
                .insert(policy.id().as_str().to_owned(), policy)
                .is_some()
            {
                return Err(VolumeLocalError::InvalidSpec);
            }
        }
        Ok(catalog)
    }

    /// Validate the opaque policy selected by a Volume.
    pub fn validate(&self, spec: &VolumeSpec) -> Result<(), VolumeLocalError> {
        if spec.source().settings().kind() == SourceKind::Tmpfs {
            // The v3 base contract carries no sourcePolicyId for tmpfs:
            // the kernel mount is created by the selected execution domain.
            return Ok(());
        }
        let policy_id = spec
            .source()
            .settings()
            .source_policy_id()
            .ok_or(VolumeLocalError::SourcePolicyNotFound)?;
        let policy = self
            .policies
            .get(policy_id.as_str())
            .ok_or(VolumeLocalError::SourcePolicyNotFound)?;
        if policy.class() != spec.source().settings().kind() || !policy.permits_kind(spec.kind()) {
            return Err(VolumeLocalError::SourcePolicyMismatch);
        }
        Ok(())
    }

    /// Return the number of policy entries.
    pub fn len(&self) -> usize {
        self.policies.len()
    }

    /// Return whether the catalog contains no entries.
    pub fn is_empty(&self) -> bool {
        self.policies.is_empty()
    }
}

/// Validate source-kind-specific constraints that are not represented by the
/// current base contract constructor.
pub fn validate_source_spec(spec: &VolumeSpec) -> Result<(), VolumeLocalError> {
    match spec.source().settings().kind() {
        SourceKind::LocalPath => {
            if spec
                .attachments()
                .iter()
                .any(|attachment| attachment.transport() != AttachmentTransport::Virtiofs)
            {
                return Err(VolumeLocalError::InvalidSpec);
            }
            Ok(())
        }
        SourceKind::Tmpfs => {
            if !matches!(spec.kind(), VolumeKind::Ephemeral | VolumeKind::Tmp) {
                return Err(VolumeLocalError::SourceKindVolumeKindMismatch);
            }
            let Some(quota) = spec.quota() else {
                return Err(VolumeLocalError::TmpfsQuotaMissing);
            };
            if quota.max_bytes().is_none() || quota.max_inodes().is_none() {
                return Err(VolumeLocalError::TmpfsQuotaMissing);
            }
            if quota.enforcement() != d2b_contracts::v3::volume::QuotaEnforcement::Hard {
                return Err(VolumeLocalError::InvalidSpec);
            }
            if spec
                .attachments()
                .iter()
                .any(|attachment| attachment.transport() != AttachmentTransport::Virtiofs)
            {
                return Err(VolumeLocalError::InvalidSpec);
            }
            for entry in spec.layout() {
                let rendered =
                    serde_json::to_value(entry).map_err(|_| VolumeLocalError::InvalidSpec)?;
                let create_policy = rendered
                    .get("createPolicy")
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok());
                let restart_policy = rendered
                    .get("restartPolicy")
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok());
                if create_policy == Some(CreatePolicy::CreateIfNeverProvisioned)
                    || restart_policy == Some(EntryRestartPolicy::PreserveAcrossControllerRestart)
                {
                    return Err(VolumeLocalError::InvalidSpec);
                }
            }
            Ok(())
        }
        SourceKind::BlockImage => {
            if !matches!(spec.kind(), VolumeKind::Durable | VolumeKind::Ephemeral) {
                return Err(VolumeLocalError::SourceKindVolumeKindMismatch);
            }
            if spec.quota().and_then(|quota| quota.max_bytes()).is_none() {
                return Err(VolumeLocalError::BlockImageQuotaMissing);
            }
            if spec
                .attachments()
                .iter()
                .any(|attachment| attachment.transport() != AttachmentTransport::VirtioBlk)
            {
                return Err(VolumeLocalError::BlockImageTransportMismatch);
            }
            Ok(())
        }
    }
}

/// Kernel mount limits for a tmpfs Volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TmpfsMountOptions {
    size_bytes: u64,
    max_inodes: u64,
}

impl TmpfsMountOptions {
    /// Derive the two bounded kernel options from a validated Volume.
    pub fn from_spec(spec: &VolumeSpec) -> Result<Self, VolumeLocalError> {
        if spec.source().settings().kind() != SourceKind::Tmpfs {
            return Err(VolumeLocalError::InvalidSpec);
        }
        validate_source_spec(spec)?;
        let quota = spec.quota().ok_or(VolumeLocalError::TmpfsQuotaMissing)?;
        Ok(Self {
            size_bytes: quota
                .max_bytes()
                .ok_or(VolumeLocalError::TmpfsQuotaMissing)?,
            max_inodes: quota
                .max_inodes()
                .ok_or(VolumeLocalError::TmpfsQuotaMissing)?,
        })
    }

    /// Return the byte limit used for `size=`.
    pub const fn size_bytes(self) -> u64 {
        self.size_bytes
    }

    /// Return the inode limit used for `nr_inodes=`.
    pub const fn max_inodes(self) -> u64 {
        self.max_inodes
    }

    /// Render the closed mount option pair.
    pub fn mount_options(self) -> [String; 2] {
        [
            format!("size={}", self.size_bytes),
            format!("nr_inodes={}", self.max_inodes),
        ]
    }
}

/// The file lifecycle plan for a block-image Volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockImagePlan {
    max_bytes: u64,
    format: BlockImageFormat,
    preallocate: bool,
}

impl BlockImagePlan {
    /// Derive the bounded image lifecycle plan from a Volume.
    pub fn from_spec(spec: &VolumeSpec) -> Result<Self, VolumeLocalError> {
        if spec.source().settings().kind() != SourceKind::BlockImage {
            return Err(VolumeLocalError::InvalidSpec);
        }
        validate_source_spec(spec)?;
        Ok(Self {
            max_bytes: spec
                .quota()
                .and_then(|quota| quota.max_bytes())
                .ok_or(VolumeLocalError::BlockImageQuotaMissing)?,
            format: spec.source().settings().image_format(),
            preallocate: spec.source().settings().preallocate(),
        })
    }

    /// Return the declared image ceiling.
    pub const fn max_bytes(self) -> u64 {
        self.max_bytes
    }

    /// Return the requested on-disk image format.
    pub const fn format(self) -> BlockImageFormat {
        self.format
    }

    /// Whether the image should be preallocated.
    pub const fn preallocate(self) -> bool {
        self.preallocate
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn source(kind: &str, policy: Option<&str>) -> serde_json::Value {
        let mut settings = json!({ "kind": kind });
        if let Some(policy) = policy {
            settings["sourcePolicyId"] = json!(policy);
        }
        json!({
            "source": {
                "executionRef": "Host/host-system",
                "settings": settings,
            },
            "kind": "durable",
            "layout": [],
            "views": { "controller": { "path": "", "rights": ["read", "traverse"] } },
        })
    }

    #[test]
    fn policy_catalog_matches_opaque_id_class_and_volume_kind() {
        let spec: VolumeSpec =
            serde_json::from_value(source("local-path", Some("state-root"))).expect("valid source");
        let catalog = SourcePolicyCatalog::new([SourcePolicy::new(
            "state-root",
            SourceKind::LocalPath,
            [VolumeKind::Durable],
        )
        .expect("valid policy")])
        .expect("valid catalog");
        assert!(catalog.validate(&spec).is_ok());

        let wrong = SourcePolicyCatalog::new([SourcePolicy::new(
            "state-root",
            SourceKind::Tmpfs,
            [VolumeKind::Durable],
        )
        .expect("valid policy")])
        .expect("valid catalog");
        assert_eq!(
            wrong.validate(&spec),
            Err(VolumeLocalError::SourcePolicyMismatch)
        );
        let empty = SourcePolicyCatalog::new([]).expect("an empty catalog is valid");
        assert_eq!(
            empty.validate(&spec),
            Err(VolumeLocalError::SourcePolicyNotFound)
        );
    }

    #[test]
    fn block_images_require_a_byte_ceiling_and_virtio_blk() {
        let mut value = source("block-image", Some("disk-root"));
        value["quota"] = json!({ "maxBytes": 4096, "enforcement": "none" });
        value["attachments"] = json!([{
            "executionRef": "Guest/work-vm",
            "transport": "virtiofs",
            "view": "controller",
            "access": "read-only",
            "mountPath": "/disk"
        }]);
        assert!(serde_json::from_value::<VolumeSpec>(value).is_err());
    }

    #[test]
    fn tmpfs_limits_render_to_kernel_options() {
        let mut value = source("tmpfs", None);
        value["kind"] = json!("tmp");
        value["quota"] = json!({ "maxBytes": 4096, "maxInodes": 32, "enforcement": "hard" });
        let spec: VolumeSpec = serde_json::from_value(value).expect("valid tmpfs");
        let options = TmpfsMountOptions::from_spec(&spec)
            .expect("limits")
            .mount_options();
        assert_eq!(options, ["size=4096", "nr_inodes=32"]);
    }

    #[test]
    fn block_image_plan_keeps_the_declared_byte_ceiling_opaque() {
        let mut value = source("block-image", Some("disk-root"));
        value["quota"] = json!({ "maxBytes": 8192, "enforcement": "none" });
        value["source"]["settings"]["imageFormat"] = json!("qcow2");
        value["source"]["settings"]["preallocate"] = json!(true);
        let spec: VolumeSpec = serde_json::from_value(value).expect("valid block image");
        let plan = BlockImagePlan::from_spec(&spec).unwrap();
        assert_eq!(plan.max_bytes(), 8192);
        assert_eq!(plan.format(), BlockImageFormat::Qcow2);
        assert!(plan.preallocate());
    }

    #[test]
    fn source_policy_catalog_debug_does_not_publish_opaque_ids() {
        let catalog = SourcePolicyCatalog::new([SourcePolicy::new(
            "state-root",
            SourceKind::LocalPath,
            [VolumeKind::State],
        )
        .unwrap()])
        .unwrap();
        assert_eq!(format!("{catalog:?}"), "SourcePolicyCatalog(<redacted>)");
    }
}
