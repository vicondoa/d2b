//! A bounded trace context propagated across constellation peers (ADR
//! 0032). Deliberately minimal: opaque trace/span ids only. It carries
//! **no** baggage, secrets, store paths, or payload, and field lengths
//! are bounded so it cannot become a side channel.

use schemars::{
    gen::SchemaGenerator,
    schema::{InstanceType, ObjectValidation, Schema, SchemaObject, SingleOrVec, StringValidation},
    JsonSchema,
};
use serde::{Deserialize, Deserializer, Serialize};

/// Maximum length of a trace/span id token.
pub const MAX_TRACE_FIELD_LEN: usize = 64;

/// ECMA-regex for a bounded printable-ASCII trace/span token (no spaces).
const TRACE_FIELD_PATTERN: &str = "^[\\x21-\\x7e]+$";

/// A bounded W3C-style trace context. Both fields are opaque, bounded,
/// printable-ASCII tokens. Fields are private so they can only be set
/// through the validating constructor / fail-closed deserializer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TraceContext {
    /// Opaque trace id (correlates a request across peers).
    trace_id: String,
    /// Opaque span id of the current hop.
    span_id: String,
}

impl TraceContext {
    /// Validate and construct. Rejects empty/over-long/non-printable
    /// tokens (fail-closed).
    pub fn new(trace_id: impl Into<String>, span_id: impl Into<String>) -> Option<Self> {
        let trace_id = trace_id.into();
        let span_id = span_id.into();
        if Self::valid_field(&trace_id) && Self::valid_field(&span_id) {
            Some(Self { trace_id, span_id })
        } else {
            None
        }
    }

    /// The opaque trace id.
    pub fn trace_id(&self) -> &str {
        &self.trace_id
    }

    /// The opaque span id.
    pub fn span_id(&self) -> &str {
        &self.span_id
    }

    fn valid_field(s: &str) -> bool {
        !s.is_empty()
            && s.len() <= MAX_TRACE_FIELD_LEN
            && s.chars().all(|c| c.is_ascii_graphic() && c != ' ')
    }

    fn field_schema() -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
            string: Some(Box::new(StringValidation {
                max_length: Some(MAX_TRACE_FIELD_LEN as u32),
                min_length: Some(1),
                pattern: Some(TRACE_FIELD_PATTERN.to_owned()),
            })),
            ..Default::default()
        })
    }
}

// Fail-closed decode: a codec/serde path cannot instantiate an unbounded
// or unsafe trace context.
impl<'de> Deserialize<'de> for TraceContext {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            trace_id: String,
            span_id: String,
        }
        let raw = Raw::deserialize(deserializer)?;
        Self::new(raw.trace_id, raw.span_id)
            .ok_or_else(|| serde::de::Error::custom("trace context field out of bounds"))
    }
}

// Schema advertises the same bounds the validator enforces.
impl JsonSchema for TraceContext {
    fn schema_name() -> String {
        "TraceContext".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        let mut properties = schemars::Map::new();
        properties.insert("trace_id".to_owned(), Self::field_schema());
        properties.insert("span_id".to_owned(), Self::field_schema());
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::Object))),
            object: Some(Box::new(ObjectValidation {
                properties,
                required: ["trace_id", "span_id"].iter().map(|s| s.to_string()).collect(),
                additional_properties: Some(Box::new(Schema::Bool(false))),
                ..Default::default()
            })),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unbounded_or_unsafe_fields() {
        assert!(TraceContext::new("t1", "s1").is_some());
        assert!(TraceContext::new("", "s1").is_none());
        assert!(TraceContext::new("t1", "x".repeat(MAX_TRACE_FIELD_LEN + 1)).is_none());
        assert!(TraceContext::new("t 1", "s1").is_none());
    }

    #[test]
    fn deserialize_is_fail_closed() {
        assert!(
            serde_json::from_str::<TraceContext>("{\"trace_id\":\"t1\",\"span_id\":\"s1\"}").is_ok()
        );
        // empty / overlong / unsafe fields are rejected at decode.
        assert!(
            serde_json::from_str::<TraceContext>("{\"trace_id\":\"\",\"span_id\":\"s1\"}").is_err()
        );
        let overlong = format!(
            "{{\"trace_id\":\"t1\",\"span_id\":\"{}\"}}",
            "x".repeat(MAX_TRACE_FIELD_LEN + 1)
        );
        assert!(serde_json::from_str::<TraceContext>(&overlong).is_err());
        assert!(
            serde_json::from_str::<TraceContext>("{\"trace_id\":\"t 1\",\"span_id\":\"s1\"}").is_err()
        );
    }
}
