//! Bounded Provider configuration and controller-only projection.

use d2b_contracts::v3::ResourceRef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Default QMP greeting timeout in seconds.
pub const DEFAULT_QMP_READY_TIMEOUT_SECONDS: u32 = 30;
/// Default QMP command timeout in seconds.
pub const DEFAULT_QMP_OPERATION_TIMEOUT_SECONDS: u32 = 60;
/// Default runtime tmpfs quota in bytes.
pub const DEFAULT_RUNTIME_TMPFS_QUOTA_BYTES: u64 = 10 * 1024 * 1024;
/// Default runtime tmpfs inode quota.
pub const DEFAULT_RUNTIME_TMPFS_QUOTA_INODES: u32 = 1024;

/// Provider root configuration.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderConfig {
    /// Host on which the controller Process runs.
    pub controller_execution_ref: ResourceRef,
    /// Artifact catalog id for QEMU.
    pub qemu_binary_artifact_id: String,
    /// Initial QMP greeting deadline.
    pub qmp_ready_timeout_seconds: u32,
    /// Per-command QMP deadline.
    pub qmp_operation_timeout_seconds: u32,
    /// Default pause-at-boot setting.
    pub paused_at_boot_default: bool,
    /// Optional display Provider.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_provider_ref: Option<ResourceRef>,
    /// Network Provider.
    pub network_provider_ref: ResourceRef,
    /// Volume Provider.
    pub volume_provider_ref: ResourceRef,
    /// Runtime tmpfs byte quota.
    pub runtime_tmpfs_quota_bytes: u64,
    /// Runtime tmpfs inode quota.
    pub runtime_tmpfs_quota_inodes: u32,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            controller_execution_ref: ResourceRef::parse("Guest/invalid").expect("valid reference"),
            qemu_binary_artifact_id: "qemu-system-x86_64".to_owned(),
            qmp_ready_timeout_seconds: DEFAULT_QMP_READY_TIMEOUT_SECONDS,
            qmp_operation_timeout_seconds: DEFAULT_QMP_OPERATION_TIMEOUT_SECONDS,
            paused_at_boot_default: true,
            display_provider_ref: None,
            network_provider_ref: ResourceRef::parse("Provider/network-local")
                .expect("valid reference"),
            volume_provider_ref: ResourceRef::parse("Provider/volume-local")
                .expect("valid reference"),
            runtime_tmpfs_quota_bytes: DEFAULT_RUNTIME_TMPFS_QUOTA_BYTES,
            runtime_tmpfs_quota_inodes: DEFAULT_RUNTIME_TMPFS_QUOTA_INODES,
        }
    }
}

impl ProviderConfig {
    /// Construct a Provider configuration with defaults for bounded values.
    pub fn new(
        controller_execution_ref: impl Into<String>,
        qemu_binary_artifact_id: impl Into<String>,
        network_provider_ref: impl Into<String>,
        volume_provider_ref: impl Into<String>,
        display_provider_ref: Option<String>,
    ) -> Result<Self, ProviderConfigError> {
        let config = Self {
            controller_execution_ref: parse_ref(controller_execution_ref.into())?,
            qemu_binary_artifact_id: qemu_binary_artifact_id.into(),
            qmp_ready_timeout_seconds: DEFAULT_QMP_READY_TIMEOUT_SECONDS,
            qmp_operation_timeout_seconds: DEFAULT_QMP_OPERATION_TIMEOUT_SECONDS,
            paused_at_boot_default: true,
            display_provider_ref: display_provider_ref.map(parse_ref).transpose()?,
            network_provider_ref: parse_ref(network_provider_ref.into())?,
            volume_provider_ref: parse_ref(volume_provider_ref.into())?,
            runtime_tmpfs_quota_bytes: DEFAULT_RUNTIME_TMPFS_QUOTA_BYTES,
            runtime_tmpfs_quota_inodes: DEFAULT_RUNTIME_TMPFS_QUOTA_INODES,
        };
        config.validate()?;
        Ok(config)
    }

    /// Validate all Provider bounds and typed references.
    pub fn validate(&self) -> Result<(), ProviderConfigError> {
        if self.controller_execution_ref.resource_type().as_str() != "Host"
            || self.network_provider_ref.resource_type().as_str() != "Provider"
            || self.volume_provider_ref.resource_type().as_str() != "Provider"
            || self
                .display_provider_ref
                .as_ref()
                .is_some_and(|reference| reference.resource_type().as_str() != "Provider")
            || !valid_token(&self.qemu_binary_artifact_id)
            || !(5..=300).contains(&self.qmp_ready_timeout_seconds)
            || !(5..=300).contains(&self.qmp_operation_timeout_seconds)
            || !(1024 * 1024..=256 * 1024 * 1024).contains(&self.runtime_tmpfs_quota_bytes)
            || !(64..=65_536).contains(&self.runtime_tmpfs_quota_inodes)
        {
            return Err(ProviderConfigError::Invalid);
        }
        Ok(())
    }

    /// Project the fields needed by the controller Process only.
    pub fn project_controller(&self) -> ControllerConfigProjection {
        ControllerConfigProjection {
            controller_execution_ref: self.controller_execution_ref.to_canonical_string(),
            qmp_ready_timeout_seconds: self.qmp_ready_timeout_seconds,
            qmp_operation_timeout_seconds: self.qmp_operation_timeout_seconds,
            paused_at_boot_default: self.paused_at_boot_default,
            display_provider_configured: self.display_provider_ref.is_some(),
            runtime_tmpfs_quota_bytes: self.runtime_tmpfs_quota_bytes,
            runtime_tmpfs_quota_inodes: self.runtime_tmpfs_quota_inodes,
        }
    }

    /// Return the intentionally empty worker projection.
    pub const fn project_worker(&self) -> WorkerConfigProjection {
        WorkerConfigProjection
    }
}

impl core::fmt::Debug for ProviderConfig {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ProviderConfig")
            .field("controller_execution_ref", &"<redacted>")
            .field("qemu_binary_artifact_id", &"<redacted>")
            .field("qmp_ready_timeout_seconds", &self.qmp_ready_timeout_seconds)
            .field(
                "qmp_operation_timeout_seconds",
                &self.qmp_operation_timeout_seconds,
            )
            .field("paused_at_boot_default", &self.paused_at_boot_default)
            .field("display_provider_ref", &self.display_provider_ref.is_some())
            .field("network_provider_ref", &"<redacted>")
            .field("volume_provider_ref", &"<redacted>")
            .field("runtime_tmpfs_quota_bytes", &self.runtime_tmpfs_quota_bytes)
            .field(
                "runtime_tmpfs_quota_inodes",
                &self.runtime_tmpfs_quota_inodes,
            )
            .finish()
    }
}

/// Controller-only projection of Provider configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControllerConfigProjection {
    /// Host placement reference.
    pub controller_execution_ref: String,
    /// QMP greeting deadline.
    pub qmp_ready_timeout_seconds: u32,
    /// QMP operation deadline.
    pub qmp_operation_timeout_seconds: u32,
    /// Pause-at-boot default.
    pub paused_at_boot_default: bool,
    /// Whether display Provider configuration is available.
    pub display_provider_configured: bool,
    /// Runtime tmpfs byte quota.
    pub runtime_tmpfs_quota_bytes: u64,
    /// Runtime tmpfs inode quota.
    pub runtime_tmpfs_quota_inodes: u32,
}

impl ControllerConfigProjection {
    /// Borrow the controller placement reference.
    pub fn controller_execution_ref(&self) -> &str {
        &self.controller_execution_ref
    }
}

/// Empty worker configuration projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorkerConfigProjection;

/// Provider configuration validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderConfigError {
    /// A typed reference or bound is invalid.
    Invalid,
}

impl core::fmt::Display for ProviderConfigError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("runtime-qemu-media-config-invalid")
    }
}

impl std::error::Error for ProviderConfigError {}

fn parse_ref(value: String) -> Result<ResourceRef, ProviderConfigError> {
    ResourceRef::parse(&value).map_err(|_| ProviderConfigError::Invalid)
}

fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}
