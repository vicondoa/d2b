use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use async_trait::async_trait;
use d2b_contracts::v3::{
    AuthenticatedSubjectContext, BindingDigest, EvidenceClass, ResourceRef, ResourceUid, ZoneId,
    component_session::{
        AttachmentPolicy, AuthorizationLease, EndpointPolicy, EndpointPurpose, EndpointRole,
        IdentityEvidenceRequirement, LimitProfile, Locality, NoiseProfile, ServicePackage,
        TransportBinding, TransportClass,
    },
};
use d2b_session::{
    HandshakeCredentials, SessionAcceptor, SessionAuthenticationBinding, SessionAuthority,
    SessionAuthorizationRequest, SessionEngine, TransportEvidence,
};
use d2b_session_unix::{
    SeqpacketSocket, StreamSocket, UnixStreamTransport, UnixSubjectIdentity, VerifiedUnixSubject,
    prearmed_seqpacket_pair,
};
use rustix::net::{AddressFamily, SocketFlags, SocketType, socketpair};

fn endpoint_policy(transport: TransportClass) -> EndpointPolicy {
    EndpointPolicy {
        purpose: EndpointPurpose::LocalLifecycle,
        purpose_class: d2b_session::contract::PurposeClass::Local,
        initiator_role: EndpointRole::ZoneController,
        responder_role: EndpointRole::Component,
        service: ServicePackage::ResourceV3,
        schema_fingerprint: [0x11; 32],
        noise_profile: NoiseProfile::Nn25519ChaChaPolySha256,
        limits: LimitProfile::local_default(),
        transport_binding: TransportBinding {
            transport,
            locality: Locality::HostLocal,
            channel_binding: [0x22; 32],
            identity_evidence: IdentityEvidenceRequirement::DirectionalUnix,
        },
        reconnect_generation: 7,
        attachment_policy: if transport == TransportClass::UnixSeqpacket {
            AttachmentPolicy {
                kind: d2b_session::contract::AttachmentPolicyKind::PacketAtomic,
                max_per_packet: 1,
                max_per_request: 1,
                max_per_operation: 1,
                max_per_session: 1,
                credentials_allowed: false,
            }
        } else {
            AttachmentPolicy::disabled()
        },
    }
}

async fn stream_engine(
    policy: &EndpointPolicy,
    initiator_socket: StreamSocket,
    responder_socket: StreamSocket,
) -> SessionEngine<UnixStreamTransport> {
    let initiator_transport = UnixStreamTransport::new(
        initiator_socket,
        policy.transport_binding.locality,
        policy.limits,
    );
    let responder_transport = UnixStreamTransport::new(
        responder_socket,
        policy.transport_binding.locality,
        policy.limits,
    );
    let (initiator, responder) = tokio::join!(
        SessionEngine::establish_initiator(
            initiator_transport,
            policy.clone(),
            HandshakeCredentials::Nn,
            Instant::now(),
        ),
        SessionEngine::establish_responder(
            responder_transport,
            policy.clone(),
            HandshakeCredentials::Nn,
            Instant::now(),
        ),
    );
    let _responder = responder.unwrap();
    initiator.unwrap()
}

fn evidence() -> TransportEvidence {
    TransportEvidence::new(
        EvidenceClass::UnixPeer,
        BindingDigest::parse(format!("sha256:{}", "22".repeat(32))).unwrap(),
    )
}

struct MappingAuthority {
    subject: Option<VerifiedUnixSubject>,
    observed_type: Arc<Mutex<Option<String>>>,
}

#[async_trait]
impl SessionAuthority for MappingAuthority {
    async fn authenticate_connect(
        &mut self,
        evidence: TransportEvidence,
        binding: &SessionAuthenticationBinding,
        expected_zone: &ZoneId,
        now_tick: u64,
    ) -> d2b_session::Result<(AuthenticatedSubjectContext, AuthorizationLease)> {
        let context = self
            .subject
            .take()
            .unwrap()
            .bind(&evidence, binding, expected_zone)?;
        *self.observed_type.lock().unwrap() =
            Some(context.subject_ref().resource_type().as_str().to_owned());
        Ok((context, AuthorizationLease::new(1, now_tick + 10).unwrap()))
    }

    async fn authorize(
        &mut self,
        _subject: &AuthenticatedSubjectContext,
        _request: &SessionAuthorizationRequest,
        _previous_lease: AuthorizationLease,
        now_tick: u64,
    ) -> d2b_session::Result<AuthorizationLease> {
        Ok(AuthorizationLease::new(1, now_tick + 10).unwrap())
    }
}

#[tokio::test]
async fn so_peercred_maps_host_and_guest_subjects() {
    for (subject_ref, expected_type) in [("Host/alice-host", "Host"), ("Guest/corp-vm", "Guest")] {
        let (left, right) = socketpair(
            AddressFamily::UNIX,
            SocketType::STREAM,
            SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
            None,
        )
        .unwrap();
        let initiator_socket = StreamSocket::from_owned(left).unwrap();
        let responder_socket = StreamSocket::from_owned(right).unwrap();
        let expected_peer = initiator_socket.acceptor_peer_credentials().unwrap();
        let subject = if expected_type == "Host" {
            UnixSubjectIdentity::host(
                ResourceRef::parse(subject_ref).unwrap(),
                ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
                ResourceRef::parse("Zone/work").unwrap(),
                expected_peer,
            )
            .unwrap()
            .verify_stream(&initiator_socket)
            .unwrap()
        } else {
            UnixSubjectIdentity::guest(
                ResourceRef::parse(subject_ref).unwrap(),
                ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
                ResourceRef::parse("Zone/work").unwrap(),
                expected_peer,
            )
            .unwrap()
            .verify_stream(&initiator_socket)
            .unwrap()
        };
        let observed_type = Arc::new(Mutex::new(None));
        let authority = MappingAuthority {
            subject: Some(subject),
            observed_type: Arc::clone(&observed_type),
        };
        let policy = endpoint_policy(TransportClass::UnixStream);
        let engine = stream_engine(&policy, initiator_socket, responder_socket).await;
        let zone = ZoneId::parse("work").unwrap();
        let _session = SessionAcceptor::new(policy.clone(), zone, Box::new(authority))
            .unwrap()
            .admit(engine, evidence(), 1)
            .await
            .unwrap();
        assert_eq!(
            observed_type.lock().unwrap().as_deref(),
            Some(expected_type)
        );
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
    let authority = MappingAuthority {
        subject: Some(subject),
        observed_type: Arc::new(Mutex::new(None)),
    };
    let policy = endpoint_policy(TransportClass::UnixStream);
    let (left, right) = socketpair(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
        None,
    )
    .unwrap();
    let engine = stream_engine(
        &policy,
        StreamSocket::from_owned(left).unwrap(),
        StreamSocket::from_owned(right).unwrap(),
    )
    .await;
    let error = SessionAcceptor::new(
        policy.clone(),
        ZoneId::parse("work").unwrap(),
        Box::new(authority),
    )
    .unwrap()
    .admit(engine, evidence(), 1)
    .await
    .unwrap_err();
    assert_eq!(
        error.code(),
        d2b_session::contract::SessionErrorCode::SubjectMismatch
    );
}
