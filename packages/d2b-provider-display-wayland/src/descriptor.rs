//! Signed display Provider descriptor projection.

/// Provider-specific descriptor validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayDescriptorError {
    /// A required service or resource type is missing.
    MissingContract,
    /// A Provider state Volume was declared.
    StateVolumeDeclared,
}

impl core::fmt::Display for DisplayDescriptorError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::MissingContract => "display-descriptor-contract-missing",
            Self::StateVolumeDeclared => "display-provider-state-volume-forbidden",
        })
    }
}

impl std::error::Error for DisplayDescriptorError {}

/// Immutable descriptor emitted by the display artifact catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayProviderDescriptor {
    /// Signed schema version.
    pub schema_version: u32,
    /// Whether this Provider declares a state Volume.
    pub provider_state_volume: bool,
}

impl Default for DisplayProviderDescriptor {
    fn default() -> Self {
        Self {
            schema_version: 1,
            provider_state_volume: false,
        }
    }
}

impl DisplayProviderDescriptor {
    /// Qualified ResourceTypes exported by the Provider.
    pub const fn resource_types(&self) -> &'static [&'static str] {
        &[
            "display-wayland.d2bus.org.WaylandSession",
            "display-wayland.d2bus.org.WaylandPolicy",
            "Endpoint",
        ]
    }

    /// ComponentSession services consumed or served by the Provider.
    pub const fn service_packages(&self) -> &'static [&'static str] {
        &["d2b.display.host-clipboard.v3", "d2b.clipboard.bridge.v3"]
    }

    /// Validate the descriptor's closed contract.
    pub const fn validate(&self) -> Result<(), DisplayDescriptorError> {
        if self.provider_state_volume {
            Err(DisplayDescriptorError::StateVolumeDeclared)
        } else if self.schema_version == 0 {
            Err(DisplayDescriptorError::MissingContract)
        } else {
            Ok(())
        }
    }
}
