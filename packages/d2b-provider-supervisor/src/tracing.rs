//! Process launch and stop span projections.

#![allow(dead_code)]

use std::collections::BTreeMap;

use d2b_telemetry::{
    BoundedEmitter, EmitOutcome, EmitterError, RedactionError, RedactionGuard, Signal,
    TraceContext, emitter::encode_frame,
};

/// Launch span.
pub const PROCESS_LAUNCH_SPAN: &str = "d2b.process.launch";
/// Stop span.
pub const PROCESS_STOP_SPAN: &str = "d2b.process.stop";

/// A redacted process span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSpan {
    name: &'static str,
    fields: BTreeMap<String, String>,
    trace: Option<TraceContext>,
}

impl ProcessSpan {
    /// Construct a launch span.
    pub fn launch(
        fields: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
        trace: Option<TraceContext>,
    ) -> Result<Self, RedactionError> {
        Self::new(PROCESS_LAUNCH_SPAN, fields, trace)
    }

    /// Construct a stop span.
    pub fn stop(
        fields: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
        trace: Option<TraceContext>,
    ) -> Result<Self, RedactionError> {
        Self::new(PROCESS_STOP_SPAN, fields, trace)
    }

    /// Name.
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Emit this bounded process span through the telemetry port.
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_span_does_not_accept_process_identity() {
        assert!(ProcessSpan::launch([("pid", "1")], None).is_err());
        assert!(ProcessSpan::launch([("provider", "systemd")], None).is_ok());
    }

    #[test]
    fn process_span_has_a_bounded_emit_port() {
        let emitter = BoundedEmitter::new("/nonexistent", 1024).unwrap();
        ProcessSpan::launch([("provider", "systemd")], None)
            .unwrap()
            .emit(&emitter)
            .unwrap();
    }
}
