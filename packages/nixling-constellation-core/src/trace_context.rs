//! A bounded trace context propagated across constellation peers (ADR
//! 0032). Deliberately minimal: opaque trace/span ids only. It carries
//! **no** baggage, secrets, store paths, or payload, and field lengths
//! are bounded so it cannot become a side channel.

use serde::{Deserialize, Serialize};

/// Maximum length of a trace/span id token.
pub const MAX_TRACE_FIELD_LEN: usize = 64;

/// A bounded W3C-style trace context. Both fields are opaque, bounded,
/// printable-ASCII tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceContext {
    /// Opaque trace id (correlates a request across peers).
    pub trace_id: String,
    /// Opaque span id of the current hop.
    pub span_id: String,
}

impl TraceContext {
    /// Validate and construct. Rejects empty/over-long/non-printable
    /// tokens (fail-closed).
    pub fn new(trace_id: impl Into<String>, span_id: impl Into<String>) -> Option<Self> {
        let ctx = Self {
            trace_id: trace_id.into(),
            span_id: span_id.into(),
        };
        if Self::valid_field(&ctx.trace_id) && Self::valid_field(&ctx.span_id) {
            Some(ctx)
        } else {
            None
        }
    }

    fn valid_field(s: &str) -> bool {
        !s.is_empty()
            && s.len() <= MAX_TRACE_FIELD_LEN
            && s.chars().all(|c| c.is_ascii_graphic() && c != ' ')
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
}
