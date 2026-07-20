use d2b_contracts::v2_component_session::{
    AttachmentAccess, AttachmentCreditClass, AttachmentDescriptor, AttachmentKind,
    AttachmentPacket, AttachmentPolicy, AttachmentPolicyKind, AttachmentPurpose, BoundedVec,
    EndpointPolicy, EndpointPolicyIdentity, EndpointPurpose, EndpointRole,
    IdentityEvidenceRequirement, KernelObjectType, LimitProfile, Locality, NoiseProfile,
    PurposeClass, RequestId, ServicePackage, TransportBinding, TransportClass,
};
use d2b_session::{
    AttachmentPayload, AttachmentValidationError, HandshakeCredentials, OwnedAttachment,
    OwnedTransport, SessionEngine, SessionEvent, TransportPacket,
};
use d2b_session_unix::{
    AncillaryCapacity, CreditPool, CreditScopeSet, DescriptorPolicy, ObjectIdentity,
    OutboundPacket, OwnedUnixAttachment, PeerIdentityPolicy, PidfdEvidence, PidfdIdentityPolicy,
    PidfdInfoSource, ProcPidfdIdentityVerifier, ProcSelfFdInfoSource, ProcessCreditLimit,
    SentPacket, SeqpacketSocket, StreamRead, StreamSocket, UnixAttachmentPayload,
    UnixSeqpacketTransport, UnixSessionError, UnixStreamTransport, parse_pidfd_fdinfo,
    prearmed_seqpacket_pair,
};
use rustix::{
    fd::BorrowedFd,
    fs::fstat,
    io::{DupFlags, FdFlags, dup3, fcntl_getfd},
    net::{AddressFamily, SocketFlags, SocketType, socketpair},
    pipe::{PipeFlags, pipe, pipe_with},
    process::{PidfdFlags, getpid, getppid, pidfd_open},
};
use std::{
    any::Any,
    collections::VecDeque,
    fs,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, MutexGuard};

static FD_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn serialize_fd_test() -> MutexGuard<'static, ()> {
    FD_TEST_LOCK.lock().await
}

fn attachment_policy(max_per_packet: u16, credentials_allowed: bool) -> AttachmentPolicy {
    AttachmentPolicy {
        kind: AttachmentPolicyKind::PacketAtomic,
        max_per_packet,
        max_per_request: max_per_packet.max(1),
        max_per_operation: max_per_packet.max(1),
        max_per_session: max_per_packet.max(1),
        credentials_allowed,
    }
}

fn scopes(limit: usize) -> (CreditScopeSet, [CreditPool; 6]) {
    let pools = [
        CreditPool::new(limit).unwrap(),
        CreditPool::new(limit).unwrap(),
        CreditPool::new(limit).unwrap(),
        CreditPool::new(limit).unwrap(),
        CreditPool::new(limit).unwrap(),
        CreditPool::new(limit).unwrap(),
    ];
    (
        CreditScopeSet::new(
            pools[0].clone(),
            pools[1].clone(),
            pools[2].clone(),
            pools[3].clone(),
            pools[4].clone(),
            pools[5].clone(),
        ),
        pools,
    )
}

fn seqpacket_pair() -> (SeqpacketSocket, SeqpacketSocket) {
    let (left, right) = socketpair(
        AddressFamily::UNIX,
        SocketType::SEQPACKET,
        SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
        None,
    )
    .unwrap();
    (
        SeqpacketSocket::from_owned(left).unwrap(),
        SeqpacketSocket::from_owned(right).unwrap(),
    )
}

fn stream_pair() -> (StreamSocket, StreamSocket) {
    let (left, right) = socketpair(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
        None,
    )
    .unwrap();
    (
        StreamSocket::from_owned(left).unwrap(),
        StreamSocket::from_owned(right).unwrap(),
    )
}

fn protected_record(payload: &[u8]) -> Vec<u8> {
    let length = u16::try_from(payload.len()).unwrap();
    let mut record = Vec::with_capacity(payload.len() + 2);
    record.extend_from_slice(&length.to_be_bytes());
    record.extend_from_slice(payload);
    record
}

fn descriptor(
    index: u16,
    object_type: KernelObjectType,
    access: AttachmentAccess,
    duplicate_object_allowed: bool,
) -> AttachmentDescriptor {
    AttachmentDescriptor {
        index,
        kind: AttachmentKind::FileDescriptor,
        object_type,
        access,
        purpose: AttachmentPurpose::RequestInput,
        service: ServicePackage::DaemonV2,
        method_id: 1,
        request_id: RequestId::new([7_u8; 16]).unwrap(),
        operation_id: None,
        packet_sequence: 1,
        reconnect_generation: 1,
        duplicate_object_allowed,
        cloexec_required: true,
        credit_classes: BoundedVec::new(vec![
            AttachmentCreditClass::Packet,
            AttachmentCreditClass::Request,
            AttachmentCreditClass::Operation,
            AttachmentCreditClass::Session,
            AttachmentCreditClass::Process,
            AttachmentCreditClass::Host,
        ])
        .unwrap(),
    }
}

fn packet(descriptors: Vec<AttachmentDescriptor>) -> AttachmentPacket {
    AttachmentPacket {
        declared_count: descriptors.len() as u16,
        descriptors: BoundedVec::new(descriptors).unwrap(),
    }
}

#[test]
fn ancillary_capacity_is_derived_from_closed_hard_bounds() {
    let one = AncillaryCapacity::from_policy(attachment_policy(1, false)).unwrap();
    let maximum = AncillaryCapacity::from_policy(attachment_policy(32, true)).unwrap();
    assert_eq!(one.max_files(), 1);
    assert_eq!(maximum.max_files(), 32);
    assert!(maximum.credentials_allowed());
    assert!(maximum.bytes() > one.bytes());
    let credentials_only = AncillaryCapacity::from_policy(attachment_policy(0, true)).unwrap();
    assert_eq!(credentials_only.max_files(), 0);
    assert!(credentials_only.credentials_allowed());
    assert!(credentials_only.bytes() > 0);
    assert_eq!(
        AncillaryCapacity::from_policy(attachment_policy(33, false)),
        Err(UnixSessionError::AncillaryCapacity)
    );
    assert_eq!(
        AncillaryCapacity::from_policy(AttachmentPolicy::disabled()),
        Err(UnixSessionError::AncillaryCapacity)
    );
}

#[test]
fn process_limit_preserves_emergency_headroom() {
    assert_eq!(
        ProcessCreditLimit::derive(2_200, 100)
            .unwrap()
            .transferable(),
        2_036
    );
    assert!(ProcessCreditLimit::derive(164, 100).is_err());
    assert!(ProcessCreditLimit::from_current(0).unwrap().transferable() <= 2_048);
}

#[test]
fn failed_multiscope_reservation_rolls_back_every_prior_scope() {
    let (set, pools) = scopes(2);
    let blocker = pools[4].clone();
    let _held = CreditScopeSet::new(
        CreditPool::new(8).unwrap(),
        CreditPool::new(8).unwrap(),
        CreditPool::new(8).unwrap(),
        CreditPool::new(8).unwrap(),
        blocker.clone(),
        CreditPool::new(8).unwrap(),
    )
    .reserve(2)
    .unwrap();
    assert!(set.reserve(1).is_err());
    for pool in &pools[..4] {
        assert_eq!(pool.used(), 0);
    }
    assert_eq!(pools[5].used(), 0);
}

#[test]
fn staged_credit_reservations_release_once_at_each_scope() {
    let (set, pools) = scopes(2);
    let mut credits = set.reserve_ingress(1).unwrap();
    assert_eq!(
        pools.iter().map(CreditPool::used).collect::<Vec<_>>(),
        vec![1, 0, 0, 1, 1, 1]
    );
    credits.acquire_dispatch(&set, 1).unwrap();
    assert!(pools.iter().all(|pool| pool.used() == 1));
    credits.release(d2b_session_unix::CreditScope::Packet);
    credits.release(d2b_session_unix::CreditScope::Packet);
    assert_eq!(pools[0].used(), 0);
    assert!(pools[1..].iter().all(|pool| pool.used() == 1));
    drop(credits);
    assert!(pools.iter().all(|pool| pool.used() == 0));
}

#[tokio::test(flavor = "current_thread")]
async fn inherited_passcred_is_verified_but_never_repaired() {
    let _serial = serialize_fd_test().await;
    let (left, right) = prearmed_seqpacket_pair().unwrap();
    assert!(SeqpacketSocket::from_parent_prearmed(left).is_ok());
    assert!(SeqpacketSocket::from_parent_prearmed(right).is_ok());

    let (left, _) = socketpair(
        AddressFamily::UNIX,
        SocketType::SEQPACKET,
        SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
        None,
    )
    .unwrap();
    assert!(matches!(
        SeqpacketSocket::from_parent_prearmed(left),
        Err(UnixSessionError::PasscredNotPrearmed)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn first_packet_has_exact_directional_credentials() {
    let _serial = serialize_fd_test().await;
    let (left, right) = prearmed_seqpacket_pair().unwrap();
    let sender = SeqpacketSocket::from_parent_prearmed(left).unwrap();
    let receiver = SeqpacketSocket::from_parent_prearmed(right).unwrap();
    let expected = sender.acceptor_peer_credentials().unwrap();
    let policy = attachment_policy(1, true);
    let capacity = AncillaryCapacity::from_policy(policy).unwrap();
    let (sender_scopes, _) = scopes(64);
    let (receiver_scopes, _) = scopes(8);
    let mut queue = VecDeque::new();
    for payload in [b"preface".as_slice(), b"later".as_slice()] {
        queue.push_back(
            OutboundPacket::with_current_credentials(
                payload.to_vec(),
                Vec::new(),
                LimitProfile::local_default(),
                capacity,
                &sender_scopes,
            )
            .unwrap(),
        );
    }
    let sent = sender.send_burst(&mut queue, capacity, 8).await.unwrap();
    assert!(sent.queue_empty);

    let received = receiver
        .recv_burst(LimitProfile::local_default(), capacity, &receiver_scopes, 8)
        .await
        .unwrap();
    assert_eq!(received.packets.len(), 2);
    received.packets[0]
        .verify_first_packet_credentials(expected)
        .unwrap();
    assert_eq!(
        received.packets[1].verify_first_packet_credentials(expected),
        Err(UnixSessionError::CredentialMismatch)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn seqpacket_transfer_is_atomic_cloexec_and_object_exact() {
    let _serial = serialize_fd_test().await;
    let (sender, receiver) = seqpacket_pair();
    let (read_end, _write_end) = pipe_with(PipeFlags::CLOEXEC).unwrap();
    let expected = ObjectIdentity::from_trusted(
        &read_end,
        KernelObjectType::PipeRead,
        AttachmentAccess::ReadOnly,
    )
    .unwrap();
    let read_end = Arc::new(read_end);
    let policy = attachment_policy(1, false);
    let capacity = AncillaryCapacity::from_policy(policy).unwrap();
    let (sender_scopes, sender_pools) = scopes(4);
    let (receiver_scopes, receiver_pools) = scopes(4);
    let outbound = OutboundPacket::new(
        b"record".to_vec(),
        vec![read_end],
        None,
        LimitProfile::local_default(),
        capacity,
        &sender_scopes,
    )
    .unwrap();
    let mut queue = VecDeque::from([outbound]);
    let sent = sender.send_burst(&mut queue, capacity, 8).await.unwrap();
    assert_eq!(sent.sent.len(), 1);
    assert!(sender_pools.iter().all(|pool| pool.used() == 1));

    let mut received = receiver
        .recv_burst(LimitProfile::local_default(), capacity, &receiver_scopes, 8)
        .await
        .unwrap()
        .packets;
    let raw = received.pop().unwrap();
    assert!(receiver_pools[0].used() == 1 && receiver_pools[1].used() == 0);
    let verified = raw
        .verify(
            &packet(vec![descriptor(
                0,
                KernelObjectType::PipeRead,
                AttachmentAccess::ReadOnly,
                false,
            )]),
            policy,
            &[DescriptorPolicy::File(expected)],
            &receiver_scopes,
        )
        .unwrap();
    let d2b_session_unix::AcceptedAttachment::File(received_fd) = &verified.attachments()[0] else {
        panic!("expected file");
    };
    assert!(fcntl_getfd(received_fd).unwrap().contains(FdFlags::CLOEXEC));
    drop(verified);
    assert!(receiver_pools.iter().all(|pool| pool.used() == 0));
    drop(sent);
    assert!(sender_pools.iter().all(|pool| pool.used() == 0));
}

#[tokio::test(flavor = "current_thread")]
async fn duplicate_kernel_objects_are_rejected_and_cleaned_up() {
    let _serial = serialize_fd_test().await;
    let (sender, receiver) = seqpacket_pair();
    let (read_end, _) = pipe_with(PipeFlags::CLOEXEC).unwrap();
    let expected = ObjectIdentity::from_trusted(
        &read_end,
        KernelObjectType::PipeRead,
        AttachmentAccess::ReadOnly,
    )
    .unwrap();
    let read_end = Arc::new(read_end);
    let policy = attachment_policy(2, false);
    let capacity = AncillaryCapacity::from_policy(policy).unwrap();
    let (sender_scopes, sender_pools) = scopes(4);
    let (receiver_scopes, receiver_pools) = scopes(4);
    let outbound = OutboundPacket::new(
        b"record".to_vec(),
        vec![read_end.clone(), read_end],
        None,
        LimitProfile::local_default(),
        capacity,
        &sender_scopes,
    )
    .unwrap();
    let mut queue = VecDeque::from([outbound]);
    let sent = sender.send_burst(&mut queue, capacity, 8).await.unwrap();
    let raw = receiver
        .recv_burst(LimitProfile::local_default(), capacity, &receiver_scopes, 8)
        .await
        .unwrap()
        .packets
        .pop()
        .unwrap();
    assert!(matches!(
        raw.verify(
            &packet(vec![
                descriptor(
                    0,
                    KernelObjectType::PipeRead,
                    AttachmentAccess::ReadOnly,
                    false,
                ),
                descriptor(
                    1,
                    KernelObjectType::PipeRead,
                    AttachmentAccess::ReadOnly,
                    false,
                ),
            ]),
            policy,
            &[
                DescriptorPolicy::File(expected.clone()),
                DescriptorPolicy::File(expected),
            ],
            &receiver_scopes,
        ),
        Err(UnixSessionError::DuplicateObject)
    ));
    assert!(receiver_pools.iter().all(|pool| pool.used() == 0));
    drop(sent);
    assert!(sender_pools.iter().all(|pool| pool.used() == 0));
}

#[tokio::test(flavor = "current_thread")]
async fn owned_transport_adapters_transfer_packets_and_owned_files_end_to_end() {
    let _serial = serialize_fd_test().await;
    let (sender_socket, receiver_socket) = seqpacket_pair();
    let policy = attachment_policy(1, false);
    let (sender_scopes, sender_pools) = scopes(8);
    let (receiver_scopes, receiver_pools) = scopes(8);
    let (read_end, write_end) = pipe_with(PipeFlags::CLOEXEC).unwrap();
    let pipe_inode = fstat(&read_end).unwrap().st_ino;
    let identity = ObjectIdentity::from_trusted(
        &read_end,
        KernelObjectType::PipeRead,
        AttachmentAccess::ReadOnly,
    )
    .unwrap();
    let sender_identity = identity.clone();
    let sender_resolver = Arc::new(move |_: &AttachmentDescriptor| {
        Ok(DescriptorPolicy::File(sender_identity.clone()))
    });
    let receiver_identity = identity.clone();
    let receiver_resolver = Arc::new(move |_: &AttachmentDescriptor| {
        Ok(DescriptorPolicy::File(receiver_identity.clone()))
    });
    let sender_peer = sender_socket.acceptor_peer_credentials().unwrap();
    let receiver_peer = receiver_socket.acceptor_peer_credentials().unwrap();
    let mut sender = UnixSeqpacketTransport::new(
        sender_socket,
        d2b_contracts::v2_component_session::Locality::HostLocal,
        LimitProfile::local_default(),
        policy,
        sender_scopes,
        sender_resolver,
        PeerIdentityPolicy::accepted(sender_peer),
    )
    .unwrap();
    let mut receiver = UnixSeqpacketTransport::new(
        receiver_socket,
        d2b_contracts::v2_component_session::Locality::HostLocal,
        LimitProfile::local_default(),
        policy,
        receiver_scopes,
        receiver_resolver,
        PeerIdentityPolicy::accepted(receiver_peer),
    )
    .unwrap();

    let mut metadata = descriptor(
        0,
        KernelObjectType::PipeRead,
        AttachmentAccess::ReadOnly,
        false,
    );
    metadata.service = ServicePackage::BrokerV2;
    let attachment =
        OwnedUnixAttachment::file(metadata.clone(), read_end, DescriptorPolicy::File(identity))
            .unwrap();
    let unix_payload = attachment
        .payload()
        .and_then(|payload| payload.downcast_ref::<UnixAttachmentPayload>())
        .unwrap();
    let mut wrong_generation = metadata.clone();
    wrong_generation.reconnect_generation += 1;
    assert_eq!(
        AttachmentPayload::validate_descriptor(unix_payload, &wrong_generation),
        Err(AttachmentValidationError::Other)
    );
    sender
        .send(TransportPacket::with_attachments(
            b"protected-record".to_vec(),
            vec![attachment],
        ))
        .await
        .unwrap();
    assert!(sender_pools.iter().all(|pool| pool.used() == 0));
    assert_eq!(pipe_handle_count(pipe_inode), 1);
    let received = receiver
        .receive(LimitProfile::local_default().protected_ciphertext_bytes as usize)
        .await
        .unwrap();
    let (payload, attachments) = received.into_parts();
    assert_eq!(payload, b"protected-record");
    assert_eq!(attachments.len(), 1);
    assert!(attachments[0].descriptor().is_none());
    let unix_payload = attachments[0]
        .payload()
        .and_then(|payload| payload.downcast_ref::<UnixAttachmentPayload>())
        .unwrap();
    AttachmentPayload::validate_descriptor(unix_payload, &metadata).unwrap();
    assert!(receiver_pools.iter().all(|pool| pool.used() == 1));
    assert_eq!(pipe_handle_count(pipe_inode), 2);
    let received_read_end = unix_payload.file().unwrap();
    rustix::io::write(&write_end, b"x").unwrap();
    let mut byte = [0_u8; 1];
    assert_eq!(rustix::io::read(received_read_end, &mut byte).unwrap(), 1);
    assert_eq!(byte, *b"x");
    drop(attachments);
    assert!(receiver_pools.iter().all(|pool| pool.used() == 0));
    assert_eq!(pipe_handle_count(pipe_inode), 1);

    let foreign_closes = Arc::new(AtomicUsize::new(0));
    let foreign = OwnedAttachment::new(
        metadata.clone(),
        Box::new(CountingPayload(foreign_closes.clone())),
    );
    assert_eq!(
        sender
            .send(TransportPacket::with_attachments(
                b"rejected".to_vec(),
                vec![foreign],
            ))
            .await,
        Err(d2b_session::TransportError::InvalidAttachment)
    );
    assert_eq!(foreign_closes.load(Ordering::Acquire), 1);

    let (left, right) = socketpair(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
        None,
    )
    .unwrap();
    let mut stream_sender = UnixStreamTransport::new(
        StreamSocket::from_owned(left).unwrap(),
        d2b_contracts::v2_component_session::Locality::HostLocal,
        LimitProfile::local_default(),
    );
    let mut stream_receiver = UnixStreamTransport::new(
        StreamSocket::from_owned(right).unwrap(),
        d2b_contracts::v2_component_session::Locality::HostLocal,
        LimitProfile::local_default(),
    );
    let stream_record = protected_record(b"stream-record");
    stream_sender
        .send(TransportPacket::new(stream_record.clone()))
        .await
        .unwrap();
    let stream_packet = stream_receiver.receive(64).await.unwrap();
    assert_eq!(stream_packet.as_bytes(), stream_record);
    let (stream_read, _stream_write) = pipe_with(PipeFlags::CLOEXEC).unwrap();
    let stream_identity = ObjectIdentity::from_trusted(
        &stream_read,
        KernelObjectType::PipeRead,
        AttachmentAccess::ReadOnly,
    )
    .unwrap();
    let stream_attachment = OwnedUnixAttachment::file(
        metadata,
        stream_read,
        DescriptorPolicy::File(stream_identity),
    )
    .unwrap();
    assert_eq!(
        stream_sender
            .send(TransportPacket::with_attachments(
                b"rejected".to_vec(),
                vec![stream_attachment],
            ))
            .await,
        Err(d2b_session::TransportError::InvalidAttachment)
    );
    stream_sender.close().await.unwrap();
    stream_receiver.close().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn stream_transport_reassembles_partial_and_coalesced_records() {
    let (sender, receiver) = stream_pair();
    let first = protected_record(b"first-record");
    let second = protected_record(b"second-record");
    let first_for_sender = first.clone();
    let second_for_sender = second.clone();
    let sender_task = tokio::spawn(async move {
        sender.write_all(&first_for_sender[..1]).await.unwrap();
        tokio::task::yield_now().await;
        sender.write_all(&first_for_sender[1..4]).await.unwrap();
        tokio::task::yield_now().await;
        let mut coalesced = first_for_sender[4..].to_vec();
        coalesced.extend_from_slice(&second_for_sender);
        sender.write_all(&coalesced).await.unwrap();
    });
    let mut receiver =
        UnixStreamTransport::new(receiver, Locality::HostLocal, LimitProfile::local_default());
    assert_eq!(receiver.receive(64).await.unwrap().as_bytes(), first);
    assert_eq!(receiver.receive(64).await.unwrap().as_bytes(), second);
    sender_task.await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn stream_transport_distinguishes_clean_and_partial_eof() {
    let (sender, receiver) = stream_pair();
    sender.close().unwrap();
    let mut receiver =
        UnixStreamTransport::new(receiver, Locality::HostLocal, LimitProfile::local_default());
    assert!(matches!(
        receiver.receive(64).await,
        Err(d2b_session::TransportError::Disconnected)
    ));

    let (sender, receiver) = stream_pair();
    sender.write_all(&[0]).await.unwrap();
    sender.close().unwrap();
    let mut receiver =
        UnixStreamTransport::new(receiver, Locality::HostLocal, LimitProfile::local_default());
    assert!(matches!(
        receiver.receive(64).await,
        Err(d2b_session::TransportError::Truncated)
    ));

    let (sender, receiver) = stream_pair();
    sender.write_all(&[0, 3, 1]).await.unwrap();
    sender.close().unwrap();
    let mut receiver =
        UnixStreamTransport::new(receiver, Locality::HostLocal, LimitProfile::local_default());
    assert!(matches!(
        receiver.receive(64).await,
        Err(d2b_session::TransportError::Truncated)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn stream_socket_zero_byte_read_is_exact_graceful_eof() {
    let (sender, receiver) = stream_pair();
    sender.close().unwrap();
    let mut output = Vec::new();
    for _ in 0..2 {
        let read = tokio::time::timeout(
            Duration::from_secs(1),
            receiver.read_available(&mut output, 16, 8),
        )
        .await
        .expect("EOF read must terminate")
        .unwrap();
        assert_eq!(
            read,
            StreamRead {
                bytes: 0,
                eof: true,
                drained_to_would_block: false,
            }
        );
        assert!(output.is_empty());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn seqpacket_zero_byte_read_is_a_clean_disconnect() {
    let (sender, receiver) = seqpacket_pair();
    sender.close().unwrap();
    let (credit_scopes, _pools) = scopes(1);
    let capacity = AncillaryCapacity::from_policy(attachment_policy(1, false)).unwrap();
    assert_eq!(
        receiver
            .recv_burst(LimitProfile::local_default(), capacity, &credit_scopes, 1,)
            .await
            .unwrap_err(),
        UnixSessionError::Closed
    );
}

#[tokio::test(flavor = "current_thread")]
async fn stream_transport_rejects_oversize_and_incomplete_records() {
    let mut limits = LimitProfile::local_default();
    limits.protected_ciphertext_bytes = 4;

    let (sender, receiver) = stream_pair();
    sender.write_all(&5_u16.to_be_bytes()).await.unwrap();
    let mut receiver = UnixStreamTransport::new(receiver, Locality::HostLocal, limits);
    assert!(matches!(
        receiver.receive(6).await,
        Err(d2b_session::TransportError::LimitExceeded)
    ));

    let (sender, _receiver) = stream_pair();
    let mut sender = UnixStreamTransport::new(sender, Locality::HostLocal, limits);
    assert_eq!(
        sender
            .send(TransportPacket::new(protected_record(&[0; 5])))
            .await,
        Err(d2b_session::TransportError::LimitExceeded)
    );
    assert_eq!(
        sender.send(TransportPacket::new(vec![0, 2, 1])).await,
        Err(d2b_session::TransportError::Truncated)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn inherited_transport_consumes_stable_credentials_as_identity_evidence() {
    let _serial = serialize_fd_test().await;
    let (left, right) = prearmed_seqpacket_pair().unwrap();
    let sender_socket = SeqpacketSocket::from_parent_prearmed(left).unwrap();
    let receiver_socket = SeqpacketSocket::from_parent_prearmed(right).unwrap();
    let sender_peer = sender_socket.acceptor_peer_credentials().unwrap();
    let receiver_peer = receiver_socket.acceptor_peer_credentials().unwrap();
    let attachment_policy = attachment_policy(1, false);
    let (sender_scopes, sender_pools) = scopes(8);
    let (receiver_scopes, receiver_pools) = scopes(8);
    let resolver =
        Arc::new(move |_: &AttachmentDescriptor| Err(UnixSessionError::DescriptorMismatch));
    let mut sender = UnixSeqpacketTransport::new(
        sender_socket,
        Locality::HostLocal,
        LimitProfile::local_default(),
        attachment_policy,
        sender_scopes,
        resolver.clone(),
        PeerIdentityPolicy::inherited_socketpair(sender_peer),
    )
    .unwrap();
    let mut receiver = UnixSeqpacketTransport::new(
        receiver_socket,
        Locality::HostLocal,
        LimitProfile::local_default(),
        attachment_policy,
        receiver_scopes,
        resolver,
        PeerIdentityPolicy::inherited_socketpair(receiver_peer),
    )
    .unwrap();
    for payload in [b"preface".as_slice(), b"subsequent".as_slice()] {
        sender
            .send(TransportPacket::new(payload.to_vec()))
            .await
            .unwrap();
        let received = receiver
            .receive(LimitProfile::local_default().protected_ciphertext_bytes as usize)
            .await
            .unwrap();
        let (actual, attachments) = received.into_parts();
        assert_eq!(actual, payload);
        assert!(attachments.is_empty());
    }
    assert!(sender_pools.iter().all(|pool| pool.used() == 0));
    assert!(receiver_pools.iter().all(|pool| pool.used() == 0));
}

#[tokio::test(flavor = "current_thread")]
async fn inherited_transport_handshakes_with_zero_semantic_fd_capacity() {
    let _serial = serialize_fd_test().await;
    let (left, right) = prearmed_seqpacket_pair().unwrap();
    let sender_socket = SeqpacketSocket::from_parent_prearmed(left).unwrap();
    let receiver_socket = SeqpacketSocket::from_parent_prearmed(right).unwrap();
    let sender_peer = sender_socket.acceptor_peer_credentials().unwrap();
    let receiver_peer = receiver_socket.acceptor_peer_credentials().unwrap();
    let (sender_scopes, sender_pools) = scopes(1);
    let (receiver_scopes, receiver_pools) = scopes(1);
    let resolver =
        Arc::new(move |_: &AttachmentDescriptor| Err(UnixSessionError::DescriptorMismatch));
    let mut sender = UnixSeqpacketTransport::new(
        sender_socket,
        Locality::HostLocal,
        LimitProfile::local_default(),
        AttachmentPolicy::disabled(),
        sender_scopes,
        resolver.clone(),
        PeerIdentityPolicy::inherited_socketpair(sender_peer),
    )
    .unwrap();
    let mut receiver = UnixSeqpacketTransport::new(
        receiver_socket,
        Locality::HostLocal,
        LimitProfile::local_default(),
        AttachmentPolicy::disabled(),
        receiver_scopes,
        resolver,
        PeerIdentityPolicy::inherited_socketpair(receiver_peer),
    )
    .unwrap();

    sender
        .send(TransportPacket::new(b"preface".to_vec()))
        .await
        .unwrap();
    let received = receiver.receive(64).await.unwrap();
    let (bytes, attachments) = received.into_parts();
    assert_eq!(bytes, b"preface");
    assert!(attachments.is_empty());
    assert!(sender_pools.iter().all(|pool| pool.used() == 0));
    assert!(receiver_pools.iter().all(|pool| pool.used() == 0));
}

#[tokio::test(flavor = "current_thread")]
async fn pathname_transport_verifies_provenance_and_accepts_attachment_free_preface() {
    let _serial = serialize_fd_test().await;
    let (sender, receiver_socket) = seqpacket_pair();
    let expected = receiver_socket.acceptor_peer_credentials().unwrap();
    let verified = Arc::new(AtomicBool::new(false));
    let verifier_called = verified.clone();
    let verifier = Arc::new(move |socket: &SeqpacketSocket| {
        if socket.acceptor_peer_credentials()? != expected {
            return Err(UnixSessionError::CredentialMismatch);
        }
        verifier_called.store(true, Ordering::SeqCst);
        Ok(())
    });
    let policy = attachment_policy(1, false);
    let capacity = AncillaryCapacity::from_policy(policy).unwrap();
    let (sender_scopes, _) = scopes(8);
    let (receiver_scopes, receiver_pools) = scopes(8);
    let resolver =
        Arc::new(move |_: &AttachmentDescriptor| Err(UnixSessionError::DescriptorMismatch));
    let mut receiver = UnixSeqpacketTransport::new(
        receiver_socket,
        Locality::HostLocal,
        LimitProfile::local_default(),
        policy,
        receiver_scopes,
        resolver,
        PeerIdentityPolicy::pathname(verifier),
    )
    .unwrap();
    assert!(verified.load(Ordering::SeqCst));

    let outbound = OutboundPacket::new(
        b"preface".to_vec(),
        Vec::new(),
        None,
        LimitProfile::local_default(),
        capacity,
        &sender_scopes,
    )
    .unwrap();
    let mut queue = VecDeque::from([outbound]);
    sender.send_burst(&mut queue, capacity, 8).await.unwrap();
    let received = receiver
        .receive(LimitProfile::local_default().protected_ciphertext_bytes as usize)
        .await
        .unwrap();
    let (bytes, attachments) = received.into_parts();
    assert_eq!(bytes, b"preface");
    assert!(attachments.is_empty());
    assert!(receiver_pools.iter().all(|pool| pool.used() == 0));
}

#[tokio::test(flavor = "current_thread")]
async fn unix_semantic_credential_descriptor_is_rejected_and_closed() {
    let _serial = serialize_fd_test().await;
    let (read_end, write_end) = pipe_with(PipeFlags::CLOEXEC).unwrap();
    let inode = fstat(&read_end).unwrap().st_ino;
    let identity = ObjectIdentity::from_trusted(
        &read_end,
        KernelObjectType::PipeRead,
        AttachmentAccess::ReadOnly,
    )
    .unwrap();
    let mut metadata = descriptor(
        0,
        KernelObjectType::PipeRead,
        AttachmentAccess::ReadOnly,
        false,
    );
    metadata.kind = AttachmentKind::Credentials;
    assert!(matches!(
        OwnedUnixAttachment::file(metadata, read_end, DescriptorPolicy::File(identity)),
        Err(UnixSessionError::DescriptorMismatch)
    ));
    assert_eq!(pipe_handle_count(inode), 1);
    drop(write_end);
}

#[tokio::test(flavor = "current_thread")]
async fn session_engine_transfers_and_binds_seqpacket_attachments_end_to_end() {
    let _serial = serialize_fd_test().await;
    let (sender_socket, receiver_socket) = seqpacket_pair();
    let sender_peer = sender_socket.acceptor_peer_credentials().unwrap();
    let receiver_peer = receiver_socket.acceptor_peer_credentials().unwrap();
    let attachment_policy = attachment_policy(1, false);
    let (sender_scopes, sender_pools) = scopes(8);
    let (receiver_scopes, receiver_pools) = scopes(8);
    let (read_end, write_end) = pipe_with(PipeFlags::CLOEXEC).unwrap();
    let pipe_inode = fstat(&read_end).unwrap().st_ino;
    let identity = ObjectIdentity::from_trusted(
        &read_end,
        KernelObjectType::PipeRead,
        AttachmentAccess::ReadOnly,
    )
    .unwrap();
    let sender_identity = identity.clone();
    let sender_resolver = Arc::new(move |_: &AttachmentDescriptor| {
        Ok(DescriptorPolicy::File(sender_identity.clone()))
    });
    let receiver_identity = identity.clone();
    let receiver_resolver = Arc::new(move |_: &AttachmentDescriptor| {
        Ok(DescriptorPolicy::File(receiver_identity.clone()))
    });
    let sender = UnixSeqpacketTransport::new(
        sender_socket,
        Locality::HostLocal,
        LimitProfile::local_default(),
        attachment_policy,
        sender_scopes,
        sender_resolver,
        PeerIdentityPolicy::accepted(sender_peer),
    )
    .unwrap();
    let receiver = UnixSeqpacketTransport::new(
        receiver_socket,
        Locality::HostLocal,
        LimitProfile::local_default(),
        attachment_policy,
        receiver_scopes,
        receiver_resolver,
        PeerIdentityPolicy::accepted(receiver_peer),
    )
    .unwrap();
    let endpoint = EndpointPolicy {
        purpose: EndpointPurpose::PrivilegedBroker,
        purpose_class: PurposeClass::Local,
        initiator_role: EndpointRole::LocalRootController,
        responder_role: EndpointRole::LocalRootBroker,
        service: ServicePackage::BrokerV2,
        schema_fingerprint: [0x11; 32],
        noise_profile: NoiseProfile::Nn25519ChaChaPolySha256,
        limits: LimitProfile::local_default(),
        transport_binding: TransportBinding {
            transport: TransportClass::UnixSeqpacket,
            locality: Locality::HostLocal,
            channel_binding: [0x22; 32],
            identity_evidence: IdentityEvidenceRequirement::DirectionalUnix,
        },
        reconnect_generation: 7,
        attachment_policy,
    };
    let now = Instant::now();
    let endpoint_identity = EndpointPolicyIdentity::from(&endpoint);
    let (initiator, responder) = tokio::join!(
        SessionEngine::establish_initiator_with_generation_discovery(
            sender,
            endpoint_identity,
            HandshakeCredentials::Nn,
            now,
        ),
        SessionEngine::establish_responder(receiver, endpoint, HandshakeCredentials::Nn, now,),
    );
    let mut initiator = initiator.unwrap();
    let mut responder = responder.unwrap();
    let mut metadata = descriptor(
        0,
        KernelObjectType::PipeRead,
        AttachmentAccess::ReadOnly,
        false,
    );
    metadata.service = ServicePackage::BrokerV2;
    metadata.reconnect_generation = 7;
    let attachment =
        OwnedUnixAttachment::file(metadata, read_end, DescriptorPolicy::File(identity)).unwrap();
    initiator.send_attachments(vec![attachment]).await.unwrap();
    assert!(sender_pools.iter().all(|pool| pool.used() == 0));
    assert_eq!(pipe_handle_count(pipe_inode), 1);

    let attachments = match responder.receive().await.unwrap() {
        SessionEvent::Attachments(attachments) => attachments,
        event => panic!("unexpected event {event:?}"),
    };
    assert_eq!(attachments.len(), 1);
    assert!(attachments[0].descriptor().is_some());
    assert!(receiver_pools.iter().all(|pool| pool.used() == 1));
    let unix_payload = attachments[0]
        .payload()
        .and_then(|payload| payload.downcast_ref::<UnixAttachmentPayload>())
        .unwrap();
    let received_read_end = unix_payload.file().unwrap();
    rustix::io::write(&write_end, b"y").unwrap();
    let mut byte = [0_u8; 1];
    assert_eq!(rustix::io::read(received_read_end, &mut byte).unwrap(), 1);
    assert_eq!(byte, *b"y");
    drop(attachments);
    assert!(receiver_pools.iter().all(|pool| pool.used() == 0));
    assert_eq!(pipe_handle_count(pipe_inode), 1);
    assert!(matches!(
        initiator.receive().await.unwrap(),
        SessionEvent::AttachmentAcknowledged { count: 1 }
    ));
}

struct CountingPayload(Arc<AtomicUsize>);

impl AttachmentPayload for CountingPayload {
    fn close(self: Box<Self>) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }

    fn validate_descriptor(
        &self,
        _descriptor: &AttachmentDescriptor,
    ) -> Result<(), AttachmentValidationError> {
        Ok(())
    }
}

#[tokio::test(flavor = "current_thread")]
async fn pidfd_identity_requires_live_launch_evidence_and_rejects_unrelated_process() {
    let _serial = serialize_fd_test().await;
    let (left, right) = prearmed_seqpacket_pair().unwrap();
    let left = SeqpacketSocket::from_parent_prearmed(left).unwrap();
    let right = SeqpacketSocket::from_parent_prearmed(right).unwrap();
    let credentials = right.acceptor_peer_credentials().unwrap();
    let policy = attachment_policy(1, true);
    let capacity = AncillaryCapacity::from_policy(policy).unwrap();
    let (sender_scopes, _) = scopes(8);
    let (receiver_scopes, _) = scopes(8);
    let outbound = OutboundPacket::new(
        b"preface".to_vec(),
        Vec::new(),
        Some(credentials),
        LimitProfile::local_default(),
        capacity,
        &sender_scopes,
    )
    .unwrap();
    let mut queue = VecDeque::from([outbound]);
    let sent = left.send_burst(&mut queue, capacity, 1).await.unwrap();
    let mut packets = right
        .recv_burst(LimitProfile::local_default(), capacity, &receiver_scopes, 1)
        .await
        .unwrap()
        .packets;
    let first_packet_credentials = packets
        .pop()
        .unwrap()
        .verify_first_packet_credentials(credentials)
        .unwrap();
    sent.sent.into_iter().for_each(SentPacket::acknowledge);
    let expected_pid = getpid();
    let executable_digest = [0x31; 32];
    let cgroup_digest = [0x42; 32];
    let evidence = PidfdEvidence::new(
        expected_pid,
        first_packet_credentials,
        executable_digest,
        cgroup_digest,
    )
    .unwrap();
    let verifier = Arc::new(ProcPidfdIdentityVerifier::new(
        ProcSelfFdInfoSource,
        Arc::new(move |_| Ok(executable_digest)),
        Arc::new(move |_| Ok(cgroup_digest)),
    ));
    let own_pidfd = pidfd_open(expected_pid, PidfdFlags::empty()).unwrap();
    assert!(
        PidfdIdentityPolicy::new(
            &own_pidfd,
            AttachmentAccess::ReadWrite,
            evidence,
            verifier.clone(),
        )
        .is_ok()
    );
    let injected_verifier = Arc::new(ProcPidfdIdentityVerifier::new(
        FixedPidfdInfo {
            contents: format!("Pid:\t{}\n", expected_pid.as_raw_nonzero()),
        },
        Arc::new(move |_| Ok(executable_digest)),
        Arc::new(move |_| Ok(cgroup_digest)),
    ));
    assert!(
        PidfdIdentityPolicy::new(
            &own_pidfd,
            AttachmentAccess::ReadWrite,
            evidence,
            injected_verifier,
        )
        .is_ok()
    );
    let recycled_verifier = Arc::new(ProcPidfdIdentityVerifier::new(
        SequencePidfdInfo {
            contents: std::sync::Mutex::new(VecDeque::from([
                format!("Pid:\t{}\n", expected_pid.as_raw_nonzero()),
                "Pid:\t-1\n".to_owned(),
            ])),
        },
        Arc::new(move |_| Ok(executable_digest)),
        Arc::new(move |_| Ok(cgroup_digest)),
    ));
    assert!(matches!(
        PidfdIdentityPolicy::new(
            &own_pidfd,
            AttachmentAccess::ReadWrite,
            evidence,
            recycled_verifier,
        ),
        Err(UnixSessionError::PidfdEvidenceUnavailable)
    ));
    let digest_mismatch_verifier = Arc::new(ProcPidfdIdentityVerifier::new(
        FixedPidfdInfo {
            contents: format!("Pid:\t{}\n", expected_pid.as_raw_nonzero()),
        },
        Arc::new(|_| Ok([0x99; 32])),
        Arc::new(move |_| Ok(cgroup_digest)),
    ));
    assert!(matches!(
        PidfdIdentityPolicy::new(
            &own_pidfd,
            AttachmentAccess::ReadWrite,
            evidence,
            digest_mismatch_verifier,
        ),
        Err(UnixSessionError::PidfdIdentityMismatch)
    ));
    assert_eq!(
        ObjectIdentity::from_trusted(
            &own_pidfd,
            KernelObjectType::Pidfd,
            AttachmentAccess::ReadWrite,
        ),
        Err(UnixSessionError::PidfdEvidenceUnavailable)
    );

    let parent = getppid().expect("test process has a parent");
    let unrelated = pidfd_open(parent, PidfdFlags::empty()).unwrap();
    assert!(matches!(
        PidfdIdentityPolicy::new(&unrelated, AttachmentAccess::ReadWrite, evidence, verifier,),
        Err(UnixSessionError::PidfdIdentityMismatch)
    ));
}

struct FixedPidfdInfo {
    contents: String,
}

impl PidfdInfoSource for FixedPidfdInfo {
    fn read_fdinfo(&self, _pidfd: BorrowedFd<'_>) -> Result<String, UnixSessionError> {
        Ok(self.contents.clone())
    }
}

struct SequencePidfdInfo {
    contents: std::sync::Mutex<VecDeque<String>>,
}

impl PidfdInfoSource for SequencePidfdInfo {
    fn read_fdinfo(&self, _pidfd: BorrowedFd<'_>) -> Result<String, UnixSessionError> {
        self.contents
            .lock()
            .expect("pidfd sequence lock")
            .pop_front()
            .ok_or(UnixSessionError::PidfdEvidenceUnavailable)
    }
}

#[test]
fn pidfd_fdinfo_parser_is_strict_and_redacted_errors_are_stable() {
    let pid = getpid();
    let input = format!(
        "pos:\t0\nflags:\t02000002\nPid:\t{}\nNSpid:\t1\n",
        pid.as_raw_nonzero()
    );
    assert_eq!(parse_pidfd_fdinfo(&input).unwrap(), pid);
    assert_eq!(
        parse_pidfd_fdinfo("Pid:\t7\nPid:\t8\n"),
        Err(UnixSessionError::PidfdEvidenceUnavailable)
    );
    assert_eq!(
        parse_pidfd_fdinfo("Pid:\t-1\n"),
        Err(UnixSessionError::PidfdEvidenceUnavailable)
    );
    assert_eq!(
        format!("{:?}", UnixSessionError::PidfdIdentityMismatch),
        "unix-session-pidfd-identity-mismatch"
    );
}

#[test]
fn io_errors_retain_only_the_actionable_errno() {
    assert_eq!(
        UnixSessionError::Io { errno: Some(2) }.to_string(),
        "unix-session-io(errno=2)"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn payload_and_control_truncation_scavenge_received_files() {
    let _serial = serialize_fd_test().await;
    {
        let (sender, receiver) = seqpacket_pair();
        let policy = attachment_policy(1, false);
        let capacity = AncillaryCapacity::from_policy(policy).unwrap();
        let (sender_scopes, _) = scopes(64);
        let (receiver_scopes, _) = scopes(8);
        let outbound = OutboundPacket::new(
            vec![9_u8; 128],
            Vec::new(),
            None,
            LimitProfile::local_default(),
            capacity,
            &sender_scopes,
        )
        .unwrap();
        let mut queue = VecDeque::from([outbound]);
        let _sent = sender.send_burst(&mut queue, capacity, 8).await.unwrap();
        let mut tiny = LimitProfile::local_default();
        tiny.protected_ciphertext_bytes = 8;
        assert!(matches!(
            receiver
                .recv_burst(tiny, capacity, &receiver_scopes, 8)
                .await,
            Err(UnixSessionError::MessageTruncated)
        ));
    }
    {
        let (sender, receiver) = seqpacket_pair();
        let sender_policy = attachment_policy(32, false);
        let receiver_policy = attachment_policy(1, false);
        let sender_capacity = AncillaryCapacity::from_policy(sender_policy).unwrap();
        let receiver_capacity = AncillaryCapacity::from_policy(receiver_policy).unwrap();
        let (one, _) = pipe_with(PipeFlags::CLOEXEC).unwrap();
        let one = Arc::new(one);
        let pipe_inode = fstat(&one).unwrap().st_ino;
        let (sender_scopes, _) = scopes(64);
        let (receiver_scopes, _) = scopes(8);
        let outbound = OutboundPacket::new(
            b"record".to_vec(),
            (0..32).map(|_| one.clone()).collect(),
            None,
            LimitProfile::local_default(),
            sender_capacity,
            &sender_scopes,
        )
        .unwrap();
        let mut queue = VecDeque::from([outbound]);
        let _sent = sender
            .send_burst(&mut queue, sender_capacity, 8)
            .await
            .unwrap();
        let before_receive = pipe_handle_count(pipe_inode);
        assert!(matches!(
            receiver
                .recv_burst(
                    LimitProfile::local_default(),
                    receiver_capacity,
                    &receiver_scopes,
                    8,
                )
                .await,
            Err(UnixSessionError::ControlTruncated)
        ));
        assert_eq!(pipe_handle_count(pipe_inode), before_receive);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn readiness_drains_bursts_and_preserves_cached_continuation() {
    let _serial = serialize_fd_test().await;
    let (left, right) = socketpair(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
        None,
    )
    .unwrap();
    let writer = StreamSocket::from_owned(left).unwrap();
    let reader = StreamSocket::from_owned(right).unwrap();
    writer.write_all(b"abcdefgh").await.unwrap();
    let mut output = Vec::new();
    let first = reader.read_available(&mut output, 1, 3).await.unwrap();
    assert_eq!(first.bytes, 3);
    assert!(!first.drained_to_would_block);
    let second = reader.read_available(&mut output, 1, 16).await.unwrap();
    assert_eq!(second.bytes, 5);
    assert!(second.drained_to_would_block);
    assert_eq!(output, b"abcdefgh");
}

#[tokio::test(flavor = "current_thread")]
async fn fd_reuse_does_not_defeat_object_identity() {
    let _serial = serialize_fd_test().await;
    let (mut first, _first_writer) = pipe_with(PipeFlags::CLOEXEC).unwrap();
    let identity = ObjectIdentity::from_trusted(
        &first,
        KernelObjectType::PipeRead,
        AttachmentAccess::ReadOnly,
    )
    .unwrap();
    let (replacement, _replacement_writer) = pipe_with(PipeFlags::CLOEXEC).unwrap();
    dup3(&replacement, &mut first, DupFlags::CLOEXEC).unwrap();
    let replacement_identity = ObjectIdentity::from_trusted(
        &first,
        KernelObjectType::PipeRead,
        AttachmentAccess::ReadOnly,
    )
    .unwrap();
    assert_ne!(identity, replacement_identity);

    let (not_cloexec, _) = pipe().unwrap();
    assert_eq!(
        ObjectIdentity::from_trusted(
            &not_cloexec,
            KernelObjectType::PipeRead,
            AttachmentAccess::ReadOnly,
        ),
        Err(UnixSessionError::MissingCloexec)
    );
    assert_eq!(
        ObjectIdentity::from_trusted(
            &replacement,
            KernelObjectType::PipeWrite,
            AttachmentAccess::WriteOnly,
        ),
        Err(UnixSessionError::DescriptorMismatch)
    );
}

fn pipe_handle_count(inode: u64) -> usize {
    let expected = format!("pipe:[{inode}]");
    fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| fs::read_link(entry.path()).ok())
        .filter(|target| target.to_string_lossy() == expected)
        .count()
}
