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
#[serde(
    rename_all = "camelCase",
    deny_unknown_fields,
    try_from = "VsockTransportSettingsWire"
)]
pub struct VsockTransportSettings {
    guest_ref: String,
    port_class: PortClass,
    connect_timeout_seconds: u16,
}

/// Untrusted wire mirror for [`VsockTransportSettings`]; deserialization
/// routes through the validating conversion so a derived path can never admit
/// unvalidated settings.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VsockTransportSettingsWire {
    guest_ref: String,
    #[serde(default)]
    port_class: PortClass,
    #[serde(default = "default_timeout_seconds")]
    connect_timeout_seconds: u16,
}

impl VsockTransportSettings {
    /// Construct validated settings.
    ///
    /// # Errors
    ///
    /// Returns [`SettingsError::InvalidValue`] when the guest reference is
    /// not a bounded `Guest/...` reference or the timeout is outside
    /// `1..=60` seconds.
    pub fn new(guest_ref: impl Into<String>) -> Result<Self, SettingsError> {
        let settings = Self {
            guest_ref: guest_ref.into(),
            port_class: PortClass::default(),
            connect_timeout_seconds: default_timeout_seconds(),
        };
        settings.validate()?;
        Ok(settings)
    }

    /// Borrow the same-child-Zone Guest reference.
    pub fn guest_ref(&self) -> &str {
        &self.guest_ref
    }

    /// Return the allocator-owned port class.
    pub const fn port_class(&self) -> PortClass {
        self.port_class
    }

    /// Return the open deadline in seconds.
    pub const fn connect_timeout_seconds(&self) -> u16 {
        self.connect_timeout_seconds
    }

    /// Validate settings and reject raw endpoint material.
    ///
    /// # Errors
    ///
    /// Returns [`SettingsError::InvalidValue`] when the guest reference is
    /// not a bounded `Guest/...` reference or the timeout is outside
    /// `1..=60` seconds.
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

impl TryFrom<VsockTransportSettingsWire> for VsockTransportSettings {
    type Error = SettingsError;

    fn try_from(wire: VsockTransportSettingsWire) -> Result<Self, Self::Error> {
        let settings = Self {
            guest_ref: wire.guest_ref,
            port_class: wire.port_class,
            connect_timeout_seconds: wire.connect_timeout_seconds,
        };
        settings.validate()?;
        Ok(settings)
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
