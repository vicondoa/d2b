//! Bounded transport telemetry dimensions.

/// Closed transport operation metric dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportMetricOperation {
    /// Transport-open attempts.
    Open,
    /// Transport-close attempts.
    Close,
    /// Transport observations.
    Observe,
}

/// Closed transport outcome metric dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportMetricOutcome {
    /// The operation completed.
    Success,
    /// The operation was refused.
    Refused,
    /// The operation encountered a local failure.
    Failure,
}
