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
