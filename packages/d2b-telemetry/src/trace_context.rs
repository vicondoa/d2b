//! Bounded trace context shared by the v3 telemetry and audit surfaces.

use serde::{Deserialize, Deserializer, Serialize};

/// Maximum length of a trace or span identifier.
pub const MAX_TRACE_FIELD_LEN: usize = 64;

/// An opaque, bounded trace context.
///
/// The fields are private so every deserialization path goes through the same
/// validation as the constructor.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct TraceContext {
    trace_id: String,
    span_id: String,
}

impl TraceContext {
    /// Construct a context from printable, non-empty identifiers.
    pub fn new(trace_id: impl Into<String>, span_id: impl Into<String>) -> Option<Self> {
        let trace_id = trace_id.into();
        let span_id = span_id.into();
        (valid_field(&trace_id) && valid_field(&span_id)).then_some(Self { trace_id, span_id })
    }

    /// Borrow the trace identifier.
    pub fn trace_id(&self) -> &str {
        &self.trace_id
    }

    /// Borrow the span identifier.
    pub fn span_id(&self) -> &str {
        &self.span_id
    }
}

impl core::fmt::Debug for TraceContext {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("TraceContext(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for TraceContext {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            trace_id: String,
            span_id: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.trace_id, wire.span_id)
            .ok_or_else(|| serde::de::Error::custom("trace-context-invalid"))
    }
}

fn valid_field(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_TRACE_FIELD_LEN
        && value.bytes().all(|byte| byte.is_ascii_graphic())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructor_and_decoder_share_bounds() {
        assert!(TraceContext::new("trace", "span").is_some());
        assert!(TraceContext::new("", "span").is_none());
        assert!(TraceContext::new("trace", "span id").is_none());
        assert!(
            serde_json::from_str::<TraceContext>(r#"{"trace_id":"trace","span_id":"span"}"#)
                .is_ok()
        );
        assert!(
            serde_json::from_str::<TraceContext>(r#"{"trace_id":"","span_id":"span"}"#).is_err()
        );
    }

    #[test]
    fn debug_does_not_render_identifiers() {
        let context = TraceContext::new("sensitive-trace", "sensitive-span").unwrap();
        let rendered = format!("{context:?}");
        assert_eq!(rendered, "TraceContext(<redacted>)");
        assert!(!rendered.contains("sensitive"));
    }
}
