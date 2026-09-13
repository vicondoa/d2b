//! Closed ACA telemetry labels.

use std::fmt;

/// Stable lifecycle operation labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcaMetricOutcome {
    /// Operation succeeded.
    Success,
    /// Operation failed.
    Failure,
    /// Operation was cancelled.
    Cancelled,
    /// Operation reached its deadline.
    DeadlineExpired,
}

impl AcaMetricOutcome {
    /// Return the stable label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Cancelled => "cancelled",
            Self::DeadlineExpired => "deadline-expired",
        }
    }
}

/// One bounded ACA metric event.
#[derive(Clone, PartialEq, Eq)]
pub struct AcaMetricEvent {
    component: &'static str,
    operation: &'static str,
    outcome: AcaMetricOutcome,
    error: &'static str,
}

impl AcaMetricEvent {
    /// Construct a metric with the closed label set.
    pub fn new(
        component: &'static str,
        operation: &'static str,
        outcome: AcaMetricOutcome,
        error: &'static str,
    ) -> Result<Self, AcaMetricValidationError> {
        if !valid_component(component) || !valid_operation(operation) || !valid_error(error) {
            return Err(AcaMetricValidationError::LabelNotAllowed);
        }
        Ok(Self {
            component,
            operation,
            outcome,
            error,
        })
    }

    /// Return the component label.
    pub const fn component(&self) -> &'static str {
        self.component
    }

    /// Return the operation label.
    pub const fn operation(&self) -> &'static str {
        self.operation
    }

    /// Return the outcome label.
    pub const fn outcome(&self) -> AcaMetricOutcome {
        self.outcome
    }

    /// Return the error label.
    pub const fn error(&self) -> &'static str {
        self.error
    }
}

impl fmt::Debug for AcaMetricEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcaMetricEvent")
            .field("component", &self.component)
            .field("operation", &self.operation)
            .field("outcome", &self.outcome)
            .field("error", &self.error)
            .finish()
    }
}

/// Stable telemetry validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcaMetricValidationError {
    /// A caller supplied a dynamic or forbidden label.
    LabelNotAllowed,
}

fn valid_component(value: &str) -> bool {
    matches!(value, "aca-controller" | "aca-deployment-service")
}

fn valid_operation(value: &str) -> bool {
    matches!(
        value,
        "provision" | "start" | "stop" | "inspect" | "adopt" | "destroy" | "health"
    )
}

fn valid_error(value: &str) -> bool {
    value == "none"
        || value.starts_with("aca-control-")
        || value == "aca-invalid-state"
        || value == "aca-ambiguous-adoption"
}
