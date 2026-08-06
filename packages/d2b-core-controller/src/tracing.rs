//! Core-controller span projections and trace propagation.

use std::collections::BTreeMap;

use d2b_telemetry::{
    BoundedEmitter, EmitOutcome, EmitterError, RedactionError, RedactionGuard, Signal,
    TraceContext, emitter::encode_frame,
};

/// Hint span name.
pub const HINT_SPAN: &str = "d2b.controller.hint";
/// Reconcile span name.
pub const RECONCILE_SPAN: &str = "d2b.controller.reconcile";

/// Build a bounded hint span.
pub fn hint_span(
    fields: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    trace: Option<TraceContext>,
) -> Result<ControllerSpan, RedactionError> {
    ControllerSpan::new(HINT_SPAN, fields, trace)
}

/// Build a bounded reconcile child span.
pub fn reconcile_span(
    fields: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    trace: Option<TraceContext>,
) -> Result<ControllerSpan, RedactionError> {
    ControllerSpan::new(RECONCILE_SPAN, fields, trace)
}

/// Redacted controller span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControllerSpan {
    name: &'static str,
    fields: BTreeMap<String, String>,
    trace: Option<TraceContext>,
}

impl ControllerSpan {
    fn new(
        name: &'static str,
        fields: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
        trace: Option<TraceContext>,
    ) -> Result<Self, RedactionError> {
        Ok(Self {
            name,
            fields: RedactionGuard::span_attributes(fields)?,
            trace,
        })
    }

    /// Name.
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Trace context.
    pub const fn trace(&self) -> Option<&TraceContext> {
        self.trace.as_ref()
    }

    /// Emit this bounded span through the core-process telemetry port.
    pub fn emit(&self, emitter: &BoundedEmitter) -> Result<EmitOutcome, EmitterError> {
        let frame = encode_frame(
            Signal::Trace,
            &serde_json::json!({
                "name": self.name,
                "fields": self.fields,
                "trace_id": self.trace.as_ref().map(TraceContext::trace_id),
            }),
        )
        .map_err(|_| EmitterError::FrameTooLarge)?;
        emitter.emit(Signal::Trace, &frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconcile_is_a_child_of_the_hint_context() {
        let context = TraceContext::new("trace", "hint");
        let hint = hint_span([("handler", "provider")], context.clone()).unwrap();
        let child = reconcile_span([("handler", "provider")], context).unwrap();
        assert_eq!(hint.name(), HINT_SPAN);
        assert_eq!(child.name(), RECONCILE_SPAN);
        assert_eq!(child.trace().unwrap().trace_id(), "trace");
    }

    #[test]
    fn controller_span_has_a_bounded_emit_port() {
        let emitter = BoundedEmitter::new("/nonexistent", 1024).unwrap();
        let span = hint_span([("handler", "provider")], None).unwrap();
        span.emit(&emitter).unwrap();
    }
}
