mod common;

use d2b_credential_service::{
    CredentialMethod, CredentialResponse, CredentialServer, CredentialTransport,
    SensitiveDeliveryRecord,
};

use common::{admitted, delivery, request, setup};

#[test]
fn provider_returns_the_adapter_supplied_binding_unchanged() {
    let expected = delivery(CredentialMethod::AcquireToken, 1);
    let (provider, _) = setup();
    let server = CredentialServer::new(provider, admitted());
    let response = server
        .call(CredentialMethod::AcquireToken, request("idem-binding"))
        .unwrap();
    let CredentialResponse::AcquireToken(response) = response else {
        panic!("acquire response");
    };
    assert_eq!(response.delivery_session_params, expected);
}

#[test]
fn delivery_zeroizes_and_replay_sequence_advances() {
    let mut record = SensitiveDeliveryRecord::new(b"managed-token".to_vec(), 64).unwrap();
    let mut destination = [0; 13];
    record.copy_to(&mut destination).unwrap();
    destination.fill(0);
    record.clear();
    assert!(record.is_zeroized());
    assert!(
        delivery(CredentialMethod::RefreshToken, 2).sequence()
            > delivery(CredentialMethod::AcquireToken, 1).sequence()
    );
}
