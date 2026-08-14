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
        let error_ok = matches!(
            error,
            "none"
                | "arm-quota-exceeded"
                | "arm-resource-conflict"
                | "arm-provisioning-failed"
                | "arm-network-unavailable"
                | "arm-credential-denied"
                | "arm-throttled"
                | "bootstrap-psk-expired"
                | "bootstrap-psk-replayed"
                | "bootstrap-enrollment-failed"
                | "bootstrap-failed"
                | "credential-unavailable"
                | "deletion-ambiguous"
                | "child-zone-drain-timeout"
                | "image-change-requires-confirm"
                | "opaque-azure-ref-invalid"
                | "adoption-zone-mismatch"
        );
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

#[cfg(test)]
mod tests {
    use super::AzureVmMetricEvent;

    #[test]
    fn accepts_all_documented_stable_errors() {
        for error in [
            "none",
            "arm-quota-exceeded",
            "arm-resource-conflict",
            "arm-provisioning-failed",
            "arm-network-unavailable",
            "arm-credential-denied",
            "arm-throttled",
            "bootstrap-psk-expired",
            "bootstrap-psk-replayed",
            "bootstrap-enrollment-failed",
            "bootstrap-failed",
            "credential-unavailable",
            "deletion-ambiguous",
            "child-zone-drain-timeout",
            "image-change-requires-confirm",
            "opaque-azure-ref-invalid",
            "adoption-zone-mismatch",
        ] {
            assert!(AzureVmMetricEvent::new("provision", "failure", error).is_some());
        }
    }

    #[test]
    fn rejects_unregistered_error_labels() {
        assert!(AzureVmMetricEvent::new("provision", "failure", "arm-secret").is_none());
    }
}
