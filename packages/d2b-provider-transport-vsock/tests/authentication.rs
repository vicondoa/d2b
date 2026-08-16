use d2b_contracts::v3::{ResourceRef, ZoneId};
use d2b_provider_transport_vsock::{
    GuestControlKey, GuestIdentity, PeerCid, SessionAuthority, SessionProof, SessionRejectReason,
    SessionState,
};

fn identity(cid: u32) -> GuestIdentity {
    GuestIdentity::new(
        ResourceRef::parse("Guest/guest-a").unwrap(),
        ZoneId::parse("work").unwrap(),
        PeerCid::from_core(cid).unwrap(),
        "boot-a",
    )
    .unwrap()
}

#[test]
fn correct_cid_signature_guest_zone_and_session_establish_ready() {
    let key = GuestControlKey::from_core([7; 32]);
    let expected = identity(42);
    let mut authority = SessionAuthority::new(expected.clone(), key.clone(), 3);
    let proof = SessionProof::sign(&key, &expected, [9; 32], 3);

    let session = authority.authenticate(proof).unwrap();
    assert_eq!(session.state(), SessionState::Ready);
    assert!(session.matches(&expected));
    assert_eq!(session.disconnect(), SessionState::Disconnected);
}

#[test]
fn cid_reuse_and_replay_are_rejected() {
    let key = GuestControlKey::from_core([8; 32]);
    let expected = identity(42);
    let mut authority = SessionAuthority::new(expected.clone(), key.clone(), 3);
    let proof = SessionProof::sign(&key, &expected, [4; 32], 3);
    authority.authenticate(proof.clone()).unwrap();
    assert_eq!(
        authority.authenticate(proof).unwrap_err(),
        SessionRejectReason::Replay
    );

    let mut other = identity(43);
    let proof = SessionProof::sign(&key, &other, [5; 32], 3);
    assert_eq!(
        authority.authenticate(proof).unwrap_err(),
        SessionRejectReason::CidMismatch
    );
    other = identity(42);
    let proof = SessionProof::sign(&key, &other, [6; 32], 2);
    assert_eq!(
        authority.authenticate(proof).unwrap_err(),
        SessionRejectReason::StaleSignature
    );
}
