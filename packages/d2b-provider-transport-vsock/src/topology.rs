//! Child-local ZoneLink topology validation.

use crate::{SettingsError, VsockTransportSettings};
use std::fmt;

/// Bounded ZoneLink reconnect limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportLimits {
    /// Maximum pending intents.
    pub max_pending_intents: u16,
    /// Maximum active streams.
    pub max_active_streams: u16,
    /// Maximum reconnect attempts.
    pub reconnect_max_attempts: u16,
    /// Reconnect window in seconds.
    pub reconnect_window_secs: u32,
}

impl Default for TransportLimits {
    fn default() -> Self {
        Self {
            max_pending_intents: 256,
            max_active_streams: 32,
            reconnect_max_attempts: 10,
            reconnect_window_secs: 300,
        }
    }
}

/// Exact child-local ZoneLink desired shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneLinkSpec {
    /// Self-matching child Zone name.
    pub child_zone_name: String,
    /// Selected Provider reference.
    pub transport_provider_ref: String,
    /// Provider-specific closed settings.
    pub transport_settings: VsockTransportSettings,
    /// Vsock credentials must be empty.
    pub transport_credentials: Vec<String>,
    /// Whether the link is disabled.
    pub disabled: bool,
    /// Bounded reconnect and stream limits.
    pub limits: TransportLimits,
}

/// Topology validation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopologyError {
    /// The Provider is not the canonical vsock Provider.
    ProviderMismatch,
    /// The child name does not match the owning Zone.
    ChildZoneMismatch,
    /// The Provider settings are invalid.
    InvalidSettings,
    /// Vsock transport credentials are not permitted.
    CredentialsNotEmpty,
    /// The parent store contains a reciprocal resource.
    ParentStoreReciprocalResource,
}

impl fmt::Display for TopologyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ProviderMismatch => "transport-provider-mismatch",
            Self::ChildZoneMismatch => "transport-child-zone-mismatch",
            Self::InvalidSettings => "transport-settings-invalid",
            Self::CredentialsNotEmpty => "transport-credentials-not-empty",
            Self::ParentStoreReciprocalResource => "parent-store-reciprocal-resource",
        })
    }
}

impl std::error::Error for TopologyError {}

impl From<SettingsError> for TopologyError {
    fn from(_: SettingsError) -> Self {
        Self::InvalidSettings
    }
}

impl ZoneLinkSpec {
    /// Validate the exact child-local topology.
    pub fn validate(&self, owning_child_zone: &str) -> Result<(), TopologyError> {
        if self.transport_provider_ref != crate::PROVIDER_REF {
            return Err(TopologyError::ProviderMismatch);
        }
        if self.child_zone_name != owning_child_zone {
            return Err(TopologyError::ChildZoneMismatch);
        }
        self.transport_settings.validate()?;
        if !self.transport_credentials.is_empty() {
            return Err(TopologyError::CredentialsNotEmpty);
        }
        Ok(())
    }
}

/// Resource census for the selected parent store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ParentStoreResourceCensus {
    /// Number of reciprocal Provider rows.
    pub provider_rows: u16,
    /// Number of reciprocal ZoneLink rows.
    pub zone_link_rows: u16,
}

impl ParentStoreResourceCensus {
    /// Refuse any parent-side resource row for a child-local transport.
    pub fn validate(self) -> Result<(), TopologyError> {
        if self.provider_rows != 0 || self.zone_link_rows != 0 {
            return Err(TopologyError::ParentStoreReciprocalResource);
        }
        Ok(())
    }
}
