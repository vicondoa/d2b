//! Closed, redacted transport audit events.

/// Provider lifecycle operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportAuditOperation {
    /// A vsock transport was acquired.
    Acquire,
    /// A vsock transport was released.
    Release,
}

/// Provider lifecycle outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportAuditOutcome {
    /// The operation completed.
    Success,
    /// The operation was refused.
    Refused,
    /// The operation remains retryable.
    Retryable,
}

/// Redacted audit event with only closed semantic dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportAuditEvent {
    /// Closed operation.
    pub operation: TransportAuditOperation,
    /// Closed outcome.
    pub outcome: TransportAuditOutcome,
}
