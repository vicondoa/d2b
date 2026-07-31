mod common;

use std::sync::atomic::Ordering;

use d2b_contracts::v3::ResourceRef;
use d2b_credential_service::{
    CredentialMethod, CredentialServer, CredentialServiceErrorCode, CredentialTransport,
};
use d2b_provider_credential_managed_identity::ManagedIdentityClientState;

use common::{Admission, admitted, request, setup};

#[test]
fn unavailable_maps_to_provider_unavailable_without_fallback() {
    let (provider, client) = setup();
    *client.state.lock().unwrap() = ManagedIdentityClientState::Unavailable;
    let server = CredentialServer::new(provider, admitted());
    assert_eq!(
        server
            .call(CredentialMethod::AcquireToken, request("idem-unavailable"))
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::ProviderUnavailable
    );
    assert_eq!(client.issue_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn exact_sdk_consumer_mismatch_denies_before_client_dispatch() {
    let (provider, client) = setup();
    assert!(!provider.authorizes_consumer(&ResourceRef::parse("Provider/other").unwrap()));
    let server = CredentialServer::new(
        provider,
        Admission {
            authenticated_consumer: ResourceRef::parse("Provider/other").unwrap(),
        },
    );
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
fn sign_challenge_is_schema_invalid_before_client_use() {
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
