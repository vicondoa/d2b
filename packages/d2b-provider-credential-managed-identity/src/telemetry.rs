//! Managed identity Credential telemetry producer.

use d2b_contracts_provider::v3::credential::PlacementBinding;
use d2b_contracts_provider::v3::credential_controller::{
    CredentialObservabilityError, CredentialProviderKind, CredentialTelemetryFrame,
    CredentialTelemetryOperation, CredentialTelemetryOutcome,
};

pub(crate) fn credential_frame(
    zone: &str,
    operation: CredentialTelemetryOperation,
    outcome: CredentialTelemetryOutcome,
    placement: PlacementBinding,
    rotation_generation: u64,
) -> Result<CredentialTelemetryFrame, CredentialObservabilityError> {
    d2b_provider_toolkit::credential::credential_frame(
        CredentialProviderKind::ManagedIdentity,
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
    use d2b_contracts_provider::v3::credential_controller::CredentialTelemetryField;

    #[test]
    fn collector_allowlist_rejects_nonclosed_values_for_allowed_keys() {
        let marker = format!("managed-identity-canary-{:x}", std::process::id());
        assert!(
            CredentialTelemetryFrame::validate_collector_fields([CredentialTelemetryField {
                key: "outcome",
                value: marker,
            }])
            .is_err()
        );
        let frame = credential_frame(
            "dev",
            CredentialTelemetryOperation::AcquireToken,
            CredentialTelemetryOutcome::Success,
            PlacementBinding::GuestAgent,
            1,
        )
        .unwrap();
        assert!(CredentialTelemetryFrame::validate_collector_fields(frame.all_fields()).is_ok());
    }
}
