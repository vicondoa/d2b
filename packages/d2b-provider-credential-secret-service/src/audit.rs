//! Secret Service Credential audit producer.

use d2b_contracts_provider::v3::credential::CredentialMethod;
use d2b_contracts_provider::v3::credential_controller::{
    CredentialAuditOutcome, CredentialAuditRecord, CredentialObservabilityError,
    CredentialProviderKind,
};

#[allow(clippy::too_many_arguments)]
pub(super) fn authorized_service_record(
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
        CredentialProviderKind::SecretService,
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
    fn process_unique_identity_canary_is_hashed_only_after_authorization() {
        let marker = format!("secret-canary-{:x}", std::process::id());
        let denied = authorized_service_record(
            false,
            "zone-secret-canary",
            marker.as_bytes(),
            marker.as_bytes(),
            CredentialMethod::AcquireToken,
            CredentialAuditOutcome::Denied,
            1,
            Some(marker.as_bytes()),
        )
        .unwrap();
        assert!(denied.is_none());
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
        let wire = record.to_wire_record();
        assert!(wire.contains("resource_name_digest=sha256:"));
        assert!(!wire.contains(&marker));
        assert!(!format!("{record:?}").contains(&marker));
    }
}
