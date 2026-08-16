//! Closed `ZoneLink.spec.transportSettings` validation.

use serde::{Deserialize, Serialize};
use std::fmt;

/// The allocator port class used by ZoneLink vsock sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PortClass {
    /// The reserved ZoneLink range.
    #[default]
    D2bLink,
}

/// Provider-specific transport settings.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VsockTransportSettings {
    /// Same-child-Zone Guest reference.
    pub guest_ref: String,
    /// Allocator-owned port class.
    #[serde(default)]
    pub port_class: PortClass,
    /// Open deadline in seconds.
    #[serde(default = "default_timeout_seconds")]
    pub connect_timeout_seconds: u16,
}

impl VsockTransportSettings {
    /// Construct validated settings.
    pub fn new(guest_ref: impl Into<String>) -> Result<Self, SettingsError> {
        let settings = Self {
            guest_ref: guest_ref.into(),
            port_class: PortClass::default(),
            connect_timeout_seconds: default_timeout_seconds(),
        };
        settings.validate()?;
        Ok(settings)
    }

    /// Validate settings and reject raw endpoint material.
    pub fn validate(&self) -> Result<(), SettingsError> {
        if !self.guest_ref.starts_with("Guest/")
            || self.guest_ref.len() <= "Guest/".len()
            || self.guest_ref.len() > 128
            || !(1..=60).contains(&self.connect_timeout_seconds)
        {
            return Err(SettingsError::InvalidValue);
        }
        Ok(())
    }

    /// Return the committed schema source.
    pub const fn schema_json() -> &'static str {
        include_str!(
            "../../../docs/reference/schemas/v3/providers/transport-vsock.transport-binding.json"
        )
    }
}

impl fmt::Debug for VsockTransportSettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VsockTransportSettings")
            .field("guest_ref", &"<redacted>")
            .field("port_class", &self.port_class)
            .field("connect_timeout_seconds", &self.connect_timeout_seconds)
            .finish()
    }
}

/// Settings validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsError {
    /// A field is empty, out of range, or carries a raw endpoint.
    InvalidValue,
}

impl fmt::Display for SettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("transport-settings-invalid")
    }
}

impl std::error::Error for SettingsError {}

const fn default_timeout_seconds() -> u16 {
    30
}
