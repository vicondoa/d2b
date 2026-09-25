//! Secret Service Credential telemetry producer.

use d2b_contracts_provider::v3::credential::PlacementBinding;
use d2b_contracts_provider::v3::credential_controller::{
    CredentialObservabilityError, CredentialProviderKind, CredentialTelemetryFrame,
    CredentialTelemetryOperation, CredentialTelemetryOutcome,
};

pub(super) fn frame(
    zone: &str,
    operation: CredentialTelemetryOperation,
    outcome: CredentialTelemetryOutcome,
    rotation_generation: u64,
) -> Result<CredentialTelemetryFrame, CredentialObservabilityError> {
    d2b_provider_toolkit::credential::credential_frame(
        CredentialProviderKind::SecretService,
        zone,
        operation,
        outcome,
        PlacementBinding::UserAgent,
        rotation_generation,
        env!("CARGO_PKG_VERSION"),
    )
}


