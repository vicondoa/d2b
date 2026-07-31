mod common;

use d2b_contracts::v3::ResourceRef;
use d2b_contracts::v3::credential::{
    CredentialInteractionState, CredentialLeaseHandle, CredentialLeaseStatus, CredentialMethod,
    CredentialRequest, CredentialResponse, CredentialServiceErrorCode, CredentialSourceVersion,
    CredentialStatus, PlacementBinding, encode_outer,
};

use common::{ProviderHarness, admitted, setup};

#[test]
fn process_unique_entra_secret_and_identity_canaries_are_absent_from_rendered_surfaces() {
    let nonce = format!("{:x}", std::process::id());
    let credential_name = format!("credential-name-{nonce}");
    let credential_ref = format!("Credential/{credential_name}");
    let credential_uid = format!("credential-uid-{nonce}");
    let credential_digest = format!("credential-digest-{nonce}");
    let (provider, client) = setup();
    let operation_id = format!("{}-{credential_uid}", client.endpoint_canary);
    let idempotency_key = format!("{}-{credential_digest}", client.cookie_canary);
    let request = CredentialRequest::new(
        ResourceRef::parse(&credential_ref).unwrap(),
        &operation_id,
        &idempotency_key,
        common::EXPIRY,
        15_000,
    )
    .unwrap();
    let request_debug = format!("{request:?}");
    let provider_debug = format!("{provider:?}");
    let server = ProviderHarness::new(provider, admitted());
    let response = server
        .call(CredentialMethod::AcquireToken, request)
        .unwrap();
    let CredentialResponse::AcquireToken(delivery) = &response else {
        panic!("acquire response");
    };
    assert_eq!(
        delivery.metadata.lease_handle,
        CredentialLeaseHandle::parse(&client.token_canary).unwrap()
    );
    assert_eq!(
        delivery.metadata.source_version,
        CredentialSourceVersion::parse(&client.endpoint_canary).unwrap()
    );
    assert_eq!(
        client.observed_request.lock().unwrap().as_ref(),
        Some(&(
            credential_ref.clone(),
            operation_id.clone(),
            idempotency_key.clone(),
        ))
    );
    let status = CredentialStatus::new(
        CredentialInteractionState::NotRequired,
        None,
        None,
        Some(
            CredentialLeaseStatus::new(
                delivery.metadata.lease_handle.clone(),
                delivery.metadata.state,
                delivery.metadata.rotation_generation,
                delivery.metadata.source_version.clone(),
                delivery.metadata.expires_at_unix_ms,
                1,
                None,
                None,
                PlacementBinding::GuestAgent,
            )
            .unwrap(),
        ),
    )
    .unwrap();
    let (error_provider, error_client) = setup();
    *error_client.issue_error.lock().unwrap() =
        Some(d2b_provider_credential_entra::EntraClientError::Unavailable);
    let error = ProviderHarness::new(error_provider, admitted())
        .call(
            CredentialMethod::AcquireToken,
            CredentialRequest::new(
                ResourceRef::parse(&credential_ref).unwrap(),
                &operation_id,
                &idempotency_key,
                common::EXPIRY,
                15_000,
            )
            .unwrap(),
        )
        .unwrap_err();
    assert_eq!(
        error.code(),
        CredentialServiceErrorCode::ProviderUnavailable
    );
    assert_eq!(
        error_client.observed_request.lock().unwrap().as_ref(),
        Some(&(
            credential_ref.clone(),
            operation_id.clone(),
            idempotency_key.clone(),
        ))
    );
    let surfaces = [
        provider_debug,
        request_debug,
        format!("{response:?}"),
        String::from_utf8_lossy(&encode_outer(delivery).unwrap()).into_owned(),
        format!("{:?}", delivery.metadata.lease_handle),
        delivery.metadata.lease_handle.to_string(),
        format!("{:?}", delivery.metadata.source_version),
        delivery.metadata.source_version.to_string(),
        format!("{status:?}"),
        serde_json::to_string(&status).unwrap(),
        format!("{error:?}"),
        error.to_string(),
    ];
    let markers = [
        client.token_canary.as_str(),
        client.endpoint_canary.as_str(),
        client.cookie_canary.as_str(),
        credential_name.as_str(),
        credential_ref.as_str(),
        credential_uid.as_str(),
        credential_digest.as_str(),
    ];
    for surface in surfaces {
        for marker in markers {
            assert!(
                !surface.contains(marker),
                "Entra canary reached a rendered surface"
            );
        }
    }
}
