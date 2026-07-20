use d2b_contracts::v2_component_session::{
    BootstrapPskBinding, GuestBootstrapCredentialV1, GuestBootstrapPsk, GuestIdentityBindingV1,
    GuestSessionCredentialV1, OperationId,
};

#[test]
fn guestd_can_decode_the_shared_guest_session_credential() {
    let psk = GuestBootstrapPsk::generate_with(|bytes| {
        bytes.fill(0x88);
        Ok(())
    })
    .unwrap();
    let bootstrap = GuestBootstrapCredentialV1::new(
        BootstrapPskBinding {
            operation_id: OperationId::new(vec![0x66; 16]).unwrap(),
            replay_nonce: [0x77; 32],
            expires_at_unix_ms: 9_000,
        },
        1_000,
        psk,
    )
    .unwrap();
    let encoded = GuestSessionCredentialV1::new(
        7,
        [0x11; 32],
        [0x22; 32],
        GuestIdentityBindingV1::UnboundBootstrap,
        Some(bootstrap),
    )
    .unwrap()
    .encode()
    .unwrap();
    let decoded = GuestSessionCredentialV1::decode(encoded.as_slice()).unwrap();
    assert_eq!(decoded.session_generation(), 7);
    assert!(decoded.guest_identity_is_unbound());
    assert!(decoded.guest_identity_digest().is_none());
    assert!(decoded.guest_static_public_key().is_none());
    assert_eq!(decoded.bootstrap().unwrap().expose_psk(), &[0x88; 32]);
}
