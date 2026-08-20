//! Span and resource-attribute redaction policy.

use std::collections::BTreeMap;

use crate::metric_label_policy::OTEL_RESOURCE_ATTRIBUTES;

/// Maximum bytes in one OTEL resource attribute value.
pub const MAX_RESOURCE_ATTRIBUTE_BYTES: usize = 256;

/// A redaction policy failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedactionError {
    /// The attribute key is outside the v3 resource allowlist.
    AttributeNotAllowlisted,
    /// A high-risk field was attempted as a span attribute.
    ForbiddenSpanField,
    /// A span field is not in the closed semantic registry.
    SemanticFieldNotAllowlisted,
    /// A span field value is outside its closed semantic domain.
    SemanticValueNotAllowlisted,
}

impl core::fmt::Display for RedactionError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::AttributeNotAllowlisted => "otel-resource-attribute-not-allowlisted",
            Self::ForbiddenSpanField => "otel-span-field-forbidden",
            Self::SemanticFieldNotAllowlisted => "otel-span-field-not-allowlisted",
            Self::SemanticValueNotAllowlisted => "otel-span-value-not-allowlisted",
        })
    }
}

impl std::error::Error for RedactionError {}

/// A validated set of resource attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactionGuard {
    attributes: BTreeMap<String, String>,
}

impl RedactionGuard {
    /// Validate resource attributes against the closed allowlist.
    pub fn new(
        attributes: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Result<Self, RedactionError> {
        let mut values = BTreeMap::new();
        for (key, value) in attributes {
            let key = key.into();
            let value = value.into();
            if !OTEL_RESOURCE_ATTRIBUTES.contains(&key.as_str()) {
                return Err(RedactionError::AttributeNotAllowlisted);
            }
            if value.is_empty()
                || value.len() > MAX_RESOURCE_ATTRIBUTE_BYTES
                || value
                    .bytes()
                    .any(|byte| !byte.is_ascii_graphic() || byte == b'/')
            {
                return Err(RedactionError::AttributeNotAllowlisted);
            }
            let value = redact_attribute_value(&key, &value);
            if values.insert(key, value).is_some() {
                return Err(RedactionError::AttributeNotAllowlisted);
            }

            fn redact_attribute_value(key: &str, value: &str) -> String {
                let identity_key = matches!(
                    key,
                    "d2b.zone" | "vm.name" | "vm.env" | "vm.role" | "host.name" | "source"
                );
                if identity_key || value.contains('/') || value.chars().any(char::is_whitespace) {
                    if d2b_contracts_zone_session::v3::is_canonical_digest(value) {
                        value.to_owned()
                    } else {
                        d2b_contracts_zone_session::v3::canonical_digest(
                            "d2b:telemetry-redaction:v1",
                            value.as_bytes(),
                        )
                    }
                } else {
                    value.to_owned()
                }
            }
        }
        Ok(Self { attributes: values })
    }

    /// Borrow the validated resource attributes.
    pub fn attributes(&self) -> &BTreeMap<String, String> {
        &self.attributes
    }

    /// Validate a span field name before it is emitted.
    pub fn validate_span_field(key: &str) -> Result<(), RedactionError> {
        const FORBIDDEN: &[&str] = &[
            "path",
            "socket",
            "argv",
            "env",
            "pid",
            "exe",
            "realm",
            "node",
            "workload",
            "workload_id",
            "credential",
            "secret",
            "token",
            "resource_ref",
            "resource_uid",
            "subject",
            "principal",
            "name",
            "uid",
            "trace_id",
            "span_id",
            "no_isolation",
            "zone",
            "zone_id",
            "zone_uid",
            "resource_name",
            "metadata",
            "uid",
        ];
        if FORBIDDEN.contains(&key)
            || key.ends_with("_path")
            || key.ends_with("_uid")
            || key.ends_with("_name")
            || key.ends_with("_name_hash")
            || key.ends_with("_name_digest")
            || key.ends_with("_digest")
        {
            return Err(RedactionError::ForbiddenSpanField);
        }
        Ok(())
    }

    /// Validate all span fields and return an owned redacted map.
    pub fn span_attributes(
        fields: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Result<BTreeMap<String, String>, RedactionError> {
        let mut output = BTreeMap::new();
        for (key, value) in fields {
            let key = key.into();
            Self::validate_span_field(&key)?;
            let value = value.into();
            if value.is_empty()
                || value.len() > MAX_RESOURCE_ATTRIBUTE_BYTES
                || value
                    .bytes()
                    .any(|byte| !byte.is_ascii_graphic() || byte == b'/')
            {
                return Err(RedactionError::ForbiddenSpanField);
            }
            let allowed = d2b_contracts_provider::v3::telemetry_policy::allowed_values(&key)
                .ok_or(RedactionError::SemanticFieldNotAllowlisted)?;
            if !allowed.contains(&value.as_str()) {
                return Err(RedactionError::SemanticValueNotAllowlisted);
            }
            let stored = value;
            if output.insert(key, stored).is_some() {
                return Err(RedactionError::ForbiddenSpanField);
            }
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_allowlist_contains_v3_identity_attributes() {
        let guard = RedactionGuard::new([
            ("service.name", "d2b-test"),
            ("d2b.zone", "local"),
            ("service.version", "0.0.0"),
        ])
        .unwrap();
        assert!(guard.attributes().contains_key("d2b.zone"));
        assert!(!guard.attributes().values().any(|value| value == "local"));
    }

    #[test]
    fn sensitive_span_fields_are_rejected() {
        for field in [
            "path",
            "socket",
            "argv",
            "pid",
            "realm",
            "node",
            "resource_ref",
            "subject",
            "no_isolation",
        ] {
            assert_eq!(
                RedactionGuard::validate_span_field(field),
                Err(RedactionError::ForbiddenSpanField)
            );
        }
    }

    #[test]
    fn unknown_semantic_fields_and_values_fail_closed() {
        assert_eq!(
            RedactionGuard::span_attributes([("unknown", "value")]),
            Err(RedactionError::SemanticFieldNotAllowlisted)
        );
        assert_eq!(
            RedactionGuard::span_attributes([("operation", "not-a-store-operation")]),
            Err(RedactionError::SemanticValueNotAllowlisted)
        );
    }

    #[test]
    fn canonical_semantic_fields_are_preserved() {
        let fields = RedactionGuard::span_attributes([
            ("kind", "single"),
            ("operation", "scan"),
            ("op", "vmStart"),
            ("service", "store"),
            ("transport", "unix"),
            ("profile", "NN"),
        ])
        .expect("closed semantic fields");
        assert_eq!(fields.get("kind").map(String::as_str), Some("single"));
        assert_eq!(fields.get("operation").map(String::as_str), Some("scan"));
        assert_eq!(fields.get("op").map(String::as_str), Some("vmStart"));
        assert_eq!(fields.get("service").map(String::as_str), Some("store"));
        assert_eq!(fields.get("transport").map(String::as_str), Some("unix"));
        assert_eq!(fields.get("profile").map(String::as_str), Some("NN"));
        assert!(!fields.values().any(|value| value == "<bounded>"));
    }
}
