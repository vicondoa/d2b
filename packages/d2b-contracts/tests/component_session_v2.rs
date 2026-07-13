#![cfg(feature = "v2-component-session")]

use d2b_contracts::v2_component_session::*;
use schemars::{JsonSchema, schema_for};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;
use std::{fmt::Debug, fs, path::PathBuf};

fn packet_policy() -> AttachmentPolicy {
    AttachmentPolicy {
        kind: AttachmentPolicyKind::PacketAtomic,
        max_per_packet: MAX_PACKET_ATTACHMENTS,
        max_per_request: MAX_REQUEST_ATTACHMENTS,
        max_per_operation: MAX_OPERATION_ATTACHMENTS,
        max_per_session: MAX_SESSION_ATTACHMENTS,
        credentials_allowed: true,
    }
}

fn offer() -> HandshakeOffer {
    HandshakeOffer {
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
        attachment_policy: packet_policy(),
    }
}

fn policy(value: &HandshakeOffer) -> EndpointPolicy {
    EndpointPolicy {
        purpose: value.purpose,
        purpose_class: value.purpose_class,
        initiator_role: value.initiator_role,
        responder_role: value.responder_role,
        service: value.service,
        schema_fingerprint: value.schema_fingerprint,
        noise_profile: value.noise_profile,
        limits: value.limits,
        transport_binding: value.transport_binding,
        reconnect_generation: value.reconnect_generation,
        attachment_policy: value.attachment_policy,
    }
}

fn request_id() -> RequestId {
    RequestId::new(vec![0x31; 16]).unwrap()
}

fn operation_id() -> OperationId {
    OperationId::new(vec![0x42; 16]).unwrap()
}

fn assert_wire_enum<T>(values: &[T], spelling: impl Fn(T) -> &'static str)
where
    T: Copy + Debug + Eq + JsonSchema + Serialize + DeserializeOwned,
{
    let expected: Vec<_> = values
        .iter()
        .copied()
        .map(|value| json!(spelling(value)))
        .collect();
    for (value, wire) in values.iter().copied().zip(&expected) {
        assert_eq!(serde_json::to_value(value).unwrap(), *wire);
        assert_eq!(serde_json::from_value::<T>(wire.clone()).unwrap(), value);
    }
    let schema = schema_for!(T);
    assert_eq!(schema.schema.enum_values.as_ref(), Some(&expected));
}

#[test]
fn preface_is_exact_network_order_and_strict() {
    let preface = ComponentSessionPreface::new(MAX_HANDSHAKE_OFFER_BYTES).unwrap();
    assert_eq!(
        preface.encode(),
        [
            b'D', b'2', b'B', b'C', b'S', b'2', b'\r', b'\n', 0, 2, 0, 0, 0, 0, 0x40, 0,
        ]
    );
    assert_eq!(
        ComponentSessionPreface::parse(&preface.encode()).unwrap(),
        preface
    );
    assert_eq!(
        ComponentSessionPreface::new(0),
        Err(PrefaceError::EmptyOffer)
    );
    assert!(ComponentSessionPreface::new(MAX_HANDSHAKE_OFFER_BYTES - 1).is_ok());
    assert!(ComponentSessionPreface::new(MAX_HANDSHAKE_OFFER_BYTES).is_ok());
    assert_eq!(
        ComponentSessionPreface::new(MAX_HANDSHAKE_OFFER_BYTES + 1),
        Err(PrefaceError::OfferTooLarge)
    );

    let mut bad = preface.encode();
    bad[0] ^= 1;
    assert_eq!(
        ComponentSessionPreface::parse(&bad),
        Err(PrefaceError::InvalidMagic)
    );
    let mut bad = preface.encode();
    bad[9] = 3;
    assert_eq!(
        ComponentSessionPreface::parse(&bad),
        Err(PrefaceError::UnsupportedMajor)
    );
    let mut bad = preface.encode();
    bad[11] = 1;
    assert_eq!(
        ComponentSessionPreface::parse(&bad),
        Err(PrefaceError::UnsupportedMinor)
    );
    assert_eq!(
        ComponentSessionPreface::parse(&preface.encode()[..15]),
        Err(PrefaceError::Truncated)
    );
    let mut oversized = preface.encode().to_vec();
    oversized.push(0);
    assert_eq!(
        ComponentSessionPreface::parse(&oversized),
        Err(PrefaceError::InvalidLength)
    );
}

#[test]
fn offer_accept_and_reject_have_canonical_binary_round_trips() {
    let offer = offer();
    let bytes = offer.encode_canonical().unwrap();
    assert_eq!(bytes.len(), HANDSHAKE_OFFER_CANONICAL_LEN);
    assert_eq!(HandshakeOffer::decode_canonical(&bytes).unwrap(), offer);

    let mut undersized = offer.clone();
    undersized.limits.handshake_offer_bytes = HANDSHAKE_OFFER_CANONICAL_LEN as u32 - 1;
    assert_eq!(
        undersized.encode_canonical(),
        Err(BinaryError::InvalidContract(ContractError::LimitExceeded))
    );
    let mut self_declared_undersized = bytes.clone();
    self_declared_undersized[39..43]
        .copy_from_slice(&(HANDSHAKE_OFFER_CANONICAL_LEN as u32 - 1).to_be_bytes());
    assert_eq!(
        HandshakeOffer::decode_canonical(&self_declared_undersized),
        Err(BinaryError::LengthExceeded)
    );
    let mut noncanonical = bytes.clone();
    *noncanonical.last_mut().unwrap() = 2;
    assert_eq!(
        HandshakeOffer::decode_canonical(&noncanonical),
        Err(BinaryError::NonCanonical)
    );

    let mut trailing = bytes.clone();
    trailing.push(0);
    assert_eq!(
        HandshakeOffer::decode_canonical(&trailing),
        Err(BinaryError::TrailingBytes)
    );
    let mut unknown = bytes.clone();
    unknown[1] = 0xff;
    assert_eq!(
        HandshakeOffer::decode_canonical(&unknown),
        Err(BinaryError::UnknownEnumTag)
    );
    assert_eq!(
        HandshakeOffer::decode_canonical(&bytes[..bytes.len() - 1]),
        Err(BinaryError::Truncated)
    );

    let accept = HandshakeAccept {
        offer,
        transcript_binding: [0x7a; 32],
    };
    let encoded = accept.encode_canonical().unwrap();
    assert_eq!(HandshakeAccept::decode_canonical(&encoded).unwrap(), accept);
    let reject = HandshakeReject {
        reason: HandshakeRejectReason::SchemaMismatch,
        remediation: Remediation::RepairConfiguration,
    };
    assert_eq!(
        HandshakeReject::decode_canonical(&reject.encode_canonical()).unwrap(),
        reject
    );
    assert_eq!(
        HandshakeReject::decode_canonical(&[1, 255, 0]),
        Err(BinaryError::UnknownEnumTag)
    );
}

#[test]
fn exact_offer_validation_rejects_every_downgrade_dimension() {
    let base = offer();
    let expected = policy(&base);
    assert_eq!(base.validate_exact(&expected), Ok(()));

    let mut mutation = base.clone();
    mutation.purpose = EndpointPurpose::ClipboardBridge;
    assert_eq!(
        mutation.validate_exact(&expected),
        Err(HandshakeRejectReason::PurposeMismatch)
    );
    let mut mutation = base.clone();
    mutation.purpose_class = PurposeClass::Enrolled;
    assert_eq!(
        mutation.validate_exact(&expected),
        Err(HandshakeRejectReason::PurposeClassMismatch)
    );
    let mut mutation = base.clone();
    mutation.initiator_role = EndpointRole::CommandClient;
    assert_eq!(
        mutation.validate_exact(&expected),
        Err(HandshakeRejectReason::RoleMismatch)
    );
    let mut mutation = base.clone();
    mutation.service = ServicePackage::DaemonV2;
    assert_eq!(
        mutation.validate_exact(&expected),
        Err(HandshakeRejectReason::ServiceMismatch)
    );
    let mut mutation = base.clone();
    mutation.schema_fingerprint[0] ^= 1;
    assert_eq!(
        mutation.validate_exact(&expected),
        Err(HandshakeRejectReason::SchemaMismatch)
    );
    let mut mutation = base.clone();
    mutation.noise_profile = NoiseProfile::Kk25519ChaChaPolySha256;
    assert_eq!(
        mutation.validate_exact(&expected),
        Err(HandshakeRejectReason::NoiseProfileMismatch)
    );
    let mut mutation = base.clone();
    mutation.limits.active_named_streams -= 1;
    assert_eq!(
        mutation.validate_exact(&expected),
        Err(HandshakeRejectReason::LimitProfileMismatch)
    );
    let mut mutation = base.clone();
    mutation.transport_binding.channel_binding[0] ^= 1;
    assert_eq!(
        mutation.validate_exact(&expected),
        Err(HandshakeRejectReason::ChannelBindingMismatch)
    );
    let mut mutation = base.clone();
    mutation.reconnect_generation += 1;
    assert_eq!(
        mutation.validate_exact(&expected),
        Err(HandshakeRejectReason::GenerationMismatch)
    );
    let mut mutation = base;
    mutation.attachment_policy.max_per_packet -= 1;
    assert_eq!(
        mutation.validate_exact(&expected),
        Err(HandshakeRejectReason::AttachmentPolicyMismatch)
    );
}

#[test]
fn closed_inventory_and_noise_identity_requirements_are_complete() {
    assert_eq!(ServicePackage::ALL.len(), 15);
    assert!(
        ServicePackage::ALL
            .iter()
            .all(|package| package.as_str().ends_with(".v2"))
    );
    assert_eq!(EndpointPurpose::ALL.len(), 19);
    assert_eq!(EndpointRole::ALL.len(), 19);
    assert_eq!(
        NoiseProfile::Nn25519ChaChaPolySha256.identity_evidence(),
        IdentityEvidenceRequirement::DirectionalUnix
    );
    assert_eq!(
        NoiseProfile::Kk25519ChaChaPolySha256.identity_evidence(),
        IdentityEvidenceRequirement::EnrolledStaticKeys
    );
    assert_eq!(
        NoiseProfile::Ikpsk2_25519ChaChaPolySha256.identity_evidence(),
        IdentityEvidenceRequirement::ParentStaticAndSingleUsePsk
    );
    for purpose_class in PurposeClass::ALL {
        let matching = NoiseProfile::ALL
            .iter()
            .filter(|profile| profile.valid_for(*purpose_class))
            .count();
        assert_eq!(matching, 1, "{purpose_class:?}");
    }
}

#[test]
fn every_closed_wire_enum_uses_its_canonical_spelling_in_serde_and_schema() {
    macro_rules! check {
        ($type:ty) => {
            assert_wire_enum(<$type>::ALL, <$type>::as_str);
        };
    }

    check!(EndpointPurpose);
    check!(PurposeClass);
    check!(EndpointRole);
    check!(ServicePackage);
    check!(NoiseProfile);
    check!(IdentityEvidenceRequirement);
    check!(Locality);
    check!(TransportClass);
    check!(AttachmentPolicyKind);
    check!(HandshakeRejectReason);
    check!(Remediation);
    check!(SessionErrorCode);
    check!(RecordKind);
    check!(ChannelClass);
    check!(CloseReason);
    check!(CancelResult);
    check!(AttachmentKind);
    check!(KernelObjectType);
    check!(AttachmentAccess);
    check!(AttachmentPurpose);
    check!(AttachmentCreditClass);
    check!(MetricResult);
    check!(MetricReason);
    check!(HealthState);
    check!(ProviderTypeLabel);
}

#[test]
fn every_limit_is_exactly_bounded_and_overhead_is_checked() {
    let exact = LimitProfile::local_default();
    assert_eq!(exact.validate(), Ok(()));
    assert_eq!(
        exact.protected_plaintext_bytes().unwrap(),
        MAX_PROTECTED_PLAINTEXT_BYTES
    );
    assert_eq!(
        exact
            .checked_ciphertext_allocation(
                MAX_PROTECTED_PLAINTEXT_BYTES - RECORD_HEADER_LEN as u32,
                RECORD_HEADER_LEN as u32,
            )
            .unwrap(),
        u32::from(u16::MAX) + RECORD_LENGTH_BYTES
    );
    assert_eq!(
        exact.checked_ciphertext_allocation(
            MAX_PROTECTED_PLAINTEXT_BYTES - RECORD_HEADER_LEN as u32 + 1,
            RECORD_HEADER_LEN as u32,
        ),
        Err(ContractError::LimitExceeded)
    );
    assert_eq!(
        exact.checked_ciphertext_allocation(u32::MAX, 1),
        Err(ContractError::ArithmeticOverflow)
    );
    assert_eq!(
        exact.checked_handshake_allocation(u32::MAX, 1, 1),
        Err(ContractError::ArithmeticOverflow)
    );
    assert_eq!(
        exact.checked_handshake_allocation(MAX_PROTECTED_CIPHERTEXT_BYTES - NOISE_TAG_BYTES, 0, 1,),
        Ok(MAX_PROTECTED_CIPHERTEXT_BYTES + RECORD_LENGTH_BYTES)
    );
    assert_eq!(
        exact.checked_handshake_allocation(
            MAX_PROTECTED_CIPHERTEXT_BYTES - NOISE_TAG_BYTES + 1,
            0,
            1,
        ),
        Err(ContractError::LimitExceeded)
    );

    macro_rules! boundary {
        ($field:ident, $max:expr) => {{
            let mut one_below = exact;
            one_below.$field = $max - 1;
            assert_eq!(one_below.validate(), Ok(()), stringify!($field));
            let mut at = exact;
            at.$field = $max;
            assert_eq!(at.validate(), Ok(()), stringify!($field));
            let mut above = exact;
            above.$field = $max + 1;
            assert_eq!(
                above.validate(),
                Err(ContractError::LimitExceeded),
                stringify!($field)
            );
        }};
    }
    boundary!(handshake_offer_bytes, MAX_HANDSHAKE_OFFER_BYTES as u32);
    let mut undersized_offer = exact;
    undersized_offer.handshake_offer_bytes = HANDSHAKE_OFFER_CANONICAL_LEN as u32 - 1;
    assert_eq!(
        undersized_offer.validate(),
        Err(ContractError::LimitExceeded)
    );
    let mut exact_offer = exact;
    exact_offer.handshake_offer_bytes = HANDSHAKE_OFFER_CANONICAL_LEN as u32;
    assert_eq!(exact_offer.validate(), Ok(()));
    boundary!(protected_ciphertext_bytes, MAX_PROTECTED_CIPHERTEXT_BYTES);
    boundary!(logical_ttrpc_bytes, MAX_LOGICAL_MESSAGE_BYTES);
    boundary!(logical_named_stream_bytes, MAX_LOGICAL_MESSAGE_BYTES);
    boundary!(active_named_streams, MAX_ACTIVE_NAMED_STREAMS);
    boundary!(named_stream_queue_bytes, MAX_NAMED_STREAM_QUEUE_BYTES);
    boundary!(
        aggregate_named_stream_queue_bytes,
        MAX_AGGREGATE_NAMED_STREAM_QUEUE_BYTES
    );
    boundary!(ttrpc_control_queue_bytes, MAX_TTRPC_CONTROL_QUEUE_BYTES);
    boundary!(session_control_queue_bytes, MAX_SESSION_CONTROL_QUEUE_BYTES);
    boundary!(keepalive_interval_ms, MAX_KEEPALIVE_INTERVAL_MS);
    boundary!(handshake_deadline_ms, REMOTE_HANDSHAKE_DEADLINE_MS);
    boundary!(reconnect_deadline_ms, REMOTE_RECONNECT_DEADLINE_MS);
    boundary!(reconnect_attempts, MAX_RECONNECT_ATTEMPTS);
    boundary!(reconnect_window_ms, MAX_RECONNECT_WINDOW_MS);
}

#[test]
fn record_fragment_and_sequence_contracts_fail_closed() {
    let limits = LimitProfile::local_default();
    let header = RecordHeader {
        kind: RecordKind::Ttrpc,
        flags: 0,
        channel: ChannelId::TTRPC_CONTROL,
        sequence: 9,
        reconnect_generation: 2,
        payload_len: 128,
    };
    let bytes = header.encode(limits).unwrap();
    assert_eq!(RecordHeader::decode(&bytes, limits).unwrap(), header);
    let wrong_channel = RecordHeader {
        channel: ChannelId::SESSION_CONTROL,
        ..header
    };
    assert_eq!(
        wrong_channel.validate(limits),
        Err(ContractError::InvalidChannel)
    );
    assert_eq!(ChannelId::named(0xff), Err(ContractError::InvalidChannel));
    assert_eq!(
        ChannelId::named(0x100).unwrap().class(),
        ChannelClass::NamedStream
    );
    for invalid in [3, 4, 0xff] {
        assert!(serde_json::from_value::<ChannelId>(json!(invalid)).is_err());
    }
    for valid in [0, 1, 2, 0x100, u16::MAX] {
        assert_eq!(
            serde_json::from_value::<ChannelId>(json!(valid))
                .unwrap()
                .value(),
            valid
        );
    }
    let channel_schema = schema_for!(ChannelId);
    let ranges: Vec<_> = channel_schema
        .schema
        .subschemas
        .as_ref()
        .and_then(|subschemas| subschemas.any_of.as_ref())
        .unwrap()
        .iter()
        .map(|schema| {
            let number = schema.clone().into_object().number.unwrap();
            (
                number.minimum.unwrap() as u16,
                number.maximum.unwrap() as u16,
            )
        })
        .collect();
    assert_eq!(ranges, [(0, 0), (1, 1), (2, 2), (0x100, u16::MAX)]);

    let fragment = FragmentHeader {
        message_id: 1,
        index: 1,
        count: 2,
        total_plaintext_len: 100,
        offset: 60,
    };
    let encoded = fragment.encode(40, 100).unwrap();
    assert_eq!(FragmentHeader::decode(&encoded, 40, 100).unwrap(), fragment);
    assert_eq!(
        FragmentHeader {
            offset: u32::MAX,
            ..fragment
        }
        .validate(2, u32::MAX),
        Err(ContractError::ArithmeticOverflow)
    );
    assert_eq!(
        FragmentHeader {
            index: 0,
            offset: 1,
            ..fragment
        }
        .validate(40, 100),
        Ok(())
    );
    assert_eq!(
        FragmentHeader {
            index: 1,
            offset: 59,
            ..fragment
        }
        .validate(40, 100),
        Err(ContractError::InvalidFragment)
    );

    let mut sequence = ReceiveSequence::new();
    assert_eq!(sequence.accept(1), Err(SequenceError::OutOfOrder));
    assert_eq!(sequence.accept(0), Ok(()));
    assert_eq!(sequence.accept(0), Err(SequenceError::Replay));
    let mut final_sequence = ReceiveSequence::new();
    for value in 0..3 {
        final_sequence.accept(value).unwrap();
    }
    assert_eq!(final_sequence.accept(2), Err(SequenceError::Replay));
    let mut exhausted = ReceiveSequence::from_expected(u64::MAX - 1);
    assert_eq!(exhausted.accept(u64::MAX - 1), Ok(()));
    assert_eq!(
        exhausted.accept(u64::MAX),
        Err(SequenceError::NonceExhausted)
    );
    let mut reserved = ReceiveSequence::from_expected(u64::MAX);
    assert_eq!(
        reserved.accept(u64::MAX),
        Err(SequenceError::NonceExhausted)
    );
    let mut sender = SendSequence::from_next(u64::MAX - 1);
    assert_eq!(sender.take(), Ok(u64::MAX - 1));
    assert_eq!(sender.take(), Err(SequenceError::NonceExhausted));
    let mut reserved_sender = SendSequence::from_next(u64::MAX);
    assert_eq!(reserved_sender.take(), Err(SequenceError::NonceExhausted));

    let first = FragmentHeader {
        message_id: 9,
        index: 0,
        count: 2,
        total_plaintext_len: 100,
        offset: 0,
    };
    let mut fragments = FragmentSequence::begin(first, 60, 100).unwrap();
    assert_eq!(
        fragments.accept(first, 60, 100),
        Err(FragmentSequenceError::Duplicate)
    );
    let second = FragmentHeader {
        message_id: 9,
        ..fragment
    };
    assert_eq!(fragments.accept(second, 40, 100), Ok(true));
    assert_eq!(
        fragments.accept(second, 40, 100),
        Err(FragmentSequenceError::Complete)
    );
}

#[test]
fn authenticated_deadline_and_relative_timeout_never_encode_epoch() {
    let now = 1_800_000_000_000_u64;
    let request = RequestEnvelope {
        request_id: request_id(),
        correlation_id: Some(CorrelationId::new(vec![1]).unwrap()),
        trace_id: Some(TraceId::new(vec![2; 16]).unwrap()),
        idempotency_key: Some(IdempotencyKey::new(vec![3; MAX_ID_BYTES]).unwrap()),
        issued_at_unix_ms: now,
        expires_at_unix_ms: now + MAX_REQUEST_LIFETIME_MS,
    };
    let admitted = request
        .admit(
            now,
            MAX_REQUEST_LIFETIME_MS,
            Some(2_000_000_000),
            Some(1_000_000_000),
        )
        .unwrap();
    assert_eq!(admitted.absolute_expiry_unix_ms, request.expires_at_unix_ms);
    assert_eq!(admitted.remaining_nanos, 1_000_000_000);
    assert!(admitted.remaining_nanos < now * 1_000_000);

    let mut future = request.clone();
    future.issued_at_unix_ms = now + MAX_CLOCK_SKEW_MS;
    future.expires_at_unix_ms = future.issued_at_unix_ms + 1;
    assert!(
        future
            .admit(now, MAX_REQUEST_LIFETIME_MS, None, None)
            .is_ok()
    );
    future.issued_at_unix_ms += 1;
    future.expires_at_unix_ms += 1;
    assert_eq!(
        future.admit(now, MAX_REQUEST_LIFETIME_MS, None, None),
        Err(ContractError::InvalidDeadline)
    );

    let mut lifetime = request.clone();
    lifetime.expires_at_unix_ms = lifetime.issued_at_unix_ms + MAX_REQUEST_LIFETIME_MS + 1;
    assert_eq!(
        lifetime.admit(now, MAX_REQUEST_LIFETIME_MS, None, None),
        Err(ContractError::InvalidDeadline)
    );
    let mut expired = request;
    expired.expires_at_unix_ms = now;
    assert_eq!(
        expired.admit(now, MAX_REQUEST_LIFETIME_MS, None, None),
        Err(ContractError::InvalidDeadline)
    );
}

#[test]
fn cancellation_is_generation_and_request_bound() {
    let request = CancelRequest {
        reconnect_generation: 4,
        request_id: request_id(),
    };
    let mismatch = request
        .clone()
        .acknowledge(5, CancelResult::CancellationSignalled);
    assert_eq!(mismatch.result, CancelResult::GenerationMismatch);
    assert_eq!(mismatch.reconnect_generation, 4);
    assert_eq!(mismatch.request_id, request.request_id);
    let accepted = request.acknowledge(4, CancelResult::CancelledBeforeDispatch);
    assert_eq!(accepted.result, CancelResult::CancelledBeforeDispatch);
}

#[test]
fn bootstrap_psk_state_enforces_operation_expiry_and_single_use() {
    let binding = BootstrapPskBinding {
        operation_id: operation_id(),
        replay_nonce: [0x51; 32],
        expires_at_unix_ms: 10_000,
    };
    let wrong_operation = OperationId::new(vec![0x43; 16]).unwrap();

    let mut wrong = BootstrapPskState::new(binding.clone()).unwrap();
    assert_eq!(
        wrong.admit(&wrong_operation, &binding.replay_nonce, 9_999),
        Err(HandshakeRejectReason::BootstrapOperationMismatch)
    );
    assert!(!wrong.is_consumed());

    let mut expired = BootstrapPskState::new(binding.clone()).unwrap();
    assert_eq!(
        expired.admit(&binding.operation_id, &binding.replay_nonce, 10_000),
        Err(HandshakeRejectReason::BootstrapExpired)
    );
    assert!(!expired.is_consumed());

    let mut single_use = BootstrapPskState::new(binding.clone()).unwrap();
    assert_eq!(
        single_use.admit(&binding.operation_id, &binding.replay_nonce, 9_999),
        Ok(())
    );
    assert!(single_use.is_consumed());
    assert_eq!(
        single_use.admit(&binding.operation_id, &binding.replay_nonce, 9_999),
        Err(HandshakeRejectReason::BootstrapReplayed)
    );
}

fn descriptor(index: u16) -> AttachmentDescriptor {
    AttachmentDescriptor {
        index,
        kind: AttachmentKind::FileDescriptor,
        object_type: KernelObjectType::Pidfd,
        access: AttachmentAccess::ReadOnly,
        purpose: AttachmentPurpose::ProcessIdentity,
        service: ServicePackage::BrokerV2,
        method_id: 7,
        request_id: request_id(),
        operation_id: Some(operation_id()),
        packet_sequence: 3,
        reconnect_generation: 2,
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
    }
}

#[test]
fn attachments_are_packet_atomic_and_exactly_accounted() {
    let packet = AttachmentPacket {
        declared_count: 2,
        descriptors: BoundedVec::new(vec![descriptor(0), descriptor(1)]).unwrap(),
    };
    assert_eq!(
        packet.validate(packet_policy(), 2, false, false, false),
        Ok(())
    );
    assert_eq!(
        packet.validate(packet_policy(), 1, false, false, false),
        Err(AttachmentReceiveError::CountMismatch)
    );
    assert_eq!(
        packet.validate(packet_policy(), 2, true, false, false),
        Err(AttachmentReceiveError::MessageTruncated)
    );
    assert_eq!(
        packet.validate(packet_policy(), 2, false, true, false),
        Err(AttachmentReceiveError::ControlTruncated)
    );
    assert_eq!(
        packet.validate(packet_policy(), 2, false, false, true),
        Err(AttachmentReceiveError::UnknownControl)
    );
    let mut bad = packet.clone();
    bad.descriptors[1].cloexec_required = false;
    assert_eq!(
        bad.validate(packet_policy(), 2, false, false, false),
        Err(AttachmentReceiveError::DescriptorMismatch)
    );

    let mut credentials_descriptor = descriptor(0);
    credentials_descriptor.kind = AttachmentKind::Credentials;
    credentials_descriptor.object_type = KernelObjectType::UnixSeqpacketSocket;
    let credentials = AttachmentPacket {
        declared_count: 1,
        descriptors: BoundedVec::new(vec![credentials_descriptor.clone()]).unwrap(),
    };
    let mut credentials_disabled = packet_policy();
    credentials_disabled.credentials_allowed = false;
    assert_eq!(
        credentials.validate(credentials_disabled, 1, false, false, false),
        Err(AttachmentReceiveError::PolicyDenied)
    );
    assert_eq!(
        credentials.validate(packet_policy(), 1, false, false, false),
        Ok(())
    );
    assert_eq!(
        credentials.validate(credentials_disabled, 0, false, false, false),
        Err(AttachmentReceiveError::CountMismatch)
    );
    let credentials_over_credit = AttachmentPacket {
        declared_count: 2,
        descriptors: BoundedVec::new(vec![credentials_descriptor, descriptor(1)]).unwrap(),
    };
    credentials_disabled.max_per_packet = 1;
    assert_eq!(
        credentials_over_credit.validate(credentials_disabled, 2, false, false, false),
        Err(AttachmentReceiveError::CreditExceeded)
    );

    let credits = AttachmentCredits {
        packet: 0,
        request: 0,
        operation: 0,
        session: 0,
        process: 0,
        host: 0,
    };
    assert_eq!(
        credits
            .reserve(MAX_PACKET_ATTACHMENTS, packet_policy())
            .unwrap()
            .packet,
        32
    );
    assert_eq!(
        credits.reserve(MAX_PACKET_ATTACHMENTS + 1, packet_policy()),
        Err(ContractError::CreditExceeded)
    );
    assert_eq!(
        AttachmentCredits::process_pool(RESERVED_CONTROL_FDS as u64, 0),
        Ok(0)
    );
    assert_eq!(
        AttachmentCredits::process_pool(RESERVED_CONTROL_FDS as u64 - 1, 0),
        Err(ContractError::CreditExceeded)
    );
    assert_eq!(
        AttachmentCredits::process_pool(
            u64::from(MAX_PROCESS_ATTACHMENT_CREDITS + RESERVED_CONTROL_FDS),
            0,
        ),
        Ok(MAX_PROCESS_ATTACHMENT_CREDITS)
    );
}

#[test]
fn serde_is_strict_and_bounded_and_metric_labels_are_closed() {
    let value = serde_json::to_value(offer()).unwrap();
    let mut object = value.as_object().unwrap().clone();
    object.insert("legacyFallback".to_owned(), json!(true));
    assert!(serde_json::from_value::<HandshakeOffer>(object.into()).is_err());
    assert!(serde_json::from_value::<ServicePackage>(json!("d2b.daemon.v1")).is_err());
    assert!(serde_json::from_value::<RequestId>(json!([1, 2, 3])).is_err());
    assert!(serde_json::from_value::<CorrelationId>(json!(vec![0; MAX_ID_BYTES + 1])).is_err());
    assert!(serde_json::from_value::<BoundedVec<u8, 0, 32>>(json!(vec![0; 33])).is_err());

    let schema = serde_json::to_value(schema_for!(HandshakeOffer)).unwrap();
    let definitions = schema["definitions"].as_object().unwrap();
    assert!(definitions.contains_key("ServicePackage"));
    let rendered = serde_json::to_string(&schema_for!(MetricLabels)).unwrap();
    for forbidden in [
        "sessionId",
        "requestId",
        "streamId",
        "realmId",
        "workloadId",
        "providerId",
        "operationId",
        "userId",
    ] {
        assert!(!rendered.contains(forbidden), "{forbidden}");
    }
}

fn schema_fixture() -> serde_json::Value {
    json!({
            "contract": "d2b-component-session-v2",
            "formatVersion": 1,
            "schemas": {
                "attachmentPacket": schema_for!(AttachmentPacket),
                "bootstrapPskBinding": schema_for!(BootstrapPskBinding),
                "cancelAck": schema_for!(CancelAck),
                "cancelRequest": schema_for!(CancelRequest),
                "closeRecord": schema_for!(CloseRecord),
                "fragmentHeader": schema_for!(FragmentHeader),
                "handshakeAccept": schema_for!(HandshakeAccept),
                "handshakeOffer": schema_for!(HandshakeOffer),
                "handshakeReject": schema_for!(HandshakeReject),
                "keepaliveRecord": schema_for!(KeepaliveRecord),
                "metricLabels": schema_for!(MetricLabels),
                "recordHeader": schema_for!(RecordHeader),
                "requestEnvelope": schema_for!(RequestEnvelope),
            }
    })
}

#[test]
fn committed_schema_fixture_matches_contract_types() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/reference/component-session-v2-schema.json");
    let generated = schema_fixture();
    let committed: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(committed, generated);
}
