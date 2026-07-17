use d2b_contracts::v2_component_session::{
    GUEST_SESSION_CREDENTIAL_V1_BASE_BYTES, GuestSessionCredentialV1,
};

#[test]
fn broker_can_encode_the_shared_guest_session_credential() {
    let credential =
        GuestSessionCredentialV1::new(7, [0x11; 32], [0x22; 32], [0x33; 32], [0x44; 32], None)
            .unwrap();
    let encoded = credential.encode().unwrap();
    assert_eq!(encoded.len(), GUEST_SESSION_CREDENTIAL_V1_BASE_BYTES);
}
