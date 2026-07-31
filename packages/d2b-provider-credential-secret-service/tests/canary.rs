mod common;

use d2b_contracts::v3::credential::{
    CredentialInteractionState, CredentialLeaseStatus, CredentialStatus, PlacementBinding,
};
use d2b_credential_service::{
    CredentialMethod, CredentialResponse, CredentialServer, CredentialServiceError,
    CredentialServiceErrorCode, CredentialTransport, encode_outer,
};

use common::{Admission, request, setup};

#[test]
fn process_unique_secret_service_canaries_are_absent_from_every_rendered_surface() {
    let (provider, port) = setup(64);
    let provider_debug = format!("{provider:?}");
    let server = CredentialServer::new(provider, Admission);
    let response = server
        .call(CredentialMethod::AcquireToken, request("idem-canary"))
        .unwrap();
    let CredentialResponse::AcquireToken(delivery) = &response else {
        panic!("acquire response");
    };
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
                PlacementBinding::UserAgent,
            )
            .unwrap(),
        ),
    )
    .unwrap();
    let error = CredentialServiceError::new(CredentialServiceErrorCode::ProviderUnavailable);
    let log_line = "provider=credential-secret-service operation=acquire-token outcome=success";
    let audit = "provider=credential-secret-service operation=acquire-token outcome=success";
    let telemetry = "d2b.credential.provider=credential-secret-service operation_class=acquire-token outcome=success";
    let surfaces = [
        provider_debug,
        format!("{response:?}"),
        String::from_utf8_lossy(&encode_outer(delivery).unwrap()).into_owned(),
        format!("{status:?}"),
        serde_json::to_string(&status).unwrap(),
        format!("{error:?}"),
        error.to_string(),
        log_line.to_owned(),
        audit.to_owned(),
        telemetry.to_owned(),
    ];
    for surface in surfaces {
        for canary in [&port.credential_canary, &port.object_path_canary] {
            assert!(
                !surface.contains(canary),
                "secret-service canary reached a rendered surface"
            );
        }
    }
}
