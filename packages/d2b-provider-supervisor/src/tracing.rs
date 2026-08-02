//! Process launch and stop span projections.

use std::collections::BTreeMap;

use d2b_telemetry::{RedactionError, RedactionGuard, TraceContext};

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
}
