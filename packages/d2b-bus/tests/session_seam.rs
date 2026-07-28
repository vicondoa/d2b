use std::{collections::BTreeSet, time::Instant};

use async_trait::async_trait;
use d2b_bus::{
    BusAuthorizer, BusConfig, BusError, ComponentSessionAdmission, OperationId, OperationSpec,
    ResourceCall, RouteGenerations, RouteKey, RouteMember, RouteTarget, ZoneBus, ZoneRegistrar,
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
    AuthenticatedComponentSession, ComponentSessionDriver, HandshakeCredentials,
    NativeSessionAuthority, NativeSessionSubject, OwnedTransport, SessionDriverHandle,
    SessionEngine, TransportDescriptor, TransportError, TransportEvidence, TransportPacket,
    ttrpc_request_id, ttrpc_stream_id,
};
use tokio::sync::mpsc;

const PROVIDER_GENERATION: u64 = 2;
const CONTROLLER_GENERATION: u64 = 3;

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
    allowed: impl IntoIterator<Item = SessionVerb>,
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
    };
    let right = TestTransport {
        sender: right_sender,
        receiver: left_receiver,
        descriptor,
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
    let mut allowed = allowed.into_iter().collect::<BTreeSet<_>>();
    allowed.insert(SessionVerb::Connect);
    let catalog = ApiCatalog::standard();
    let rule = PolicyRule::new(
        &catalog,
        [],
        [],
        allowed,
        [],
        [],
        [ZoneId::parse("dev").unwrap()],
        [],
    )
    .unwrap();
    let role = CompiledRole::new(
        ResourceRef::parse("Role/session-authority").unwrap(),
        vec![rule],
    )
    .unwrap();
    let binding = CompiledRoleBinding::new(
        role.role_ref.clone(),
        [BoundSubject {
            subject_ref: subject_ref.clone(),
            subject_uid: subject_uid.clone(),
        }],
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
    let authority = NativeSessionAuthority::new(
        NativeSessionSubject::new(subject_ref, subject_uid, ZoneId::parse("dev").unwrap())
            .unwrap()
            .with_provider(
                ResourceRef::parse(provider).unwrap(),
                ResourceGeneration::new(PROVIDER_GENERATION).unwrap(),
            )
            .with_controller_generation(ControllerGeneration::new(CONTROLLER_GENERATION).unwrap()),
        native,
        state,
        10_000,
    )
    .unwrap();
    let session = registrar
        .component_session_acceptor(policy, Box::new(authority))
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
    ZoneBus::new(
        zone,
        BusAuthorizer::new(native, state).unwrap(),
        BusConfig::default(),
    )
    .unwrap()
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
        "Provider/resource",
        "11111111-1111-4111-8111-111111111111",
        "Provider/resource",
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
            route,
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

    registrar
        .disconnect_component_session(endpoint)
        .await
        .unwrap();
    caller_echo.abort();
}

#[tokio::test]
async fn malformed_responses_release_every_correlation_slot() {
    const MALFORMED_RESPONSES: usize = 300;

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
        for _ in 0..MALFORMED_RESPONSES {
            let _ = remote.receive_ttrpc().await.unwrap();
            remote.send_ttrpc(vec![0x01]).await.unwrap();
        }
        let request = remote.receive_ttrpc().await.unwrap();
        let stream_id = ttrpc_stream_id(&request).unwrap();
        remote
            .send_ttrpc(ttrpc_frame(stream_id, b"healthy"))
            .await
            .unwrap();
    });

    for index in 0..MALFORMED_RESPONSES {
        let result = caller
            .invoke_resource(
                route.clone(),
                OperationSpec::new(
                    OperationId::parse(format!("malformed-{index}")).unwrap(),
                    10_000,
                )
                .unwrap(),
                ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
                ttrpc_frame(9, b"request"),
            )
            .await;
        assert_eq!(
            result,
            Err(BusError::Endpoint(d2b_bus::EndpointError::Rejected))
        );
    }
    let response = caller
        .invoke_resource(
            route,
            OperationSpec::new(OperationId::parse("after-malformed").unwrap(), 10_000).unwrap(),
            ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
            ttrpc_frame(9, b"request"),
        )
        .await
        .unwrap();
    assert!(response.as_bytes().ends_with(b"healthy"));
    remote_task.await.unwrap();

    registrar
        .disconnect_component_session(endpoint)
        .await
        .unwrap();
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
