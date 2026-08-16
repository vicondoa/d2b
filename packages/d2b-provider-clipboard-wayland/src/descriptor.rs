//! Signed clipboard Provider service and attachment descriptor.

/// Clipboard descriptor validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardDescriptorError {
    /// A required service or attachment class is missing.
    MissingContract,
    /// A Provider state Volume was declared.
    StateVolumeDeclared,
}

impl core::fmt::Display for ClipboardDescriptorError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::MissingContract => "clipboard-descriptor-contract-missing",
            Self::StateVolumeDeclared => "clipboard-provider-state-volume-forbidden",
        })
    }
}

impl std::error::Error for ClipboardDescriptorError {}

/// Immutable descriptor emitted by the clipboard artifact catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipboardProviderDescriptor {
    /// Signed schema version.
    pub schema_version: u32,
    /// Whether the Provider declares a state Volume.
    pub provider_state_volume: bool,
}

impl Default for ClipboardProviderDescriptor {
    fn default() -> Self {
        Self {
            schema_version: 1,
            provider_state_volume: false,
        }
    }
}

impl ClipboardProviderDescriptor {
    /// ComponentSession service packages.
    pub const fn service_packages(&self) -> &'static [&'static str] {
        &[
            "d2b.display.host-clipboard.v3",
            "d2b.clipboard.bridge.v3",
            "d2b.clipboard.picker-coord.v3",
            "d2b.clipboard.v3",
        ]
    }

    /// Attachment classes allowed by the descriptor.
    pub const fn attachment_classes(&self) -> &'static [&'static str] {
        &[
            "clipboard-transfer-fd",
            "host-selection-transfer-fd",
            "host-selection-supply-fd",
        ]
    }

    /// Validate the descriptor contract.
    pub const fn validate(&self) -> Result<(), ClipboardDescriptorError> {
        if self.provider_state_volume {
            Err(ClipboardDescriptorError::StateVolumeDeclared)
        } else if self.schema_version == 0 {
            Err(ClipboardDescriptorError::MissingContract)
        } else {
            Ok(())
        }
    }
}
