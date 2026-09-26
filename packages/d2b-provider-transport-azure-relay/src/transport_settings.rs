//! Azure Relay transport settings schema and validation.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Bounded non-secret Relay settings.
///
/// Deserialization runs the same validation as [`RelayTransportSettings::new`]
/// through [`RelayTransportSettingsWire`], so a value that arrives over the
/// wire cannot be admitted when the constructor would refuse it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RelayTransportSettingsWire")]
pub struct RelayTransportSettings {
    /// Bare namespace identifier, without scheme or DNS suffix.
    pub relay_namespace_id: String,
    /// Hybrid Connection entity identifier.
    pub relay_entity_id: String,
}

/// Unvalidated wire form of [`RelayTransportSettings`].
///
/// The shape is the only thing this type decides; every admitted value is
/// handed to [`RelayTransportSettings::new`] before it becomes settings.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RelayTransportSettingsWire {
    relay_namespace_id: String,
    relay_entity_id: String,
}

impl TryFrom<RelayTransportSettingsWire> for RelayTransportSettings {
    type Error = RelayTransportSettingsError;

    fn try_from(wire: RelayTransportSettingsWire) -> Result<Self, Self::Error> {
        Self::new(wire.relay_namespace_id, wire.relay_entity_id)
    }
}

impl RelayTransportSettings {
    /// Construct validated settings.
    ///
    /// # Errors
    ///
    /// Returns [`RelayTransportSettingsError`] when the namespace or entity
    /// fails validation.
    pub fn new(
        relay_namespace_id: impl Into<String>,
        relay_entity_id: impl Into<String>,
    ) -> Result<Self, RelayTransportSettingsError> {
        let settings = Self {
            relay_namespace_id: relay_namespace_id.into(),
            relay_entity_id: relay_entity_id.into(),
        };
        settings.validate()?;
        Ok(settings)
    }

    /// Validate settings and reject secret-shaped fields.
    pub fn validate(&self) -> Result<(), RelayTransportSettingsError> {
        if !valid_namespace(&self.relay_namespace_id)
            || !valid_entity(&self.relay_entity_id)
            || self.relay_namespace_id.contains('/')
            || self.relay_namespace_id.contains(':')
            || self.relay_entity_id.contains("SharedAccessSignature")
        {
            return Err(RelayTransportSettingsError::InvalidIdentifier);
        }
        Ok(())
    }

    /// Return the signed JSON schema source.
    pub const fn schema_json() -> &'static str {
        include_str!(
            "../../../docs/reference/schemas/v3/providers/transport-azure-relay.transport-settings.json"
        )
    }
}

impl fmt::Debug for RelayTransportSettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RelayTransportSettings")
            .field("relay_namespace_id", &"<redacted>")
            .field("relay_entity_id", &"<redacted>")
            .finish()
    }
}

/// Settings validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayTransportSettingsError {
    /// Identifier grammar, bound, or secret-shape check failed.
    InvalidIdentifier,
}

impl fmt::Display for RelayTransportSettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentifier => formatter.write_str(
                "relay transport settings must carry a bounded non-secret namespace and entity",
            ),
        }
    }
}

impl std::error::Error for RelayTransportSettingsError {}

fn valid_namespace(value: &str) -> bool {
    (3..=50).contains(&value.len())
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn valid_entity(value: &str) -> bool {
    (2..=50).contains(&value.len())
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}
