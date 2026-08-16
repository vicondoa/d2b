//! Closed telemetry dimensions for transport-vsock.

/// Provider metric operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportMetricOperation {
    /// Open attempts.
    Open,
    /// Close attempts.
    Close,
    /// Observe attempts.
    Observe,
}

/// Provider metric outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportMetricOutcome {
    /// The operation completed.
    Success,
    /// The operation was refused.
    Refused,
    /// The operation failed locally.
    Failure,
}

/// Bounded metric labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportMetricLabels {
    /// Fixed Provider name.
    pub provider: &'static str,
    /// Fixed component name.
    pub component: &'static str,
    /// Closed operation label.
    pub operation: TransportMetricOperation,
    /// Closed outcome label.
    pub outcome: TransportMetricOutcome,
}
