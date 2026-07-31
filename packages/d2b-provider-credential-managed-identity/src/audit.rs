//! Managed identity Credential audit producer.

use d2b_contracts::v3::credential::CredentialMethod;
use d2b_contracts::v3::credential_controller::{
    CredentialAuditDigest, CredentialAuditOperation, CredentialAuditOutcome, CredentialAuditRecord,
    CredentialObservabilityError, CredentialProviderKind,
};

/// Managed identity audit errors use the shared field-free error.
pub type ManagedIdentityAuditError = CredentialObservabilityError;

/// Backward-compatible closed managed identity audit operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedIdentityAuditOperation {
    /// Token acquisition.
    AcquireToken,
    /// Token refresh.
    RefreshToken,
    /// Lease revocation.
    RevokeToken,
    /// Metadata inspection.
    InspectMetadata,
    /// Agent start.
    AgentStart,
    /// Agent stop.
    AgentStop,
}

/// Backward-compatible closed managed identity audit outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedIdentityAuditOutcome {
    /// The operation completed.
    Success,
    /// The client was unavailable.
    ProviderUnavailable,
    /// Policy denied the operation.
    Denied,
    /// The lease had already been revoked.
    AlreadyRevoked,
}

/// Compatibility wrapper over the shared bounded audit record.
pub struct ManagedIdentityAuditRecord(CredentialAuditRecord);

impl ManagedIdentityAuditRecord {
    /// Construct one bounded controller audit event.
    pub fn new(
        resource_name_digest: impl Into<String>,
        operation: ManagedIdentityAuditOperation,
        outcome: ManagedIdentityAuditOutcome,
        rotation_generation: u64,
    ) -> Result<Self, ManagedIdentityAuditError> {
        let operation = match operation {
            ManagedIdentityAuditOperation::AcquireToken => CredentialAuditOperation::AcquireToken,
            ManagedIdentityAuditOperation::RefreshToken => CredentialAuditOperation::RefreshToken,
            ManagedIdentityAuditOperation::RevokeToken => CredentialAuditOperation::RevokeToken,
            ManagedIdentityAuditOperation::InspectMetadata => {
                CredentialAuditOperation::InspectMetadata
            }
            ManagedIdentityAuditOperation::AgentStart => CredentialAuditOperation::AgentStart,
            ManagedIdentityAuditOperation::AgentStop => CredentialAuditOperation::AgentStop,
        };
        let outcome = match outcome {
            ManagedIdentityAuditOutcome::Success => CredentialAuditOutcome::Success,
            ManagedIdentityAuditOutcome::ProviderUnavailable => {
                CredentialAuditOutcome::ProviderUnavailable
            }
            ManagedIdentityAuditOutcome::Denied => CredentialAuditOutcome::Denied,
            ManagedIdentityAuditOutcome::AlreadyRevoked => CredentialAuditOutcome::AlreadyRevoked,
        };
        Ok(Self(CredentialAuditRecord::controller_event(
            CredentialProviderKind::ManagedIdentity,
            "system",
            CredentialAuditDigest::parse(resource_name_digest)?,
            operation,
            outcome,
            rotation_generation,
            None,
            None,
        )?))
    }

    /// Render the bounded shared audit payload.
    pub fn to_wire_record(&self) -> String {
        self.0.to_wire_record()
    }
}

impl core::fmt::Debug for ManagedIdentityAuditRecord {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ManagedIdentityAuditRecord(<redacted>)")
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn authorized_service_record(
    authorized: bool,
    zone: &str,
    subject_identity: &[u8],
    credential_name: &[u8],
    method: CredentialMethod,
    outcome: CredentialAuditOutcome,
    rotation_generation: u64,
    idempotency_key: Option<&[u8]>,
) -> Result<Option<CredentialAuditRecord>, CredentialObservabilityError> {
    if !authorized {
        return CredentialAuditRecord::authorized_service(
            false,
            CredentialProviderKind::ManagedIdentity,
            "",
            "",
            "",
            method,
            outcome,
            rotation_generation,
            None,
        );
    }
    let subject = CredentialAuditDigest::after_authorization(subject_identity);
    let resource = CredentialAuditDigest::after_authorization(credential_name);
    let idempotency = idempotency_key.map(CredentialAuditDigest::after_authorization);
    CredentialAuditRecord::authorized_service(
        true,
        CredentialProviderKind::ManagedIdentity,
        zone,
        subject.as_str(),
        resource.as_str(),
        method,
        outcome,
        rotation_generation,
        idempotency.map(|digest| digest.as_str().to_owned()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_unique_managed_identity_canary_never_renders() {
        let marker = format!("managed-identity-canary-{:x}", std::process::id());
        let record = authorized_service_record(
            true,
            "dev",
            marker.as_bytes(),
            marker.as_bytes(),
            CredentialMethod::AcquireToken,
            CredentialAuditOutcome::Success,
            1,
            Some(marker.as_bytes()),
        )
        .unwrap()
        .unwrap();
        assert!(!record.to_wire_record().contains(&marker));
        assert!(!format!("{record:?}").contains(&marker));
    }
}
