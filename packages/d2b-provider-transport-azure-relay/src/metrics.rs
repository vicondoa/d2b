//! Closed Relay metric labels.

/// Relay operation outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayMetricOutcome {
    /// Success.
    Success,
    /// Failure.
    Failure,
    /// Retry.
    Retry,
    /// Deadline expired.
    DeadlineExpired,
}

/// One bounded metric event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayMetricEvent {
    /// Stable operation.
    pub operation: &'static str,
    /// Stable outcome.
    pub outcome: RelayMetricOutcome,
}

impl RelayMetricEvent {
    /// Construct a closed metric event.
    pub fn new(operation: &'static str, outcome: RelayMetricOutcome) -> Option<Self> {
        matches!(
            operation,
            "open" | "close" | "reconnect" | "send" | "receive"
        )
        .then_some(Self { operation, outcome })
    }
}
