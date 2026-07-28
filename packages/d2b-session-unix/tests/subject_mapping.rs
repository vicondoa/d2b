use d2b_contracts::v3::{ResourceRef, ResourceUid};
use d2b_session::contract::TransportClass;
use d2b_session_unix::{
    SeqpacketSocket, StreamSocket, UnixSubjectIdentity, prearmed_seqpacket_pair,
};
use rustix::net::{AddressFamily, SocketFlags, SocketType, socketpair};

#[tokio::test]
async fn so_peercred_maps_host_and_guest_subjects() {
    for (subject_ref, expected_type) in [("Host/alice-host", "Host"), ("Guest/corp-vm", "Guest")] {
        let (left, _right) = socketpair(
            AddressFamily::UNIX,
            SocketType::STREAM,
            SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
            None,
        )
        .unwrap();
        let socket = StreamSocket::from_owned(left).unwrap();
        let expected_peer = socket.acceptor_peer_credentials().unwrap();
        let subject = if expected_type == "Host" {
            UnixSubjectIdentity::host(
                ResourceRef::parse(subject_ref).unwrap(),
                ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
                ResourceRef::parse("Zone/work").unwrap(),
                expected_peer,
            )
            .unwrap()
            .verify_stream(&socket)
            .unwrap()
        } else {
            UnixSubjectIdentity::guest(
                ResourceRef::parse(subject_ref).unwrap(),
                ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
                ResourceRef::parse("Zone/work").unwrap(),
                expected_peer,
            )
            .unwrap()
            .verify_stream(&socket)
            .unwrap()
        };
        assert_eq!(
            subject.subject_ref().resource_type().as_str(),
            expected_type
        );
        subject
            .validate_transport(TransportClass::UnixStream)
            .unwrap();
    }
}

#[tokio::test]
async fn unix_subject_proof_rejects_transport_rebinding() {
    let (left, _right) = prearmed_seqpacket_pair().unwrap();
    let socket = SeqpacketSocket::from_parent_prearmed(left).unwrap();
    let expected_peer = socket.acceptor_peer_credentials().unwrap();
    let subject = UnixSubjectIdentity::host(
        ResourceRef::parse("Host/alice-host").unwrap(),
        ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
        ResourceRef::parse("Zone/work").unwrap(),
        expected_peer,
    )
    .unwrap()
    .verify_seqpacket(&socket)
    .unwrap();
    let error = subject
        .validate_transport(TransportClass::UnixStream)
        .unwrap_err();
    assert_eq!(
        error.code(),
        d2b_session::contract::SessionErrorCode::SubjectMismatch
    );
}
