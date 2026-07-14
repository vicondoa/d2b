use std::{
    any::Any,
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use d2b_contracts::v2_component_session::{
    AttachmentAccess, AttachmentCreditClass, AttachmentDescriptor, AttachmentKind,
    AttachmentPolicy, AttachmentPurpose, BootstrapPskBinding, BoundedVec, CancelAck, CancelRequest,
    CancelResult, ChannelId, CloseReason, EndpointPolicy, EndpointPurpose, EndpointRole,
    HandshakeOffer, IdentityEvidenceRequirement, KernelObjectType, LimitProfile, Locality,
    MAX_LOGICAL_MESSAGE_BYTES, MAX_REQUEST_LIFETIME_MS, MetricLabels, MetricReason, MetricResult,
    NoiseProfile, OperationId, ProviderTypeLabel, PurposeClass, RecordKind, Remediation,
    RequestEnvelope, RequestId, ServicePackage, SessionErrorCode, TransportBinding, TransportClass,
};
use d2b_session::{
    AttachmentPayload, BootstrapAdmission, BootstrapPsk, ComponentSessionDriver, DeadlineBudget,
    FairScheduler, Fragmenter, HandshakeCredentials, HandshakeRole, KeepaliveAction, MetricEvent,
    MetricsSink, NamedStreamMux, NoiseHandshake, OutboundFrame, OwnedAttachment, OwnedTransport,
    QueueClass, Reassembler, RecordProtector, Secret32, SessionEngine, SessionEvent,
    SessionLifecycle, StreamEvent, StreamId, StreamPhase, TransportDescriptor, TransportError,
    TransportPacket, negotiate_offer,
};
use snow::{
    params::DHChoice,
    resolvers::{CryptoResolver, DefaultResolver},
};
use tokio::sync::mpsc;

fn offer(profile: NoiseProfile) -> HandshakeOffer {
    let (purpose, class, transport, locality, evidence, initiator, responder, service) =
        match profile {
            NoiseProfile::Nn25519ChaChaPolySha256 => (
                EndpointPurpose::PrivilegedBroker,
                PurposeClass::Local,
                TransportClass::UnixSeqpacket,
                Locality::HostLocal,
                IdentityEvidenceRequirement::DirectionalUnix,
                EndpointRole::LocalRootController,
                EndpointRole::LocalRootBroker,
                ServicePackage::BrokerV2,
            ),
            NoiseProfile::Kk25519ChaChaPolySha256 => (
                EndpointPurpose::RealmPeer,
                PurposeClass::Enrolled,
                TransportClass::ProviderStream,
                Locality::Remote,
                IdentityEvidenceRequirement::EnrolledStaticKeys,
                EndpointRole::RealmController,
                EndpointRole::RemotePeer,
                ServicePackage::RealmV2,
            ),
            NoiseProfile::Ikpsk2_25519ChaChaPolySha256 => (
                EndpointPurpose::GuestBootstrap,
                PurposeClass::Bootstrap,
                TransportClass::NativeVsock,
                Locality::GuestLocal,
                IdentityEvidenceRequirement::ParentStaticAndSingleUsePsk,
                EndpointRole::RealmController,
                EndpointRole::GuestAgent,
                ServicePackage::GuestV2,
            ),
        };
    HandshakeOffer {
        purpose,
        purpose_class: class,
        initiator_role: initiator,
        responder_role: responder,
        service,
        schema_fingerprint: [0x11; 32],
        noise_profile: profile,
        limits: LimitProfile::local_default(),
        transport_binding: TransportBinding {
            transport,
            locality,
            channel_binding: [0x22; 32],
            identity_evidence: evidence,
        },
        reconnect_generation: 7,
        attachment_policy: if transport == TransportClass::UnixSeqpacket {
            AttachmentPolicy {
                kind: d2b_session::contract::AttachmentPolicyKind::PacketAtomic,
                max_per_packet: 1,
                max_per_request: 1,
                max_per_operation: 1,
                max_per_session: 1,
                credentials_allowed: true,
            }
        } else {
            AttachmentPolicy::disabled()
        },
    }
}

fn policy(offer: &HandshakeOffer) -> EndpointPolicy {
    EndpointPolicy {
        purpose: offer.purpose,
        purpose_class: offer.purpose_class,
        initiator_role: offer.initiator_role,
        responder_role: offer.responder_role,
        service: offer.service,
        schema_fingerprint: offer.schema_fingerprint,
        noise_profile: offer.noise_profile,
        limits: offer.limits,
        transport_binding: offer.transport_binding,
        reconnect_generation: offer.reconnect_generation,
        attachment_policy: offer.attachment_policy,
    }
}

fn negotiated(offer: &HandshakeOffer) -> d2b_session::NegotiatedOffer {
    let encoded = offer.encode_canonical().unwrap();
    let preface = d2b_session::contract::ComponentSessionPreface::new(encoded.len())
        .unwrap()
        .encode();
    negotiate_offer(&preface, &encoded, &policy(offer)).unwrap()
}

fn public(private: &[u8; 32]) -> [u8; 32] {
    let mut dh = DefaultResolver.resolve_dh(&DHChoice::Curve25519).unwrap();
    dh.set(private);
    dh.pubkey().try_into().unwrap()
}

fn credentials(profile: NoiseProfile) -> (HandshakeCredentials, HandshakeCredentials) {
    match profile {
        NoiseProfile::Nn25519ChaChaPolySha256 => {
            (HandshakeCredentials::Nn, HandshakeCredentials::Nn)
        }
        NoiseProfile::Kk25519ChaChaPolySha256 => {
            let initiator = [0x31; 32];
            let responder = [0x42; 32];
            (
                HandshakeCredentials::Kk {
                    local_private: Secret32::new(initiator).unwrap(),
                    remote_public: public(&responder),
                },
                HandshakeCredentials::Kk {
                    local_private: Secret32::new(responder).unwrap(),
                    remote_public: public(&initiator),
                },
            )
        }
        NoiseProfile::Ikpsk2_25519ChaChaPolySha256 => {
            let initiator = [0x31; 32];
            let responder = [0x42; 32];
            let admitted = || {
                let operation = OperationId::new(vec![0x66; 16]).unwrap();
                let nonce = [0x77; 32];
                let mut admission = BootstrapAdmission::new(
                    BootstrapPskBinding {
                        operation_id: operation.clone(),
                        replay_nonce: nonce,
                        expires_at_unix_ms: 2,
                    },
                    BootstrapPsk::new([0x55; 32]).unwrap(),
                )
                .unwrap();
                admission.consume(&operation, &nonce, 1).unwrap()
            };
            (
                HandshakeCredentials::IkPsk2Initiator {
                    local_private: Secret32::new(initiator).unwrap(),
                    remote_public: public(&responder),
                    psk: admitted(),
                },
                HandshakeCredentials::IkPsk2Responder {
                    local_private: Secret32::new(responder).unwrap(),
                    psk: admitted(),
                },
            )
        }
    }
}

fn establish(
    profile: NoiseProfile,
) -> (
    d2b_session::EstablishedHandshake,
    d2b_session::EstablishedHandshake,
) {
    let offer = offer(profile);
    let negotiated = negotiated(&offer);
    let (initiator_credentials, responder_credentials) = credentials(profile);
    let mut initiator =
        NoiseHandshake::new(HandshakeRole::Initiator, &negotiated, initiator_credentials).unwrap();
    let mut responder =
        NoiseHandshake::new(HandshakeRole::Responder, &negotiated, responder_credentials).unwrap();
    let message = initiator.write_next().unwrap();
    responder.read_next(&message).unwrap();
    let message = responder.write_next().unwrap();
    initiator.read_next(&message).unwrap();
    let initiator = initiator.finish().unwrap();
    let responder = responder.finish().unwrap();
    assert_eq!(initiator.transcript_hash(), responder.transcript_hash());
    (initiator, responder)
}

#[test]
fn fixed_negotiation_and_all_noise_profiles_are_strict() {
    for profile in NoiseProfile::ALL {
        establish(*profile);
    }

    let original = offer(NoiseProfile::Nn25519ChaChaPolySha256);
    let encoded = original.encode_canonical().unwrap();
    let mut preface = d2b_session::contract::ComponentSessionPreface::new(encoded.len())
        .unwrap()
        .encode();
    preface[0] ^= 1;
    assert_eq!(
        negotiate_offer(&preface, &encoded, &policy(&original))
            .unwrap_err()
            .code(),
        SessionErrorCode::MalformedPreface
    );

    let mut expected = policy(&original);
    expected.schema_fingerprint[0] ^= 1;
    let preface = d2b_session::contract::ComponentSessionPreface::new(encoded.len())
        .unwrap()
        .encode();
    assert_eq!(
        negotiate_offer(&preface, &encoded, &expected)
            .unwrap_err()
            .code(),
        SessionErrorCode::SchemaMismatch
    );

    let other = offer(NoiseProfile::Nn25519ChaChaPolySha256);
    let mut crossed = other.clone();
    crossed.purpose = EndpointPurpose::ClipboardBridge;
    crossed.initiator_role = EndpointRole::ClipboardDaemon;
    crossed.responder_role = EndpointRole::WaylandProxy;
    crossed.service = ServicePackage::ClipboardV2;
    let mut initiator = NoiseHandshake::new(
        HandshakeRole::Initiator,
        &negotiated(&other),
        HandshakeCredentials::Nn,
    )
    .unwrap();
    let mut responder = NoiseHandshake::new(
        HandshakeRole::Responder,
        &negotiated(&crossed),
        HandshakeCredentials::Nn,
    )
    .unwrap();
    let first = initiator.write_next().unwrap();
    responder.read_next(&first).unwrap();
    let second = responder.write_next().unwrap();
    assert_eq!(
        initiator.read_next(&second).unwrap_err().code(),
        SessionErrorCode::AuthenticationFailed
    );
}

#[test]
fn protected_records_are_directional_sequenced_and_replay_safe() {
    let (initiator, responder) = establish(NoiseProfile::Nn25519ChaChaPolySha256);
    let mut sender = RecordProtector::from_handshake(initiator);
    let mut receiver = RecordProtector::from_handshake(responder);
    let record = sender
        .protect(
            RecordKind::Ttrpc,
            ChannelId::TTRPC_CONTROL,
            b"opaque generated ttrpc frame",
        )
        .unwrap();
    let replay = record.as_bytes().to_vec();
    let (header, plaintext) = receiver.unprotect(record.as_bytes()).unwrap();
    assert_eq!(header.sequence, 0);
    assert_eq!(plaintext, b"opaque generated ttrpc frame");
    assert_eq!(
        receiver.unprotect(&replay).unwrap_err().code(),
        SessionErrorCode::RecordReplay
    );

    let mut truncated = sender
        .protect(
            RecordKind::SessionControl,
            ChannelId::SESSION_CONTROL,
            b"close",
        )
        .unwrap()
        .into_bytes();
    truncated.pop();
    assert_eq!(
        receiver.unprotect(&truncated).unwrap_err().code(),
        SessionErrorCode::RecordTruncated
    );
    assert!(!format!("{sender:?}").contains("opaque"));
}

#[test]
fn protected_record_boundaries_and_tampering_fail_closed() {
    let limits = LimitProfile::local_default();
    let max_payload = limits.protected_plaintext_bytes().unwrap() as usize
        - d2b_session::contract::RECORD_HEADER_LEN;
    let (initiator, responder) = establish(NoiseProfile::Nn25519ChaChaPolySha256);
    let mut sender = RecordProtector::from_handshake(initiator);
    let mut receiver = RecordProtector::from_handshake(responder);
    let exact = sender
        .protect(
            RecordKind::Ttrpc,
            ChannelId::TTRPC_CONTROL,
            &vec![0x41; max_payload],
        )
        .unwrap();
    assert_eq!(
        exact.as_bytes().len(),
        limits.protected_ciphertext_bytes as usize + 2
    );
    assert_eq!(
        receiver.unprotect(exact.as_bytes()).unwrap().1.len(),
        max_payload
    );
    assert_eq!(
        sender
            .protect(
                RecordKind::Ttrpc,
                ChannelId::TTRPC_CONTROL,
                &vec![0x41; max_payload + 1]
            )
            .unwrap_err()
            .code(),
        SessionErrorCode::QueueBackpressure
    );

    let (initiator, responder) = establish(NoiseProfile::Nn25519ChaChaPolySha256);
    let mut sender = RecordProtector::from_handshake(initiator);
    let mut receiver = RecordProtector::from_handshake(responder);
    let mut tampered = sender
        .protect(
            RecordKind::SessionControl,
            ChannelId::SESSION_CONTROL,
            b"control",
        )
        .unwrap()
        .into_bytes();
    *tampered.last_mut().unwrap() ^= 1;
    assert_eq!(
        receiver.unprotect(&tampered).unwrap_err().code(),
        SessionErrorCode::AuthenticationFailed
    );
}

#[test]
fn fragmentation_is_bounded_and_rejects_reordering() {
    let limits = LimitProfile::local_default();
    let fragmenter = Fragmenter::new(limits, MAX_LOGICAL_MESSAGE_BYTES).unwrap();
    let message = vec![0x5a; 200_000];
    let fragments = fragmenter.fragment(9, &message).unwrap();
    assert!(fragments.len() > 1);
    let mut reassembler = Reassembler::new(MAX_LOGICAL_MESSAGE_BYTES).unwrap();
    let mut result = None;
    for fragment in fragmenter.fragment(9, &message).unwrap() {
        result = reassembler.accept(fragment).unwrap();
    }
    assert_eq!(result.unwrap(), message);

    let mut reordered = fragmenter.fragment(10, &vec![1; 200_000]).unwrap();
    reordered.swap(0, 1);
    assert_eq!(
        reassembler.accept(reordered.remove(0)).unwrap_err().code(),
        SessionErrorCode::FragmentReordered
    );
    assert_eq!(
        fragmenter
            .fragment(11, &vec![0; MAX_LOGICAL_MESSAGE_BYTES as usize + 1])
            .unwrap_err()
            .code(),
        SessionErrorCode::ReassemblyLimitExceeded
    );

    let mut duplicate = Reassembler::new(MAX_LOGICAL_MESSAGE_BYTES).unwrap();
    let mut first_copy = fragmenter.fragment(12, &vec![2; 200_000]).unwrap();
    let first = first_copy.remove(0);
    duplicate.accept(first).unwrap();
    let replayed_first = fragmenter
        .fragment(12, &vec![2; 200_000])
        .unwrap()
        .remove(0);
    assert_eq!(
        duplicate.accept(replayed_first).unwrap_err().code(),
        SessionErrorCode::FragmentDuplicate
    );
}

#[test]
fn deadline_intersects_wall_monotonic_and_ttrpc_budgets() {
    let wall = 1_800_000_000_000;
    let now = Instant::now();
    let envelope = RequestEnvelope {
        request_id: RequestId::new(vec![1; 16]).unwrap(),
        correlation_id: None,
        trace_id: None,
        idempotency_key: None,
        issued_at_unix_ms: wall,
        expires_at_unix_ms: wall + MAX_REQUEST_LIFETIME_MS,
    };
    let budget = DeadlineBudget::admit(
        envelope,
        wall,
        now,
        MAX_REQUEST_LIFETIME_MS,
        Some(2_000_000_000),
    )
    .unwrap();
    let context = budget
        .ttrpc_context(wall, now, Some(1_000_000_000))
        .unwrap();
    assert_eq!(context.timeout_nano, 1_000_000_000);
    assert!(context.timeout_nano < wall as i64);
    assert_eq!(DeadlineBudget::peer_timeout(0), None);
    assert_eq!(DeadlineBudget::peer_timeout(-1), None);
    assert_eq!(
        budget
            .remaining_nanos(
                wall + MAX_REQUEST_LIFETIME_MS,
                now + Duration::from_millis(1),
                None
            )
            .unwrap_err()
            .code(),
        SessionErrorCode::DeadlineExpired
    );

    let mut metadata = d2b_contracts::v2_services::common::RequestMetadata::new();
    metadata.request_id = vec![2; 16];
    metadata.idempotency_key = vec![3; 16];
    metadata.issued_at_unix_ms = wall;
    metadata.expires_at_unix_ms = wall + 1_000;
    metadata.session_generation = 7;
    let generated =
        DeadlineBudget::admit_metadata(&metadata, 7, true, wall, now, 1_000, None).unwrap();
    assert_eq!(
        generated
            .ttrpc_context(wall, now, None)
            .unwrap()
            .timeout_nano,
        1_000_000_000
    );
    assert_eq!(
        DeadlineBudget::admit_metadata(&metadata, 8, true, wall, now, 1_000, None)
            .unwrap_err()
            .code(),
        SessionErrorCode::GenerationMismatch
    );
}

#[tokio::test]
async fn cancellation_is_generation_bound_and_shared() {
    let id = RequestId::new(vec![0x61; 16]).unwrap();
    let mut registry = d2b_session::RequestRegistry::new(4).unwrap();
    let token = registry.register(id.clone()).unwrap();
    assert_eq!(
        registry.register(id.clone()).unwrap_err().code(),
        SessionErrorCode::RequestIdDuplicate
    );
    let wrong = registry.cancel(CancelRequest {
        reconnect_generation: 5,
        request_id: id.clone(),
    });
    assert_eq!(wrong.result, CancelResult::GenerationMismatch);
    registry.mark_dispatched(&id).unwrap();
    let wait = token.clone();
    let task = tokio::spawn(async move {
        wait.cancelled().await;
    });
    let ack = registry.cancel(CancelRequest {
        reconnect_generation: 4,
        request_id: id.clone(),
    });
    assert_eq!(ack.result, CancelResult::CancellationSignalled);
    task.await.unwrap();
    assert!(token.is_cancelled());
    assert!(registry.complete(&id));

    let generated_id = RequestId::new(vec![0x62; 16]).unwrap();
    registry.register(generated_id.clone()).unwrap();
    let mut generated = d2b_contracts::v2_services::common::CancelRequest::new();
    generated.session_generation = 4;
    generated.request_id = generated_id.as_bytes().to_vec();
    let response = registry.cancel_generated(&generated).unwrap();
    assert_eq!(
        response.outcome.enum_value().unwrap(),
        d2b_contracts::v2_services::common::CancelOutcome::CANCEL_OUTCOME_CANCELLED_BEFORE_DISPATCH
    );
}

#[test]
fn lifecycle_keepalive_close_and_reconnect_change_generation() {
    let now = Instant::now();
    let limits = LimitProfile::local_default();
    let mut lifecycle = SessionLifecycle::new(1, limits, now).unwrap();
    let ping_at = now + Duration::from_millis(u64::from(limits.keepalive_interval_ms));
    let ping = match lifecycle.poll_keepalive(ping_at) {
        KeepaliveAction::SendPing(record) => record,
        other => panic!("expected ping, got {other:?}"),
    };
    lifecycle
        .receive_pong(ping, ping_at + Duration::from_millis(1))
        .unwrap();
    let next_ping_at = ping_at + Duration::from_millis(u64::from(limits.keepalive_interval_ms) + 1);
    assert!(matches!(
        lifecycle.poll_keepalive(next_ping_at),
        KeepaliveAction::SendPing(_)
    ));
    assert!(matches!(
        lifecycle.poll_keepalive(
            next_ping_at + Duration::from_millis(u64::from(limits.keepalive_timeout_ms))
        ),
        KeepaliveAction::Close(_)
    ));

    let mut reconnect = SessionLifecycle::new(8, limits, now).unwrap();
    reconnect.disconnect(now);
    assert_eq!(reconnect.begin_reconnect(now).unwrap(), 9);
    reconnect.reconnect_established(now).unwrap();
    assert_eq!(reconnect.generation(), 9);
    let close = reconnect.close(CloseReason::Normal, Remediation::None);
    assert_eq!(close.reconnect_generation, 9);
}

#[test]
fn named_stream_state_and_scheduler_have_independent_credit_and_fairness() {
    let limits = LimitProfile::local_default();
    let first = StreamId::new(0x100).unwrap();
    let second = StreamId::new(0x101).unwrap();
    let mut mux = NamedStreamMux::new(limits).unwrap();
    mux.open(first, 5, 5).unwrap();
    mux.open(second, 5, 5).unwrap();
    mux.reserve_send(first, 5).unwrap();
    assert_eq!(
        mux.reserve_send(first, 1).unwrap_err().code(),
        SessionErrorCode::QueueBackpressure
    );
    match mux.receive_data(second, b"data".to_vec()).unwrap() {
        StreamEvent::Data { bytes, .. } => assert_eq!(bytes, b"data"),
        event => panic!("unexpected event {event:?}"),
    }
    assert_eq!(
        mux.close_local(first).unwrap(),
        StreamPhase::HalfClosedLocal
    );
    mux.receive_close(first).unwrap();
    assert_eq!(mux.phase(first), Some(StreamPhase::Closed));
    assert!(mux.remove_terminal(first));

    let mut scheduler = FairScheduler::new(limits).unwrap();
    scheduler.register_stream(first, 0).unwrap();
    scheduler.register_stream(second, 8).unwrap();
    scheduler
        .enqueue(OutboundFrame::named(first, b"stalled".to_vec()).unwrap())
        .unwrap();
    scheduler
        .enqueue(OutboundFrame::named(second, b"ready".to_vec()).unwrap())
        .unwrap();
    scheduler
        .enqueue(OutboundFrame::control(QueueClass::TtrpcControl, b"rpc".to_vec()).unwrap())
        .unwrap();
    scheduler
        .enqueue(
            OutboundFrame::control(QueueClass::SessionControl, b"fatal-close".to_vec()).unwrap(),
        )
        .unwrap();
    assert_eq!(
        scheduler.dequeue().unwrap().class(),
        QueueClass::SessionControl
    );
    assert_eq!(
        scheduler.dequeue().unwrap().class(),
        QueueClass::TtrpcControl
    );
    assert_eq!(scheduler.dequeue().unwrap().stream(), Some(second));
    assert!(scheduler.dequeue().is_none());
    scheduler.grant_stream_credit(first, 8).unwrap();
    assert_eq!(scheduler.dequeue().unwrap().stream(), Some(first));

    let mut fair = FairScheduler::new(limits).unwrap();
    fair.register_stream(first, 8).unwrap();
    fair.register_stream(second, 8).unwrap();
    for stream in [first, second, first, second] {
        fair.enqueue(OutboundFrame::named(stream, vec![1]).unwrap())
            .unwrap();
    }
    assert_eq!(
        (0..4)
            .map(|_| fair.dequeue().unwrap().stream().unwrap())
            .collect::<Vec<_>>(),
        [first, second, first, second]
    );

    let ttrpc = OutboundFrame::control(QueueClass::TtrpcControl, vec![1]).unwrap();
    assert_eq!(ttrpc.channel(), ChannelId::TTRPC_CONTROL);
}

#[test]
fn bootstrap_is_operation_bound_expiring_single_use_and_redacted() {
    let operation = OperationId::new(vec![0x44; 16]).unwrap();
    let nonce = [0x33; 32];
    let binding = BootstrapPskBinding {
        operation_id: operation.clone(),
        replay_nonce: nonce,
        expires_at_unix_ms: 100,
    };
    let mut admission =
        BootstrapAdmission::new(binding, BootstrapPsk::new([0x55; 32]).unwrap()).unwrap();
    let wrong = OperationId::new(vec![0x45; 16]).unwrap();
    assert_eq!(
        admission.consume(&wrong, &nonce, 99).unwrap_err().code(),
        SessionErrorCode::BootstrapOperationMismatch
    );
    let key = admission.consume(&operation, &nonce, 99).unwrap();
    assert!(format!("{key:?}").contains("<redacted>"));
    assert!(format!("{admission:?}").contains("<redacted>"));
    assert_eq!(
        admission
            .consume(&operation, &nonce, 99)
            .unwrap_err()
            .code(),
        SessionErrorCode::BootstrapReplayed
    );

    let expired_binding = BootstrapPskBinding {
        operation_id: operation.clone(),
        replay_nonce: nonce,
        expires_at_unix_ms: 100,
    };
    let mut expired =
        BootstrapAdmission::new(expired_binding, BootstrapPsk::new([0x56; 32]).unwrap()).unwrap();
    assert_eq!(
        expired.consume(&operation, &nonce, 100).unwrap_err().code(),
        SessionErrorCode::BootstrapExpired
    );
}

#[derive(Default)]
struct MemoryTransport {
    packets: VecDeque<TransportPacket>,
    closed: bool,
}

#[async_trait]
impl OwnedTransport for MemoryTransport {
    fn descriptor(&self) -> TransportDescriptor {
        TransportDescriptor {
            class: TransportClass::ProviderStream,
            locality: Locality::Remote,
            packet_atomic: false,
            supports_attachments: false,
        }
    }

    async fn receive(
        &mut self,
        protected_limit: usize,
    ) -> std::result::Result<TransportPacket, TransportError> {
        let packet = self.packets.pop_front().ok_or(TransportError::WouldBlock)?;
        if packet.as_bytes().len() > protected_limit {
            return Err(TransportError::LimitExceeded);
        }
        Ok(packet)
    }

    async fn send(&mut self, packet: TransportPacket) -> std::result::Result<(), TransportError> {
        self.packets.push_back(packet);
        Ok(())
    }

    async fn close(&mut self) -> std::result::Result<(), TransportError> {
        self.closed = true;
        Ok(())
    }
}

#[tokio::test]
async fn owned_transport_is_portable_and_payload_debug_is_redacted() {
    let mut transport = MemoryTransport::default();
    transport
        .send(TransportPacket::new(b"secret endpoint payload".to_vec()))
        .await
        .unwrap();
    let packet = transport.receive(64).await.unwrap();
    assert!(!format!("{packet:?}").contains("secret endpoint payload"));
    transport.close().await.unwrap();
    assert!(transport.closed);
}

#[derive(Default)]
struct CapturingMetrics(Mutex<Vec<(MetricEvent, MetricLabels, u64)>>);

impl MetricsSink for CapturingMetrics {
    fn record(&self, event: MetricEvent, labels: MetricLabels, value: u64) {
        self.0.lock().unwrap().push((event, labels, value));
    }
}

#[test]
fn metrics_accept_only_closed_low_cardinality_labels() {
    let sink = Arc::new(CapturingMetrics::default());
    sink.record(
        MetricEvent::Handshake,
        MetricLabels {
            transport: TransportClass::ProviderStream,
            purpose: EndpointPurpose::RealmPeer,
            channel_class: d2b_session::contract::ChannelClass::TtrpcControl,
            noise: NoiseProfile::Kk25519ChaChaPolySha256,
            locality: Locality::Remote,
            provider_type: Some(ProviderTypeLabel::Transport),
            health_state: d2b_session::contract::HealthState::Healthy,
            result: MetricResult::Accepted,
            reason: MetricReason::None,
        },
        1,
    );
    assert_eq!(sink.0.lock().unwrap().len(), 1);
}

struct FakeTransport {
    sender: mpsc::Sender<TransportPacket>,
    receiver: mpsc::Receiver<TransportPacket>,
    corrupt_attachment: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
}

#[async_trait]
impl OwnedTransport for FakeTransport {
    fn descriptor(&self) -> TransportDescriptor {
        TransportDescriptor {
            class: TransportClass::UnixSeqpacket,
            locality: Locality::HostLocal,
            packet_atomic: true,
            supports_attachments: true,
        }
    }

    async fn receive(
        &mut self,
        protected_limit: usize,
    ) -> std::result::Result<TransportPacket, TransportError> {
        let packet = self
            .receiver
            .recv()
            .await
            .ok_or(TransportError::Disconnected)?;
        if packet.as_bytes().len() > protected_limit {
            return Err(TransportError::LimitExceeded);
        }
        Ok(packet)
    }

    async fn send(&mut self, packet: TransportPacket) -> std::result::Result<(), TransportError> {
        let (mut bytes, attachments) = packet.into_parts();
        if !attachments.is_empty() && self.corrupt_attachment.swap(false, Ordering::AcqRel) {
            let last = bytes.last_mut().ok_or(TransportError::Truncated)?;
            *last ^= 1;
        }
        self.sender
            .send(TransportPacket::with_attachments(bytes, attachments))
            .await
            .map_err(|_| TransportError::Disconnected)
    }

    async fn close(&mut self) -> std::result::Result<(), TransportError> {
        self.closed.store(true, Ordering::Release);
        Ok(())
    }
}

struct FakeHandles {
    corrupt_a: Arc<AtomicBool>,
    closed_a: Arc<AtomicBool>,
    closed_b: Arc<AtomicBool>,
}

fn fake_transport_pair() -> (FakeTransport, FakeTransport, FakeHandles) {
    let (a_to_b_tx, a_to_b_rx) = mpsc::channel(128);
    let (b_to_a_tx, b_to_a_rx) = mpsc::channel(128);
    let corrupt_a = Arc::new(AtomicBool::new(false));
    let closed_a = Arc::new(AtomicBool::new(false));
    let closed_b = Arc::new(AtomicBool::new(false));
    (
        FakeTransport {
            sender: a_to_b_tx,
            receiver: b_to_a_rx,
            corrupt_attachment: Arc::clone(&corrupt_a),
            closed: Arc::clone(&closed_a),
        },
        FakeTransport {
            sender: b_to_a_tx,
            receiver: a_to_b_rx,
            corrupt_attachment: Arc::new(AtomicBool::new(false)),
            closed: Arc::clone(&closed_b),
        },
        FakeHandles {
            corrupt_a,
            closed_a,
            closed_b,
        },
    )
}

async fn engine_pair() -> (
    SessionEngine<FakeTransport>,
    SessionEngine<FakeTransport>,
    FakeHandles,
) {
    let (initiator_transport, responder_transport, handles) = fake_transport_pair();
    let session_offer = offer(NoiseProfile::Nn25519ChaChaPolySha256);
    let initiator_policy = policy(&session_offer);
    let responder_policy = policy(&session_offer);
    let now = Instant::now();
    let (initiator, responder) = tokio::join!(
        SessionEngine::establish_initiator(
            initiator_transport,
            initiator_policy,
            HandshakeCredentials::Nn,
            now
        ),
        SessionEngine::establish_responder(
            responder_transport,
            responder_policy,
            HandshakeCredentials::Nn,
            now
        )
    );
    (initiator.unwrap(), responder.unwrap(), handles)
}

async fn receive_ttrpc(engine: &mut SessionEngine<FakeTransport>) -> Vec<u8> {
    loop {
        match engine.receive().await.unwrap() {
            SessionEvent::Ttrpc(bytes) => return bytes,
            SessionEvent::ControlProcessed => {}
            event => panic!("unexpected event {event:?}"),
        }
    }
}

struct CountingAttachment(Arc<AtomicUsize>);

impl AttachmentPayload for CountingAttachment {
    fn close(self: Box<Self>) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn engine_attachment(counter: Arc<AtomicUsize>) -> OwnedAttachment {
    OwnedAttachment::new(
        AttachmentDescriptor {
            index: 0,
            kind: AttachmentKind::FileDescriptor,
            object_type: KernelObjectType::Pidfd,
            access: AttachmentAccess::ReadOnly,
            purpose: AttachmentPurpose::ProcessIdentity,
            service: ServicePackage::BrokerV2,
            method_id: 7,
            request_id: RequestId::new(vec![0x71; 16]).unwrap(),
            operation_id: Some(OperationId::new(vec![0x72; 16]).unwrap()),
            packet_sequence: 0,
            reconnect_generation: 1,
            duplicate_object_allowed: false,
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
        },
        Box::new(CountingAttachment(counter)),
    )
}

#[tokio::test]
async fn engine_drives_fragmented_ttrpc_and_request_cancellation() {
    let (mut initiator, mut responder, _) = engine_pair().await;
    let request_id = RequestId::new(vec![0x61; 16]).unwrap();
    let payload = vec![0x5a; 200_000];
    let cancelled =
        ComponentSessionDriver::invoke(&mut initiator, request_id.clone(), payload.clone())
            .await
            .unwrap();
    assert_eq!(receive_ttrpc(&mut responder).await, payload);

    let inbound = responder.register_inbound_call(request_id.clone()).unwrap();
    let generation = ComponentSessionDriver::generation(&initiator);
    ComponentSessionDriver::cancel(&mut initiator, generation, &request_id)
        .await
        .unwrap();
    let event = responder.receive().await.unwrap();
    assert!(matches!(
        event,
        SessionEvent::CancelRequest(CancelAck {
            result: CancelResult::CancelledBeforeDispatch,
            ..
        })
    ));
    assert!(inbound.is_cancelled());
    assert!(matches!(
        ComponentSessionDriver::receive(&mut initiator)
            .await
            .unwrap(),
        SessionEvent::CancelAck(CancelAck {
            result: CancelResult::CancelledBeforeDispatch,
            ..
        })
    ));
    assert!(cancelled.is_cancelled());
}

#[tokio::test]
async fn engine_enforces_named_stream_backpressure_and_credit() {
    let (mut initiator, mut responder, _) = engine_pair().await;
    let stream = StreamId::new(0x100).unwrap();
    initiator.open_named_stream(stream, 4, 4).unwrap();
    responder.open_named_stream(stream, 4, 4).unwrap();
    initiator
        .send_named_stream(stream, b"data".to_vec())
        .await
        .unwrap();
    assert_eq!(
        initiator
            .send_named_stream(stream, b"x".to_vec())
            .await
            .unwrap_err()
            .code(),
        SessionErrorCode::QueueBackpressure
    );
    match responder.receive().await.unwrap() {
        SessionEvent::NamedStream(StreamEvent::Data { bytes, .. }) => {
            assert_eq!(bytes, b"data")
        }
        event => panic!("unexpected event {event:?}"),
    }
    responder
        .grant_named_stream_credit(stream, 4)
        .await
        .unwrap();
    assert!(matches!(
        initiator.receive().await.unwrap(),
        SessionEvent::ControlProcessed
    ));
    initiator
        .send_named_stream(stream, b"more".to_vec())
        .await
        .unwrap();
}

#[tokio::test]
async fn engine_binds_acknowledges_and_releases_owned_attachments() {
    let (mut initiator, mut responder, _) = engine_pair().await;
    let closes = Arc::new(AtomicUsize::new(0));
    initiator
        .send_attachments(vec![engine_attachment(Arc::clone(&closes))])
        .await
        .unwrap();
    assert_eq!(initiator.outstanding_attachment_credits(), 1);
    let attachments = match responder.receive().await.unwrap() {
        SessionEvent::Attachments(attachments) => attachments,
        event => panic!("unexpected event {event:?}"),
    };
    assert_eq!(attachments.len(), 1);
    assert_eq!(closes.load(Ordering::Acquire), 0);
    drop(attachments);
    assert_eq!(closes.load(Ordering::Acquire), 1);
    assert!(matches!(
        initiator.receive().await.unwrap(),
        SessionEvent::AttachmentAcknowledged { count: 1 }
    ));
    assert_eq!(initiator.outstanding_attachment_credits(), 0);
}

#[tokio::test]
async fn invalid_protected_attachment_drops_payload_once_and_closes_session() {
    let (mut initiator, mut responder, handles) = engine_pair().await;
    let closes = Arc::new(AtomicUsize::new(0));
    handles.corrupt_a.store(true, Ordering::Release);
    initiator
        .send_attachments(vec![engine_attachment(Arc::clone(&closes))])
        .await
        .unwrap();
    assert_eq!(
        responder.receive().await.unwrap_err().code(),
        SessionErrorCode::AuthenticationFailed
    );
    assert_eq!(closes.load(Ordering::Acquire), 1);
    assert!(handles.closed_b.load(Ordering::Acquire));
    assert!(!handles.closed_a.load(Ordering::Acquire));
}

#[tokio::test]
async fn attachment_local_validation_and_explicit_close_are_exactly_once() {
    let (mut initiator, _, _) = engine_pair().await;
    let closes = Arc::new(AtomicUsize::new(0));
    let descriptor = engine_attachment(Arc::clone(&closes));
    descriptor.close();
    assert_eq!(closes.load(Ordering::Acquire), 1);

    let rejected = Arc::new(AtomicUsize::new(0));
    let attachment = OwnedAttachment::new(
        AttachmentDescriptor {
            index: 0,
            kind: AttachmentKind::FileDescriptor,
            object_type: KernelObjectType::Pidfd,
            access: AttachmentAccess::ReadOnly,
            purpose: AttachmentPurpose::ProcessIdentity,
            service: ServicePackage::DaemonV2,
            method_id: 7,
            request_id: RequestId::new(vec![0x73; 16]).unwrap(),
            operation_id: None,
            packet_sequence: 0,
            reconnect_generation: 1,
            duplicate_object_allowed: false,
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
        },
        Box::new(CountingAttachment(Arc::clone(&rejected))),
    );
    assert_eq!(
        initiator
            .send_attachments(vec![attachment])
            .await
            .unwrap_err()
            .code(),
        SessionErrorCode::AttachmentDescriptorMismatch
    );
    assert_eq!(rejected.load(Ordering::Acquire), 1);
    assert_eq!(initiator.outstanding_attachment_credits(), 0);
}

#[tokio::test]
async fn engine_reconnect_rehandshakes_with_the_next_generation() {
    let (initiator, responder, old_handles) = engine_pair().await;
    let (new_initiator_transport, new_responder_transport, _) = fake_transport_pair();
    let mut reconnect_offer = offer(NoiseProfile::Nn25519ChaChaPolySha256);
    reconnect_offer.reconnect_generation = 8;
    let initiator_policy = policy(&reconnect_offer);
    let responder_policy = policy(&reconnect_offer);
    let now = Instant::now();
    let (initiator, mut responder) = tokio::join!(
        initiator.reconnect_initiator(
            new_initiator_transport,
            initiator_policy,
            HandshakeCredentials::Nn,
            now
        ),
        responder.reconnect_responder(
            new_responder_transport,
            responder_policy,
            HandshakeCredentials::Nn,
            now
        )
    );
    let mut initiator = initiator.unwrap();
    let responder = responder.as_mut().unwrap();
    assert_eq!(initiator.generation(), 8);
    assert_eq!(responder.generation(), 8);
    assert!(old_handles.closed_a.load(Ordering::Acquire));
    assert!(old_handles.closed_b.load(Ordering::Acquire));
    initiator
        .send_ttrpc(b"after-reconnect".to_vec())
        .await
        .unwrap();
    assert_eq!(receive_ttrpc(responder).await, b"after-reconnect");
}
