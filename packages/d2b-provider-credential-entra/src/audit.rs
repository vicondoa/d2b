//! Entra Credential audit producer.

use d2b_contracts_provider::v3::credential::CredentialMethod;
use d2b_contracts_provider::v3::credential_controller::{
    CredentialAuditOutcome, CredentialAuditRecord, CredentialObservabilityError,
    CredentialProviderKind,
};
use d2b_provider_toolkit::credential;

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
    credential::authorized_service_record(
        CredentialProviderKind::Entra,
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
