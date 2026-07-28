use d2b_session::contract::TransportClass;
use d2b_session_unix::{SeqpacketSocket, StreamSocket, VerifiedUnixPeer, prearmed_seqpacket_pair};
use rustix::net::{AddressFamily, SocketFlags, SocketType, socketpair};

#[tokio::test]
async fn so_peercred_produces_claim_free_peer_evidence() {
    let (left, _right) = socketpair(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
        None,
    )
    .unwrap();
    let socket = StreamSocket::from_owned(left).unwrap();
    let expected_peer = socket.acceptor_peer_credentials().unwrap();
    let peer = VerifiedUnixPeer::verify_stream(&socket).unwrap();

    assert_eq!(peer.credentials(), expected_peer);
    peer.validate_transport(TransportClass::UnixStream).unwrap();
}

#[tokio::test]
async fn unix_peer_evidence_rejects_transport_rebinding() {
    let (left, _right) = prearmed_seqpacket_pair().unwrap();
    let socket = SeqpacketSocket::from_parent_prearmed(left).unwrap();
    let peer = VerifiedUnixPeer::verify_seqpacket(&socket).unwrap();
    let error = peer
        .validate_transport(TransportClass::UnixStream)
        .unwrap_err();
    assert_eq!(
        error.code(),
        d2b_session::contract::SessionErrorCode::SubjectMismatch
    );
}
