use std::{collections::BTreeSet, time::Instant};

use async_trait::async_trait;
use d2b_bus::{
    BusAuthorizer, BusConfig, OperationId, OperationSpec, ResourceCall, RouteGenerations, RouteKey,
    RouteMember, RouteTarget, ZoneBus,
};
use d2b_contracts::v3::{
    AuthenticatedSubjectContext, BindingDigest, ConfigurationGeneration, ControllerGeneration,
    EvidenceClass, ResourceGeneration, ResourceRef, ResourceUid, SchemaFingerprint, ServiceName,
    SessionBinding, ZoneId, ZoneRevision,
    component_session::{
        AttachmentPolicy, AuthorizationLease, EndpointPolicy, EndpointPurpose, EndpointRole,
        IdentityEvidenceRequirement, LimitProfile, Locality, NoiseProfile, PurposeClass,
        ServicePackage, SessionErrorCode, TransportBinding, TransportClass,
    },
};
use d2b_resource_api::authz::{
    ApiCatalog, AuthorizationState, BootstrapPhase, NativeAuthorizer, SessionVerb,
};
use d2b_resource_store::PolicySnapshot;
use d2b_session::{
    AdmittedComponentSession, ComponentSessionDriver, ComponentSessionRegistrar,
    HandshakeCredentials, OwnedTransport, SessionAcceptor, SessionAuthenticationBinding,
    SessionAuthority, SessionAuthorizationRequest, SessionEngine, TransportDescriptor,
    TransportError, TransportEvidence, TransportPacket,
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

struct AllowAuthority {
    subject: ResourceRef,
    uid: ResourceUid,
    provider: ResourceRef,
    allowed: BTreeSet<SessionVerb>,
}

#[async_trait]
impl SessionAuthority for AllowAuthority {
    async fn authenticate_connect(
        &mut self,
        evidence: TransportEvidence,
        binding: &SessionAuthenticationBinding,
        expected_zone: &ZoneId,
        now_tick: u64,
    ) -> d2b_session::Result<(AuthenticatedSubjectContext, AuthorizationLease)> {
        if evidence.class() != binding.evidence_class() {
            return Err(d2b_session::SessionError::new(
                SessionErrorCode::IdentityEvidenceMismatch,
            ));
        }
        let context = AuthenticatedSubjectContext::new(
            self.subject.clone(),
            self.uid.clone(),
            ResourceRef::parse(&format!("Zone/{}", expected_zone.as_str())).unwrap(),
            binding.evidence_class(),
            binding.purpose().clone(),
            binding.service().clone(),
            SessionBinding::new(
                binding.schema_fingerprint().clone(),
                binding.transport_binding().clone(),
                binding.reconnect_generation(),
                binding.transcript_hash().clone(),
            ),
        )
        .with_provider_ref(self.provider.clone())
        .with_provider_generation(ResourceGeneration::new(PROVIDER_GENERATION).unwrap())
        .with_controller_generation(ControllerGeneration::new(CONTROLLER_GENERATION).unwrap());
        Ok((
            context,
            AuthorizationLease::new(1, now_tick + 10_000).unwrap(),
        ))
    }

    async fn authorize(
        &mut self,
        _subject: &AuthenticatedSubjectContext,
        request: &SessionAuthorizationRequest,
        _previous_lease: AuthorizationLease,
        now_tick: u64,
    ) -> d2b_session::Result<AuthorizationLease> {
        if !self.allowed.contains(&request.verb()) {
            return Err(d2b_session::SessionError::new(
                SessionErrorCode::PolicyDenied,
            ));
        }
        AuthorizationLease::new(1, now_tick + 10_000).map_err(d2b_session::SessionError::from)
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
    policy: EndpointPolicy,
    subject: &str,
    uid: &str,
    provider: &str,
    allowed: impl IntoIterator<Item = SessionVerb>,
) -> (AdmittedComponentSession, tokio::task::JoinHandle<()>) {
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
    let echo = tokio::spawn(async move {
        while let Ok(frame) = remote.receive_ttrpc().await {
            if remote.send_ttrpc(frame).await.is_err() {
                break;
            }
        }
    });
    let authority = AllowAuthority {
        subject: ResourceRef::parse(subject).unwrap(),
        uid: ResourceUid::parse(uid).unwrap(),
        provider: ResourceRef::parse(provider).unwrap(),
        allowed: allowed.into_iter().collect(),
    };
    let zone = ZoneId::parse("dev").unwrap();
    let session = SessionAcceptor::new(policy, zone, Box::new(authority))
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
    (session, echo)
}

fn bus() -> (ZoneBus, d2b_bus::ZoneRegistrar) {
    let native = NativeAuthorizer::new(ApiCatalog::standard(), None).unwrap();
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
        ZoneId::parse("dev").unwrap(),
        BusAuthorizer::new(native, state).unwrap(),
        BusConfig::default(),
    )
    .unwrap()
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

#[tokio::test]
async fn admitted_sessions_route_resource_and_diagnostic_calls_and_revoke_lifecycle() {
    let (_bus, mut registrar) = bus();
    let (resource_endpoint, resource_echo) = admit(
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
    let (resource_caller, caller_echo) = admit(
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
            b"resource".to_vec(),
        )
        .await
        .unwrap();
    assert_eq!(response.as_bytes(), b"resource");
    let (reconnected_endpoint, reconnected_echo) = admit(
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
    let reconnected_route = route(
        "d2b.resource.v3",
        "ResourceService/Get",
        2,
        "Provider/system-core",
    );
    let response = resource_caller
        .invoke_resource(
            reconnected_route,
            OperationSpec::new(
                OperationId::parse("resource-after-reconnect").unwrap(),
                10_000,
            )
            .unwrap(),
            ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
            b"reconnected".to_vec(),
        )
        .await
        .unwrap();
    assert_eq!(response.as_bytes(), b"reconnected");

    let (audit_endpoint, audit_echo) = admit(
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
    let (audit_caller, audit_caller_echo) = admit(
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
            b"diagnostic".to_vec(),
        )
        .await
        .unwrap();
    assert_eq!(response.as_bytes(), b"diagnostic");

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

    registrar
        .disconnect_component_session(resource_endpoint)
        .await
        .unwrap();
    drop((resource_caller, audit_caller));
    for task in [
        resource_echo,
        reconnected_echo,
        caller_echo,
        audit_echo,
        audit_caller_echo,
    ] {
        task.abort();
    }
}
