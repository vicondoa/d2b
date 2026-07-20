#![forbid(unsafe_code)]

#[test]
fn guest_vsock_is_component_session_framed_before_ttrpc() {
    let service = include_str!("../../d2b-guestd/src/service.rs");
    let session = include_str!("../../d2b-guestd/src/service_v2.rs");
    let guest_service = include_str!("../../d2b-guestd/src/guest_service.rs");

    assert!(service.contains("establish_bootstrap_initiator"));
    assert!(service.contains("establish_responder"));
    assert!(session.contains("FramedGuestTransport"));
    assert!(guest_service.contains("serve_guest_session"));
    assert!(!service.contains("Listener::bind(\"vsock://"));
    assert!(!session.contains("GuestControlService"));
}
