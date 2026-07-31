//! Closed telemetry frames with no Credential identity labels.

use std::collections::BTreeMap;

use d2b_contracts::v3::credential::PlacementBinding;

const FORBIDDEN_KEYS: &[&str] = &[
    "vm",
    "zone",
    "zone_id",
    "zone_uid",
    "credential_name",
    "credential_ref",
    "credential_uid",
    "credential_digest",
    "resource_name_digest",
    "d2b.credential.name",
    "d2b.credential.ref",
    "d2b.credential.uid",
    "d2b.credential.digest",
];

/// Closed telemetry operations.
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

impl ManagedIdentityTelemetryOperation {
    const fn as_str(self) -> &'static str {
        match self {
            Self::AcquireToken => "acquire-token",
            Self::RefreshToken => "refresh-token",
            Self::RevokeToken => "revoke-token",
            Self::InspectMetadata => "inspect-metadata",
            Self::Reconcile => "reconcile",
        }
    }
}

/// Closed telemetry outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedIdentityTelemetryOutcome {
    /// The operation completed.
    Success,
    /// The Provider was unavailable.
    ProviderUnavailable,
    /// Policy denied the operation.
    Denied,
    /// The request violated a fixed invariant.
    InvariantFailure,
}

impl ManagedIdentityTelemetryOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::ProviderUnavailable => "provider-unavailable",
            Self::Denied => "denied",
            Self::InvariantFailure => "invariant-failure",
        }
    }
}

/// One telemetry field exposed for structural conformance tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryField {
    /// Closed field key.
    pub key: &'static str,
    /// Bounded field value.
    pub value: String,
}

/// Telemetry frame rejected by the closed allowlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryFrameError {
    /// A field used a Credential identity key or non-allowlisted key.
    ForbiddenField,
}

impl core::fmt::Display for TelemetryFrameError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("credential telemetry frame is invalid")
    }
}

impl std::error::Error for TelemetryFrameError {}

/// Complete Resource, span, and metric frame using only closed low-cardinality
/// operation fields.
pub struct ManagedIdentityTelemetryFrame {
    resource_attributes: Vec<TelemetryField>,
    span_attributes: Vec<TelemetryField>,
    metric_labels: Vec<TelemetryField>,
}

impl ManagedIdentityTelemetryFrame {
    /// Build the fixed frame. The Zone value is retained only as the generic
    /// trusted-ingress Resource attribute.
    pub fn new(
        zone: impl Into<String>,
        operation: ManagedIdentityTelemetryOperation,
        outcome: ManagedIdentityTelemetryOutcome,
        placement: PlacementBinding,
    ) -> Self {
        let operation = operation.as_str().to_owned();
        let outcome = outcome.as_str().to_owned();
        let placement = match placement {
            PlacementBinding::HostSystem => "host-system",
            PlacementBinding::GuestAgent => "guest-agent",
            PlacementBinding::UserAgent => "user-agent",
        }
        .to_owned();
        Self {
            resource_attributes: vec![
                TelemetryField {
                    key: "d2b.zone",
                    value: zone.into(),
                },
                TelemetryField {
                    key: "d2b.provider",
                    value: "credential-managed-identity".to_owned(),
                },
                TelemetryField {
                    key: "d2b.component",
                    value: "managed-identity-agent".to_owned(),
                },
                TelemetryField {
                    key: "service.name",
                    value: "d2b-managed-identity-agent".to_owned(),
                },
                TelemetryField {
                    key: "service.namespace",
                    value: "d2b".to_owned(),
                },
                TelemetryField {
                    key: "service.version",
                    value: env!("CARGO_PKG_VERSION").to_owned(),
                },
            ],
            span_attributes: vec![
                TelemetryField {
                    key: "d2b.credential.provider",
                    value: "credential-managed-identity".to_owned(),
                },
                TelemetryField {
                    key: "d2b.credential.operation_class",
                    value: operation.clone(),
                },
                TelemetryField {
                    key: "d2b.credential.placement_binding",
                    value: placement.clone(),
                },
                TelemetryField {
                    key: "d2b.credential.outcome",
                    value: outcome.clone(),
                },
            ],
            metric_labels: vec![
                TelemetryField {
                    key: "operation_class",
                    value: operation,
                },
                TelemetryField {
                    key: "placement_binding",
                    value: placement,
                },
                TelemetryField {
                    key: "outcome",
                    value: outcome,
                },
            ],
        }
    }

    /// Borrow generic trusted-ingress Resource attributes.
    pub fn resource_attributes(&self) -> &[TelemetryField] {
        &self.resource_attributes
    }

    /// Borrow the closed span attributes.
    pub fn span_attributes(&self) -> &[TelemetryField] {
        &self.span_attributes
    }

    /// Borrow the closed metric labels.
    pub fn metric_labels(&self) -> &[TelemetryField] {
        &self.metric_labels
    }

    /// Validate a complete collector frame and reject the whole frame when a
    /// non-allowlisted or identity-bearing key is present.
    pub fn validate_collector_fields(
        fields: impl IntoIterator<Item = TelemetryField>,
    ) -> Result<(), TelemetryFrameError> {
        let resource = [
            "d2b.zone",
            "d2b.provider",
            "d2b.component",
            "service.name",
            "service.namespace",
            "service.version",
        ];
        let span = [
            "d2b.credential.provider",
            "d2b.credential.operation_class",
            "d2b.credential.placement_binding",
            "d2b.credential.outcome",
        ];
        let metric = ["operation_class", "placement_binding", "outcome"];
        for field in fields {
            if FORBIDDEN_KEYS.contains(&field.key)
                || !(resource.contains(&field.key)
                    || span.contains(&field.key)
                    || metric.contains(&field.key))
            {
                return Err(TelemetryFrameError::ForbiddenField);
            }
        }
        Ok(())
    }

    /// Return all fields for collector validation.
    pub fn all_fields(&self) -> Vec<TelemetryField> {
        self.resource_attributes
            .iter()
            .chain(&self.span_attributes)
            .chain(&self.metric_labels)
            .cloned()
            .collect()
    }

    /// Group metric values by their fixed key for descriptor inspection.
    pub fn metric_map(&self) -> BTreeMap<&'static str, &str> {
        self.metric_labels
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collector_allowlist_rejects_credential_identity_keys() {
        let frame = ManagedIdentityTelemetryFrame::new(
            "dev",
            ManagedIdentityTelemetryOperation::AcquireToken,
            ManagedIdentityTelemetryOutcome::Success,
            PlacementBinding::GuestAgent,
        );
        assert!(
            ManagedIdentityTelemetryFrame::validate_collector_fields(frame.all_fields()).is_ok()
        );
        assert_eq!(
            ManagedIdentityTelemetryFrame::validate_collector_fields([TelemetryField {
                key: "d2b.credential.name",
                value: "canary".to_owned(),
            }]),
            Err(TelemetryFrameError::ForbiddenField)
        );
    }
}
