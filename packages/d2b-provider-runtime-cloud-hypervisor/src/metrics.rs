//! Closed Cloud Hypervisor metric labels.

/// One bounded metric event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloudHypervisorMetricEvent {
    /// Stable operation.
    pub operation: &'static str,
    /// Stable outcome.
    pub outcome: &'static str,
}

impl CloudHypervisorMetricEvent {
    /// Construct a closed metric.
    pub fn new(operation: &'static str, outcome: &'static str) -> Option<Self> {
        let operation_ok = matches!(operation, "launch" | "adopt" | "health" | "finalize");
        let outcome_ok = matches!(outcome, "success" | "failure" | "retry" | "degraded");
        operation_ok
            .then_some(())
            .and(outcome_ok.then_some(()))
            .map(|_| Self { operation, outcome })
    }
}
