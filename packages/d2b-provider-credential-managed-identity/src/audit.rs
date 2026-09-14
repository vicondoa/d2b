//! Managed identity Credential audit producer.

use d2b_contracts_provider::v3::credential::CredentialMethod;
use d2b_contracts_provider::v3::credential_controller::{
    CredentialAuditOutcome, CredentialAuditRecord, CredentialObservabilityError,
    CredentialProviderKind,
};

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
    d2b_provider_toolkit::credential::authorized_service_record(
        CredentialProviderKind::ManagedIdentity,
        authorized,
        zone,
        subject_identity,
        credential_name,
        method,
        outcome,
        rotation_generation,
        idempotency_key,
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
