use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Instant,
};

use async_trait::async_trait;
use d2b_bus::{
    BusAuthorizer, BusConfig, BusError, ComponentSessionAdmission, OperationId, OperationSpec,
    ResourceCall, RouteGenerations, RouteKey, RouteMember, RouteTarget, UnixSubjectConfig, ZoneBus,
    ZoneRegistrar,
};
use d2b_contracts::v3::{
    BindingDigest, ConfigurationGeneration, ControllerGeneration, EvidenceClass,
    ResourceGeneration, ResourceRef, ResourceUid, SchemaFingerprint, ServiceName, ZoneId,
    ZoneRevision,
    component_session::{
        AttachmentPolicy, CloseReason, EndpointPolicy, EndpointPurpose, EndpointRole,
        IdentityEvidenceRequirement, LimitProfile, Locality, NoiseProfile, PurposeClass,
        Remediation, ServicePackage, TransportBinding, TransportClass,
    },
};
use d2b_resource_api::authz::{
    ApiCatalog, AuthorizationState, BindingScope, BootstrapPhase, BoundSubject, CompiledRole,
    CompiledRoleBinding, NativeAuthorizer, PolicyRule, PolicySet, RelayGrantAuthority,
    ResourceVerb, SessionVerb,
};
use d2b_resource_store::PolicySnapshot;
use d2b_session::{
    AuthenticatedComponentSession, ComponentSessionDriver, HandshakeCredentials, OwnedTransport,
    SessionDriverHandle, SessionEngine, TransportDescriptor, TransportError, TransportEvidence,
    TransportPacket, ttrpc_request_id, ttrpc_stream_id,
};
use d2b_session_unix::{SeqpacketSocket, VerifiedUnixPeer, prearmed_seqpacket_pair};
use tokio::sync::{Notify, mpsc};

const PROVIDER_GENERATION: u64 = 2;
const CONTROLLER_GENERATION: u64 = 3;

struct TestTransport {
    sender: mpsc::Sender<TransportPacket>,
    receiver: mpsc::Receiver<TransportPacket>,
    descriptor: TransportDescriptor,
    writer_pause: Option<Arc<WriterPause>>,
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
            writer_pause,
        } = *self;
        (
            Box::new(TestTransportReader { receiver }),
            Box::new(TestTransportWriter {
                sender,
                writer_pause,
            }),
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
    writer_pause: Option<Arc<WriterPause>>,
}

#[async_trait]
impl d2b_session::TransportWriter for TestTransportWriter {
    async fn send(&mut self, packet: TransportPacket) -> Result<(), TransportError> {
        if let Some(pause) = &self.writer_pause {
            if !pause.entered.swap(true, Ordering::AcqRel) {
                pause.notify.notify_waiters();
                pause.release.notified().await;
            }
            pause.sent.fetch_add(1, Ordering::AcqRel);
        }
        self.sender
            .send(packet)
            .await
            .map_err(|_| TransportError::Disconnected)
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        Ok(())
    }
}

#[derive(Default)]
struct WriterPause {
    entered: AtomicBool,
    sent: AtomicUsize,
    notify: Notify,
    release: Notify,
}

impl WriterPause {
    async fn wait_until_entered(&self) {
        loop {
            let notified = self.notify.notified();
            if self.entered.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
}

fn policy(service: ServicePackage, purpose: EndpointPurpose, generation: u64) -> EndpointPolicy {
    EndpointPolicy {
        purpose,
        purpose_class: PurposeClass::Local,
        initiator_role: EndpointRole::ZoneController,
        responder_role: EndpointRole::Component,
        service,
        schema_fingerprint: [0x11; 32],
        noise_profile: NoiseProfile::Nn25519ChaChaPolySha256,
        limits: LimitProfile::local_default(),
        transport_binding: TransportBinding {
            transport: TransportClass::UnixSeqpacket,
            locality: Locality::HostLocal,
            channel_binding: [0x22; 32],
            identity_evidence: IdentityEvidenceRequirement::DirectionalUnix,
        },
        reconnect_generation: generation,
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

async fn admit(
    registrar: &ZoneRegistrar,
    policy: EndpointPolicy,
    subject: &str,
    uid: &str,
    provider: &str,
    _allowed: impl IntoIterator<Item = SessionVerb>,
) -> (
    AuthenticatedComponentSession<ComponentSessionAdmission>,
    SessionDriverHandle,
    tokio::task::JoinHandle<()>,
) {
    admit_with_writer_pause(registrar, policy, subject, uid, provider, _allowed, None).await
}

async fn admit_with_writer_pause(
    registrar: &ZoneRegistrar,
    policy: EndpointPolicy,
    subject: &str,
    uid: &str,
    provider: &str,
    _allowed: impl IntoIterator<Item = SessionVerb>,
    writer_pause: Option<Arc<WriterPause>>,
) -> (
    AuthenticatedComponentSession<ComponentSessionAdmission>,
    SessionDriverHandle,
    tokio::task::JoinHandle<()>,
) {
    let descriptor = TransportDescriptor {
        class: policy.transport_binding.transport,
        locality: policy.transport_binding.locality,
        packet_atomic: true,
        supports_attachments: true,
    };
    let (left_sender, left_receiver) = mpsc::channel(16);
    let (right_sender, right_receiver) = mpsc::channel(16);
    let left = TestTransport {
        sender: left_sender,
        receiver: right_receiver,
        descriptor,
        writer_pause,
    };
    let right = TestTransport {
        sender: right_sender,
        receiver: left_receiver,
        descriptor,
        writer_pause: None,
    };
    let (initiator, responder) = tokio::join!(
        SessionEngine::establish_initiator(
            left,
            policy.clone(),
            HandshakeCredentials::Nn,
            Instant::now(),
        ),
        SessionEngine::establish_responder(
            right,
            policy.clone(),
            HandshakeCredentials::Nn,
            Instant::now(),
        ),
    );
    let remote = responder.unwrap().into_driver();
    let echo_remote = remote.clone();
    let echo = tokio::spawn(async move {
        while let Ok(frame) = echo_remote.receive_ttrpc().await {
            if echo_remote.send_ttrpc(frame).await.is_err() {
                break;
            }
        }
    });
    let subject_ref = ResourceRef::parse(subject).unwrap();
    let subject_uid = ResourceUid::parse(uid).unwrap();
    let provider_ref = ResourceRef::parse(provider).unwrap();
    let provider_generation = ResourceGeneration::new(PROVIDER_GENERATION).unwrap();
    let (proof_fd, _peer_fd) = prearmed_seqpacket_pair().unwrap();
    let proof_socket = SeqpacketSocket::from_parent_prearmed(proof_fd).unwrap();
    let expected_peer = proof_socket.acceptor_peer_credentials().unwrap();
    let subject_config = if subject_ref.resource_type().as_str() == "Provider" {
        UnixSubjectConfig::provider(
            subject_ref,
            subject_uid,
            ResourceRef::parse("Zone/dev").unwrap(),
            expected_peer,
            provider_generation,
        )
        .unwrap()
    } else {
        UnixSubjectConfig::host(
            subject_ref,
            subject_uid,
            ResourceRef::parse("Zone/dev").unwrap(),
            expected_peer,
        )
        .unwrap()
        .with_provider(provider_ref, provider_generation)
        .unwrap()
    }
    .with_controller_generation(ControllerGeneration::new(CONTROLLER_GENERATION).unwrap());
    registrar.register_unix_subject(subject_config).unwrap();
    let verified_peer = VerifiedUnixPeer::verify_seqpacket(&proof_socket).unwrap();
    let session = registrar
        .component_session_acceptor(policy, verified_peer)
        .unwrap()
        .admit(
            initiator.unwrap(),
            TransportEvidence::new(
                EvidenceClass::UnixPeer,
                BindingDigest::parse(format!("sha256:{}", "22".repeat(32))).unwrap(),
            ),
            1,
        )
        .await
        .unwrap();
    (session, remote, echo)
}

fn bus() -> (ZoneBus, d2b_bus::ZoneRegistrar) {
    bus_with_config(BusConfig::default())
}

fn bus_with_config(config: BusConfig) -> (ZoneBus, d2b_bus::ZoneRegistrar) {
    let catalog = ApiCatalog::standard();
    let zone = ZoneId::parse("dev").unwrap();
    let rule = PolicyRule::new(
        &catalog,
        [d2b_contracts::v3::ResourceTypeName::parse("Host").unwrap()],
        [ResourceVerb::Get],
        [
            SessionVerb::Connect,
            SessionVerb::Invoke,
            SessionVerb::Cancel,
            SessionVerb::AuditExport,
            SessionVerb::SupportBundle,
        ],
        [],
        [],
        [zone.clone()],
        [],
    )
    .unwrap();
    let role =
        CompiledRole::new(ResourceRef::parse("Role/session-seam").unwrap(), vec![rule]).unwrap();
    let subjects = [
        (
            "Provider/system-core",
            "11111111-1111-4111-8111-111111111111",
        ),
        ("Host/alice", "22222222-2222-4222-8222-222222222222"),
        ("Provider/audit", "33333333-3333-4333-8333-333333333333"),
        ("Host/bob", "44444444-4444-4444-8444-444444444444"),
    ]
    .into_iter()
    .map(|(subject, uid)| BoundSubject {
        subject_ref: ResourceRef::parse(subject).unwrap(),
        subject_uid: ResourceUid::parse(uid).unwrap(),
    });
    let binding = CompiledRoleBinding::new(
        role.role_ref.clone(),
        subjects,
        BindingScope::default(),
        RelayGrantAuthority::None,
    )
    .unwrap();
    let policy_set = PolicySet::new(&catalog, 1, vec![role], vec![binding]).unwrap();
    let native = NativeAuthorizer::new(catalog, Some(policy_set)).unwrap();
    let state = AuthorizationState {
        snapshot: PolicySnapshot {
            policy_revision: 1,
            api_catalog_revision: 1,
            active_configuration_revision: ConfigurationGeneration::new(1).unwrap(),
            controller_generation: Some(ControllerGeneration::new(CONTROLLER_GENERATION).unwrap()),
        },
        zone_policy_revision: ZoneRevision::new(1),
        bootstrap_phase: BootstrapPhase::Disabled,
        now_tick: 1,
    };
    ZoneBus::new(zone, BusAuthorizer::new(native, state).unwrap(), config).unwrap()
}

#[tokio::test]
async fn verified_peer_evidence_cannot_author_a_subject_without_registrar_state() {
    let (_bus, registrar) = bus();
    let (proof_fd, _peer_fd) = prearmed_seqpacket_pair().unwrap();
    let proof_socket = SeqpacketSocket::from_parent_prearmed(proof_fd).unwrap();
    let verified_peer = VerifiedUnixPeer::verify_seqpacket(&proof_socket).unwrap();
    let error = match registrar.component_session_acceptor(
        policy(
            ServicePackage::ResourceV3,
            EndpointPurpose::ResourceService,
            1,
        ),
        verified_peer,
    ) {
        Ok(_) => panic!("unmapped peer evidence minted a session acceptor"),
        Err(error) => error,
    };
    assert_eq!(
        error.code(),
        d2b_session::contract::SessionErrorCode::SubjectMismatch
    );
}

#[tokio::test]
async fn registrar_rejects_a_session_minted_for_another_bus_instance() {
    let (_first_bus, first_registrar) = bus();
    let (_second_bus, mut second_registrar) = bus();
    let (candidate, _remote, echo) = admit(
        &first_registrar,
        policy(
            ServicePackage::ResourceV3,
            EndpointPurpose::ResourceService,
            1,
        ),
        "Provider/system-core",
        "11111111-1111-4111-8111-111111111111",
        "Provider/system-core",
        [SessionVerb::Connect],
    )
    .await;

    let error = second_registrar
        .register_component_session(candidate)
        .await
        .unwrap_err();
    assert!(matches!(error, BusError::SessionMismatch));
    echo.abort();
}

fn route(service: &str, member: &str, generation: u64, provider: &str) -> RouteKey {
    RouteKey::new(
        ZoneId::parse("dev").unwrap(),
        ServiceName::parse(service).unwrap(),
        RouteMember::method(member).unwrap(),
        RouteTarget::provider(ResourceRef::parse(provider).unwrap()).unwrap(),
        SchemaFingerprint::parse(format!("sha256:{}", "11".repeat(32))).unwrap(),
        RouteGenerations::new(
            Some(ResourceGeneration::new(PROVIDER_GENERATION).unwrap()),
            Some(ControllerGeneration::new(CONTROLLER_GENERATION).unwrap()),
            d2b_contracts::v3::ReconnectGeneration::new(generation).unwrap(),
        ),
    )
}

fn ttrpc_frame(stream_id: u32, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(10 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&stream_id.to_be_bytes());
    frame.extend_from_slice(&[0, 0]);
    frame.extend_from_slice(payload);
    frame
}

#[tokio::test]
async fn admitted_sessions_route_resource_and_diagnostic_calls_and_revoke_lifecycle() {
    let (bus, mut registrar) = bus();
    let (resource_endpoint, _, resource_echo) = admit(
        &registrar,
        policy(
            ServicePackage::ResourceV3,
            EndpointPurpose::ResourceService,
            1,
        ),
        "Provider/system-core",
        "11111111-1111-4111-8111-111111111111",
        "Provider/system-core",
        [SessionVerb::Invoke],
    )
    .await;
    let resource_endpoint = registrar
        .register_component_session(resource_endpoint)
        .await
        .unwrap();
    let (resource_caller, _, caller_echo) = admit(
        &registrar,
        policy(
            ServicePackage::ResourceV3,
            EndpointPurpose::ResourceService,
            1,
        ),
        "Host/alice",
        "22222222-2222-4222-8222-222222222222",
        "Provider/system-core",
        [SessionVerb::Invoke],
    )
    .await;
    let resource_caller = registrar
        .register_component_session(resource_caller)
        .await
        .unwrap();
    let resource_route = route(
        "d2b.resource.v3",
        "ResourceService/Get",
        1,
        "Provider/system-core",
    );
    let response = resource_caller
        .invoke_resource(
            resource_route,
            OperationSpec::new(OperationId::parse("resource-get").unwrap(), 10_000).unwrap(),
            ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
            ttrpc_frame(1, b"resource"),
        )
        .await
        .unwrap();
    assert!(response.as_bytes().ends_with(b"resource"));
    let (reconnected_endpoint, _, reconnected_echo) = admit(
        &registrar,
        policy(
            ServicePackage::ResourceV3,
            EndpointPurpose::ResourceService,
            2,
        ),
        "Provider/system-core",
        "11111111-1111-4111-8111-111111111111",
        "Provider/system-core",
        [SessionVerb::Invoke],
    )
    .await;
    let resource_endpoint = registrar
        .reconnect_component_session(resource_endpoint, reconnected_endpoint)
        .await
        .unwrap();
    let (reconnected_caller, _, reconnected_caller_echo) = admit(
        &registrar,
        policy(
            ServicePackage::ResourceV3,
            EndpointPurpose::ResourceService,
            2,
        ),
        "Host/alice",
        "22222222-2222-4222-8222-222222222222",
        "Provider/system-core",
        [SessionVerb::Invoke],
    )
    .await;
    let resource_caller = registrar
        .reconnect_component_session(resource_caller, reconnected_caller)
        .await
        .unwrap();
    let reconnected_route = route(
        "d2b.resource.v3",
        "ResourceService/Get",
        2,
        "Provider/system-core",
    );
    let response = resource_caller
        .invoke_resource(
            reconnected_route.clone(),
            OperationSpec::new(
                OperationId::parse("resource-after-reconnect").unwrap(),
                10_000,
            )
            .unwrap(),
            ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
            ttrpc_frame(2, b"reconnected"),
        )
        .await
        .unwrap();
    assert!(response.as_bytes().ends_with(b"reconnected"));

    let (audit_endpoint, _, audit_echo) = admit(
        &registrar,
        policy(ServicePackage::AuditV3, EndpointPurpose::AuditExport, 1),
        "Provider/audit",
        "33333333-3333-4333-8333-333333333333",
        "Provider/audit",
        [SessionVerb::AuditExport],
    )
    .await;
    let audit_endpoint = registrar
        .register_component_session(audit_endpoint)
        .await
        .unwrap();
    let (audit_caller, _, audit_caller_echo) = admit(
        &registrar,
        policy(ServicePackage::AuditV3, EndpointPurpose::AuditExport, 1),
        "Host/bob",
        "44444444-4444-4444-8444-444444444444",
        "Provider/audit",
        [SessionVerb::AuditExport],
    )
    .await;
    let audit_caller = registrar
        .register_component_session(audit_caller)
        .await
        .unwrap();
    let audit_route = route("d2b.audit.v3", "AuditService/Export", 1, "Provider/audit");
    let response = audit_caller
        .invoke(
            audit_route.clone(),
            OperationSpec::new(OperationId::parse("audit-export").unwrap(), 10_000).unwrap(),
            ttrpc_frame(3, b"diagnostic"),
        )
        .await
        .unwrap();
    assert!(response.as_bytes().ends_with(b"diagnostic"));

    registrar
        .disconnect_component_session(audit_endpoint)
        .await
        .unwrap();
    let error = audit_caller
        .invoke(
            audit_route,
            OperationSpec::new(OperationId::parse("audit-after-close").unwrap(), 10_000).unwrap(),
            b"closed".to_vec(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        d2b_bus::BusError::Registry(d2b_bus::registry::RegistryError::RouteNotFound)
    ));

    bus.mark_policy_unavailable();
    assert!(matches!(
        resource_caller
            .invoke_resource(
                reconnected_route,
                OperationSpec::new(
                    OperationId::parse("resource-policy-outage").unwrap(),
                    10_000,
                )
                .unwrap(),
                ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
                Vec::new(),
            )
            .await,
        Err(d2b_bus::BusError::Authorization(
            d2b_bus::AuthorizationError::Native(
                d2b_resource_api::authz::AuthorizationDenial::PolicyUnavailable
            )
        ))
    ));

    registrar
        .disconnect_component_session(resource_endpoint)
        .await
        .unwrap();
    drop((resource_caller, audit_caller));
    for task in [
        resource_echo,
        reconnected_echo,
        caller_echo,
        reconnected_caller_echo,
        audit_echo,
        audit_caller_echo,
    ] {
        task.abort();
    }
}

#[tokio::test]
async fn cancelled_stream_id_reuse_rejects_the_late_response() {
    let (_bus, mut registrar) = bus_with_config(BusConfig {
        max_correlations_per_generation: 2,
        ..BusConfig::default()
    });
    let (endpoint, remote, endpoint_echo) = admit(
        &registrar,
        policy(
            ServicePackage::ResourceV3,
            EndpointPurpose::ResourceService,
            1,
        ),
        "Provider/system-core",
        "11111111-1111-4111-8111-111111111111",
        "Provider/system-core",
        [SessionVerb::Invoke],
    )
    .await;
    endpoint_echo.abort();
    let endpoint = registrar
        .register_component_session(endpoint)
        .await
        .unwrap();
    let (caller, _, caller_echo) = admit(
        &registrar,
        policy(
            ServicePackage::ResourceV3,
            EndpointPurpose::ResourceService,
            1,
        ),
        "Host/alice",
        "22222222-2222-4222-8222-222222222222",
        "Provider/system-core",
        [SessionVerb::Invoke, SessionVerb::Cancel],
    )
    .await;
    let caller = registrar.register_component_session(caller).await.unwrap();
    let route = route(
        "d2b.resource.v3",
        "ResourceService/Get",
        1,
        "Provider/system-core",
    );
    let first_id = OperationId::parse("reuse-first").unwrap();
    let second_id = OperationId::parse("reuse-second").unwrap();
    let first = caller.invoke_resource(
        route.clone(),
        OperationSpec::new(first_id.clone(), 10_000).unwrap(),
        ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
        ttrpc_frame(7, b"first"),
    );
    let sequence = async {
        let first_frame = remote.receive_ttrpc().await.unwrap();
        let first_internal_id = ttrpc_stream_id(&first_frame).unwrap();
        caller.cancel(&first_id).await.unwrap();
        let second = caller.invoke_resource(
            route.clone(),
            OperationSpec::new(second_id, 10_000).unwrap(),
            ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
            ttrpc_frame(7, b"second"),
        );
        let responses = async {
            let second_frame = remote.receive_ttrpc().await.unwrap();
            let second_internal_id = ttrpc_stream_id(&second_frame).unwrap();
            assert_ne!(first_internal_id, second_internal_id);
            remote
                .send_ttrpc(ttrpc_frame(first_internal_id, b"late-first"))
                .await
                .unwrap();
            remote
                .send_ttrpc(ttrpc_frame(second_internal_id, b"second-response"))
                .await
                .unwrap();
        };
        let (second, ()) = tokio::join!(second, responses);
        second
    };
    let (first, second) = tokio::join!(first, sequence);
    assert_eq!(first, Err(BusError::Cancelled));
    let second = second.unwrap();
    assert_eq!(ttrpc_stream_id(second.as_bytes()).unwrap(), 7);
    assert!(second.as_bytes().ends_with(b"second-response"));
    let exhausted = caller
        .invoke_resource(
            route,
            OperationSpec::new(OperationId::parse("requires-reconnect").unwrap(), 10_000).unwrap(),
            ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
            ttrpc_frame(7, b"third"),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        exhausted,
        BusError::Endpoint(d2b_bus::EndpointError::Session(failure))
            if failure.code()
                == d2b_session::contract::SessionErrorCode::SessionDisconnected
    ));

    registrar
        .disconnect_component_session(endpoint)
        .await
        .unwrap();
    caller_echo.abort();
}

#[tokio::test]
async fn preseeded_counter_response_is_not_accepted_by_a_later_invocation() {
    let (_bus, mut registrar) = bus();
    let (endpoint, remote, endpoint_echo) = admit(
        &registrar,
        policy(
            ServicePackage::ResourceV3,
            EndpointPurpose::ResourceService,
            1,
        ),
        "Provider/system-core",
        "11111111-1111-4111-8111-111111111111",
        "Provider/system-core",
        [SessionVerb::Invoke],
    )
    .await;
    endpoint_echo.abort();
    let endpoint = registrar
        .register_component_session(endpoint)
        .await
        .unwrap();
    let (caller, _, caller_echo) = admit(
        &registrar,
        policy(
            ServicePackage::ResourceV3,
            EndpointPurpose::ResourceService,
            1,
        ),
        "Host/alice",
        "22222222-2222-4222-8222-222222222222",
        "Provider/system-core",
        [SessionVerb::Invoke],
    )
    .await;
    let caller = registrar.register_component_session(caller).await.unwrap();
    remote
        .send_ttrpc(ttrpc_frame(1, b"preseeded-counter-response"))
        .await
        .unwrap();

    let invocation = caller.invoke_resource(
        route(
            "d2b.resource.v3",
            "ResourceService/Get",
            1,
            "Provider/system-core",
        ),
        OperationSpec::new(
            OperationId::parse("unpredictable-correlation").unwrap(),
            10_000,
        )
        .unwrap(),
        ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
        ttrpc_frame(7, b"request"),
    );
    let response = async {
        let request = remote.receive_ttrpc().await.unwrap();
        let internal_id = ttrpc_stream_id(&request).unwrap();
        assert_ne!(internal_id, 1);
        remote
            .send_ttrpc(ttrpc_frame(internal_id, b"genuine-response"))
            .await
            .unwrap();
    };
    let (result, ()) = tokio::join!(invocation, response);
    let result = result.unwrap();
    assert!(result.as_bytes().ends_with(b"genuine-response"));

    registrar
        .disconnect_component_session(endpoint)
        .await
        .unwrap();
    caller_echo.abort();
}

#[tokio::test]
async fn concurrent_invocations_dispatch_out_of_order_responses() {
    let (_bus, mut registrar) = bus();
    let (endpoint, remote, endpoint_echo) = admit(
        &registrar,
        policy(
            ServicePackage::ResourceV3,
            EndpointPurpose::ResourceService,
            1,
        ),
        "Provider/system-core",
        "11111111-1111-4111-8111-111111111111",
        "Provider/system-core",
        [SessionVerb::Invoke],
    )
    .await;
    endpoint_echo.abort();
    let endpoint = registrar
        .register_component_session(endpoint)
        .await
        .unwrap();
    let (caller, _, caller_echo) = admit(
        &registrar,
        policy(
            ServicePackage::ResourceV3,
            EndpointPurpose::ResourceService,
            1,
        ),
        "Host/alice",
        "22222222-2222-4222-8222-222222222222",
        "Provider/system-core",
        [SessionVerb::Invoke],
    )
    .await;
    let caller = registrar.register_component_session(caller).await.unwrap();
    let invoke = |id: &str, stream_id, payload: &'static [u8]| {
        caller.invoke_resource(
            route(
                "d2b.resource.v3",
                "ResourceService/Get",
                1,
                "Provider/system-core",
            ),
            OperationSpec::new(OperationId::parse(id).unwrap(), 10_000).unwrap(),
            ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
            ttrpc_frame(stream_id, payload),
        )
    };
    let first = invoke("concurrent-first", 7, b"first-request");
    let second = invoke("concurrent-second", 8, b"second-request");
    let respond = async {
        let one = remote.receive_ttrpc().await.unwrap();
        let two = remote.receive_ttrpc().await.unwrap();
        let (first_id, second_id) = if one.ends_with(b"first-request") {
            (
                ttrpc_stream_id(&one).unwrap(),
                ttrpc_stream_id(&two).unwrap(),
            )
        } else {
            (
                ttrpc_stream_id(&two).unwrap(),
                ttrpc_stream_id(&one).unwrap(),
            )
        };
        remote
            .send_ttrpc(ttrpc_frame(second_id, b"second-response"))
            .await
            .unwrap();
        remote
            .send_ttrpc(ttrpc_frame(first_id, b"first-response"))
            .await
            .unwrap();
    };
    let (first, second, ()) = tokio::join!(first, second, respond);
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(ttrpc_stream_id(first.as_bytes()).unwrap(), 7);
    assert_eq!(ttrpc_stream_id(second.as_bytes()).unwrap(), 8);
    assert!(first.as_bytes().ends_with(b"first-response"));
    assert!(second.as_bytes().ends_with(b"second-response"));

    registrar
        .disconnect_component_session(endpoint)
        .await
        .unwrap();
    caller_echo.abort();
}

#[tokio::test]
async fn uncorrelatable_response_terminates_every_waiter() {
    let (_bus, mut registrar) = bus();
    let (endpoint, remote, endpoint_echo) = admit(
        &registrar,
        policy(
            ServicePackage::ResourceV3,
            EndpointPurpose::ResourceService,
            1,
        ),
        "Provider/system-core",
        "11111111-1111-4111-8111-111111111111",
        "Provider/system-core",
        [SessionVerb::Invoke],
    )
    .await;
    endpoint_echo.abort();
    let endpoint = registrar
        .register_component_session(endpoint)
        .await
        .unwrap();
    let (caller, _, caller_echo) = admit(
        &registrar,
        policy(
            ServicePackage::ResourceV3,
            EndpointPurpose::ResourceService,
            1,
        ),
        "Host/alice",
        "22222222-2222-4222-8222-222222222222",
        "Provider/system-core",
        [SessionVerb::Invoke],
    )
    .await;
    let caller = registrar.register_component_session(caller).await.unwrap();
    let route = route(
        "d2b.resource.v3",
        "ResourceService/Get",
        1,
        "Provider/system-core",
    );
    let remote_task = tokio::spawn(async move {
        let first = remote.receive_ttrpc().await.unwrap();
        let second = remote.receive_ttrpc().await.unwrap();
        let later_valid = ttrpc_stream_id(&second).unwrap();
        remote.send_ttrpc(vec![0x01]).await.unwrap();
        remote
            .send_ttrpc(ttrpc_frame(later_valid, b"must-not-deliver"))
            .await
            .unwrap();
        (first, second)
    });
    let invoke = |id: &str, stream_id| {
        caller.invoke_resource(
            route.clone(),
            OperationSpec::new(OperationId::parse(id).unwrap(), 10_000).unwrap(),
            ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
            ttrpc_frame(stream_id, b"request"),
        )
    };
    let (first, second, frames) = tokio::join!(
        invoke("malformed-first", 9),
        invoke("malformed-second", 10),
        remote_task
    );
    assert_eq!(
        first,
        Err(BusError::Endpoint(d2b_bus::EndpointError::Rejected))
    );
    assert_eq!(
        second,
        Err(BusError::Endpoint(d2b_bus::EndpointError::Rejected))
    );
    let (first_frame, second_frame) = frames.unwrap();
    assert_ne!(
        ttrpc_stream_id(&first_frame).unwrap(),
        ttrpc_stream_id(&second_frame).unwrap()
    );
    assert_eq!(
        invoke("after-malformed", 11).await,
        Err(BusError::Endpoint(d2b_bus::EndpointError::Rejected))
    );

    registrar
        .disconnect_component_session(endpoint)
        .await
        .unwrap();
    caller_echo.abort();
}

#[tokio::test]
async fn revocation_waits_for_an_admitted_batch_before_returning() {
    let (_bus, mut registrar) = bus();
    let pause = Arc::new(WriterPause::default());
    let (endpoint, _remote, endpoint_echo) = admit_with_writer_pause(
        &registrar,
        policy(
            ServicePackage::ResourceV3,
            EndpointPurpose::ResourceService,
            1,
        ),
        "Provider/system-core",
        "11111111-1111-4111-8111-111111111111",
        "Provider/system-core",
        [SessionVerb::Invoke],
        Some(Arc::clone(&pause)),
    )
    .await;
    let endpoint = registrar
        .register_component_session(endpoint)
        .await
        .unwrap();
    let (caller, _, caller_echo) = admit(
        &registrar,
        policy(
            ServicePackage::ResourceV3,
            EndpointPurpose::ResourceService,
            1,
        ),
        "Host/alice",
        "22222222-2222-4222-8222-222222222222",
        "Provider/system-core",
        [SessionVerb::Invoke],
    )
    .await;
    let caller = registrar.register_component_session(caller).await.unwrap();
    let invocation = tokio::spawn(async move {
        caller
            .invoke_resource(
                route(
                    "d2b.resource.v3",
                    "ResourceService/Get",
                    1,
                    "Provider/system-core",
                ),
                OperationSpec::new(OperationId::parse("revoked-write").unwrap(), 10_000).unwrap(),
                ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
                ttrpc_frame(12, b"must-not-send"),
            )
            .await
    });

    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        pause.wait_until_entered(),
    )
    .await
    .expect("request batch must reach the paused writer");
    let mut revocation = Box::pin(registrar.disconnect_component_session(endpoint));
    tokio::select! {
        result = &mut revocation => {
            panic!("revocation returned before admitted transport send: {result:?}")
        }
        () = tokio::task::yield_now() => {}
    }
    pause.release.notify_one();
    revocation.await.unwrap();

    assert!(invocation.await.unwrap().is_err());
    assert_eq!(pause.sent.load(Ordering::Acquire), 1);
    endpoint_echo.abort();
    caller_echo.abort();
}

#[tokio::test]
async fn receive_failure_terminates_without_retaining_the_operation() {
    let (_bus, mut registrar) = bus();
    let (endpoint, remote, endpoint_echo) = admit(
        &registrar,
        policy(
            ServicePackage::ResourceV3,
            EndpointPurpose::ResourceService,
            1,
        ),
        "Provider/system-core",
        "11111111-1111-4111-8111-111111111111",
        "Provider/system-core",
        [SessionVerb::Invoke],
    )
    .await;
    endpoint_echo.abort();
    let endpoint = registrar
        .register_component_session(endpoint)
        .await
        .unwrap();
    let (caller, _, caller_echo) = admit(
        &registrar,
        policy(
            ServicePackage::ResourceV3,
            EndpointPurpose::ResourceService,
            1,
        ),
        "Host/alice",
        "22222222-2222-4222-8222-222222222222",
        "Provider/system-core",
        [SessionVerb::Invoke, SessionVerb::Cancel],
    )
    .await;
    let caller = registrar.register_component_session(caller).await.unwrap();
    let operation_id = OperationId::parse("receive-failure").unwrap();
    let invoke = caller.invoke_resource(
        route(
            "d2b.resource.v3",
            "ResourceService/Get",
            1,
            "Provider/system-core",
        ),
        OperationSpec::new(operation_id.clone(), 10_000).unwrap(),
        ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
        ttrpc_frame(10, b"request"),
    );
    let disconnect = async {
        let _ = remote.receive_ttrpc().await.unwrap();
        remote
            .close(CloseReason::Normal, Remediation::None)
            .await
            .unwrap();
    };
    let (result, ()) = tokio::join!(invoke, disconnect);
    assert!(matches!(result, Err(BusError::Endpoint(_))));
    assert!(matches!(
        caller.cancel(&operation_id).await,
        Err(BusError::Operation(_))
    ));

    registrar
        .disconnect_component_session(endpoint)
        .await
        .unwrap();
    caller_echo.abort();
}

#[tokio::test]
async fn deadline_signals_the_correlated_remote_request() {
    let (_bus, mut registrar) = bus();
    let (endpoint, remote, echo) = admit(
        &registrar,
        policy(
            ServicePackage::ResourceV3,
            EndpointPurpose::ResourceService,
            1,
        ),
        "Provider/system-core",
        "11111111-1111-4111-8111-111111111111",
        "Provider/system-core",
        [SessionVerb::Invoke],
    )
    .await;
    echo.abort();
    let endpoint = registrar
        .register_component_session(endpoint)
        .await
        .unwrap();
    let (caller, _, caller_echo) = admit(
        &registrar,
        policy(
            ServicePackage::ResourceV3,
            EndpointPurpose::ResourceService,
            1,
        ),
        "Host/alice",
        "22222222-2222-4222-8222-222222222222",
        "Provider/system-core",
        [SessionVerb::Invoke],
    )
    .await;
    let caller = registrar.register_component_session(caller).await.unwrap();
    let (dispatched, dispatched_wait) = tokio::sync::oneshot::channel();
    let (cancelled, cancelled_wait) = tokio::sync::oneshot::channel();
    let remote_task = tokio::spawn(async move {
        let frame = remote.receive_ttrpc().await.unwrap();
        let request_id = ttrpc_request_id(remote.generation(), &frame).unwrap();
        let token = remote
            .register_inbound_call(request_id.clone())
            .await
            .unwrap();
        remote.mark_inbound_dispatched(request_id).await.unwrap();
        dispatched.send(()).unwrap();
        token.cancelled().await;
        cancelled.send(()).unwrap();
    });
    let mut invoke = tokio::spawn(async move {
        caller
            .invoke_resource(
                route(
                    "d2b.resource.v3",
                    "ResourceService/Get",
                    1,
                    "Provider/system-core",
                ),
                OperationSpec::new(OperationId::parse("deadline-cancel").unwrap(), 500).unwrap(),
                ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
                ttrpc_frame(41, b"wait"),
            )
            .await
    });
    tokio::select! {
        dispatched = dispatched_wait => dispatched.unwrap(),
        result = &mut invoke => panic!("invoke completed before remote dispatch: {result:?}"),
        () = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
            panic!("invoke did not reach the remote request")
        }
    }
    assert!(matches!(
        invoke.await.unwrap(),
        Err(d2b_bus::BusError::Operation(
            d2b_bus::operations::OperationError::DeadlineExceeded
        ))
    ));
    tokio::time::timeout(std::time::Duration::from_secs(1), cancelled_wait)
        .await
        .expect("remote cancellation must be signalled")
        .unwrap();
    registrar
        .disconnect_component_session(endpoint)
        .await
        .unwrap();
    caller_echo.abort();
    remote_task.await.unwrap();
}

#[tokio::test]
async fn explicit_cancel_signals_the_correlated_remote_request() {
    let (_bus, mut registrar) = bus();
    let (endpoint, remote, echo) = admit(
        &registrar,
        policy(
            ServicePackage::ResourceV3,
            EndpointPurpose::ResourceService,
            1,
        ),
        "Provider/system-core",
        "11111111-1111-4111-8111-111111111111",
        "Provider/system-core",
        [SessionVerb::Invoke],
    )
    .await;
    echo.abort();
    let endpoint = registrar
        .register_component_session(endpoint)
        .await
        .unwrap();
    let (caller, _, caller_echo) = admit(
        &registrar,
        policy(
            ServicePackage::ResourceV3,
            EndpointPurpose::ResourceService,
            1,
        ),
        "Host/alice",
        "22222222-2222-4222-8222-222222222222",
        "Provider/system-core",
        [SessionVerb::Invoke, SessionVerb::Cancel],
    )
    .await;
    let caller = std::sync::Arc::new(registrar.register_component_session(caller).await.unwrap());
    let (dispatched, dispatched_wait) = tokio::sync::oneshot::channel();
    let (cancelled, cancelled_wait) = tokio::sync::oneshot::channel();
    let remote_task = tokio::spawn(async move {
        let frame = remote.receive_ttrpc().await.unwrap();
        let request_id = ttrpc_request_id(remote.generation(), &frame).unwrap();
        let token = remote
            .register_inbound_call(request_id.clone())
            .await
            .unwrap();
        remote.mark_inbound_dispatched(request_id).await.unwrap();
        dispatched.send(()).unwrap();
        token.cancelled().await;
        cancelled.send(()).unwrap();
    });
    let operation_id = OperationId::parse("explicit-cancel").unwrap();
    let invoking = std::sync::Arc::clone(&caller);
    let invoked_operation_id = operation_id.clone();
    let invoke = tokio::spawn(async move {
        invoking
            .invoke_resource(
                route(
                    "d2b.resource.v3",
                    "ResourceService/Get",
                    1,
                    "Provider/system-core",
                ),
                OperationSpec::new(invoked_operation_id, 10_000).unwrap(),
                ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
                ttrpc_frame(42, b"wait"),
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(1), dispatched_wait)
        .await
        .expect("invoke must reach the remote request")
        .unwrap();
    caller.cancel(&operation_id).await.unwrap();
    assert!(matches!(
        invoke.await.unwrap(),
        Err(d2b_bus::BusError::Cancelled)
    ));
    tokio::time::timeout(std::time::Duration::from_secs(1), cancelled_wait)
        .await
        .expect("remote cancellation must be signalled")
        .unwrap();
    registrar
        .disconnect_component_session(endpoint)
        .await
        .unwrap();
    caller_echo.abort();
    remote_task.await.unwrap();
}

#[tokio::test]
async fn dropped_invoke_signals_the_correlated_remote_request() {
    let (_bus, mut registrar) = bus();
    let (endpoint, remote, echo) = admit(
        &registrar,
        policy(
            ServicePackage::ResourceV3,
            EndpointPurpose::ResourceService,
            1,
        ),
        "Provider/system-core",
        "11111111-1111-4111-8111-111111111111",
        "Provider/system-core",
        [SessionVerb::Invoke],
    )
    .await;
    echo.abort();
    let endpoint = registrar
        .register_component_session(endpoint)
        .await
        .unwrap();
    let (caller, _, caller_echo) = admit(
        &registrar,
        policy(
            ServicePackage::ResourceV3,
            EndpointPurpose::ResourceService,
            1,
        ),
        "Host/alice",
        "22222222-2222-4222-8222-222222222222",
        "Provider/system-core",
        [SessionVerb::Invoke],
    )
    .await;
    let caller = registrar.register_component_session(caller).await.unwrap();
    let (dispatched, dispatched_wait) = tokio::sync::oneshot::channel();
    let (cancelled, cancelled_wait) = tokio::sync::oneshot::channel();
    let remote_task = tokio::spawn(async move {
        let frame = remote.receive_ttrpc().await.unwrap();
        let request_id = ttrpc_request_id(remote.generation(), &frame).unwrap();
        let token = remote
            .register_inbound_call(request_id.clone())
            .await
            .unwrap();
        remote.mark_inbound_dispatched(request_id).await.unwrap();
        dispatched.send(()).unwrap();
        token.cancelled().await;
        cancelled.send(()).unwrap();
    });
    let invoke = tokio::spawn(async move {
        caller
            .invoke_resource(
                route(
                    "d2b.resource.v3",
                    "ResourceService/Get",
                    1,
                    "Provider/system-core",
                ),
                OperationSpec::new(OperationId::parse("dropped-invoke").unwrap(), 10_000).unwrap(),
                ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
                ttrpc_frame(43, b"wait"),
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(1), dispatched_wait)
        .await
        .expect("invoke must reach the remote request")
        .unwrap();
    invoke.abort();
    assert!(invoke.await.unwrap_err().is_cancelled());
    tokio::time::timeout(std::time::Duration::from_secs(1), cancelled_wait)
        .await
        .expect("dropping an invoke must signal remote cancellation")
        .unwrap();
    registrar
        .disconnect_component_session(endpoint)
        .await
        .unwrap();
    caller_echo.abort();
    remote_task.await.unwrap();
}
