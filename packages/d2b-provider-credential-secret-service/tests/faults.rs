mod common;

use std::sync::atomic::Ordering;

use d2b_credential_service::{
    CredentialMethod, CredentialServer, CredentialServiceErrorCode, CredentialTransport,
};
use d2b_provider_credential_secret_service::SecretServicePortError;

use common::{Admission, request, setup};

#[test]
fn locked_and_unavailable_map_to_provider_unavailable() {
    for failure in [
        SecretServicePortError::Locked,
        SecretServicePortError::Unavailable,
    ] {
        let (provider, port) = setup(64);
        *port.issue_error.lock().unwrap() = Some(failure);
        let server = CredentialServer::new(provider, Admission);
        assert_eq!(
            server
                .call(CredentialMethod::AcquireToken, request("idem-failure"))
                .unwrap_err()
                .code(),
            CredentialServiceErrorCode::ProviderUnavailable
        );
        assert_eq!(port.issue_calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn unsupported_sign_challenge_never_calls_the_port() {
    let (provider, port) = setup(64);
    let server = CredentialServer::new(provider, Admission);
    assert_eq!(
        server
            .call(CredentialMethod::SignChallenge, request("idem-sign"))
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::Malformed
    );
    assert_eq!(port.issue_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn cardinality_is_enforced_before_a_second_port_call() {
    let (provider, port) = setup(1);
    let server = CredentialServer::new(provider, Admission);
    server
        .call(CredentialMethod::AcquireToken, request("idem-first"))
        .unwrap();
    let other = d2b_credential_service::CredentialRequest::new(
        d2b_contracts::v3::ResourceRef::parse("Credential/other-keyring").unwrap(),
        "operation-2",
        "idem-second",
        common::EXPIRY,
        15_000,
    )
    .unwrap();
    assert_eq!(
        server
            .call(CredentialMethod::AcquireToken, other)
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::ProviderUnavailable
    );
    assert_eq!(port.issue_calls.load(Ordering::SeqCst), 1);
}
