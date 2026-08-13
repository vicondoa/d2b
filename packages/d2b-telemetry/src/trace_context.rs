//! Bounded trace context shared by the v3 telemetry and audit surfaces.

use serde::{Deserialize, Deserializer, Serialize, ser::SerializeStruct};

/// Maximum length of a trace or span identifier.
pub const MAX_TRACE_FIELD_LEN: usize = 64;
/// Domain used for every exported trace and span identity.
pub const TRACE_CONTEXT_DIGEST_DOMAIN: &str = "d2b:telemetry-trace-context:v1";

/// An opaque, bounded trace context.
///
/// The fields are private so every deserialization path goes through the same
/// validation as the constructor.
#[derive(Clone, PartialEq, Eq)]
pub struct TraceContext {
    trace_id: String,
    span_id: String,
}

impl Serialize for TraceContext {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("TraceContext", 2)?;
        state.serialize_field("trace_id", &self.exported_trace_id())?;
        state.serialize_field("span_id", &self.exported_span_id())?;
        state.end()
    }
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

    /// Return the canonical trace identity used by telemetry and audit
    /// exporters.
    pub fn exported_trace_id(&self) -> String {
        canonical_export_id(&self.trace_id)
    }

    /// Return the canonical span identity used by telemetry exporters.
    pub fn exported_span_id(&self) -> String {
        canonical_export_id(&self.span_id)
    }

    /// Derive a child span while preserving the validated trace identifier.
    pub fn child_span(&self, span_id: impl Into<String>) -> Option<Self> {
        Self::new(self.trace_id.clone(), span_id)
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

/// Canonicalize one validated trace-context field for an external sink.
pub fn canonical_export_id(value: &str) -> String {
    if d2b_contracts::v3::is_canonical_digest(value) {
        value.to_owned()
    } else {
        d2b_contracts::v3::canonical_digest(TRACE_CONTEXT_DIGEST_DOMAIN, value.as_bytes())
    }
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

    #[test]
    fn child_span_propagates_only_the_validated_trace_id() {
        let parent = TraceContext::new("trace", "parent").unwrap();
        let child = parent.child_span("child").unwrap();
        assert_eq!(child.trace_id(), "trace");
        assert_eq!(child.span_id(), "child");
        assert!(parent.child_span("span id").is_none());
    }

    #[test]
    fn exported_ids_share_one_canonical_digest_domain() {
        let context = TraceContext::new("trace", "span").unwrap();
        assert_eq!(
            context.exported_trace_id(),
            d2b_contracts::v3::canonical_digest(TRACE_CONTEXT_DIGEST_DOMAIN, b"trace")
        );
        assert_eq!(
            context.exported_span_id(),
            d2b_contracts::v3::canonical_digest(TRACE_CONTEXT_DIGEST_DOMAIN, b"span")
        );
    }
}
