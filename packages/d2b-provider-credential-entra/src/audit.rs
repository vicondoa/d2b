//! Entra Credential audit producer.

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
            CredentialProviderKind::Entra,
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
        CredentialProviderKind::Entra,
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
    fn process_unique_token_and_identity_canaries_never_render() {
        let marker = format!("entra-token-canary-{:x}", std::process::id());
        let record = authorized_service_record(
            true,
            "dev",
            marker.as_bytes(),
            marker.as_bytes(),
            CredentialMethod::RefreshToken,
            CredentialAuditOutcome::Success,
            2,
            Some(marker.as_bytes()),
        )
        .unwrap()
        .unwrap();
        assert!(!record.to_wire_record().contains(&marker));
        assert!(!format!("{record:?}").contains(&marker));
    }
}
