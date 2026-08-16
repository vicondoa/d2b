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
        "d2b.notification.v3"
    }

    /// Notification named streams.
    pub const fn streams(&self) -> &'static [&'static str] {
        &["DesktopNotificationSink", "DesktopNotificationObserver"]
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
