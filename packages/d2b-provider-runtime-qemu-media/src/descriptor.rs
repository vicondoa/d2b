//! Signed Provider descriptor projection.

/// Descriptor validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptorError {
    /// The signed descriptor is missing a required contract.
    MissingContract,
    /// A Provider state Volume was declared.
    StateVolumeDeclared,
}

impl core::fmt::Display for DescriptorError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::MissingContract => "runtime-qemu-media-descriptor-contract-missing",
            Self::StateVolumeDeclared => "runtime-qemu-media-state-volume-forbidden",
        })
    }
}

impl std::error::Error for DescriptorError {}

/// Immutable qemu-media Provider descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QemuMediaProviderDescriptor {
    /// Signed schema version.
    pub schema_version: u32,
    /// Whether the descriptor declares Provider state namespaces.
    pub provider_state_volume: bool,
}

impl Default for QemuMediaProviderDescriptor {
    fn default() -> Self {
        Self {
            schema_version: 1,
            provider_state_volume: false,
        }
    }
}

impl QemuMediaProviderDescriptor {
    /// ResourceTypes owned by the Provider.
    pub const fn resource_types(&self) -> &'static [&'static str] {
        &["Guest"]
    }

    /// Provider state namespaces. This list is intentionally empty.
    pub const fn state_namespaces(&self) -> &'static [&'static str] {
        &[]
    }

    /// Process templates declared by the signed descriptor.
    pub const fn process_templates(&self) -> &'static [&'static str] {
        &["qemu-media-runner", "runtime-qemu-media-controller"]
    }

    /// Validate the closed descriptor contract.
    pub const fn validate(&self) -> Result<(), DescriptorError> {
        if self.schema_version == 0 {
            Err(DescriptorError::MissingContract)
        } else if self.provider_state_volume {
            Err(DescriptorError::StateVolumeDeclared)
        } else {
            Ok(())
        }
    }
}

/// Compatibility alias used by Provider catalog code.
pub type ProviderDescriptor = QemuMediaProviderDescriptor;
