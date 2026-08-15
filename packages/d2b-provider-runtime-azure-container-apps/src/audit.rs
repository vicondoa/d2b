//! Bounded ACA audit events.

use std::fmt;

/// Outcome of one ACA lifecycle operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcaAuditOutcome {
    /// The operation completed.
    Success,
    /// The operation failed with a stable error code.
    Failure,
}

impl AcaAuditOutcome {
    /// Return the stable wire label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
        }
    }
}

/// One redacted ACA audit event.
#[derive(Clone, PartialEq, Eq)]
pub struct AcaAuditEvent {
    operation: &'static str,
    outcome: AcaAuditOutcome,
    error_code: Option<&'static str>,
}

impl AcaAuditEvent {
    /// Construct a bounded lifecycle event.
    pub const fn new(
        operation: &'static str,
        outcome: AcaAuditOutcome,
        error_code: Option<&'static str>,
    ) -> Self {
        Self {
            operation,
            outcome,
            error_code,
        }
    }

    /// Return the fixed operation label.
    pub const fn operation(&self) -> &'static str {
        self.operation
    }

    /// Return the outcome.
    pub const fn outcome(&self) -> AcaAuditOutcome {
        self.outcome
    }

    /// Return the stable error code, if any.
    pub const fn error_code(&self) -> Option<&'static str> {
        self.error_code
    }
}

impl fmt::Debug for AcaAuditEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcaAuditEvent")
            .field("operation", &self.operation)
            .field("outcome", &self.outcome)
            .field("error_code", &self.error_code)
            .finish()
    }
}

/// Sink for critical ACA audit records.
pub trait AcaAuditSink: Send + Sync {
    /// Append one event before reporting a critical operation complete.
    fn append(&self, event: AcaAuditEvent) -> Result<(), &'static str>;
}
