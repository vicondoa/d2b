//! Entra Credential telemetry producer.

use d2b_contracts_provider::v3::credential::PlacementBinding;
use d2b_contracts_provider::v3::credential_controller::{
    CredentialObservabilityError, CredentialProviderKind, CredentialTelemetryFrame,
    CredentialTelemetryOperation, CredentialTelemetryOutcome,
};
use d2b_provider_toolkit::credential;

pub(super) fn frame(
    zone: &str,
    operation: CredentialTelemetryOperation,
    outcome: CredentialTelemetryOutcome,
    placement: PlacementBinding,
    rotation_generation: u64,
) -> Result<CredentialTelemetryFrame, CredentialObservabilityError> {
    credential::credential_frame(
        CredentialProviderKind::Entra,
        zone,
        operation,
        outcome,
        placement,
        rotation_generation,
        env!("CARGO_PKG_VERSION"),
    )
}


