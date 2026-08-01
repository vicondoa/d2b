//! Secret Service Credential audit producer.

use d2b_contracts::v3::credential::CredentialMethod;
use d2b_contracts::v3::credential_controller::{
    CredentialAuditDigest, CredentialAuditOutcome, CredentialAuditRecord,
    CredentialObservabilityError, CredentialProviderKind,
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
    if !authorized {
        return CredentialAuditRecord::authorized_service(
            false,
            CredentialProviderKind::SecretService,
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
        CredentialProviderKind::SecretService,
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
