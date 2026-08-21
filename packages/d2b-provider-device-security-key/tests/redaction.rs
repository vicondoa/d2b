use d2b_provider_device_security_key::{
    SecurityKeySessionId, SessionRecord, SessionResult, SessionRing,
};

#[test]
fn security_key_canary_stays_out_of_debug_and_errors() {
    const CANARY: &str = "ctap-payload-canary-7f4a";
    let session = SecurityKeySessionId::from_core([0xa5; 16]);
    let record = SessionRecord::new(session, SessionResult::CtapError);
    let ring = SessionRing::new(8).unwrap();
    let rendered = format!("{record:?} {ring:?}");

    assert!(!rendered.contains(CANARY));
    assert!(!rendered.contains("a5"));
    assert!(
        d2b_provider_device_security_key::SecurityKeyLeaseError::AuthorizationDenied
            .to_string()
            .starts_with("device-")
    );
}
