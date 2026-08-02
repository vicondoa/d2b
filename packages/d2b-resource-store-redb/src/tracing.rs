//! Store span names and bounded trace propagation.

use std::collections::BTreeMap;

use d2b_telemetry::{RedactionError, RedactionGuard, TraceContext};

/// Span names owned by the redb store.
pub const STORE_WRITE_SPAN: &str = "d2b.store.write";
/// Read span name.
pub const STORE_READ_SPAN: &str = "d2b.store.read";
/// Compaction span name.
pub const STORE_COMPACTION_SPAN: &str = "d2b.store.compaction";

/// A redacted store span projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreSpan {
    name: &'static str,
    fields: BTreeMap<String, String>,
    trace: Option<TraceContext>,
}

impl StoreSpan {
    /// Construct a bounded store span.
    pub fn new(
        name: &'static str,
        fields: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
        trace: Option<TraceContext>,
    ) -> Result<Self, RedactionError> {
        if !matches!(
            name,
            STORE_WRITE_SPAN | STORE_READ_SPAN | STORE_COMPACTION_SPAN
        ) {
            return Err(RedactionError::ForbiddenSpanField);
        }
        Ok(Self {
            name,
            fields: RedactionGuard::span_attributes(fields)?,
            trace,
        })
    }

    /// Span name.
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Redacted fields.
    pub const fn fields(&self) -> &BTreeMap<String, String> {
        &self.fields
    }

    /// Propagated trace context.
    pub const fn trace(&self) -> Option<&TraceContext> {
        self.trace.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_spans_accept_only_bounded_fields() {
        let span = StoreSpan::new(
            STORE_WRITE_SPAN,
            [("kind", "single"), ("outcome", "ok")],
            TraceContext::new("trace", "span"),
        )
        .unwrap();
        assert_eq!(span.name(), STORE_WRITE_SPAN);
        assert!(StoreSpan::new(STORE_WRITE_SPAN, [("path", "/tmp")], None).is_err());
    }
}
