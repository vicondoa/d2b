//! Neutral `virtiofs.d2bus.org.Export` ResourceType contracts.
//!
//! An Export names a Volume view and an execution target.  It never carries a
//! host path, socket path, shared-directory path, argv, or numeric identity.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    ResourceRef,
    execution_policy::{BoundedToken, PrimitiveSpecError},
    resource_status::{ResourceCondition, ResourcePhase},
    volume::{AttachmentAccess, validate_mount_path},
};

/// Canonical qualified Export ResourceType.
pub const VIRTIOFS_EXPORT_RESOURCE_TYPE: &str = "virtiofs.d2bus.org.Export";

/// Maximum Guest-visible mount path length.
pub const MAX_EXPORT_MOUNT_PATH_BYTES: usize = 255;

/// Strict base Export specification.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VirtiofsExportSpec {
    provider_ref: ResourceRef,
    volume_ref: ResourceRef,
    execution_ref: ResourceRef,
    view: BoundedToken,
    access: AttachmentAccess,
    mount_path: String,
}

impl VirtiofsExportSpec {
    /// Construct a strict Export specification from typed references.
    pub fn new(
        provider_ref: ResourceRef,
        volume_ref: ResourceRef,
        execution_ref: ResourceRef,
        view: impl Into<String>,
        access: AttachmentAccess,
        mount_path: impl Into<String>,
    ) -> Result<Self, PrimitiveSpecError> {
        if provider_ref.resource_type().as_str() != "Provider"
            || provider_ref.name().as_str() != "volume-virtiofs"
            || volume_ref.resource_type().as_str() != "Volume"
            || execution_ref.resource_type().as_str() != "Guest"
        {
            return Err(PrimitiveSpecError::WrongResourceType);
        }
        let view = BoundedToken::parse(view.into())?;
        let mount_path = mount_path.into();
        if !validate_mount_path(&mount_path) {
            return Err(PrimitiveSpecError::InvalidPath);
        }
        Ok(Self {
            provider_ref,
            volume_ref,
            execution_ref,
            view,
            access,
            mount_path,
        })
    }

    /// Return the qualified ResourceType name.
    pub const fn resource_type(&self) -> &'static str {
        VIRTIOFS_EXPORT_RESOURCE_TYPE
    }

    /// Borrow the selected Provider.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Borrow the Volume being exported.
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

    /// Borrow the Guest-visible mount path.
    pub fn mount_path(&self) -> &str {
        &self.mount_path
    }
}

impl core::fmt::Debug for VirtiofsExportSpec {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("VirtiofsExportSpec(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for VirtiofsExportSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            provider_ref: ResourceRef,
            volume_ref: ResourceRef,
            execution_ref: ResourceRef,
            view: String,
            access: AttachmentAccess,
            mount_path: String,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.provider_ref,
            wire.volume_ref,
            wire.execution_ref,
            wire.view,
            wire.access,
            wire.mount_path,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Public Export status resource projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VirtiofsExportStatusResource {
    /// Whether the Host-side worker is serving.
    pub export_ready: bool,
    /// Whether the target Guest reports the mount.
    pub guest_mount_ready: bool,
    /// The owned worker Process, when one exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_process_ref: Option<ResourceRef>,
}

/// Public Export status projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VirtiofsExportStatus {
    /// Universal lifecycle phase.
    pub phase: ResourcePhase,
    /// Universal conditions.
    #[serde(default)]
    pub conditions: Vec<ResourceCondition>,
    /// Export-specific readiness facts.
    pub resource: VirtiofsExportStatusResource,
}
