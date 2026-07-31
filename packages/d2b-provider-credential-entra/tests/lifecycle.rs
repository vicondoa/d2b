mod common;

use std::sync::atomic::Ordering;

use d2b_credential_service::{
    CredentialMethod, CredentialResponse, CredentialServer, CredentialTransport,
};

use common::{admitted, request, setup};

#[test]
fn acquire_refresh_revoke_and_inspect_use_the_identity_guest_client() {
    let (provider, client) = setup();
    let server = CredentialServer::new(provider, admitted());
    assert!(matches!(
        server
            .call(CredentialMethod::AcquireToken, request("idem-acquire"))
            .unwrap(),
        CredentialResponse::AcquireToken(_)
    ));
    assert!(matches!(
        server
            .call(CredentialMethod::RefreshToken, request("idem-refresh"))
            .unwrap(),
        CredentialResponse::RefreshToken(_)
    ));
    assert!(matches!(
        server
            .call(CredentialMethod::InspectMetadata, request("idem-inspect"))
            .unwrap(),
        CredentialResponse::InspectMetadata(_)
    ));
    assert!(matches!(
        server
            .call(CredentialMethod::RevokeToken, request("idem-revoke"))
            .unwrap(),
        CredentialResponse::RevokeToken(_)
    ));
    assert_eq!(client.issue_calls.load(Ordering::SeqCst), 1);
    assert_eq!(client.refresh_calls.load(Ordering::SeqCst), 1);
    assert_eq!(client.revoke_calls.load(Ordering::SeqCst), 1);
}
