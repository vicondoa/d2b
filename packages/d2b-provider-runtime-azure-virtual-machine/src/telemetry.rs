//! Closed Azure VM telemetry labels.

/// One bounded metric event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AzureVmMetricEvent {
    /// Stable operation label.
    pub operation: &'static str,
    /// Stable outcome label.
    pub outcome: &'static str,
    /// Stable error label.
    pub error: &'static str,
}

impl AzureVmMetricEvent {
    /// Construct a metric after closed-label validation.
    pub fn new(
        operation: &'static str,
        outcome: &'static str,
        error: &'static str,
    ) -> Option<Self> {
        let operation_ok = matches!(
            operation,
            "provision" | "adopt" | "bootstrap" | "delete" | "observe"
        );
        let outcome_ok = matches!(
            outcome,
            "success" | "failure" | "retry" | "deadline-expired"
        );
        let error_ok = error == "none"
            || error.starts_with("arm-")
            || error.starts_with("bootstrap-")
            || error == "credential-unavailable";
        operation_ok
            .then_some(())
            .and(outcome_ok.then_some(()))
            .and(error_ok.then_some(()))
            .map(|_| Self {
                operation,
                outcome,
                error,
            })
    }
}
