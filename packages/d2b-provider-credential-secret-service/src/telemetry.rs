//! Secret Service Credential telemetry producer.

use d2b_contracts::v3::credential::PlacementBinding;
use d2b_contracts::v3::credential_controller::{
    CredentialObservabilityError, CredentialProviderKind, CredentialTelemetryFrame,
    CredentialTelemetryOperation, CredentialTelemetryOutcome,
};

pub(super) fn frame(
    zone: &str,
    operation: CredentialTelemetryOperation,
    outcome: CredentialTelemetryOutcome,
    rotation_generation: u64,
) -> Result<CredentialTelemetryFrame, CredentialObservabilityError> {
    CredentialTelemetryFrame::new(
        CredentialProviderKind::SecretService,
        zone,
        operation,
        outcome,
        PlacementBinding::UserAgent,
        rotation_generation,
        env!("CARGO_PKG_VERSION"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::credential_controller::CredentialTelemetryField;

    #[test]
    fn process_unique_canary_is_rejected_as_an_allowed_key_value() {
        let marker = format!("secret-canary-{:x}", std::process::id());
        assert!(
            CredentialTelemetryFrame::validate_collector_fields([CredentialTelemetryField {
                key: "outcome",
                value: marker,
            }])
            .is_err()
        );
        let frame = frame(
            "dev",
            CredentialTelemetryOperation::AcquireToken,
            CredentialTelemetryOutcome::Success,
            1,
        )
        .unwrap();
        assert!(CredentialTelemetryFrame::validate_collector_fields(frame.all_fields()).is_ok());
    }
}
