//! Bounded Cloud Hypervisor Provider configuration.

use std::fmt;

use d2b_contracts_provider::v3::credential::OpaqueAzureRef;
use d2b_contracts_resource::v3::ResourceRef;
use serde::{Deserialize, Serialize};

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
    /// ComponentSession polling interval.
    pub health_check_interval_ms: u32,
    /// ComponentSession attempt timeout.
    pub health_check_timeout_ms: u32,
    /// Consecutive failures before degradation.
    pub health_check_failure_threshold: u8,
    /// Startup deadline.
    pub startup_deadline_ms: u32,
}

impl CloudHypervisorConfig {
    /// Validate root configuration.
    pub fn validate(&self) -> Result<(), ()> {
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
            return Err(());
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
