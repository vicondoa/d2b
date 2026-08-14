//! Bounded Cloud Hypervisor Provider configuration.

use std::fmt;

use d2b_contracts::v3::{ResourceRef, credential::OpaqueAzureRef};
use serde::{Deserialize, Serialize};

/// Console mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum ConsoleType {
    /// Headless console.
    Null,
    /// Virtio console.
    Virtio,
}

/// Provider root configuration.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloudHypervisorConfig {
    /// Explicit Host execution reference.
    pub controller_execution_ref: ResourceRef,
    /// Default VCPU count.
    pub default_vcpus: u16,
    /// Default memory in MiB.
    pub default_memory_mb: u32,
    /// Default machine type.
    pub default_machine_type: OpaqueAzureRef,
    /// Whether the VMM watchdog is enabled.
    pub watchdog: bool,
    /// Maximum adoption window in milliseconds.
    pub adoption_window_ms: u32,
    /// Guest-control polling interval.
    pub health_check_interval_ms: u32,
    /// Guest-control attempt timeout.
    pub health_check_timeout_ms: u32,
    /// Consecutive failures before degradation.
    pub health_check_failure_threshold: u8,
    /// Startup deadline.
    pub startup_deadline_ms: u32,
}

impl CloudHypervisorConfig {
    /// Validate root configuration.
    pub fn validate(&self) -> Result<(), CloudHypervisorConfigError> {
        if self.controller_execution_ref.resource_type().as_str() != "Host"
            || !(1..=1024).contains(&self.default_vcpus)
            || !(128..=524_288).contains(&self.default_memory_mb)
            || !matches!(self.default_machine_type.as_str(), "q35" | "microvm")
            || !(1..=900_000).contains(&self.adoption_window_ms)
            || !(5_000..=300_000).contains(&self.health_check_interval_ms)
            || !(1_000..=60_000).contains(&self.health_check_timeout_ms)
            || self.health_check_failure_threshold == 0
            || !(1..=900_000).contains(&self.startup_deadline_ms)
        {
            return Err(CloudHypervisorConfigError::Invalid);
        }
        Ok(())
    }
}

impl fmt::Debug for CloudHypervisorConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CloudHypervisorConfig")
            .field("controller_execution_ref", &"<redacted>")
            .field("default_vcpus", &self.default_vcpus)
            .field("default_memory_mb", &self.default_memory_mb)
            .field("default_machine_type", &self.default_machine_type)
            .field("watchdog", &self.watchdog)
            .field("adoption_window_ms", &self.adoption_window_ms)
            .field("health_check_interval_ms", &self.health_check_interval_ms)
            .field("health_check_timeout_ms", &self.health_check_timeout_ms)
            .field(
                "health_check_failure_threshold",
                &self.health_check_failure_threshold,
            )
            .field("startup_deadline_ms", &self.startup_deadline_ms)
            .finish()
    }
}

/// Guest-specific VMM settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloudHypervisorGuestSettings {
    /// VCPU count override.
    pub vcpus: Option<u16>,
    /// Memory override.
    pub memory_mb: Option<u32>,
    /// Machine type override.
    pub machine_type: Option<OpaqueAzureRef>,
    /// Console mode.
    pub console_type: ConsoleType,
    /// Whether a serial console is emitted.
    pub serial_port: bool,
    /// Whether pvpanic is enabled.
    pub pvpanic: bool,
    /// Optional watchdog override.
    pub watchdog_override: Option<bool>,
    /// Whether shared memory is enabled.
    pub memory_shared: bool,
    /// Whether a virtiofs attachment exists.
    pub has_virtiofs_attachment: bool,
    /// Required top-level system artifact id.
    pub system_artifact_id: Option<String>,
}

impl CloudHypervisorGuestSettings {
    /// Validate Guest settings and the system artifact rule.
    pub fn validate(&self) -> Result<(), CloudHypervisorConfigError> {
        if self.vcpus.is_some_and(|value| !(1..=1024).contains(&value))
            || self
                .memory_mb
                .is_some_and(|value| !(128..=524_288).contains(&value))
            || self
                .machine_type
                .as_ref()
                .is_some_and(|value| !matches!(value.as_str(), "q35" | "microvm"))
            || (!self.memory_shared && self.has_virtiofs_attachment)
            || self.system_artifact_id.is_none()
            || self
                .system_artifact_id
                .as_ref()
                .is_some_and(|id| id.is_empty() || id.len() > 63 || !valid_token(id))
        {
            return Err(CloudHypervisorConfigError::Invalid);
        }
        Ok(())
    }
}

/// Configuration validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudHypervisorConfigError {
    /// A bound, reference, or required field was invalid.
    Invalid,
}

fn valid_token(value: &str) -> bool {
    value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}
