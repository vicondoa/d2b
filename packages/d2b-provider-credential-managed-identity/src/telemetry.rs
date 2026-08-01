//! Managed identity Credential telemetry producer.

use std::collections::BTreeMap;

use d2b_contracts::v3::credential::PlacementBinding;
use d2b_contracts::v3::credential_controller::{
    CredentialObservabilityError, CredentialProviderKind, CredentialTelemetryField,
    CredentialTelemetryFrame, CredentialTelemetryOperation, CredentialTelemetryOutcome,
};

/// Exposed telemetry fields use the shared closed field shape.
pub type TelemetryField = CredentialTelemetryField;
/// Telemetry frame errors use the shared field-free error.
pub type TelemetryFrameError = CredentialObservabilityError;

/// Backward-compatible managed identity telemetry operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedIdentityTelemetryOperation {
    /// Token acquisition.
    AcquireToken,
    /// Token refresh.
    RefreshToken,
    /// Lease revocation.
    RevokeToken,
    /// Metadata inspection.
    InspectMetadata,
    /// Controller reconciliation.
    Reconcile,
}

/// Backward-compatible managed identity telemetry outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedIdentityTelemetryOutcome {
    /// The operation completed.
    Success,
    /// The Provider was unavailable.
    ProviderUnavailable,
    /// Policy denied the operation.
    Denied,
    /// A fixed invariant rejected the result.
    InvariantFailure,
}

/// Compatibility wrapper over the shared closed Credential telemetry frame.
pub struct ManagedIdentityTelemetryFrame(CredentialTelemetryFrame);

impl ManagedIdentityTelemetryFrame {
    /// Build the legacy frame shape through the shared validator.
    pub fn new(
        zone: impl Into<String>,
        operation: ManagedIdentityTelemetryOperation,
        outcome: ManagedIdentityTelemetryOutcome,
        placement: PlacementBinding,
    ) -> Self {
        let operation = match operation {
            ManagedIdentityTelemetryOperation::AcquireToken => {
                CredentialTelemetryOperation::AcquireToken
            }
            ManagedIdentityTelemetryOperation::RefreshToken => {
                CredentialTelemetryOperation::RefreshToken
            }
            ManagedIdentityTelemetryOperation::RevokeToken => {
                CredentialTelemetryOperation::RevokeToken
            }
            ManagedIdentityTelemetryOperation::InspectMetadata => {
                CredentialTelemetryOperation::InspectMetadata
            }
            ManagedIdentityTelemetryOperation::Reconcile => CredentialTelemetryOperation::Reconcile,
        };
        let outcome = match outcome {
            ManagedIdentityTelemetryOutcome::Success => CredentialTelemetryOutcome::Success,
            ManagedIdentityTelemetryOutcome::ProviderUnavailable => {
                CredentialTelemetryOutcome::ProviderUnavailable
            }
            ManagedIdentityTelemetryOutcome::Denied => CredentialTelemetryOutcome::Denied,
            ManagedIdentityTelemetryOutcome::InvariantFailure => {
                CredentialTelemetryOutcome::InvariantFailure
            }
        };
        Self(
            credential_frame(&zone.into(), operation, outcome, placement, 1)
                .expect("closed compatibility telemetry inputs are valid"),
        )
    }

    /// Borrow generic Resource attributes.
    pub fn resource_attributes(&self) -> &[TelemetryField] {
        self.0.resource_attributes()
    }

    /// Borrow closed span attributes.
    pub fn span_attributes(&self) -> &[TelemetryField] {
        self.0.span_attributes()
    }

    /// Borrow closed metric labels.
    pub fn metric_labels(&self) -> &[TelemetryField] {
        self.0.metric_labels()
    }

    /// Return every collector field.
    pub fn all_fields(&self) -> Vec<TelemetryField> {
        self.0.all_fields()
    }

    /// Validate a whole collector frame.
    pub fn validate_collector_fields(
        fields: impl IntoIterator<Item = TelemetryField>,
    ) -> Result<(), TelemetryFrameError> {
        CredentialTelemetryFrame::validate_collector_fields(fields)
    }

    /// Group fixed metric labels by key.
    pub fn metric_map(&self) -> BTreeMap<&'static str, &str> {
        self.metric_labels()
            .iter()
            .map(|field| (field.key, field.value.as_str()))
            .collect()
    }
}

impl core::fmt::Debug for ManagedIdentityTelemetryFrame {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ManagedIdentityTelemetryFrame(<redacted>)")
    }
}

pub(crate) fn credential_frame(
    zone: &str,
    operation: CredentialTelemetryOperation,
    outcome: CredentialTelemetryOutcome,
    placement: PlacementBinding,
    rotation_generation: u64,
) -> Result<CredentialTelemetryFrame, CredentialObservabilityError> {
    CredentialTelemetryFrame::new(
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
