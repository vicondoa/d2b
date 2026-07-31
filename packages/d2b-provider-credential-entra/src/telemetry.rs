//! Entra Credential telemetry producer.

use d2b_contracts::v3::credential::PlacementBinding;
use d2b_contracts::v3::credential_controller::{
    CredentialObservabilityError, CredentialProviderKind, CredentialTelemetryFrame,
    CredentialTelemetryOperation, CredentialTelemetryOutcome,
};

pub(super) fn frame(
    zone: &str,
    operation: CredentialTelemetryOperation,
    outcome: CredentialTelemetryOutcome,
    placement: PlacementBinding,
    rotation_generation: u64,
) -> Result<CredentialTelemetryFrame, CredentialObservabilityError> {
    CredentialTelemetryFrame::new(
        CredentialProviderKind::Entra,
        zone,
        operation,
        outcome,
        placement,
        rotation_generation,
        env!("CARGO_PKG_VERSION"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::credential_controller::CredentialTelemetryField;

    #[test]
    fn process_unique_entra_canary_is_rejected_from_closed_values() {
        let marker = format!("entra-token-canary-{:x}", std::process::id());
        assert!(
            CredentialTelemetryFrame::validate_collector_fields([CredentialTelemetryField {
                key: "outcome",
                value: marker,
            }])
            .is_err()
        );
        let frame = frame(
            "dev",
            CredentialTelemetryOperation::RefreshToken,
            CredentialTelemetryOutcome::Success,
            PlacementBinding::GuestAgent,
            2,
        )
        .unwrap();
        assert!(CredentialTelemetryFrame::validate_collector_fields(frame.all_fields()).is_ok());
    }
}
