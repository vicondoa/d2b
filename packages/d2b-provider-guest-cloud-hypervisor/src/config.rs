//! Bounded Cloud Hypervisor Provider configuration.

use std::fmt;

use d2b_contracts_resource::v3::ResourceRef;
use serde::{Deserialize, Serialize};

/// Machine type a Cloud Hypervisor VMM starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MachineType {
    /// Modern Q35 chipset.
    #[serde(rename = "q35")]
    Q35,
    /// Minimal MicroVM chipset.
    Microvm,
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
    pub default_machine_type: MachineType,
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

/// Closed validation failure for root Provider configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigValidationError;

impl fmt::Display for ConfigValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("cloud-hypervisor-config-invalid")
    }
}

impl std::error::Error for ConfigValidationError {}

impl CloudHypervisorConfig {
    /// Validate root configuration.
    pub fn validate(&self) -> Result<(), ConfigValidationError> {
        if self.controller_execution_ref.resource_type().as_str() != "Host"
            || !(1..=1024).contains(&self.default_vcpus)
            || !(128..=524_288).contains(&self.default_memory_mb)
            || !(1..=900_000).contains(&self.adoption_window_ms)
            || !(5_000..=300_000).contains(&self.health_check_interval_ms)
            || !(1_000..=60_000).contains(&self.health_check_timeout_ms)
            || self.health_check_failure_threshold == 0
            || !(1..=900_000).contains(&self.startup_deadline_ms)
        {
            return Err(ConfigValidationError);
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

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT_CONFIG: &str = r#"{
        "controllerExecutionRef": "Host/host-system",
        "defaultVcpus": 2,
        "defaultMemoryMb": 512,
        "defaultMachineType": "q35",
        "watchdog": true,
        "adoptionWindowMs": 30000,
        "healthCheckIntervalMs": 30000,
        "healthCheckTimeoutMs": 5000,
        "healthCheckFailureThreshold": 3,
        "startupDeadlineMs": 120000
    }"#;

    #[test]
    fn machine_type_is_the_closed_q35_microvm_wire() {
        let config: CloudHypervisorConfig =
            serde_json::from_str(ROOT_CONFIG).expect("q35 root config decodes");
        assert_eq!(config.default_machine_type, MachineType::Q35);
        assert!(config.validate().is_ok());

        let microvm: CloudHypervisorConfig =
            serde_json::from_str(&ROOT_CONFIG.replace(r#""q35""#, r#""microvm""#))
                .expect("microvm root config decodes");
        assert_eq!(microvm.default_machine_type, MachineType::Microvm);
        assert!(microvm.validate().is_ok());

        let unknown = ROOT_CONFIG.replace(r#""q35""#, r#""guest-vm""#);
        assert!(serde_json::from_str::<CloudHypervisorConfig>(&unknown).is_err());

        let q35 = serde_json::to_string(&MachineType::Q35).expect("q35 wire");
        let microvm_wire = serde_json::to_string(&MachineType::Microvm).expect("microvm wire");
        assert_eq!(q35, r#""q35""#);
        assert_eq!(microvm_wire, r#""microvm""#);
    }
}
