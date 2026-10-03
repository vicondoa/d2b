//! Signed notification Provider descriptor projection.

/// Notification descriptor validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationDescriptorError {
    /// A required contract is absent.
    MissingContract,
    /// A Provider state Volume was declared.
    StateVolumeDeclared,
}

impl core::fmt::Display for NotificationDescriptorError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::MissingContract => "notification-descriptor-contract-missing",
            Self::StateVolumeDeclared => "notification-provider-state-volume-forbidden",
        })
    }
}

impl std::error::Error for NotificationDescriptorError {}

/// Immutable descriptor emitted by the notification artifact catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationProviderDescriptor {
    /// Signed schema version.
    pub schema_version: u32,
    /// Whether the Provider declares a state Volume.
    pub provider_state_volume: bool,
}

impl Default for NotificationProviderDescriptor {
    fn default() -> Self {
        Self {
            schema_version: 1,
            provider_state_volume: false,
        }
    }
}

impl NotificationProviderDescriptor {
    /// Notification service package.
    pub const fn service_package(&self) -> &'static str {
        crate::SERVICE_PACKAGE
    }

    /// Notification named streams.
    ///
    /// The streams come from the Provider's declared service (U28), so the
    /// signed descriptor projection and the session layer read one source for
    /// the stream names rather than two lists that can drift.
    pub const fn streams(&self) -> &'static [&'static str] {
        crate::admission::NOTIFICATION_SERVICE.streams
    }

    /// Validate the descriptor contract.
    pub const fn validate(&self) -> Result<(), NotificationDescriptorError> {
        if self.provider_state_volume {
            Err(NotificationDescriptorError::StateVolumeDeclared)
        } else if self.schema_version == 0 {
            Err(NotificationDescriptorError::MissingContract)
        } else {
            Ok(())
        }
    }
}
