use d2b_provider_device_security_key::{
    SecurityKeySessionId, SessionRecord, SessionResult, SessionRing,
};

#[test]
fn oldest_session_is_evicted_at_capacity() {
    let mut ring = SessionRing::new(8).unwrap();
    let first = SecurityKeySessionId::from_core([1; 16]);
    for index in 0..8u8 {
        ring.push(SessionRecord::new(
            SecurityKeySessionId::from_core([index; 16]),
            SessionResult::Success,
        ));
    }
    let evicted = ring.push(SessionRecord::new(first, SessionResult::Cancelled));
    assert_eq!(
        evicted.unwrap().id(),
        SecurityKeySessionId::from_core([0; 16])
    );
    assert_eq!(ring.entries().count(), 8);
}
