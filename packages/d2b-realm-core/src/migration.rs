//! Typed migration-error envelopes for ADR 0043 clean cutover surfaces.

use crate::ids::CorrelationId;
use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{InstanceType, Schema, SchemaObject, SingleOrVec, StringValidation},
};
use serde::{Deserialize, Deserializer, Serialize};

/// Legacy surface that triggered a realm-native migration diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LegacySurface {
    Gateway,
    AcaSandbox,
    Group,
    Env,
    OldRealmEntrypoint,
    NodeQualifiedTarget,
}

/// Low-cardinality migration reason code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum MigrationReasonCode {
    LegacySurfaceDetected,
    MigrationRequired,
    OperatorMappingRequired,
    AmbiguousGroupMapping,
    UnsupportedAcaContract,
    LegacyRuntimeStateDetected,
    ImportFailed,
    RemovedOption,
}

impl MigrationReasonCode {
    /// Stable low-cardinality code.
    pub fn code(self) -> &'static str {
        match self {
            Self::LegacySurfaceDetected => "legacy-surface-detected",
            Self::MigrationRequired => "migration-required",
            Self::OperatorMappingRequired => "operator-mapping-required",
            Self::AmbiguousGroupMapping => "ambiguous-group-mapping",
            Self::UnsupportedAcaContract => "unsupported-aca-contract",
            Self::LegacyRuntimeStateDetected => "legacy-runtime-state-detected",
            Self::ImportFailed => "import-failed",
            Self::RemovedOption => "removed-option",
        }
    }
}

/// Maximum bytes in a migration diagnostic message.
pub const MAX_MIGRATION_MESSAGE_LEN: usize = 256;
/// Maximum bytes in a legacy-surface identifier.
pub const MAX_MIGRATION_LEGACY_ID_LEN: usize = 64;

/// Bounded, non-secret legacy surface id for diagnostics. Rejects path- and
/// credential-shaped strings and redacts `Debug` output.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct MigrationLegacyId(String);

impl MigrationLegacyId {
    /// Validate a bounded low-cardinality id.
    pub fn parse(raw: impl Into<String>) -> Option<Self> {
        let raw = raw.into();
        if raw.is_empty()
            || raw.len() > MAX_MIGRATION_LEGACY_ID_LEN
            || !raw
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return None;
        }
        let compact = raw
            .chars()
            .filter(|c| !matches!(c, '-' | '_' | '.'))
            .flat_map(char::to_lowercase)
            .collect::<String>();
        if ["secret", "password", "bearer", "token", "credential"]
            .iter()
            .any(|marker| compact.contains(marker))
        {
            return None;
        }
        Some(Self(raw))
    }

    /// Borrow the identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for MigrationLegacyId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "MigrationLegacyId(<{} bytes>)", self.0.len())
    }
}

impl<'de> Deserialize<'de> for MigrationLegacyId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?)
            .ok_or_else(|| serde::de::Error::custom("invalid migration legacy id"))
    }
}

impl JsonSchema for MigrationLegacyId {
    fn schema_name() -> String {
        "MigrationLegacyId".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
            string: Some(Box::new(StringValidation {
                max_length: Some(MAX_MIGRATION_LEGACY_ID_LEN as u32),
                min_length: Some(1),
                pattern: Some("^[A-Za-z0-9][A-Za-z0-9._-]*$".to_owned()),
            })),
            ..Default::default()
        })
    }
}

/// Fail-closed migration diagnostic for removed gateway/ACA/group/env surfaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MigrationErrorEnvelope {
    /// Legacy surface family.
    pub surface: LegacySurface,
    /// Low-cardinality reason.
    pub reason: MigrationReasonCode,
    /// Operator-safe bounded diagnostic.
    #[schemars(length(max = 256))]
    pub message: String,
    /// Optional bounded legacy option/state identifier; never a path or secret.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub legacy_id: Option<MigrationLegacyId>,
    /// Optional cross-log correlation id.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub correlation_id: Option<CorrelationId>,
}

impl MigrationErrorEnvelope {
    /// Construct with a bounded message.
    pub fn new(
        surface: LegacySurface,
        reason: MigrationReasonCode,
        message: impl Into<String>,
    ) -> Self {
        Self {
            surface,
            reason,
            message: bound_message(message.into()),
            legacy_id: None,
            correlation_id: None,
        }
    }

    /// Attach a bounded non-secret legacy id.
    pub fn with_legacy_id(mut self, legacy_id: MigrationLegacyId) -> Self {
        self.legacy_id = Some(legacy_id);
        self
    }

    /// Attach an audit correlation id.
    pub fn with_correlation_id(mut self, correlation_id: CorrelationId) -> Self {
        self.correlation_id = Some(correlation_id);
        self
    }
}

impl<'de> Deserialize<'de> for MigrationErrorEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Raw {
            surface: LegacySurface,
            reason: MigrationReasonCode,
            message: String,
            #[serde(default)]
            legacy_id: Option<MigrationLegacyId>,
            #[serde(default)]
            correlation_id: Option<CorrelationId>,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(Self {
            surface: raw.surface,
            reason: raw.reason,
            message: bound_message(raw.message),
            legacy_id: raw.legacy_id,
            correlation_id: raw.correlation_id,
        })
    }
}

fn bound_message(mut message: String) -> String {
    if message.len() > MAX_MIGRATION_MESSAGE_LEN {
        let mut end = MAX_MIGRATION_MESSAGE_LEN;
        while end > 0 && !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::schema_for;

    #[test]
    fn migration_reason_codes_are_stable() {
        assert_eq!(
            MigrationReasonCode::LegacySurfaceDetected.code(),
            "legacy-surface-detected"
        );
        assert_eq!(MigrationReasonCode::ImportFailed.code(), "import-failed");
    }

    #[test]
    fn migration_envelope_bounds_message_and_rejects_unknown_fields() {
        let long = "x".repeat(MAX_MIGRATION_MESSAGE_LEN + 50);
        let json = format!(
            "{{\"surface\":\"aca-sandbox\",\"reason\":\"migration-required\",\
            \"message\":\"{long}\",\"legacyId\":\"aca-sandbox\"}}"
        );
        let envelope: MigrationErrorEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(envelope.message.len(), MAX_MIGRATION_MESSAGE_LEN);
        assert_eq!(
            envelope.legacy_id.as_ref().map(MigrationLegacyId::as_str),
            Some("aca-sandbox")
        );
        assert!(!format!("{envelope:?}").contains("aca-sandbox"));
        assert!(MigrationLegacyId::parse("/var/lib/d2b").is_none());
        assert!(MigrationLegacyId::parse("bearer-token").is_none());

        let bad = "{\"surface\":\"group\",\"reason\":\"migration-required\",\
            \"message\":\"x\",\"path\":\"/var/lib/d2b\"}";
        assert!(serde_json::from_str::<MigrationErrorEnvelope>(bad).is_err());
    }

    #[test]
    fn migration_schema_is_generated() {
        assert!(
            schema_for!(MigrationErrorEnvelope)
                .schema
                .metadata
                .is_some()
        );
    }
}
