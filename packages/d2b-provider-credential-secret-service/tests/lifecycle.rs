mod common;

use std::sync::atomic::Ordering;

use d2b_contracts::v3::credential::CredentialLeaseState;
use d2b_credential_service::{
    CredentialMethod, CredentialOutcomeCode, CredentialResponse, CredentialServer,
    CredentialTransport,
};

use common::{Admission, request, setup};

#[test]
fn acquire_refresh_revoke_and_inspect_use_the_injected_port() {
    let (provider, port) = setup(64);
    let server = CredentialServer::new(provider, Admission);
    let acquired = server
        .call(CredentialMethod::AcquireToken, request("idem-acquire"))
        .unwrap();
    assert!(matches!(acquired, CredentialResponse::AcquireToken(_)));
    let refreshed = server
        .call(CredentialMethod::RefreshToken, request("idem-refresh"))
        .unwrap();
    assert!(matches!(refreshed, CredentialResponse::RefreshToken(_)));
    let inspected = server
        .call(CredentialMethod::InspectMetadata, request("idem-inspect"))
        .unwrap();
    let CredentialResponse::InspectMetadata(inspected) = inspected else {
        panic!("inspect response");
    };
    assert_eq!(inspected.metadata.state, CredentialLeaseState::Active);
    assert_eq!(inspected.metadata.rotation_generation, 2);
    let revoked = server
        .call(CredentialMethod::RevokeToken, request("idem-revoke"))
        .unwrap();
    let CredentialResponse::RevokeToken(revoked) = revoked else {
        panic!("revoke response");
    };
    assert_eq!(revoked.metadata.outcome, CredentialOutcomeCode::Revoked);
    let repeated = server
        .call(CredentialMethod::RevokeToken, request("idem-revoke-2"))
        .unwrap();
    let CredentialResponse::RevokeToken(repeated) = repeated else {
        panic!("revoke response");
    };
    assert_eq!(
        repeated.metadata.outcome,
        CredentialOutcomeCode::AlreadyRevoked
    );
    assert_eq!(port.issue_calls.load(Ordering::SeqCst), 1);
    assert_eq!(port.refresh_calls.load(Ordering::SeqCst), 1);
    assert_eq!(port.revoke_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn duplicate_acquire_is_idempotent() {
    let (provider, port) = setup(64);
    let server = CredentialServer::new(provider, Admission);
    server
        .call(CredentialMethod::AcquireToken, request("same-key"))
        .unwrap();
    server
        .call(CredentialMethod::AcquireToken, request("same-key"))
        .unwrap();
    assert_eq!(port.issue_calls.load(Ordering::SeqCst), 1);
}
