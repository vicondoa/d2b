use d2b_contracts::v2_component_session::{
    AttachmentAccess, AttachmentCreditClass, AttachmentDescriptor, AttachmentKind,
    AttachmentPacket, AttachmentPolicy, AttachmentPolicyKind, AttachmentPurpose, BoundedVec,
    KernelObjectType, LimitProfile, RequestId, ServicePackage,
};
use d2b_unix_session::{
    AncillaryCapacity, CreditPool, CreditScopeSet, DescriptorPolicy, ObjectIdentity,
    OutboundPacket, ProcessCreditLimit, SeqpacketSocket, StreamSocket, UnixSessionError,
    prearmed_seqpacket_pair,
};
use rustix::{
    fs::fstat,
    io::{DupFlags, FdFlags, dup3, fcntl_getfd},
    net::{AddressFamily, SocketFlags, SocketType, socketpair},
    pipe::{PipeFlags, pipe, pipe_with},
};
use std::{
    collections::VecDeque,
    fs,
    sync::{Arc, LazyLock},
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
    credits.release(d2b_unix_session::CreditScope::Packet);
    credits.release(d2b_unix_session::CreditScope::Packet);
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
    let outbound = OutboundPacket::with_current_credentials(
        b"preface".to_vec(),
        Vec::new(),
        LimitProfile::local_default(),
        capacity,
        &sender_scopes,
    )
    .unwrap();
    let mut queue = VecDeque::from([outbound]);
    let sent = sender.send_burst(&mut queue, capacity, 8).await.unwrap();
    assert!(sent.queue_empty);

    let received = receiver
        .recv_burst(LimitProfile::local_default(), capacity, &receiver_scopes, 8)
        .await
        .unwrap();
    assert_eq!(received.packets.len(), 1);
    received.packets[0]
        .verify_first_packet_credentials(expected)
        .unwrap();
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
    let d2b_unix_session::AcceptedAttachment::File(received_fd) = &verified.attachments()[0] else {
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
