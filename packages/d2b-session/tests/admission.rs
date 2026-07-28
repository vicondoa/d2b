use std::{collections::BTreeSet, time::Instant};

use async_trait::async_trait;
use d2b_contracts::v3::{
    AuthenticatedSubjectContext, BindingDigest, ConfigurationGeneration, EvidenceClass,
    ResourceRef, ResourceUid, SessionBinding, ZoneId, ZoneRevision,
    component_session::{
        AttachmentPolicy, AuthorizationLease, BootstrapIdentityBinding, BootstrapPskBinding,
        EndpointPolicy, EndpointPurpose, EndpointRole, IdentityEvidenceRequirement, LimitProfile,
        Locality as SessionLocality, NoiseProfile, OperationId, ServicePackage, SessionErrorCode,
        TransportBinding, TransportClass,
    },
};
use d2b_resource_api::authz::{
    ApiCatalog, AuthorizationState, BindingScope, BootstrapPhase, BoundSubject, CompiledRole,
    CompiledRoleBinding, NativeAuthorizer, PolicyRule, PolicySet, RelayGrantAuthority, SessionVerb,
};
use d2b_resource_store::PolicySnapshot;
use d2b_session::{
    BootstrapAdmission, BootstrapPsk, ComponentSessionDriver, HandshakeCredentials, OwnedTransport,
    Secret32, SessionAcceptor, SessionAuthenticationBinding, SessionAuthority,
    SessionAuthorizationRequest, SessionEngine, TransportDescriptor, TransportError,
    TransportEvidence, TransportPacket, x25519_public_key,
};
use tokio::sync::mpsc;

const POLICY_REVISION: u64 = 4;

fn endpoint_policy() -> EndpointPolicy {
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
            transport: TransportClass::UnixSeqpacket,
            locality: SessionLocality::HostLocal,
            channel_binding: [0x22; 32],
            identity_evidence: IdentityEvidenceRequirement::DirectionalUnix,
        },
        reconnect_generation: 7,
        attachment_policy: AttachmentPolicy {
            kind: d2b_session::contract::AttachmentPolicyKind::PacketAtomic,
            max_per_packet: 1,
            max_per_request: 1,
            max_per_operation: 1,
            max_per_session: 1,
            credentials_allowed: false,
        },
    }
}

fn bootstrap_policy() -> EndpointPolicy {
    EndpointPolicy {
        purpose: EndpointPurpose::Bootstrap,
        purpose_class: d2b_session::contract::PurposeClass::Bootstrap,
        initiator_role: EndpointRole::Bootstrapper,
        responder_role: EndpointRole::GuestAgent,
        service: ServicePackage::ControllerV3,
        schema_fingerprint: [0x11; 32],
        noise_profile: NoiseProfile::Ikpsk2_25519ChaChaPolySha256,
        limits: LimitProfile::local_default(),
        transport_binding: TransportBinding {
            transport: TransportClass::NativeVsock,
            locality: SessionLocality::GuestLocal,
            channel_binding: [0x22; 32],
            identity_evidence: IdentityEvidenceRequirement::ParentStaticAndSingleUsePsk,
        },
        reconnect_generation: 7,
        attachment_policy: AttachmentPolicy::disabled(),
    }
}

fn bootstrap_identity(subject: &str, zone: &str) -> BootstrapIdentityBinding {
    BootstrapIdentityBinding {
        subject_ref: ResourceRef::parse(subject).unwrap(),
        subject_uid: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
        zone: ZoneId::parse(zone).unwrap(),
        purpose: d2b_contracts::v3::SessionPurpose::parse("bootstrap").unwrap(),
    }
}

fn admitted_bootstrap(subject: &str, zone: &str) -> d2b_session::AdmittedBootstrapPsk {
    let operation = OperationId::new(vec![0x66; 16]).unwrap();
    let nonce = [0x77; 32];
    let mut admission = BootstrapAdmission::new(
        BootstrapPskBinding {
            operation_id: operation.clone(),
            replay_nonce: nonce,
            identity: bootstrap_identity(subject, zone),
            expires_at_unix_ms: 10,
        },
        BootstrapPsk::new([0x55; 32]).unwrap(),
    )
    .unwrap();
    admission
        .consume(&operation, &nonce, bootstrap_identity(subject, zone), 1)
        .unwrap()
}

struct TestTransport {
    sender: mpsc::Sender<TransportPacket>,
    receiver: mpsc::Receiver<TransportPacket>,
    descriptor: TransportDescriptor,
}

#[async_trait]
impl OwnedTransport for TestTransport {
    fn descriptor(&self) -> TransportDescriptor {
        self.descriptor
    }

    fn into_split(
        self: Box<Self>,
    ) -> (
        Box<dyn d2b_session::TransportReader>,
        Box<dyn d2b_session::TransportWriter>,
    ) {
        let Self {
            sender,
            receiver,
            descriptor: _,
        } = *self;
        (
            Box::new(TestTransportReader { receiver }),
            Box::new(TestTransportWriter { sender }),
        )
    }

    async fn receive(&mut self, protected_limit: usize) -> Result<TransportPacket, TransportError> {
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

    async fn send(&mut self, packet: TransportPacket) -> Result<(), TransportError> {
        self.sender
            .send(packet)
            .await
            .map_err(|_| TransportError::Disconnected)
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        Ok(())
    }
}

struct TestTransportReader {
    receiver: mpsc::Receiver<TransportPacket>,
}

#[async_trait]
impl d2b_session::TransportReader for TestTransportReader {
    async fn receive(&mut self, protected_limit: usize) -> Result<TransportPacket, TransportError> {
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
}

struct TestTransportWriter {
    sender: mpsc::Sender<TransportPacket>,
}

#[async_trait]
impl d2b_session::TransportWriter for TestTransportWriter {
    async fn send(&mut self, packet: TransportPacket) -> Result<(), TransportError> {
        self.sender
            .send(packet)
            .await
            .map_err(|_| TransportError::Disconnected)
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        Ok(())
    }
}

fn test_transport_pair(policy: &EndpointPolicy) -> (TestTransport, TestTransport) {
    let (left_sender, left_receiver) = mpsc::channel(16);
    let (right_sender, right_receiver) = mpsc::channel(16);
    let descriptor = TransportDescriptor {
        class: policy.transport_binding.transport,
        locality: policy.transport_binding.locality,
        packet_atomic: matches!(
            policy.transport_binding.transport,
            TransportClass::UnixSeqpacket | TransportClass::InheritedSocketpair
        ),
        supports_attachments: policy.attachment_policy != AttachmentPolicy::disabled(),
    };
    (
        TestTransport {
            sender: left_sender,
            receiver: right_receiver,
            descriptor,
        },
        TestTransport {
            sender: right_sender,
            receiver: left_receiver,
            descriptor,
        },
    )
}

async fn bootstrap_engine(
    policy: &EndpointPolicy,
    subject: &str,
    zone: &str,
) -> SessionEngine<TestTransport> {
    let initiator_static = [0x11; 32];
    let responder_static = [0x22; 32];
    let responder_public = x25519_public_key(&responder_static).unwrap();
    let (initiator_transport, responder_transport) = test_transport_pair(policy);
    let (initiator, responder) = tokio::join!(
        SessionEngine::establish_initiator(
            initiator_transport,
            policy.clone(),
            HandshakeCredentials::IkPsk2Initiator {
                local_private: Secret32::new(initiator_static).unwrap(),
                remote_public: responder_public,
                psk: admitted_bootstrap(subject, zone),
            },
            Instant::now(),
        ),
        SessionEngine::establish_responder(
            responder_transport,
            policy.clone(),
            HandshakeCredentials::IkPsk2Responder {
                local_private: Secret32::new(responder_static).unwrap(),
                psk: admitted_bootstrap("Host/responder", zone),
            },
            Instant::now(),
        ),
    );
    let _responder = responder.unwrap();
    initiator.unwrap()
}

async fn engine(policy: &EndpointPolicy) -> SessionEngine<TestTransport> {
    engine_pair(policy).await.0
}

async fn engine_pair(
    policy: &EndpointPolicy,
) -> (SessionEngine<TestTransport>, SessionEngine<TestTransport>) {
    let (initiator_transport, responder_transport) = test_transport_pair(policy);
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
    (initiator.unwrap(), responder.unwrap())
}

fn evidence() -> TransportEvidence {
    TransportEvidence::new(
        EvidenceClass::UnixPeer,
        BindingDigest::parse(format!("sha256:{}", "22".repeat(32))).unwrap(),
    )
}

fn bootstrap_evidence() -> TransportEvidence {
    TransportEvidence::new(
        EvidenceClass::BootstrapIkpsk2,
        BindingDigest::parse(format!("sha256:{}", "22".repeat(32))).unwrap(),
    )
}

struct NativeTestAuthority {
    subject_zone: ZoneId,
    allowed: BTreeSet<SessionVerb>,
    revoke_after_connect: bool,
    authorizer: Option<NativeAuthorizer>,
    state: Option<AuthorizationState>,
}

impl NativeTestAuthority {
    fn new(
        subject_zone: ZoneId,
        allowed: impl IntoIterator<Item = SessionVerb>,
        revoke_after_connect: bool,
    ) -> Self {
        Self {
            subject_zone,
            allowed: allowed.into_iter().collect(),
            revoke_after_connect,
            authorizer: None,
            state: None,
        }
    }

    fn state(revision: u64, now_tick: u64) -> AuthorizationState {
        AuthorizationState {
            snapshot: PolicySnapshot {
                policy_revision: revision,
                api_catalog_revision: 1,
                active_configuration_revision: ConfigurationGeneration::new(1).unwrap(),
                controller_generation: None,
            },
            zone_policy_revision: ZoneRevision::new(revision),
            bootstrap_phase: BootstrapPhase::Disabled,
            now_tick,
        }
    }

    fn policy(
        context: &AuthenticatedSubjectContext,
        zone: &ZoneId,
        allowed: &BTreeSet<SessionVerb>,
    ) -> PolicySet {
        let catalog = ApiCatalog::standard();
        let rule = PolicyRule::new(
            &catalog,
            [],
            [],
            allowed.iter().copied(),
            [],
            [],
            [zone.clone()],
            [],
        )
        .unwrap();
        let role = CompiledRole::new(ResourceRef::parse("Role/session-user").unwrap(), vec![rule])
            .unwrap();
        let binding = CompiledRoleBinding::new(
            role.role_ref.clone(),
            [BoundSubject {
                subject_ref: context.subject_ref().clone(),
                subject_uid: context.subject_uid().clone(),
            }],
            BindingScope::default(),
            if allowed.contains(&SessionVerb::Relay) {
                RelayGrantAuthority::DurableLocalAdmin
            } else {
                RelayGrantAuthority::None
            },
        )
        .unwrap();
        PolicySet::new(&catalog, POLICY_REVISION, vec![role], vec![binding]).unwrap()
    }
}

#[async_trait]
impl SessionAuthority for NativeTestAuthority {
    async fn authenticate_connect(
        &mut self,
        evidence: TransportEvidence,
        binding: &SessionAuthenticationBinding,
        _expected_zone: &ZoneId,
        now_tick: u64,
    ) -> d2b_session::Result<(AuthenticatedSubjectContext, AuthorizationLease)> {
        if evidence.class() != binding.evidence_class() {
            return Err(d2b_session::SessionError::new(
                SessionErrorCode::IdentityEvidenceMismatch,
            ));
        }
        let context = AuthenticatedSubjectContext::new(
            ResourceRef::parse("Host/alice-host").unwrap(),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            ResourceRef::parse(&format!("Zone/{}", self.subject_zone.as_str())).unwrap(),
            binding.evidence_class(),
            binding.purpose().clone(),
            binding.service().clone(),
            SessionBinding::new(
                binding.schema_fingerprint().clone(),
                binding.transport_binding().clone(),
                binding.reconnect_generation(),
                binding.transcript_hash().clone(),
            ),
        );
        let policy = Self::policy(&context, &self.subject_zone, &self.allowed);
        let authorizer = NativeAuthorizer::new(ApiCatalog::standard(), Some(policy)).unwrap();
        let state = Self::state(POLICY_REVISION, now_tick);
        let capabilities = authorizer
            .positive_capabilities(&context, &self.subject_zone, &state)
            .map_err(|_| d2b_session::SessionError::new(SessionErrorCode::PolicyDenied))?;
        if !capabilities.session_verbs.contains(&SessionVerb::Connect) {
            return Err(d2b_session::SessionError::new(
                SessionErrorCode::PolicyDenied,
            ));
        }
        self.authorizer = Some(authorizer);
        self.state = Some(state);
        Ok((
            context,
            AuthorizationLease::new(POLICY_REVISION, now_tick + 10).unwrap(),
        ))
    }

    async fn authorize(
        &mut self,
        subject: &AuthenticatedSubjectContext,
        request: &SessionAuthorizationRequest,
        previous_lease: AuthorizationLease,
        now_tick: u64,
    ) -> d2b_session::Result<AuthorizationLease> {
        let state = self.state.as_mut().unwrap();
        state.now_tick = now_tick;
        if self.revoke_after_connect {
            state.snapshot.policy_revision += 1;
            state.zone_policy_revision = ZoneRevision::new(state.snapshot.policy_revision);
        }
        let capabilities = self
            .authorizer
            .as_ref()
            .unwrap()
            .positive_capabilities(subject, &self.subject_zone, state)
            .map_err(|_| d2b_session::SessionError::new(SessionErrorCode::PolicyDenied))?;
        if previous_lease.policy_revision() != POLICY_REVISION
            || !capabilities.session_verbs.contains(&request.verb())
        {
            return Err(d2b_session::SessionError::new(
                SessionErrorCode::PolicyDenied,
            ));
        }
        AuthorizationLease::new(POLICY_REVISION, now_tick + 10)
            .map_err(d2b_session::SessionError::from)
    }
}

#[tokio::test]
async fn native_rbac_connect_and_invoke_are_executed() {
    let zone = ZoneId::parse("work").unwrap();
    let authority = NativeTestAuthority::new(
        zone.clone(),
        [SessionVerb::Connect, SessionVerb::Invoke],
        false,
    );
    let policy = endpoint_policy();
    let acceptor =
        SessionAcceptor::new(policy.clone(), zone.clone(), Box::new(authority), ()).unwrap();
    let mut session = acceptor
        .admit(engine(&policy).await, evidence(), 1)
        .await
        .unwrap();
    let request = SessionAuthorizationRequest::new(
        SessionVerb::Invoke,
        d2b_contracts::v3::ServiceName::parse("d2b.resource.v3").unwrap(),
        "ResourceService/Get",
        zone,
        Some(ResourceRef::parse("Process/app").unwrap()),
    )
    .unwrap();
    let permit = session.authorize(request, 2).await.unwrap();
    assert_eq!(permit.policy_revision(), POLICY_REVISION);
    assert_eq!(permit.request().verb(), SessionVerb::Invoke);
}

#[tokio::test]
async fn admitted_session_retains_transport_and_consumes_send_permits() {
    let zone = ZoneId::parse("work").unwrap();
    let authority = NativeTestAuthority::new(
        zone.clone(),
        [
            SessionVerb::Connect,
            SessionVerb::Invoke,
            SessionVerb::Observe,
        ],
        false,
    );
    let policy = endpoint_policy();
    let (initiator, responder) = engine_pair(&policy).await;
    let mut session = SessionAcceptor::new(policy, zone.clone(), Box::new(authority), ())
        .unwrap()
        .admit(initiator, evidence(), 1)
        .await
        .unwrap();
    let responder = responder.into_driver();
    let observe = session
        .authorize(
            SessionAuthorizationRequest::new(
                SessionVerb::Observe,
                d2b_contracts::v3::ServiceName::parse("d2b.resource.v3").unwrap(),
                "ResourceService/Watch",
                zone.clone(),
                None,
            )
            .unwrap(),
            2,
        )
        .await
        .unwrap();
    assert_eq!(
        session
            .send_authorized_ttrpc(observe, b"not-an-invoke".to_vec(), 2)
            .await
            .unwrap_err()
            .code(),
        SessionErrorCode::PolicyDenied
    );

    let expired = session
        .authorize(
            SessionAuthorizationRequest::new(
                SessionVerb::Invoke,
                d2b_contracts::v3::ServiceName::parse("d2b.resource.v3").unwrap(),
                "ResourceService/Get",
                zone.clone(),
                None,
            )
            .unwrap(),
            3,
        )
        .await
        .unwrap();
    assert_eq!(expired.expires_at_tick(), 13);
    assert_eq!(
        session
            .send_authorized_ttrpc(expired, b"expired-frame".to_vec(), 13)
            .await
            .unwrap_err()
            .code(),
        SessionErrorCode::PolicyDenied
    );

    let invoke = session
        .authorize(
            SessionAuthorizationRequest::new(
                SessionVerb::Invoke,
                d2b_contracts::v3::ServiceName::parse("d2b.resource.v3").unwrap(),
                "ResourceService/Get",
                zone,
                None,
            )
            .unwrap(),
            4,
        )
        .await
        .unwrap();
    session
        .send_authorized_ttrpc(invoke, b"authorized-frame".to_vec(), 4)
        .await
        .unwrap();
    assert_eq!(
        responder.receive_ttrpc().await.unwrap(),
        b"authorized-frame"
    );
    responder
        .send_ttrpc(b"inbound-frame".to_vec())
        .await
        .unwrap();
    assert_eq!(session.receive_ttrpc().await.unwrap(), b"inbound-frame");
}

#[tokio::test]
async fn cross_zone_subject_fails_before_connect_mint() {
    let expected_zone = ZoneId::parse("work").unwrap();
    let authority = NativeTestAuthority::new(
        ZoneId::parse("personal").unwrap(),
        [SessionVerb::Connect],
        false,
    );
    let policy = endpoint_policy();
    let error = SessionAcceptor::new(policy.clone(), expected_zone, Box::new(authority), ())
        .unwrap()
        .admit(engine(&policy).await, evidence(), 1)
        .await
        .unwrap_err();
    assert_eq!(error.code(), SessionErrorCode::SubjectMismatch);
}

#[tokio::test]
async fn bootstrap_identity_is_consumed_through_handshake_and_session_admission() {
    let zone = ZoneId::parse("work").unwrap();
    let policy = bootstrap_policy();
    let authority = NativeTestAuthority::new(zone.clone(), [SessionVerb::Connect], false);
    SessionAcceptor::new(policy.clone(), zone.clone(), Box::new(authority), ())
        .unwrap()
        .admit(
            bootstrap_engine(&policy, "Host/alice-host", "work").await,
            bootstrap_evidence(),
            1,
        )
        .await
        .unwrap();

    let authority = NativeTestAuthority::new(zone.clone(), [SessionVerb::Connect], false);
    assert_eq!(
        SessionAcceptor::new(policy.clone(), zone.clone(), Box::new(authority), ())
            .unwrap()
            .admit(
                bootstrap_engine(&policy, "Guest/corp-vm", "work").await,
                bootstrap_evidence(),
                1,
            )
            .await
            .unwrap_err()
            .code(),
        SessionErrorCode::SubjectMismatch
    );

    let authority = NativeTestAuthority::new(zone.clone(), [SessionVerb::Connect], false);
    assert_eq!(
        SessionAcceptor::new(policy.clone(), zone, Box::new(authority), ())
            .unwrap()
            .admit(
                bootstrap_engine(&policy, "Host/alice-host", "personal").await,
                bootstrap_evidence(),
                1,
            )
            .await
            .unwrap_err()
            .code(),
        SessionErrorCode::SubjectMismatch
    );
}

#[tokio::test]
async fn policy_revision_change_revokes_new_work() {
    let zone = ZoneId::parse("work").unwrap();
    let authority = NativeTestAuthority::new(
        zone.clone(),
        [SessionVerb::Connect, SessionVerb::Invoke],
        true,
    );
    let policy = endpoint_policy();
    let mut session = SessionAcceptor::new(policy.clone(), zone.clone(), Box::new(authority), ())
        .unwrap()
        .admit(engine(&policy).await, evidence(), 1)
        .await
        .unwrap();
    let request = SessionAuthorizationRequest::new(
        SessionVerb::Invoke,
        d2b_contracts::v3::ServiceName::parse("d2b.resource.v3").unwrap(),
        "ResourceService/Get",
        zone,
        None,
    )
    .unwrap();
    assert_eq!(
        session.authorize(request, 2).await.unwrap_err().code(),
        SessionErrorCode::PolicyDenied
    );
}

#[tokio::test]
async fn native_rbac_relay_mints_only_for_a_distinct_next_zone() {
    let zone = ZoneId::parse("work").unwrap();
    let authority = NativeTestAuthority::new(
        zone.clone(),
        [SessionVerb::Connect, SessionVerb::Relay],
        false,
    );
    let policy = endpoint_policy();
    let mut session = SessionAcceptor::new(policy.clone(), zone.clone(), Box::new(authority), ())
        .unwrap()
        .admit(engine(&policy).await, evidence(), 1)
        .await
        .unwrap();
    let invalid = SessionAuthorizationRequest::relay(
        d2b_contracts::v3::ServiceName::parse("d2b.resource.v3").unwrap(),
        "ResourceService/Get",
        ZoneId::parse("personal").unwrap(),
        None,
        SessionVerb::Invoke,
        zone,
    )
    .unwrap();
    assert_eq!(
        session.authorize(invalid, 2).await.unwrap_err().code(),
        SessionErrorCode::PolicyDenied
    );

    let valid = SessionAuthorizationRequest::relay(
        d2b_contracts::v3::ServiceName::parse("d2b.resource.v3").unwrap(),
        "ResourceService/Get",
        ZoneId::parse("personal").unwrap(),
        None,
        SessionVerb::Invoke,
        ZoneId::parse("gateway").unwrap(),
    )
    .unwrap();
    assert_eq!(
        session.authorize(valid, 2).await.unwrap().request().verb(),
        SessionVerb::Relay
    );
}

#[tokio::test]
async fn forged_evidence_class_is_rejected_before_authority() {
    let zone = ZoneId::parse("work").unwrap();
    let authority = NativeTestAuthority::new(zone.clone(), [SessionVerb::Connect], false);
    let policy = endpoint_policy();
    let forged = TransportEvidence::new(
        EvidenceClass::EnrolledKk,
        BindingDigest::parse(format!("sha256:{}", "22".repeat(32))).unwrap(),
    );
    let error = SessionAcceptor::new(policy.clone(), zone, Box::new(authority), ())
        .unwrap()
        .admit(engine(&policy).await, forged, 1)
        .await
        .unwrap_err();
    assert_eq!(error.code(), SessionErrorCode::IdentityEvidenceMismatch);
}

#[test]
fn authorization_request_enforces_relay_and_diagnostic_bindings() {
    let local = ZoneId::parse("work").unwrap();
    let remote = ZoneId::parse("personal").unwrap();
    let next_hop = ZoneId::parse("gateway").unwrap();
    let service = d2b_contracts::v3::ServiceName::parse("d2b.resource.v3").unwrap();
    let relay = SessionAuthorizationRequest::relay(
        service.clone(),
        "ResourceService/Get",
        remote.clone(),
        None,
        SessionVerb::Invoke,
        next_hop.clone(),
    )
    .unwrap();
    assert_eq!(relay.target_zone(), &remote);
    assert_eq!(relay.next_hop_zone(), Some(&next_hop));
    assert_eq!(relay.forwarded_target_verb(), Some(SessionVerb::Invoke));

    assert_eq!(
        SessionAuthorizationRequest::relay(
            service,
            "ResourceService/Get",
            remote,
            None,
            SessionVerb::Relay,
            next_hop,
        )
        .unwrap_err()
        .code(),
        SessionErrorCode::PolicyDenied
    );
    assert_eq!(
        SessionAuthorizationRequest::new(
            SessionVerb::AuditExport,
            d2b_contracts::v3::ServiceName::parse("d2b.resource.v3").unwrap(),
            "AuditService/Export",
            local.clone(),
            None,
        )
        .unwrap_err()
        .code(),
        SessionErrorCode::PolicyDenied
    );
    SessionAuthorizationRequest::new(
        SessionVerb::AuditExport,
        d2b_contracts::v3::ServiceName::parse("d2b.audit.v3").unwrap(),
        "AuditService/Export",
        local.clone(),
        None,
    )
    .unwrap();
    SessionAuthorizationRequest::new(
        SessionVerb::SupportBundle,
        d2b_contracts::v3::ServiceName::parse("d2b.support.v3").unwrap(),
        "SupportService/GenerateBundle",
        local,
        None,
    )
    .unwrap();
}
