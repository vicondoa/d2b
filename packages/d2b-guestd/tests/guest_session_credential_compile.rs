use d2b_contracts::v2_component_session::GuestSessionCredentialV1;

#[test]
fn guestd_can_decode_the_shared_guest_session_credential() {
    let encoded =
        GuestSessionCredentialV1::new(7, [0x11; 32], [0x22; 32], [0x33; 32], [0x44; 32], None)
            .unwrap()
            .encode()
            .unwrap();
    let decoded = GuestSessionCredentialV1::decode(&encoded).unwrap();
    assert_eq!(decoded.session_generation(), 7);
}
