mod common;

use std::sync::atomic::Ordering;

use d2b_contracts::v3::ResourceRef;
use d2b_credential_service::{
    CredentialMethod, CredentialServer, CredentialServiceErrorCode, CredentialTransport,
};
use d2b_provider_credential_entra::{EntraClientError, EntraClientState};

use common::{Admission, admitted, request, setup};

#[test]
fn interaction_required_is_unavailable_not_denied() {
    let (provider, client) = setup();
    *client.state.lock().unwrap() = EntraClientState::InteractionRequired;
    let server = CredentialServer::new(provider, admitted());
    assert_eq!(
        server
            .call(CredentialMethod::AcquireToken, request("idem-interaction"))
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::ProviderUnavailable
    );
}

#[test]
fn exact_consumer_mismatch_is_denied_before_client_dispatch() {
    let (provider, client) = setup();
    assert!(!provider.authorizes_consumer(&ResourceRef::parse("Provider/other").unwrap()));
    let admission = Admission {
        authenticated_consumer: ResourceRef::parse("Provider/other").unwrap(),
    };
    let server = CredentialServer::new(provider, admission);
    assert_eq!(
        server
            .call(CredentialMethod::AcquireToken, request("idem-mismatch"))
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::OperationDenied
    );
    assert_eq!(client.issue_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn generation_and_unsupported_operation_fail_closed() {
    let (provider, client) = setup();
    assert_eq!(
        provider.validate_endpoint_generation(8).unwrap_err().code(),
        CredentialServiceErrorCode::InvariantFailure
    );
    *client.issue_error.lock().unwrap() = Some(EntraClientError::GenerationMismatch);
    let server = CredentialServer::new(provider, admitted());
    assert_eq!(
        server
            .call(CredentialMethod::AcquireToken, request("idem-generation"))
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::InvariantFailure
    );

    let (provider, client) = setup();
    let server = CredentialServer::new(provider, admitted());
    assert_eq!(
        server
            .call(CredentialMethod::SignChallenge, request("idem-sign"))
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::Malformed
    );
    assert_eq!(client.issue_calls.load(Ordering::SeqCst), 0);
}
