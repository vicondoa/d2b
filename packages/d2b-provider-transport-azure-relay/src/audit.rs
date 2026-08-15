//! Bounded Relay audit events.

/// Fixed Relay audit operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayAuditOperation {
    /// Open a stream.
    Open,
    /// Close a stream.
    Close,
    /// Reconnect a stream.
    Reconnect,
}

/// Fixed Relay audit outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayAuditOutcome {
    /// Success.
    Success,
    /// Failure.
    Failure,
}

/// Redacted Relay audit event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayAuditEvent {
    /// Operation.
    pub operation: RelayAuditOperation,
    /// Outcome.
    pub outcome: RelayAuditOutcome,
}
