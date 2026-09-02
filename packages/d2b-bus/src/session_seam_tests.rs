use std::{
    os::{
        linux::net::SocketAddrExt,
        unix::net::{UnixListener, UnixStream},
    },
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Instant,
};

use async_trait::async_trait;
use d2b_bus::{
    AuthorizationError, BusAuthorizer, BusConfig, BusError, CommittedControllerProcessSubjectInput,
    ComponentSessionAdmission, EndpointError, OperationId, OperationSpec, ResourceCall,
    ResourceQuery, RouteGenerations, RouteKey, RouteMember, RouteTarget, ZoneBus, ZoneRegistrar,
};
use d2b_contracts_provider::v3::{
    ArtifactDigest, ArtifactDigestSet, BinaryRef, CompatibilityRange, ComponentDescriptor,
    ComponentExecution, ComponentTargetCapability, ComponentType, ControllerInstanceScope,
    ControllerTargetKind, EffectPortClass, PolicyEvaluation, ProviderManifest, ResourceApiBinding,
    RevocationState, SignatureState, TargetRuntimeArtifacts, TrustEvidence, UpgradeDisposition,
    UpgradePolicy,
};
use d2b_contracts_resource::resource_proto as resource_wire;
use d2b_contracts_resource::v3::identity::{
    BindingDigest, EvidenceClass, ReconnectGeneration, ServiceName,
};
use d2b_contracts_resource::v3::process::PROCESS_RESOURCE_TYPE;
use d2b_contracts_resource::v3::{
    ConfigurationGeneration, ControllerGeneration, ExecutionDomain, PlacementAnchor,
    ResourceGeneration, ResourceRef, ResourceUid, SchemaFingerprint, SchemaVersion, ZoneId,
    ZoneRevision,
};
use d2b_contracts_zone_session::v3::component_session::{
    AttachmentPolicy, CloseReason, EndpointPolicy, EndpointPurpose, EndpointRole,
    IdentityEvidenceRequirement, LimitProfile, Locality, NoiseProfile, PurposeClass, Remediation,
    ServicePackage, TransportBinding, TransportClass,
};
use d2b_core_controller::controller_assignment::{
    AssignmentError, AssignmentRequest, AssignmentVerb, ControllerAssignmentRegistry,
    ControllerRoleContract, ScopedCommitTransport,
};
use d2b_resource_api::authz::{
    ApiCatalog, AuthorizationState, BindingScope, BootstrapPhase, BoundSubject, CompiledRole,
    CompiledRoleBinding, NativeAuthorizer, PolicyRule, PolicySet, RelayGrantAuthority,
    ResourceVerb, SessionVerb,
};
use d2b_resource_api::{ResourceBusAdapter, ResourceService, ResourceStoreBackend};
use d2b_resource_store::PolicySnapshot;
use d2b_resource_store::mutation_seal::{MutationSealAcceptor, MutationSealBody};
use d2b_resource_store::{
    ResourceMutationKind, StoreCommitResult, StoreError, StoreGetRequest,
    StoreInspectSchemaRequest, StoreListRequest, StoreListResult, StoreResolveRequest,
    StoreResolvedIdentity, StoreSealIdentity, StoreSlot, StoreWatchReceipt, StoreWatchRequest,
    StoredResource, StoredSchema,
};
use d2b_session::{
    AuthenticatedComponentSession, ComponentSessionDriver, HandshakeCredentials, OwnedTransport,
    SessionDriverHandle, SessionEngine, TransportDescriptor, TransportError, TransportEvidence,
    TransportPacket, serve_ttrpc_services, ttrpc_request_id, ttrpc_stream_id,
};
use d2b_session_unix::{SeqpacketSocket, VerifiedUnixPeer, prearmed_seqpacket_pair};
use protobuf::{EnumOrUnknown, Message, MessageField};
use tokio::sync::{Notify, mpsc};
use ttrpc::proto::{MessageHeader, Request as TtrpcRequest};

use crate::router::UnixSubjectRecord;

const PROVIDER_GENERATION: u64 = 2;
const CONTROLLER_GENERATION: u64 = 3;
const CONTROLLER_PEER_CHILD_ENV: &str = "D2B_CONTROLLER_PEER_CHILD";
static CONTROLLER_PEER_LISTENER_ID: AtomicUsize = AtomicUsize::new(1);

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

#[test]
fn controller_peer_child() {
    let Some(name) = std::env::var_os(CONTROLLER_PEER_CHILD_ENV) else {
        return;
    };
    let address =
        std::os::unix::net::SocketAddr::from_abstract_name(name.to_string_lossy().as_bytes())
            .unwrap();
    UnixStream::connect_addr(&address).unwrap();
}

fn child_verified_peer() -> VerifiedUnixPeer {
    let name = format!(
        "d2b-controller-peer-{}-{}",
        std::process::id(),
        CONTROLLER_PEER_LISTENER_ID.fetch_add(1, Ordering::Relaxed)
    );
    let address = std::os::unix::net::SocketAddr::from_abstract_name(name.as_bytes()).unwrap();
    let listener = UnixListener::bind_addr(&address).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "session_seam_tests::controller_peer_child",
            "--nocapture",
        ])
        .env(CONTROLLER_PEER_CHILD_ENV, &name)
        .spawn()
        .unwrap();
    let (stream, _) = listener.accept().unwrap();
    stream.set_nonblocking(true).unwrap();
    let socket = d2b_session_unix::StreamSocket::from_owned(stream.into()).unwrap();
    let verified_peer = VerifiedUnixPeer::verify_stream(&socket).unwrap();
    assert!(child.wait().unwrap().success());
    verified_peer
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
    admit_inner(
        registrar, policy, subject, uid, provider, _allowed, None, true, None,
    )
    .await
}

async fn admit_without_echo(
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
    admit_inner(
        registrar, policy, subject, uid, provider, allowed, None, false, None,
    )
    .await
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
    admit_inner(
        registrar,
        policy,
        subject,
        uid,
        provider,
        _allowed,
        writer_pause,
        true,
        None,
    )
    .await
}

async fn admit_controller(
    registrar: &ZoneRegistrar,
    policy: EndpointPolicy,
    provider: &str,
    uid: &str,
    process_ref: &str,
    execution_ref: &str,
    allowed: impl IntoIterator<Item = SessionVerb>,
) -> (
    AuthenticatedComponentSession<ComponentSessionAdmission>,
    SessionDriverHandle,
    tokio::task::JoinHandle<()>,
) {
    admit_inner(
        registrar,
        policy,
        provider,
        uid,
        provider,
        allowed,
        None,
        true,
        Some(CommittedControllerProcessSubjectInput {
            provider_ref: ResourceRef::parse(provider).unwrap(),
            provider_uid: ResourceUid::parse(uid).unwrap(),
            process_ref: ResourceRef::parse(process_ref).unwrap(),
            zone_ref: ResourceRef::parse("Zone/dev").unwrap(),
            execution_ref: ResourceRef::parse(execution_ref).unwrap(),
            provider_generation: ResourceGeneration::new(PROVIDER_GENERATION).unwrap(),
            controller_generation: ControllerGeneration::new(CONTROLLER_GENERATION).unwrap(),
        }),
    )
    .await
}

async fn admit_with_verified_peer(
    registrar: &ZoneRegistrar,
    policy: EndpointPolicy,
    verified_peer: VerifiedUnixPeer,
) -> AuthenticatedComponentSession<ComponentSessionAdmission> {
    let descriptor = TransportDescriptor {
        class: policy.transport_binding.transport,
        locality: policy.transport_binding.locality,
        packet_atomic: false,
        supports_attachments: false,
    };
    let (left_sender, left_receiver) = mpsc::channel(16);
    let (right_sender, right_receiver) = mpsc::channel(16);
    let left = TestTransport {
        sender: left_sender,
        receiver: right_receiver,
        descriptor,
        writer_pause: None,
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
    drop(responder.unwrap());
    registrar
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
        .unwrap()
}

async fn admit_inner(
    registrar: &ZoneRegistrar,
    policy: EndpointPolicy,
    subject: &str,
    uid: &str,
    provider: &str,
    _allowed: impl IntoIterator<Item = SessionVerb>,
    writer_pause: Option<Arc<WriterPause>>,
    start_echo: bool,
    controller_subject: Option<CommittedControllerProcessSubjectInput>,
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
    let echo = if start_echo {
        let echo_remote = remote.clone();
        tokio::spawn(async move {
            while let Ok(frame) = echo_remote.receive_ttrpc().await {
                if echo_remote.send_ttrpc(frame).await.is_err() {
                    break;
                }
            }
        })
    } else {
        tokio::spawn(async {})
    };
    let subject_ref = ResourceRef::parse(subject).unwrap();
    let subject_uid = ResourceUid::parse(uid).unwrap();
    let provider_ref = ResourceRef::parse(provider).unwrap();
    let provider_generation = ResourceGeneration::new(PROVIDER_GENERATION).unwrap();
    let (proof_fd, _peer_fd) = prearmed_seqpacket_pair().unwrap();
    let proof_socket = SeqpacketSocket::from_parent_prearmed(proof_fd).unwrap();
    let expected_peer = proof_socket.acceptor_peer_credentials().unwrap();
    let verified_peer = VerifiedUnixPeer::verify_seqpacket(&proof_socket).unwrap();
    if let Some(controller_subject) = controller_subject {
        registrar
            .install_committed_controller_process_subject(&verified_peer, controller_subject)
            .unwrap();
    } else {
        let subject_config = match subject_ref.resource_type().as_str() {
            "Provider" => UnixSubjectRecord::provider(
                subject_ref,
                subject_uid,
                ResourceRef::parse("Zone/dev").unwrap(),
                expected_peer,
                provider_generation,
            )
            .unwrap(),
            "Guest" => UnixSubjectRecord::guest(
                subject_ref,
                subject_uid,
                ResourceRef::parse("Zone/dev").unwrap(),
                expected_peer,
            )
            .unwrap()
            .with_provider(provider_ref, provider_generation)
            .unwrap(),
            _ => UnixSubjectRecord::host(
                subject_ref,
                subject_uid,
                ResourceRef::parse("Zone/dev").unwrap(),
                expected_peer,
            )
            .unwrap()
            .with_provider(provider_ref, provider_generation)
            .unwrap()
            .with_execution_ref(ResourceRef::parse("Host/host-system").unwrap())
            .unwrap(),
        }
        .with_controller_generation(ControllerGeneration::new(CONTROLLER_GENERATION).unwrap());
        registrar.install_test_unix_subject(subject_config).unwrap();
    }
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
        [d2b_contracts_resource::v3::ResourceTypeName::parse("Host").unwrap()],
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
        ("Guest/bob", "44444444-4444-4444-8444-444444444444"),
        ("Provider/external", "55555555-5555-4555-8555-555555555555"),
        ("Provider/external", "66666666-6666-4666-8666-666666666666"),
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
        d2b_session::contract::SessionErrorCode::SubjectConfigurationMismatch
    );
    assert_eq!(
        error.remediation(),
        d2b_session::contract::Remediation::RepairConfiguration
    );
}

#[tokio::test]
async fn registrar_rejects_ambiguous_same_peer_subject_registration() {
    let (_bus, registrar) = bus();
    let (proof_fd, _peer_fd) = prearmed_seqpacket_pair().unwrap();
    let proof_socket = SeqpacketSocket::from_parent_prearmed(proof_fd).unwrap();
    let expected_peer = proof_socket.acceptor_peer_credentials().unwrap();
    for subject in [
        (
            "Provider/system-core",
            "11111111-1111-4111-8111-111111111111",
        ),
        (
            "Provider/system-minijail",
            "22222222-2222-4222-8222-222222222222",
        ),
    ] {
        registrar
            .install_test_unix_subject(
                UnixSubjectRecord::provider(
                    ResourceRef::parse(subject.0).unwrap(),
                    ResourceUid::parse(subject.1).unwrap(),
                    ResourceRef::parse("Zone/dev").unwrap(),
                    expected_peer,
                    ResourceGeneration::new(PROVIDER_GENERATION).unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
    }
    let error = match registrar.component_session_acceptor(
        policy(
            ServicePackage::ResourceV3,
            EndpointPurpose::ResourceService,
            1,
        ),
        VerifiedUnixPeer::verify_seqpacket(&proof_socket).unwrap(),
    ) {
        Ok(_) => panic!("ambiguous peer evidence must not mint a session acceptor"),
        Err(error) => error,
    };
    assert_eq!(
        error.code(),
        d2b_session::contract::SessionErrorCode::SubjectConfigurationMismatch
    );
}

#[tokio::test]
async fn committed_controller_subject_installation_binds_an_exact_verified_peer() {
    let (_bus, registrar) = bus();
    let (proof_fd, _peer_fd) = prearmed_seqpacket_pair().unwrap();
    let proof_socket = SeqpacketSocket::from_parent_prearmed(proof_fd).unwrap();
    let verified_peer = VerifiedUnixPeer::verify_seqpacket(&proof_socket).unwrap();

    registrar
        .install_committed_controller_process_subject(
            &verified_peer,
            CommittedControllerProcessSubjectInput {
                provider_ref: ResourceRef::parse("Provider/external").unwrap(),
                provider_uid: ResourceUid::parse("55555555-5555-4555-8555-555555555555").unwrap(),
                process_ref: ResourceRef::parse("Process/external-controller").unwrap(),
                zone_ref: ResourceRef::parse("Zone/dev").unwrap(),
                execution_ref: ResourceRef::parse("Host/host-system").unwrap(),
                provider_generation: ResourceGeneration::new(4).unwrap(),
                controller_generation: ControllerGeneration::new(5).unwrap(),
            },
        )
        .unwrap();
}

#[tokio::test]
async fn committed_controller_subject_carries_authoritative_context() {
    let (_bus, registrar) = bus();
    let (session, _remote, echo) = admit_controller(
        &registrar,
        policy(
            ServicePackage::ResourceV3,
            EndpointPurpose::ResourceService,
            1,
        ),
        "Provider/external",
        "55555555-5555-4555-8555-555555555555",
        "Process/external-controller",
        "Host/host-system",
        [SessionVerb::Connect],
    )
    .await;

    let binding = session.route_binding();
    let context = binding.context();
    assert_eq!(
        context.subject_ref(),
        &ResourceRef::parse("Provider/external").unwrap()
    );
    assert_eq!(
        context.subject_uid(),
        &ResourceUid::parse("55555555-5555-4555-8555-555555555555").unwrap()
    );
    assert_eq!(context.zone_ref(), &ResourceRef::parse("Zone/dev").unwrap());
    assert_eq!(
        context.provider_ref(),
        Some(&ResourceRef::parse("Provider/external").unwrap())
    );
    assert_eq!(
        context.process_ref(),
        Some(&ResourceRef::parse("Process/external-controller").unwrap())
    );
    assert_eq!(
        context.execution_ref(),
        Some(&ResourceRef::parse("Host/host-system").unwrap())
    );
    assert_eq!(
        context.provider_generation(),
        Some(ResourceGeneration::new(PROVIDER_GENERATION).unwrap())
    );
    assert_eq!(
        context.controller_generation(),
        Some(ControllerGeneration::new(CONTROLLER_GENERATION).unwrap())
    );
    echo.abort();
}

#[tokio::test]
async fn exact_controller_subject_records_are_consumed_once() {
    let (_bus, registrar) = bus();
    let (proof_fd, _peer_fd) = prearmed_seqpacket_pair().unwrap();
    let proof_socket = SeqpacketSocket::from_parent_prearmed(proof_fd).unwrap();
    let first_peer = VerifiedUnixPeer::verify_seqpacket(&proof_socket).unwrap();
    let second_peer = VerifiedUnixPeer::verify_seqpacket(&proof_socket).unwrap();
    registrar
        .install_committed_controller_process_subject(
            &first_peer,
            CommittedControllerProcessSubjectInput {
                provider_ref: ResourceRef::parse("Provider/external").unwrap(),
                provider_uid: ResourceUid::parse("55555555-5555-4555-8555-555555555555").unwrap(),
                process_ref: ResourceRef::parse("Process/external-controller").unwrap(),
                zone_ref: ResourceRef::parse("Zone/dev").unwrap(),
                execution_ref: ResourceRef::parse("Host/host-system").unwrap(),
                provider_generation: ResourceGeneration::new(4).unwrap(),
                controller_generation: ControllerGeneration::new(5).unwrap(),
            },
        )
        .unwrap();
    registrar
        .component_session_acceptor(
            policy(
                ServicePackage::ResourceV3,
                EndpointPurpose::ResourceService,
                1,
            ),
            first_peer,
        )
        .unwrap();
    let error = registrar
        .component_session_acceptor(
            policy(
                ServicePackage::ResourceV3,
                EndpointPurpose::ResourceService,
                1,
            ),
            second_peer,
        )
        .unwrap_err();
    assert_eq!(
        error.code(),
        d2b_session::contract::SessionErrorCode::SubjectConfigurationMismatch
    );
}

#[tokio::test]
async fn exact_resource_subject_restart_replaces_before_capacity_check() {
    let (_bus, registrar) = bus_with_config(BusConfig {
        max_routes_per_session: 1,
        max_total_routes: 1,
        ..BusConfig::default()
    });
    let (proof_fd, _peer_fd) = prearmed_seqpacket_pair().unwrap();
    let proof_socket = SeqpacketSocket::from_parent_prearmed(proof_fd).unwrap();
    let first_peer = VerifiedUnixPeer::verify_seqpacket(&proof_socket).unwrap();
    let second_peer = VerifiedUnixPeer::verify_seqpacket(&proof_socket).unwrap();
    let input = |controller_generation| CommittedControllerProcessSubjectInput {
        provider_ref: ResourceRef::parse("Provider/external").unwrap(),
        provider_uid: ResourceUid::parse("55555555-5555-4555-8555-555555555555").unwrap(),
        process_ref: ResourceRef::parse("Process/external-controller").unwrap(),
        zone_ref: ResourceRef::parse("Zone/dev").unwrap(),
        execution_ref: ResourceRef::parse("Host/host-system").unwrap(),
        provider_generation: ResourceGeneration::new(4).unwrap(),
        controller_generation: ControllerGeneration::new(controller_generation).unwrap(),
    };
    registrar
        .install_committed_controller_process_subject(&first_peer, input(5))
        .unwrap();
    registrar
        .install_committed_controller_process_subject(&second_peer, input(6))
        .unwrap();
    let acceptor = registrar
        .component_session_acceptor(
            policy(
                ServicePackage::ResourceV3,
                EndpointPurpose::ResourceService,
                1,
            ),
            second_peer,
        )
        .unwrap();
    drop(acceptor);
    let error = registrar
        .component_session_acceptor(
            policy(
                ServicePackage::ResourceV3,
                EndpointPurpose::ResourceService,
                1,
            ),
            first_peer,
        )
        .unwrap_err();
    assert_eq!(
        error.code(),
        d2b_session::contract::SessionErrorCode::SubjectConfigurationMismatch
    );
}

#[tokio::test]
async fn exact_resource_subject_restart_rejects_old_peer_and_admits_new_peer() {
    let (_bus, registrar) = bus_with_config(BusConfig {
        max_routes_per_session: 1,
        max_total_routes: 1,
        ..BusConfig::default()
    });
    let old_peer = child_verified_peer();
    let new_peer = child_verified_peer();
    assert_ne!(old_peer.credentials().pid(), new_peer.credentials().pid());
    let old_input = CommittedControllerProcessSubjectInput {
        provider_ref: ResourceRef::parse("Provider/external").unwrap(),
        provider_uid: ResourceUid::parse("55555555-5555-4555-8555-555555555555").unwrap(),
        process_ref: ResourceRef::parse("Process/external-controller").unwrap(),
        zone_ref: ResourceRef::parse("Zone/dev").unwrap(),
        execution_ref: ResourceRef::parse("Host/host-system").unwrap(),
        provider_generation: ResourceGeneration::new(4).unwrap(),
        controller_generation: ControllerGeneration::new(5).unwrap(),
    };
    registrar
        .install_committed_controller_process_subject(&old_peer, old_input)
        .unwrap();
    registrar
        .install_committed_controller_process_subject(
            &new_peer,
            CommittedControllerProcessSubjectInput {
                provider_ref: ResourceRef::parse("Provider/external").unwrap(),
                provider_uid: ResourceUid::parse("66666666-6666-4666-8666-666666666666").unwrap(),
                process_ref: ResourceRef::parse("Process/external-controller").unwrap(),
                zone_ref: ResourceRef::parse("Zone/dev").unwrap(),
                execution_ref: ResourceRef::parse("Host/host-system").unwrap(),
                provider_generation: ResourceGeneration::new(4).unwrap(),
                controller_generation: ControllerGeneration::new(6).unwrap(),
            },
        )
        .unwrap();
    let mut controller_policy = policy(
        ServicePackage::ResourceV3,
        EndpointPurpose::ResourceService,
        1,
    );
    controller_policy.transport_binding.transport = TransportClass::UnixStream;
    controller_policy.attachment_policy = AttachmentPolicy::disabled();
    let session = admit_with_verified_peer(&registrar, controller_policy.clone(), new_peer).await;
    let binding = session.route_binding();
    let context = binding.context();
    assert_eq!(
        context.subject_uid(),
        &ResourceUid::parse("66666666-6666-4666-8666-666666666666").unwrap()
    );
    assert_eq!(
        context.process_ref(),
        Some(&ResourceRef::parse("Process/external-controller").unwrap())
    );
    assert_eq!(
        context.execution_ref(),
        Some(&ResourceRef::parse("Host/host-system").unwrap())
    );
    assert_eq!(
        context.provider_generation(),
        Some(ResourceGeneration::new(4).unwrap())
    );
    assert_eq!(
        context.controller_generation(),
        Some(ControllerGeneration::new(6).unwrap())
    );
    let error = registrar
        .component_session_acceptor(controller_policy, old_peer)
        .unwrap_err();
    assert_eq!(
        error.code(),
        d2b_session::contract::SessionErrorCode::SubjectConfigurationMismatch
    );
}

#[tokio::test]
async fn exact_resource_subjects_with_different_processes_remain_distinct() {
    let (_bus, registrar) = bus_with_config(BusConfig {
        max_routes_per_session: 1,
        max_total_routes: 1,
        ..BusConfig::default()
    });
    let (proof_fd, _peer_fd) = prearmed_seqpacket_pair().unwrap();
    let proof_socket = SeqpacketSocket::from_parent_prearmed(proof_fd).unwrap();
    let verified_peer = VerifiedUnixPeer::verify_seqpacket(&proof_socket).unwrap();
    let input = |process_ref| CommittedControllerProcessSubjectInput {
        provider_ref: ResourceRef::parse("Provider/external").unwrap(),
        provider_uid: ResourceUid::parse("55555555-5555-4555-8555-555555555555").unwrap(),
        process_ref: ResourceRef::parse(process_ref).unwrap(),
        zone_ref: ResourceRef::parse("Zone/dev").unwrap(),
        execution_ref: ResourceRef::parse("Host/host-system").unwrap(),
        provider_generation: ResourceGeneration::new(4).unwrap(),
        controller_generation: ControllerGeneration::new(5).unwrap(),
    };
    registrar
        .install_committed_controller_process_subject(
            &verified_peer,
            input("Process/external-controller-a"),
        )
        .unwrap();
    let error = registrar
        .install_committed_controller_process_subject(
            &verified_peer,
            input("Process/external-controller-b"),
        )
        .unwrap_err();
    assert_eq!(
        error.code(),
        d2b_session::contract::SessionErrorCode::SubjectConfigurationMismatch
    );
}

#[tokio::test]
async fn a_shared_provider_uid_cannot_select_a_resource_controller() {
    let (_bus, registrar) = bus();
    let (proof_fd, _peer_fd) = prearmed_seqpacket_pair().unwrap();
    let proof_socket = SeqpacketSocket::from_parent_prearmed(proof_fd).unwrap();
    let verified_peer = VerifiedUnixPeer::verify_seqpacket(&proof_socket).unwrap();
    let uid_only_subject = UnixSubjectRecord::provider_for_uid(
        ResourceRef::parse("Provider/external").unwrap(),
        ResourceUid::parse("55555555-5555-4555-8555-555555555555").unwrap(),
        ResourceRef::parse("Zone/dev").unwrap(),
        verified_peer.credentials().uid().as_raw(),
    )
    .unwrap()
    .for_service(ServicePackage::ResourceV3);
    registrar
        .install_test_unix_subject(uid_only_subject)
        .unwrap();

    let error = registrar
        .component_session_acceptor(
            policy(
                ServicePackage::ResourceV3,
                EndpointPurpose::ResourceService,
                1,
            ),
            verified_peer,
        )
        .unwrap_err();
    assert_eq!(
        error.code(),
        d2b_session::contract::SessionErrorCode::SubjectConfigurationMismatch
    );
}

#[tokio::test]
async fn explicitly_installed_system_core_subject_still_registers() {
    let (_bus, registrar) = bus();
    let (proof_fd, _peer_fd) = prearmed_seqpacket_pair().unwrap();
    let proof_socket = SeqpacketSocket::from_parent_prearmed(proof_fd).unwrap();
    let verified_peer = VerifiedUnixPeer::verify_seqpacket(&proof_socket).unwrap();
    registrar
        .install_system_core_subject(&verified_peer)
        .unwrap();
    registrar
        .component_session_acceptor(
            policy(
                ServicePackage::ResourceV3,
                EndpointPurpose::ResourceService,
                1,
            ),
            verified_peer,
        )
        .unwrap();
}

#[tokio::test]
async fn service_registration_returns_the_only_transport_reader() {
    let (_bus, mut registrar) = bus();
    let (candidate, remote, echo) = admit_without_echo(
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
    let (ingress, service_driver) = registrar
        .register_component_service_session(candidate)
        .await
        .unwrap();
    assert!(ingress.component_session_driver().is_none());

    let frame = ttrpc_frame(1, b"service-request");
    remote.send_ttrpc(frame.clone()).await.unwrap();
    assert_eq!(service_driver.receive_ttrpc().await.unwrap(), frame);

    registrar.revoke(ingress).await.unwrap();
    echo.abort();
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
            d2b_contracts_resource::v3::identity::ReconnectGeneration::new(generation).unwrap(),
        ),
    )
}

const ASSIGNMENT_DIGEST: &str =
    "sha256:0000000000000000000000000000000000000000000000000000000000000000";

fn assignment_digest() -> ArtifactDigest {
    ArtifactDigest::parse(ASSIGNMENT_DIGEST).unwrap()
}

fn assignment_fingerprint() -> SchemaFingerprint {
    SchemaFingerprint::parse(ASSIGNMENT_DIGEST).unwrap()
}

fn assignment_manifest() -> ProviderManifest {
    let process =
        d2b_contracts_resource::v3::ResourceTypeName::parse(PROCESS_RESOURCE_TYPE).unwrap();
    let component = ComponentDescriptor::new(
        d2b_contracts_resource::v3::execution_policy::BoundedToken::parse("process-controller")
            .unwrap(),
        ComponentType::Controller,
        [process.clone()],
        [],
        [ExecutionDomain::System],
        1,
        assignment_digest(),
        [],
        false,
    )
    .unwrap()
    .with_execution(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse("process-controller").unwrap(),
    })
    .with_controller_placement(
        ControllerInstanceScope::PerResourceTarget,
        [ControllerTargetKind::Host, ControllerTargetKind::Guest],
    )
    .unwrap()
    .with_target_capabilities([
        ComponentTargetCapability::new(
            ControllerTargetKind::Host,
            assignment_digest(),
            [EffectPortClass::Process],
        )
        .unwrap(),
        ComponentTargetCapability::new(
            ControllerTargetKind::Guest,
            assignment_digest(),
            [EffectPortClass::Process],
        )
        .unwrap(),
    ])
    .unwrap();
    let binding = ResourceApiBinding::new_with_placement(
        process,
        SchemaVersion::new(1, 0).unwrap(),
        assignment_fingerprint(),
        SchemaVersion::new(1, 0).unwrap(),
        assignment_fingerprint(),
        Default::default(),
        None,
        None,
        PlacementAnchor::ExecutionRef,
    )
    .unwrap();
    let trust = TrustEvidence {
        publisher: d2b_contracts_resource::v3::execution_policy::BoundedToken::parse("trusted")
            .unwrap(),
        root_epoch: 1,
        publisher_trusted: true,
        signature: SignatureState::Valid,
        revocation: RevocationState::Clear,
        emergency_deny: false,
        provenance: PolicyEvaluation::Accepted,
        sbom: PolicyEvaluation::Accepted,
        license: PolicyEvaluation::Accepted,
        vulnerability: PolicyEvaluation::Accepted,
        conformance: PolicyEvaluation::Accepted,
        support_channel: d2b_contracts_resource::v3::execution_policy::BoundedToken::parse(
            "stable",
        )
        .unwrap(),
    };
    ProviderManifest::new(
        d2b_contracts_resource::v3::ArtifactId::parse("provider-system-core").unwrap(),
        ArtifactDigestSet {
            executable: assignment_digest(),
            config: assignment_digest(),
            schema: assignment_digest(),
            service: assignment_digest(),
        },
        trust,
        CompatibilityRange {
            api_major: 3,
            api_minor: 0,
            descriptor_fingerprint: assignment_fingerprint(),
            state_schema_version: SchemaVersion::new(1, 0).unwrap(),
        },
        [component],
        [binding],
        [],
        UpgradePolicy {
            drain_before_upgrade: true,
            max_automatic_disposition: UpgradeDisposition::InPlace,
            preserves_durable_state: true,
        },
    )
    .unwrap()
    .with_target_runtime_artifacts([
        TargetRuntimeArtifacts::new(
            ControllerTargetKind::Host,
            assignment_digest(),
            assignment_digest(),
        )
        .unwrap(),
        TargetRuntimeArtifacts::new(
            ControllerTargetKind::Guest,
            assignment_digest(),
            assignment_digest(),
        )
        .unwrap(),
    ])
    .unwrap()
}

fn assignment_resource() -> d2b_contracts_resource::v3::ResourceEnvelope {
    d2b_contracts_resource::v3::ResourceEnvelope::from_json(
        br#"{"apiVersion":"resources.d2bus.org/v3","type":"Process","metadata":{"configurationGeneration":null,"createdAt":"2026-07-22T00:00:00.000Z","deletionRequestedAt":null,"finalizers":[],"generation":1,"managedBy":"api","name":"work","ownerRef":null,"revision":7,"uid":"123e4567-e89b-42d3-a456-426614174000","updatedAt":"2026-07-22T00:00:00.000Z","zone":"dev"},"spec":{"executionRef":"Host/host-system","providerRef":"Provider/system-core","argv":["true"]},"status":{"completedAt":null,"conditions":[],"lastReconciledAt":null,"observedGeneration":0,"outcome":null,"phase":"Pending","resource":{},"startedAt":null,"update":{"dependencies":{"count":0,"refs":[]},"disruption":"None","lastAssessedAt":null,"observedGeneration":0,"operationId":null,"owned":{"count":0,"refs":[]},"preserveState":true,"reasons":[],"state":"Unknown","targetGeneration":1}}}"#,

    )
    .unwrap()
}

fn scoped_bus() -> (
    ZoneBus,
    ZoneRegistrar,
    std::sync::Arc<std::sync::Mutex<ControllerAssignmentRegistry>>,
    std::sync::Arc<NativeAuthorizer>,
    AuthorizationState,
) {
    let catalog = ApiCatalog::standard();
    let zone = ZoneId::parse("dev").unwrap();
    let rule = PolicyRule::new(
        &catalog,
        [d2b_contracts_resource::v3::ResourceTypeName::parse(PROCESS_RESOURCE_TYPE).unwrap()],
        [
            ResourceVerb::List,
            ResourceVerb::Watch,
            ResourceVerb::UpdateStatus,
            ResourceVerb::UpdateFinalizers,
        ],
        [
            SessionVerb::Connect,
            SessionVerb::Invoke,
            SessionVerb::Cancel,
        ],
        [],
        [],
        [zone.clone()],
        [],
    )
    .unwrap();
    let role = CompiledRole::new(
        ResourceRef::parse("Role/scoped-commit").unwrap(),
        vec![rule],
    )
    .unwrap();
    let subjects = [
        (
            "Provider/system-core",
            "11111111-1111-4111-8111-111111111111",
        ),
        ("Host/alice", "22222222-2222-4222-8222-222222222222"),
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
    let native = std::sync::Arc::new(NativeAuthorizer::new(catalog, Some(policy_set)).unwrap());
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
    let assignments = std::sync::Arc::new(std::sync::Mutex::new(
        ControllerAssignmentRegistry::default(),
    ));
    let authorizer = BusAuthorizer::from_shared(std::sync::Arc::clone(&native), state.clone())
        .unwrap()
        .with_assignment_registry(std::sync::Arc::clone(&assignments));
    let (bus, registrar) = ZoneBus::new(zone, authorizer, BusConfig::default()).unwrap();
    (bus, registrar, assignments, native, state)
}

struct ScopedStore {
    acceptor: MutationSealAcceptor,
    commits: std::sync::Arc<std::sync::Mutex<Vec<Vec<(ResourceMutationKind, bool)>>>>,
    lists: std::sync::Arc<std::sync::Mutex<Vec<StoreListRequest>>>,
    watches: std::sync::Arc<std::sync::Mutex<Vec<StoreWatchRequest>>>,
}

impl ResourceStoreBackend for ScopedStore {
    async fn get(&self, _: StoreGetRequest) -> Result<StoredResource, StoreError> {
        unreachable!("scoped commit proof does not read through the backend")
    }

    async fn list(&self, request: StoreListRequest) -> Result<StoreListResult, StoreError> {
        self.lists.lock().unwrap().push(request);
        Ok(StoreListResult {
            resources: Vec::new(),
            snapshot_revision: ZoneRevision::new(8),
            next_cursor: None,
            truncated: false,
        })
    }

    async fn watch(&self, request: StoreWatchRequest) -> Result<StoreWatchReceipt, StoreError> {
        self.watches.lock().unwrap().push(request);
        Ok(StoreWatchReceipt {
            stream_name: "scoped-watch".to_owned(),
            snapshot_revision: ZoneRevision::new(8),
        })
    }

    async fn resolve_ref(
        &self,
        _: StoreResolveRequest,
    ) -> Result<StoreResolvedIdentity, StoreError> {
        unreachable!("scoped commit proof does not resolve refs through the backend")
    }

    async fn inspect_schema(
        &self,
        _: StoreInspectSchemaRequest,
    ) -> Result<StoredSchema, StoreError> {
        unreachable!("scoped commit proof does not inspect schemas through the backend")
    }

    async fn commit_verified(
        &self,
        mutation: d2b_resource_store::SealedMutation,
    ) -> Result<StoreCommitResult, StoreError> {
        let body: MutationSealBody = self.acceptor.open(mutation).unwrap().into_body();
        let observed = body
            .mutations
            .iter()
            .map(|mutation| {
                (
                    mutation.mutation().kind,
                    mutation.mutation().assignment.is_some(),
                )
            })
            .collect::<Vec<_>>();
        self.commits.lock().unwrap().push(observed);
        Ok(StoreCommitResult {
            resources: Vec::new(),
            revision: ZoneRevision::new(8),
        })
    }
}

fn commit_batch_frame(operation_id: &str) -> Vec<u8> {
    let envelope = assignment_resource();
    let identity = d2b_contracts_resource::resource_proto::ResourceIdentity {
        zone: "dev".to_owned(),
        resource_type: PROCESS_RESOURCE_TYPE.to_owned(),
        name: "work".to_owned(),
        uid: Some(envelope.metadata().uid().as_str().to_owned()),
        generation: Some(envelope.metadata().generation().get()),
        revision: Some(envelope.metadata().revision().get()),
        special_fields: protobuf::SpecialFields::new(),
    };
    let mut precondition = d2b_contracts_resource::resource_proto::Precondition::new();
    precondition.kind = protobuf::EnumOrUnknown::new(
        d2b_contracts_resource::resource_proto::PreconditionKind::PRECONDITION_KIND_EXACT_REVISION,
    );
    precondition.expected_revision = Some(envelope.metadata().revision().get());
    precondition.expected_uid = Some(envelope.metadata().uid().as_str().to_owned());
    let mut status = d2b_contracts_resource::resource_proto::Mutation::new();
    status.kind = protobuf::EnumOrUnknown::new(
        d2b_contracts_resource::resource_proto::MutationKind::MUTATION_KIND_UPDATE_STATUS,
    );
    status.target = protobuf::MessageField::some(identity.clone());
    status.precondition = protobuf::MessageField::some(precondition.clone());
    let mut status_body = d2b_contracts_resource::resource_proto::ResourceEnvelopeBytes::new();
    status_body.identity = protobuf::MessageField::some(identity.clone());
    status_body.canonical_json = envelope.canonical_bytes().unwrap();
    status_body.payload_digest = envelope.digest().unwrap();
    status.resource = protobuf::MessageField::some(status_body);

    let mut finalizers = d2b_contracts_resource::resource_proto::Mutation::new();
    finalizers.kind = protobuf::EnumOrUnknown::new(
        d2b_contracts_resource::resource_proto::MutationKind::MUTATION_KIND_UPDATE_FINALIZERS,
    );
    finalizers.target = protobuf::MessageField::some(identity);
    finalizers.precondition = protobuf::MessageField::some(precondition);
    finalizers
        .add_finalizers
        .push("process.d2bus.org/reconcile".to_owned());

    let mut meta = d2b_contracts_resource::resource_proto::RequestMeta::new();
    meta.operation_id = operation_id.to_owned();
    meta.idempotency_key = format!("{operation_id}-key");
    meta.correlation_id = format!("{operation_id}-correlation");
    meta.deadline_ms = 10_000;
    let request = d2b_contracts_resource::resource_proto::CommitBatchRequest {
        meta: protobuf::MessageField::some(meta),
        mutations: vec![status, finalizers],
        scoped_admission: Vec::new(),
        special_fields: protobuf::SpecialFields::new(),
    };
    let rpc = TtrpcRequest {
        service: "d2b.resource.v3.ResourceService".to_owned(),
        method: "CommitBatch".to_owned(),
        payload: request.write_to_bytes().unwrap(),
        ..TtrpcRequest::default()
    };
    let body = rpc.write_to_bytes().unwrap();
    let header = MessageHeader::new_request(1, body.len() as u32);
    let mut frame = Vec::with_capacity(ttrpc::proto::MESSAGE_HEADER_LENGTH + body.len());
    frame.extend_from_slice(&Vec::from(header));
    frame.extend_from_slice(&body);
    frame
}

fn query_frame(method: &str) -> Vec<u8> {
    let mut meta = resource_wire::RequestMeta::new();
    meta.operation_id = format!("query-{method}");
    meta.idempotency_key = format!("query-{method}-key");
    meta.correlation_id = format!("query-{method}-correlation");
    meta.deadline_ms = 10_000;
    let payload = if method == "Watch" {
        let mut request = resource_wire::WatchRequest::new();
        request.meta = MessageField::some(meta);
        request.resource_types.push("Host".to_owned());
        request.filters.push(resource_wire::ListFilter {
            field: "metadata.name".to_owned(),
            values: vec!["caller-supplied".to_owned()],
            ..resource_wire::ListFilter::default()
        });
        let mut projection = resource_wire::Projection::new();
        projection.kind = EnumOrUnknown::new(resource_wire::ProjectionKind::PROJECTION_KIND_FULL);
        request.projection = MessageField::some(projection);
        request.write_to_bytes().unwrap()
    } else {
        let mut request = resource_wire::ListRequest::new();
        request.meta = MessageField::some(meta);
        request.resource_types.push("Host".to_owned());
        request.filters.push(resource_wire::ListFilter {
            field: "metadata.name".to_owned(),
            values: vec!["caller-supplied".to_owned()],
            ..resource_wire::ListFilter::default()
        });
        let mut projection = resource_wire::Projection::new();
        projection.kind = EnumOrUnknown::new(resource_wire::ProjectionKind::PROJECTION_KIND_FULL);
        request.projection = MessageField::some(projection);
        request.write_to_bytes().unwrap()
    };
    let rpc = TtrpcRequest {
        service: "d2b.resource.v3.ResourceService".to_owned(),
        method: method.to_owned(),
        payload,
        ..TtrpcRequest::default()
    };
    let body = rpc.write_to_bytes().unwrap();
    let header = MessageHeader::new_request(1, body.len() as u32);
    let mut frame = Vec::with_capacity(ttrpc::proto::MESSAGE_HEADER_LENGTH + body.len());
    frame.extend_from_slice(&Vec::from(header));
    frame.extend_from_slice(&body);
    frame
}

#[tokio::test]
async fn production_owner_child_queries_rewrite_list_and_watch_payloads() {
    let (_bus, mut registrar, assignments, native, state) = scoped_bus();
    let resource = assignment_resource();
    let role = ControllerRoleContract::from_signed_manifest(
        ResourceRef::parse("Provider/system-core").unwrap(),
        ResourceRef::parse("Process/process-controller").unwrap(),
        &assignment_manifest(),
    )
    .unwrap();
    let lease = assignments
        .lock()
        .unwrap()
        .admit(AssignmentRequest::new(
            &resource,
            &role,
            ResourceGeneration::new(PROVIDER_GENERATION).unwrap(),
            ControllerGeneration::new(CONTROLLER_GENERATION).unwrap(),
            ReconnectGeneration::new(1).unwrap(),
            true,
        ))
        .unwrap();
    let owner_uid = resource.metadata().uid().clone();
    let query = ResourceQuery::from_scoped(
        lease
            .child_query(
                vec![
                    d2b_contracts_resource::v3::ResourceTypeName::parse(PROCESS_RESOURCE_TYPE)
                        .unwrap(),
                ],
                Vec::new(),
                Vec::new(),
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        lease.child_query(
            vec![d2b_contracts_resource::v3::ResourceTypeName::parse("Host").unwrap()],
            Vec::new(),
            Vec::new(),
        ),
        Err(AssignmentError::QueryWidened)
    );

    let store_identity = StoreSealIdentity::new(
        StoreSlot::new(0).unwrap(),
        ZoneId::parse("dev").unwrap(),
        ResourceUid::parse("99999999-9999-4999-8999-999999999999").unwrap(),
    );
    let acceptor = native.take_store_seal(store_identity).unwrap();
    let lists = Arc::new(std::sync::Mutex::new(Vec::new()));
    let watches = Arc::new(std::sync::Mutex::new(Vec::new()));
    let store = Arc::new(ScopedStore {
        acceptor,
        commits: Arc::new(std::sync::Mutex::new(Vec::new())),
        lists: Arc::clone(&lists),
        watches: Arc::clone(&watches),
    });
    let service = Arc::new(ResourceService::new(Arc::clone(&store), Arc::clone(&native)).unwrap());
    let (resource_endpoint, resource_remote, resource_echo) = admit_without_echo(
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
    let endpoint_subject = native
        .issue_authenticated_subject(resource_endpoint.route_binding().context().clone(), state)
        .unwrap();
    let adapter =
        Arc::new(ResourceBusAdapter::bind_component_session(service, endpoint_subject).unwrap());
    let service_task = tokio::spawn(serve_ttrpc_services(
        Arc::new(resource_remote),
        adapter.ttrpc_services(),
    ));
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    let _resource_ingress = registrar
        .register_component_session(resource_endpoint)
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
    for (method, call) in [
        ("List", ResourceCall::List(query.clone())),
        ("Watch", ResourceCall::Watch(query)),
    ] {
        let response = caller
            .invoke_resource(
                route(
                    "d2b.resource.v3",
                    &format!("ResourceService/{method}"),
                    1,
                    "Provider/system-core",
                ),
                OperationSpec::new(
                    OperationId::parse(&format!("owner-child-{method}")).unwrap(),
                    10_000,
                )
                .unwrap(),
                call,
                query_frame(method),
            )
            .await
            .unwrap();
        let response = ttrpc::proto::Response::parse_from_bytes(
            &response.as_bytes()[ttrpc::proto::MESSAGE_HEADER_LENGTH..],
        )
        .unwrap();
        assert_eq!(
            response.status.as_ref().map(|status| status.code()),
            Some(ttrpc::proto::Code::OK),
            "ttrpc status: {:?}",
            response.status
        );
        if method == "List" {
            let response =
                resource_wire::ListResponse::parse_from_bytes(&response.payload).unwrap();
            assert!(response.error.is_none());
            assert_eq!(response.snapshot_revision, 8);
        } else {
            let response =
                resource_wire::WatchResponse::parse_from_bytes(&response.payload).unwrap();
            assert!(response.error.is_none());
            assert_eq!(response.snapshot_revision, 8);
        }
    }

    let lists = lists.lock().unwrap();
    assert_eq!(lists.len(), 1);
    assert_eq!(
        lists[0].resource_types,
        vec![d2b_contracts_resource::v3::ResourceTypeName::parse(PROCESS_RESOURCE_TYPE).unwrap()]
    );
    assert_eq!(lists[0].resource_names, Vec::new());
    assert_eq!(lists[0].filters.len(), 1);
    assert_eq!(lists[0].filters[0].field, "owner.resourceUid");
    assert_eq!(
        lists[0].filters[0].values,
        vec![owner_uid.as_str().to_owned()]
    );
    drop(lists);

    let watches = watches.lock().unwrap();
    assert_eq!(watches.len(), 1);
    assert_eq!(
        watches[0].resource_types,
        vec![d2b_contracts_resource::v3::ResourceTypeName::parse(PROCESS_RESOURCE_TYPE).unwrap()]
    );
    assert_eq!(watches[0].resource_names, Vec::new());
    assert_eq!(watches[0].filters.len(), 1);
    assert_eq!(watches[0].filters[0].field, "owner.resourceUid");
    assert_eq!(
        watches[0].filters[0].values,
        vec![owner_uid.as_str().to_owned()]
    );

    service_task.abort();
    let _ = service_task.await;
    resource_echo.abort();
    caller_echo.abort();
}

#[tokio::test]
async fn production_scoped_commit_chain_authorizes_and_fences_store_writes() {
    let (_bus, mut registrar, assignments, native, state) = scoped_bus();
    let resource = assignment_resource();
    let role = ControllerRoleContract::from_signed_manifest(
        ResourceRef::parse("Provider/system-core").unwrap(),
        ResourceRef::parse("Process/process-controller").unwrap(),
        &assignment_manifest(),
    )
    .unwrap();
    let old_lease = assignments
        .lock()
        .unwrap()
        .admit(AssignmentRequest::new(
            &resource,
            &role,
            ResourceGeneration::new(PROVIDER_GENERATION).unwrap(),
            ControllerGeneration::new(CONTROLLER_GENERATION).unwrap(),
            ReconnectGeneration::new(1).unwrap(),
            true,
        ))
        .unwrap();
    let old_identity = old_lease.identity().clone();
    let target = ResourceRef::parse("Process/work").unwrap();
    let old_mutations = vec![
        old_lease
            .mutation(target.clone(), AssignmentVerb::UpdateStatus)
            .unwrap(),
        old_lease
            .mutation(target.clone(), AssignmentVerb::UpdateFinalizers)
            .unwrap(),
    ];

    let (resource_endpoint, resource_remote, resource_echo) = admit_without_echo(
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
    let store_identity = StoreSealIdentity::new(
        StoreSlot::new(0).unwrap(),
        ZoneId::parse("dev").unwrap(),
        ResourceUid::parse("99999999-9999-4999-8999-999999999999").unwrap(),
    );
    let acceptor = native.take_store_seal(store_identity).unwrap();
    let commits = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let lists = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let watches = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let store = std::sync::Arc::new(ScopedStore {
        acceptor,
        commits: std::sync::Arc::clone(&commits),
        lists: Arc::clone(&lists),
        watches: Arc::clone(&watches),
    });
    let service = std::sync::Arc::new(
        ResourceService::new(
            std::sync::Arc::clone(&store),
            std::sync::Arc::clone(&native),
        )
        .unwrap(),
    );
    let endpoint_subject = native
        .issue_authenticated_subject(
            resource_endpoint.route_binding().context().clone(),
            state.clone(),
        )
        .unwrap();
    let adapter = std::sync::Arc::new(
        ResourceBusAdapter::bind_component_session(service, endpoint_subject).unwrap(),
    );
    let service_task = tokio::spawn(serve_ttrpc_services(
        std::sync::Arc::new(resource_remote),
        adapter.ttrpc_services(),
    ));
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    let _resource_ingress = registrar
        .register_component_session(resource_endpoint)
        .await
        .unwrap();

    let (caller, _caller_remote, caller_echo) = admit(
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
        "ResourceService/CommitBatch",
        1,
        "Provider/system-core",
    );

    assignments
        .lock()
        .unwrap()
        .begin_drain(&old_identity)
        .unwrap();
    let stale = caller
        .invoke_scoped_commit_batch(
            route.clone(),
            OperationSpec::new(OperationId::parse("scoped-stale").unwrap(), 10_000).unwrap(),
            old_identity.clone(),
            old_mutations.clone(),
            commit_batch_frame("scoped-stale"),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        stale,
        BusError::Authorization(AuthorizationError::Assignment(
            AssignmentError::StaleAssignment
        ))
    ));

    assignments.lock().unwrap().release(&old_identity).unwrap();
    let new_lease = assignments
        .lock()
        .unwrap()
        .admit(AssignmentRequest::new(
            &resource,
            &role,
            ResourceGeneration::new(PROVIDER_GENERATION).unwrap(),
            ControllerGeneration::new(CONTROLLER_GENERATION).unwrap(),
            ReconnectGeneration::new(1).unwrap(),
            true,
        ))
        .unwrap();
    let new_identity = new_lease.identity().clone();
    let new_mutations = vec![
        new_lease
            .mutation(target.clone(), AssignmentVerb::UpdateStatus)
            .unwrap(),
        new_lease
            .mutation(target.clone(), AssignmentVerb::UpdateFinalizers)
            .unwrap(),
    ];
    let revoked_mutations = new_mutations.clone();
    let mut scoped_response = None;
    for attempt in 0..32 {
        let operation_id = format!("scoped-valid-{attempt}");
        let response = caller
            .invoke_scoped_commit_batch(
                route.clone(),
                OperationSpec::new(OperationId::parse(&operation_id).unwrap(), 10_000).unwrap(),
                new_identity.clone(),
                new_mutations.clone(),
                commit_batch_frame(&operation_id),
            )
            .await
            .unwrap();
        let response = ttrpc::proto::Response::parse_from_bytes(
            &response.as_bytes()[ttrpc::proto::MESSAGE_HEADER_LENGTH..],
        )
        .unwrap();
        let response =
            d2b_contracts_resource::resource_proto::CommitBatchResponse::parse_from_bytes(
                &response.payload,
            )
            .unwrap();
        if response.error.is_some() {
            scoped_response = Some(response);
            break;
        }
        if response.revision == 8 {
            scoped_response = Some(response);
            break;
        }
        tokio::task::yield_now().await;
    }
    let response = scoped_response.expect("scoped commit response");
    assert!(response.error.is_none());
    assert_eq!(response.revision, 8);

    let forged_scope =
        ScopedCommitTransport::new(new_identity.clone(), new_mutations.clone()).unwrap();
    let forged_frame = d2b_resource_api::attach_scoped_commit_frame(
        &commit_batch_frame("plain-forged"),
        &forged_scope,
    )
    .unwrap();
    let forged = caller
        .invoke_resource(
            route.clone(),
            OperationSpec::new(OperationId::parse("plain-forged").unwrap(), 10_000).unwrap(),
            ResourceCall::CommitBatch(vec![
                (target.clone(), ResourceVerb::UpdateStatus),
                (target.clone(), ResourceVerb::UpdateFinalizers),
            ]),
            forged_frame,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        forged,
        BusError::Endpoint(EndpointError::Rejected)
    ));

    for attempt in 0..8 {
        if commits.lock().unwrap().len() >= 2 {
            break;
        }
        let operation_id = format!("plain-commit-{attempt}");
        caller
            .invoke_resource(
                route.clone(),
                OperationSpec::new(OperationId::parse(&operation_id).unwrap(), 10_000).unwrap(),
                ResourceCall::CommitBatch(vec![
                    (target.clone(), ResourceVerb::UpdateStatus),
                    (target.clone(), ResourceVerb::UpdateFinalizers),
                ]),
                commit_batch_frame(&operation_id),
            )
            .await
            .unwrap();
        for _ in 0..8 {
            if commits.lock().unwrap().len() >= 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    }

    assignments
        .lock()
        .unwrap()
        .revoke_session(ReconnectGeneration::new(1).unwrap());
    let revoked = caller
        .invoke_scoped_commit_batch(
            route,
            OperationSpec::new(OperationId::parse("scoped-revoked").unwrap(), 10_000).unwrap(),
            new_identity,
            revoked_mutations,
            commit_batch_frame("scoped-revoked"),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        revoked,
        BusError::Authorization(AuthorizationError::Assignment(
            AssignmentError::SessionRevoked
        ))
    ));

    let commits = commits.lock().unwrap().clone();
    assert_eq!(commits.len(), 2);
    assert_eq!(
        commits[0],
        vec![
            (ResourceMutationKind::UpdateStatus, true),
            (ResourceMutationKind::UpdateFinalizers, true),
        ]
    );
    assert_eq!(
        commits[1],
        vec![
            (ResourceMutationKind::UpdateStatus, false),
            (ResourceMutationKind::UpdateFinalizers, false),
        ]
    );
    assert!(commits[0].iter().all(|(_, fenced)| *fenced));
    assert!(commits[1].iter().all(|(_, fenced)| !*fenced));

    service_task.abort();
    let _ = service_task.await;
    resource_echo.abort();
    caller_echo.abort();
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
        "Guest/bob",
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
    let first_operation = OperationSpec::new(first_id, 10_000).unwrap();
    let first = caller.invoke_resource(
        route.clone(),
        first_operation.clone(),
        ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
        ttrpc_frame(7, b"first"),
    );
    let sequence = async {
        let first_frame = remote.receive_ttrpc().await.unwrap();
        let first_internal_id = ttrpc_stream_id(&first_frame).unwrap();
        assert_eq!(first_internal_id % 2, 1);
        caller.cancel(&first_operation).await.unwrap();
        let second = caller.invoke_resource(
            route.clone(),
            OperationSpec::new(second_id, 10_000).unwrap(),
            ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
            ttrpc_frame(7, b"second"),
        );
        let responses = async {
            let second_frame = remote.receive_ttrpc().await.unwrap();
            let second_internal_id = ttrpc_stream_id(&second_frame).unwrap();
            assert_eq!(second_internal_id % 2, 1);
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
async fn reconnect_rejects_a_control_batch_queued_behind_an_admitted_write() {
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
    let endpoint_cancellation = endpoint.cancellation_handle();
    let endpoint = registrar
        .register_component_session(endpoint)
        .await
        .unwrap();
    let (replacement, _, replacement_echo) = admit(
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
    let operation_id = OperationId::parse("reconnect-fenced-control").unwrap();
    let invoke = caller.invoke_resource(
        route(
            "d2b.resource.v3",
            "ResourceService/Get",
            1,
            "Provider/system-core",
        ),
        OperationSpec::new(operation_id, 10_000).unwrap(),
        ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
        ttrpc_frame(13, b"admitted"),
    );
    let reconnect = async {
        pause.wait_until_entered().await;
        let queued_control = endpoint_cancellation
            .cancel(d2b_session::contract::RequestId::new(vec![0x44; 16]).unwrap());
        tokio::task::yield_now().await;
        let mut reconnect = Box::pin(registrar.reconnect_component_session(endpoint, replacement));
        tokio::select! {
            result = &mut reconnect => {
                panic!("reconnect returned before the admitted write drained: {result:?}")
            }
            () = tokio::task::yield_now() => {}
        }
        pause.release.notify_one();
        let replacement = reconnect.await.unwrap();
        assert_eq!(
            queued_control.await.unwrap_err().code(),
            d2b_session::contract::SessionErrorCode::Cancelled
        );
        replacement
    };

    let (result, replacement) = tokio::join!(invoke, reconnect);
    assert_eq!(result, Err(BusError::Cancelled));
    registrar
        .disconnect_component_session(replacement)
        .await
        .unwrap();
    endpoint_echo.abort();
    replacement_echo.abort();
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
    let operation = OperationSpec::new(operation_id, 10_000).unwrap();
    let invoke = caller.invoke_resource(
        route(
            "d2b.resource.v3",
            "ResourceService/Get",
            1,
            "Provider/system-core",
        ),
        operation.clone(),
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
        caller.cancel(&operation).await,
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
    let operation = OperationSpec::new(operation_id, 10_000).unwrap();
    let invoking = std::sync::Arc::clone(&caller);
    let invoked_operation = operation.clone();
    let invoke = tokio::spawn(async move {
        invoking
            .invoke_resource(
                route(
                    "d2b.resource.v3",
                    "ResourceService/Get",
                    1,
                    "Provider/system-core",
                ),
                invoked_operation,
                ResourceCall::Get(ResourceRef::parse("Host/system").unwrap()),
                ttrpc_frame(42, b"wait"),
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(1), dispatched_wait)
        .await
        .expect("invoke must reach the remote request")
        .unwrap();
    caller.cancel(&operation).await.unwrap();
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
