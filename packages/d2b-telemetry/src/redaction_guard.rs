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
}

impl core::fmt::Display for RedactionError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::AttributeNotAllowlisted => "otel-resource-attribute-not-allowlisted",
            Self::ForbiddenSpanField => "otel-span-field-forbidden",
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
                    .any(|byte| byte.is_ascii_control() || byte == b'/')
            {
                return Err(RedactionError::AttributeNotAllowlisted);
            }
            values.insert(key, value);
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
            "workload_id",
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
                    .any(|byte| byte.is_ascii_control() || byte == b'/')
            {
                return Err(RedactionError::ForbiddenSpanField);
            }
            output.insert(key, "<bounded>".to_owned());
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
    }

    #[test]
    fn sensitive_span_fields_are_rejected() {
        for field in ["path", "socket", "argv", "pid", "realm", "no_isolation"] {
            assert_eq!(
                RedactionGuard::validate_span_field(field),
                Err(RedactionError::ForbiddenSpanField)
            );
        }
    }
}
