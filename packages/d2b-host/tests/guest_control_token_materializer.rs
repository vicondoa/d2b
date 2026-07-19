#![forbid(unsafe_code)]

#[test]
fn guest_service_has_retired_long_lived_token_authentication() {
    let auth = include_str!("../../d2b-guestd/src/auth.rs");
    let main = include_str!("../../d2b-guestd/src/main.rs");
    let listener = include_str!("../../d2b-guestd/src/service.rs");
    let service = include_str!("../../d2b-guestd/src/service_v2.rs");
    let old_binding = include_str!("../../d2b-guestd/src/generated/guest_control_ttrpc.rs");

    for source in [auth, main, listener, service, old_binding] {
        assert!(!source.contains("guest-control.token"));
        assert!(!source.contains("HelloRequest"));
        assert!(!source.contains("AuthenticateRequest"));
        assert!(!source.contains("Hmac<"));
    }
    assert!(service.contains("d2b.guest.v2"));
    assert!(listener.contains("systemd-creds"));
}
