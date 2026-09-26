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


