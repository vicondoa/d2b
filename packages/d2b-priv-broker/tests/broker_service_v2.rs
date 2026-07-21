use nix::libc;
use std::{
    collections::{HashMap, VecDeque},
    fs::File,
    io::{Seek, SeekFrom, Write},
    os::fd::{AsRawFd, BorrowedFd, OwnedFd},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Child, Command},
    sync::atomic::{AtomicUsize, Ordering},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{
        AttachmentAccess, AttachmentCreditClass, AttachmentDescriptor, AttachmentKind,
        AttachmentPurpose, BoundedVec, CloseReason, KernelObjectType, Remediation, RequestId,
        ServicePackage, SessionErrorCode,
    },
    v2_services::{
        SERVICE_INVENTORY,
        broker::{
            AllocateRequest, AllocateResponse, SpawnRealmChildrenRequest,
            SpawnRealmChildrenResponse,
        },
        broker_ttrpc::BrokerService,
        common::{self, Outcome, ServiceRequest, ServiceResponse},
    },
};
use d2b_host::realm_children::{
    PidfdEvidence, PidfdIdentityVerifier, RealmChildBootstrapEndpoints, RealmChildDescriptorSet,
    RealmChildIdentity, RealmChildLaunchRecord, RealmChildRole, UnixSessionError,
};
use d2b_priv_broker::allocator_service::{
    AllocatorChildBrokerService, AllocatorServiceError, PendingSpawnedRealmChild,
    PendingSpawnedRealmPair, RealmChildSpawner, RealmLaunchRecordResolver, ServiceReply,
};
use d2b_priv_broker::service_v2::{
    AllocatorServiceDispatch, AuthorizedFdRegistry, AuthorizedResourceBackend, BrokerCallContext,
    BrokerMethod, BrokerOperationHandler, BrokerPeerRole, BrokerReply, BrokerRuntimeDispatch,
    BrokerServiceFailure, BrokerServiceV2, ProductionBrokerOperationHandler,
    RealmBrokerSessionBinding,
};
use d2b_realm_core::{
    allocator::LeaseOwner,
    allocator_engine::{
        FakeAllocatorLedger, FakeAllocatorLiveness, FakeObservedAllocatorState,
        LocalRootAllocatorEngine,
    },
    ids::{ControllerGenerationId, RealmId},
    realm::RealmPath,
};
use d2b_session::{
    AttachmentPayload, AttachmentValidationError, ComponentSessionDriver, OwnedAttachment,
    OwnedTransport, RequestRegistry, SessionError, StreamEvent, StreamId, TransportPacket,
};
use d2b_session_unix::{
    CreditPool, CreditScopeSet, DescriptorPolicy, ObjectIdentity, OwnedUnixAttachment,
    PeerIdentityPolicy, SeqpacketSocket, UnixAttachmentPayload, UnixSeqpacketTransport,
};
use nix::sys::socket::{
    AddressFamily, Backlog, MsgFlags, SockFlag, SockType, UnixAddr, accept4, bind, connect, listen,
    recv, send, setsockopt, socket, socketpair, sockopt::PassCred,
};
use protobuf::{Enum, EnumOrUnknown, MessageField};
use tokio::sync::Notify;

#[test]
fn realm_broker_binding_uses_authenticated_child_roles_only() {
    let binding = RealmBrokerSessionBinding::new("work".to_owned(), 1001, 1002, 7).unwrap();
    assert_eq!(binding.realm_id(), "work");
    let policy = binding.endpoint_policy().unwrap();
    assert_eq!(
        policy.initiator_role,
        d2b_contracts::v2_component_session::EndpointRole::RealmController
    );
    assert_eq!(
        policy.responder_role,
        d2b_contracts::v2_component_session::EndpointRole::RealmBroker
    );
    assert!(RealmBrokerSessionBinding::new("work".to_owned(), 0, 1002, 7).is_err());
}

const GENERATION: u64 = 7;

#[derive(Default)]
struct RecordingHandler {
    methods: Mutex<Vec<BrokerMethod>>,
}

struct BlockingHandler {
    entered: Notify,
}

struct ClosingHandler;

#[async_trait]
impl BrokerOperationHandler for ClosingHandler {
    async fn handle(
        &self,
        _: BrokerMethod,
        request: ServiceRequest,
        attachments: Vec<OwnedAttachment>,
        _: &BrokerCallContext,
    ) -> Result<BrokerReply<ServiceResponse>, BrokerServiceFailure> {
        assert_eq!(attachments.len(), 1);
        drop(attachments);
        Ok(BrokerReply::message(ServiceResponse {
            outcome: EnumOrUnknown::new(Outcome::OUTCOME_SUCCEEDED),
            operation_id: request.operation_id,
            ..Default::default()
        }))
    }

    async fn allocate(
        &self,
        _: AllocateRequest,
        _: &BrokerCallContext,
    ) -> Result<BrokerReply<AllocateResponse>, BrokerServiceFailure> {
        panic!("unexpected allocate")
    }

    async fn spawn(
        &self,
        _: SpawnRealmChildrenRequest,
        _: Vec<OwnedAttachment>,
        _: &BrokerCallContext,
    ) -> Result<BrokerReply<SpawnRealmChildrenResponse>, BrokerServiceFailure> {
        panic!("unexpected spawn")
    }
}

struct CountedPayload(Arc<AtomicUsize>);

impl AttachmentPayload for CountedPayload {
    fn close(self: Box<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any + Send> {
        self
    }

    fn validate_descriptor(
        &self,
        _: &AttachmentDescriptor,
    ) -> Result<(), AttachmentValidationError> {
        Ok(())
    }
}

#[async_trait]
impl BrokerOperationHandler for BlockingHandler {
    async fn handle(
        &self,
        _: BrokerMethod,
        _: ServiceRequest,
        _: Vec<OwnedAttachment>,
        _: &BrokerCallContext,
    ) -> Result<BrokerReply<ServiceResponse>, BrokerServiceFailure> {
        self.entered.notify_one();
        std::future::pending().await
    }

    async fn allocate(
        &self,
        _: AllocateRequest,
        _: &BrokerCallContext,
    ) -> Result<BrokerReply<AllocateResponse>, BrokerServiceFailure> {
        panic!("unexpected allocate")
    }

    async fn spawn(
        &self,
        _: SpawnRealmChildrenRequest,
        _: Vec<OwnedAttachment>,
        _: &BrokerCallContext,
    ) -> Result<BrokerReply<SpawnRealmChildrenResponse>, BrokerServiceFailure> {
        panic!("unexpected spawn")
    }
}

#[async_trait]
impl BrokerOperationHandler for RecordingHandler {
    async fn handle(
        &self,
        method: BrokerMethod,
        request: ServiceRequest,
        attachments: Vec<OwnedAttachment>,
        _: &BrokerCallContext,
    ) -> Result<BrokerReply<ServiceResponse>, BrokerServiceFailure> {
        assert!(attachments.is_empty());
        self.methods
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(method);
        Ok(BrokerReply::message(ServiceResponse {
            outcome: EnumOrUnknown::new(Outcome::OUTCOME_SUCCEEDED),
            operation_id: request.operation_id,
            ..Default::default()
        }))
    }

    async fn allocate(
        &self,
        _: AllocateRequest,
        _: &BrokerCallContext,
    ) -> Result<BrokerReply<AllocateResponse>, BrokerServiceFailure> {
        panic!("allocator call was not admitted")
    }

    async fn spawn(
        &self,
        _: SpawnRealmChildrenRequest,
        _: Vec<OwnedAttachment>,
        _: &BrokerCallContext,
    ) -> Result<BrokerReply<SpawnRealmChildrenResponse>, BrokerServiceFailure> {
        panic!("spawn call was not admitted")
    }
}

struct TestDriver {
    generation: u64,
    requests: Mutex<RequestRegistry>,
    attachments: Mutex<VecDeque<Vec<OwnedAttachment>>>,
}

impl TestDriver {
    fn new(generation: u64) -> Self {
        Self {
            generation,
            requests: Mutex::new(RequestRegistry::new(generation).expect("registry")),
            attachments: Mutex::new(VecDeque::new()),
        }
    }

    fn unavailable<T>() -> d2b_session::Result<T> {
        Err(SessionError::new(SessionErrorCode::RecordMalformed))
    }

    fn push_attachments(&self, attachments: Vec<OwnedAttachment>) {
        self.attachments
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push_back(attachments);
    }
}

#[async_trait]
impl ComponentSessionDriver for TestDriver {
    fn generation(&self) -> u64 {
        self.generation
    }

    async fn start_ttrpc(&self, _: RequestId, _: Vec<u8>) -> d2b_session::Result<()> {
        Self::unavailable()
    }

    async fn complete_ttrpc(&self, _: RequestId) -> d2b_session::Result<bool> {
        Self::unavailable()
    }

    async fn cancel(&self, _: u64, _: RequestId) -> d2b_session::Result<()> {
        Self::unavailable()
    }

    async fn send_ttrpc(&self, _: Vec<u8>) -> d2b_session::Result<()> {
        Self::unavailable()
    }

    async fn receive_ttrpc(&self) -> d2b_session::Result<Vec<u8>> {
        Self::unavailable()
    }

    async fn register_inbound_call(
        &self,
        request_id: RequestId,
    ) -> d2b_session::Result<d2b_session::Cancellation> {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .register(request_id)
    }

    async fn complete_inbound_call(&self, request_id: RequestId) -> d2b_session::Result<bool> {
        Ok(self
            .requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .complete(&request_id))
    }

    async fn remove_inbound_call(&self, request_id: RequestId) -> d2b_session::Result<bool> {
        Ok(self
            .requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&request_id))
    }

    async fn send_attachments(&self, attachments: Vec<OwnedAttachment>) -> d2b_session::Result<()> {
        assert!(attachments.is_empty());
        Ok(())
    }

    async fn receive_attachments(&self) -> d2b_session::Result<Vec<OwnedAttachment>> {
        self.attachments
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop_front()
            .ok_or_else(|| SessionError::new(SessionErrorCode::AttachmentCountMismatch))
    }

    async fn open_named_stream(&self, _: StreamId, _: u32, _: u32) -> d2b_session::Result<()> {
        Self::unavailable()
    }

    async fn send_named_stream(&self, _: StreamId, _: Vec<u8>) -> d2b_session::Result<()> {
        Self::unavailable()
    }

    async fn receive_named_stream(&self) -> d2b_session::Result<StreamEvent> {
        Self::unavailable()
    }

    async fn grant_named_stream_credit(&self, _: StreamId, _: u32) -> d2b_session::Result<()> {
        Self::unavailable()
    }

    async fn close_named_stream(&self, _: StreamId) -> d2b_session::Result<()> {
        Self::unavailable()
    }

    async fn reset_named_stream(&self, _: StreamId) -> d2b_session::Result<()> {
        Self::unavailable()
    }

    async fn drive_keepalive(&self, _: std::time::Instant) -> d2b_session::Result<()> {
        Ok(())
    }

    async fn receive_control(&self) -> d2b_session::Result<d2b_session::SessionEvent> {
        Self::unavailable()
    }

    async fn close(&self, _: CloseReason, _: Remediation) -> d2b_session::Result<()> {
        Ok(())
    }
}

fn context() -> ttrpc::r#async::TtrpcContext {
    ttrpc::r#async::TtrpcContext {
        mh: ttrpc::proto::MessageHeader::new_request(1, 0),
        metadata: HashMap::new(),
        timeout_nano: 5_000_000_000,
    }
}

fn request(generation: u64) -> ServiceRequest {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis() as u64;
    ServiceRequest {
        metadata: MessageField::some(common::RequestMetadata {
            request_id: vec![0x11; 16],
            issued_at_unix_ms: now,
            expires_at_unix_ms: now + 5_000,
            session_generation: generation,
            ..Default::default()
        }),
        scope: MessageField::some(common::IdentityScope {
            realm_id: "aaaaaaaaaaaaaaaaaaaa".to_owned(),
            ..Default::default()
        }),
        resource_id: "intent-1".to_owned(),
        operation_id: "operation-1".to_owned(),
        ..Default::default()
    }
}

fn observe_descriptor(counter: Arc<AtomicUsize>) -> OwnedAttachment {
    let method = SERVICE_INVENTORY
        .iter()
        .find(|service| service.package == "d2b.broker.v2")
        .and_then(|service| {
            service
                .methods
                .iter()
                .find(|method| method.name == "Observe")
        })
        .expect("observe method");
    OwnedAttachment::new(
        AttachmentDescriptor {
            index: 0,
            kind: AttachmentKind::FileDescriptor,
            object_type: KernelObjectType::RegularFile,
            access: AttachmentAccess::ReadWrite,
            purpose: AttachmentPurpose::RequestInput,
            service: ServicePackage::BrokerV2,
            method_id: method.method_id("d2b.broker.v2", "BrokerService"),
            request_id: RequestId::new(vec![0x11; 16]).expect("request id"),
            operation_id: None,
            packet_sequence: 1,
            reconnect_generation: GENERATION,
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
            .expect("credit classes"),
        },
        Box::new(CountedPayload(counter)),
    )
}

#[tokio::test]
async fn authenticated_request_uses_generated_service_and_exact_generation() {
    let driver: Arc<dyn ComponentSessionDriver> = Arc::new(TestDriver::new(GENERATION));
    let handler = Arc::new(RecordingHandler::default());
    let service = BrokerServiceV2::new(
        Arc::clone(&handler),
        driver,
        BrokerPeerRole::RealmController,
    );
    let response = service
        .observe(&context(), request(GENERATION))
        .await
        .expect("observe");
    assert_eq!(
        response.outcome.enum_value(),
        Ok(Outcome::OUTCOME_SUCCEEDED)
    );
    assert_eq!(
        *handler
            .methods
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        [BrokerMethod::Observe]
    );

    let error = service
        .observe(&context(), request(GENERATION + 1))
        .await
        .expect_err("generation mismatch");
    assert!(format!("{error}").contains("broker-admission-denied"));
}

#[tokio::test]
async fn realm_controller_cannot_call_local_root_allocator_methods() {
    let driver: Arc<dyn ComponentSessionDriver> = Arc::new(TestDriver::new(GENERATION));
    let service = BrokerServiceV2::new(
        Arc::new(RecordingHandler::default()),
        driver,
        BrokerPeerRole::RealmController,
    );
    let mut allocate = AllocateRequest::new();
    allocate.metadata = request(GENERATION).metadata;
    let error = service
        .allocate(&context(), allocate)
        .await
        .expect_err("realm allocation must fail");
    assert!(format!("{error}").contains("broker-admission-denied"));
}

#[tokio::test]
async fn generated_cancel_signals_only_the_matching_active_request() {
    let driver: Arc<dyn ComponentSessionDriver> = Arc::new(TestDriver::new(GENERATION));
    let handler = Arc::new(BlockingHandler {
        entered: Notify::new(),
    });
    let service = Arc::new(BrokerServiceV2::new(
        Arc::clone(&handler),
        driver,
        BrokerPeerRole::RealmController,
    ));
    let entered = handler.entered.notified();
    let operation = {
        let service = Arc::clone(&service);
        tokio::spawn(async move { service.observe(&context(), request(GENERATION)).await })
    };
    entered.await;
    let response = service
        .cancel(
            &context(),
            common::CancelRequest {
                session_generation: GENERATION,
                request_id: vec![0x11; 16],
                ..Default::default()
            },
        )
        .await
        .expect("cancel response");
    assert_eq!(
        response.outcome.enum_value(),
        Ok(common::CancelOutcome::CANCEL_OUTCOME_CANCELLATION_SIGNALLED)
    );
    let error = operation
        .await
        .expect("operation task")
        .expect_err("operation must be cancelled");
    assert!(format!("{error}").contains("broker-request-cancelled"));
}

#[tokio::test]
async fn authenticated_attachment_table_closes_each_payload_once() {
    let driver = Arc::new(TestDriver::new(GENERATION));
    let closed = Arc::new(AtomicUsize::new(0));
    driver.push_attachments(vec![observe_descriptor(Arc::clone(&closed))]);
    let session: Arc<dyn ComponentSessionDriver> = driver;
    let service = BrokerServiceV2::new(
        Arc::new(ClosingHandler),
        session,
        BrokerPeerRole::RealmController,
    );
    let mut request = request(GENERATION);
    request.attachment_indexes = vec![0];
    service
        .observe(&context(), request)
        .await
        .expect("attachment request");
    assert_eq!(closed.load(Ordering::SeqCst), 1);
}

#[test]
fn broker_debug_surfaces_redact_request_identity() {
    let registry = RequestRegistry::new(GENERATION).expect("registry");
    let rendered = format!("{registry:?}");
    assert!(rendered.contains("<redacted>"));
    assert!(!rendered.contains("operation-1"));
}

#[test]
fn production_runtime_contains_only_real_allocator_adapters() {
    let runtime = include_str!("../src/runtime.rs");
    let production = runtime
        .split("#[cfg(test)]")
        .next()
        .expect("production runtime prefix");
    assert!(!production.contains("FakeAllocator"));
    assert!(!production.contains("RuntimeLaunchRecordResolver::new(BTreeMap::new())"));
    assert!(production.contains("DurableAllocatorLedger"));
    assert!(production.contains("RuntimeObservedAllocatorState"));
    assert!(production.contains("RuntimeAllocatorLiveness"));
    assert!(production.contains("find_realm_child_launch_record"));
}

struct NoopRuntime;

#[async_trait]
impl BrokerRuntimeDispatch for NoopRuntime {
    async fn dispatch(
        &self,
        _: BrokerMethod,
        _: ServiceRequest,
        _: Vec<OwnedAttachment>,
        _: &BrokerCallContext,
    ) -> Result<BrokerReply<ServiceResponse>, BrokerServiceFailure> {
        Err(BrokerServiceFailure::InvalidRequest)
    }
}

#[derive(Clone)]
struct TestLaunchResolver(RealmChildLaunchRecord);

impl RealmLaunchRecordResolver for TestLaunchResolver {
    fn resolve(&self, _: &str, _: &str) -> Result<RealmChildLaunchRecord, AllocatorServiceError> {
        Ok(self.0.clone())
    }
}

struct TestRealmSpawner {
    pids: (u32, u32),
}

impl RealmChildSpawner for TestRealmSpawner {
    fn spawn_pair(
        &self,
        record: &RealmChildLaunchRecord,
        controller_fds: RealmChildDescriptorSet,
        broker_fds: RealmChildDescriptorSet,
        bootstrap: RealmChildBootstrapEndpoints,
    ) -> Result<PendingSpawnedRealmPair, AllocatorServiceError> {
        assert_eq!(controller_fds.role(), RealmChildRole::Controller);
        assert_eq!(broker_fds.role(), RealmChildRole::Broker);
        Ok(PendingSpawnedRealmPair {
            controller: PendingSpawnedRealmChild {
                identity: record.controller.clone(),
                pid: self.pids.0,
                pidfd: d2b_priv_broker::sys::pidfd_sys::pidfd_open(
                    self.pids.0.try_into().unwrap(),
                    0,
                )
                .unwrap(),
            },
            broker: PendingSpawnedRealmChild {
                identity: record.broker.clone(),
                pid: self.pids.1,
                pidfd: d2b_priv_broker::sys::pidfd_sys::pidfd_open(
                    self.pids.1.try_into().unwrap(),
                    0,
                )
                .unwrap(),
            },
            bootstrap,
        })
    }
}

struct ExpectedPidVerifier(u32);

impl PidfdIdentityVerifier for ExpectedPidVerifier {
    fn verify(
        &self,
        pidfd: BorrowedFd<'_>,
        evidence: &PidfdEvidence,
    ) -> Result<(), UnixSessionError> {
        let expected = rustix::process::Pid::from_raw(self.0 as i32)
            .ok_or(UnixSessionError::PidfdIdentityMismatch)?;
        let observed = std::fs::read_to_string(format!("/proc/self/fdinfo/{}", pidfd.as_raw_fd()))
            .map_err(|_| UnixSessionError::PidfdEvidenceUnavailable)?
            .lines()
            .find_map(|line| line.strip_prefix("Pid:"))
            .and_then(|pid| pid.trim().parse::<u32>().ok())
            .ok_or(UnixSessionError::PidfdEvidenceUnavailable)?;
        if evidence.expected_pid() == expected && observed == self.0 {
            Ok(())
        } else {
            Err(UnixSessionError::PidfdIdentityMismatch)
        }
    }
}

type TestAllocatorService = AllocatorChildBrokerService<
    AuthorizedResourceBackend,
    TestLaunchResolver,
    TestRealmSpawner,
    FakeAllocatorLedger,
    FakeObservedAllocatorState,
    FakeAllocatorLiveness,
>;

struct TestAllocatorDispatch {
    service: TestAllocatorService,
    pids: (u32, u32),
}

#[async_trait]
impl AllocatorServiceDispatch for TestAllocatorDispatch {
    async fn allocate(
        &mut self,
        request: &AllocateRequest,
    ) -> Result<ServiceReply<AllocateResponse>, AllocatorServiceError> {
        self.service.allocate(request)
    }

    async fn spawn(
        &self,
        request: &SpawnRealmChildrenRequest,
        attachments: Vec<OwnedFd>,
        bootstrap: RealmChildBootstrapEndpoints,
    ) -> Result<
        ServiceReply<
            SpawnRealmChildrenResponse,
            d2b_priv_broker::allocator_service::PolicyBoundPidfdAttachments,
        >,
        AllocatorServiceError,
    > {
        let reply = self.service.spawn(request, attachments, bootstrap).await?;
        Ok(ServiceReply {
            message: reply.message,
            attachments: reply.attachments.bind_policies(
                Arc::new(ExpectedPidVerifier(self.pids.0)),
                Arc::new(ExpectedPidVerifier(self.pids.1)),
            )?,
        })
    }
}

fn allocator_owner() -> LeaseOwner {
    LeaseOwner {
        realm: RealmPath::new(vec![RealmId::parse("work").unwrap()]).unwrap(),
        controller_generation: ControllerGenerationId::parse("generation-1").unwrap(),
        node: None,
    }
}

fn launch_record() -> RealmChildLaunchRecord {
    RealmChildLaunchRecord {
        realm_id: "aaaaaaaaaaaaaaaaaaaa".into(),
        controller_generation_id: "generation-1".into(),
        launch_record_digest: [7; 32],
        controller: RealmChildIdentity {
            role: RealmChildRole::Controller,
            process_id: "controller-1".into(),
            executable: PathBuf::from("/bin/true"),
            executable_digest: [8; 32],
            cgroup_digest: [10; 32],
            uid: 1001,
            gid: 1001,
        },
        broker: RealmChildIdentity {
            role: RealmChildRole::Broker,
            process_id: "broker-1".into(),
            executable: PathBuf::from("/bin/true"),
            executable_digest: [9; 32],
            cgroup_digest: [11; 32],
            uid: 1002,
            gid: 1002,
        },
    }
}

fn production_handler(
    registry: Arc<AuthorizedFdRegistry>,
    pids: (u32, u32),
) -> ProductionBrokerOperationHandler<NoopRuntime, TestAllocatorDispatch> {
    let owner = allocator_owner();
    ProductionBrokerOperationHandler::new(
        NoopRuntime,
        TestAllocatorDispatch {
            service: AllocatorChildBrokerService::new(
                LocalRootAllocatorEngine::new(
                    owner.clone(),
                    FakeAllocatorLedger::default(),
                    FakeObservedAllocatorState::default(),
                    FakeAllocatorLiveness::new(vec![owner]),
                ),
                AuthorizedResourceBackend::new(Arc::clone(&registry)),
                TestLaunchResolver(launch_record()),
                TestRealmSpawner { pids },
            ),
            pids,
        },
        registry,
    )
}

fn call_context(request_byte: u8) -> BrokerCallContext {
    let request_id = RequestId::new(vec![request_byte; 16]).unwrap();
    let cancellation = RequestRegistry::new(GENERATION)
        .unwrap()
        .register(request_id.clone())
        .unwrap();
    BrokerCallContext {
        peer_role: BrokerPeerRole::LocalRootController,
        request_id,
        session_generation: GENERATION,
        remaining: std::time::Duration::from_secs(5),
        cancellation,
    }
}

fn allocator_metadata(request_byte: u8) -> common::RequestMetadata {
    let mut metadata = request(GENERATION).metadata.into_option().unwrap();
    metadata.request_id = vec![request_byte; 16];
    metadata.correlation_id = format!("correlation-{request_byte}");
    metadata.idempotency_key = vec![request_byte; 16];
    metadata
}

#[tokio::test]
async fn production_allocate_returns_correlated_cloexec_attachment() {
    let registry = Arc::new(AuthorizedFdRegistry::new());
    registry
        .register_resource(
            "namespace-1",
            File::open("/dev/null").unwrap().into(),
            KernelObjectType::Device,
            AttachmentAccess::ReadOnly,
            AttachmentPurpose::DeviceLease,
        )
        .unwrap();
    let handler = production_handler(registry, (1, 2));
    let mut allocate = AllocateRequest::new();
    allocate.metadata = Some(allocator_metadata(0x31)).into();
    allocate.scope = request(GENERATION).scope;
    allocate.operation_id = "allocate-1".into();
    allocate.request_digest = vec![3; 32];
    let mut owner = d2b_contracts::v2_services::broker::LeaseOwner::new();
    owner.realm_path = "work".into();
    owner.controller_generation_id = "generation-1".into();
    allocate.owner = Some(owner).into();
    let mut resource = d2b_contracts::v2_services::broker::LeaseResourceRequest::new();
    resource.resource_id = "namespace-1".into();
    resource.kind =
        d2b_contracts::v2_services::broker::HostResourceKind::HOST_RESOURCE_KIND_NAMESPACE_BOUNDARY
            .into();
    resource.share =
        d2b_contracts::v2_services::broker::ResourceShareMode::RESOURCE_SHARE_MODE_EXCLUSIVE.into();
    resource.acquisition_order = Some(
        d2b_contracts::v2_services::broker::ResourceAcquisitionOrder {
            phase: 1,
            ordinal: 0,
            ..Default::default()
        },
    )
    .into();
    allocate.resources.push(resource);

    let context = call_context(0x31);
    let reply = handler.allocate(allocate, &context).await.unwrap();
    assert_eq!(reply.message.operation_id, "allocate-1");
    assert_eq!(reply.message.resources[0].attachment_index, Some(0));
    assert_eq!(reply.attachments.len(), 1);
    let descriptor = reply.attachments[0].descriptor().unwrap();
    assert_eq!(descriptor.index, 0);
    assert_eq!(descriptor.request_id, context.request_id);
    assert!(descriptor.cloexec_required);
    let fd = reply.attachments[0]
        .payload()
        .and_then(|payload| payload.downcast_ref::<UnixAttachmentPayload>())
        .and_then(UnixAttachmentPayload::file)
        .unwrap();
    assert_ne!(
        nix::fcntl::fcntl(fd.as_raw_fd(), nix::fcntl::FcntlArg::F_GETFD).unwrap()
            & libc::FD_CLOEXEC,
        0
    );
}

fn listener(name: &[u8]) -> OwnedFd {
    let fd = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::SOCK_CLOEXEC,
        None,
    )
    .unwrap();
    bind(fd.as_raw_fd(), &UnixAddr::new_abstract(name).unwrap()).unwrap();
    listen(&fd, Backlog::new(4).unwrap()).unwrap();
    fd
}

static CREDENTIAL_SOCKET_COUNTER: AtomicUsize = AtomicUsize::new(1);

fn socket_tempdir() -> tempfile::TempDir {
    let Some(root) = std::env::var_os("D2B_VALIDATION_SOCKET_DIR").map(PathBuf::from) else {
        return tempfile::tempdir().expect("create broker service socket tempdir");
    };
    std::fs::create_dir_all(&root).expect("create broker service test socket root");
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
        .expect("harden broker service test socket root");
    tempfile::tempdir_in(root).expect("create broker service socket tempdir")
}

#[test]
fn broker_credential_peer_helper() {
    if std::env::var("D2B_BROKER_CREDENTIAL_HELPER").as_deref() != Ok("1") {
        return;
    }
    let path = PathBuf::from(std::env::var_os("D2B_BROKER_CREDENTIAL_SOCKET").unwrap());
    let socket = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::SOCK_CLOEXEC,
        None,
    )
    .unwrap();
    connect(socket.as_raw_fd(), &UnixAddr::new(path.as_path()).unwrap()).unwrap();
    let control_fd = std::env::var("D2B_BROKER_CREDENTIAL_CONTROL_FD")
        .unwrap()
        .parse::<i32>()
        .unwrap();
    d2b_priv_broker::fd_passing::send_fds(control_fd, b"child-endpoint", &[socket.as_raw_fd()])
        .unwrap();
    let mut go = [0_u8; 2];
    recv(socket.as_raw_fd(), &mut go, MsgFlags::empty()).unwrap();
    send(socket.as_raw_fd(), b"peer-ready", MsgFlags::MSG_NOSIGNAL).unwrap();
    std::thread::sleep(std::time::Duration::from_secs(5));
}

fn bootstrap_endpoint_process() -> (OwnedFd, OwnedFd, Child, tempfile::TempDir) {
    let ordinal = CREDENTIAL_SOCKET_COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = socket_tempdir();
    let path = root.path().join(format!("credential-{ordinal}.sock"));
    let listener = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::SOCK_CLOEXEC,
        None,
    )
    .unwrap();
    bind(
        listener.as_raw_fd(),
        &UnixAddr::new(path.as_path()).unwrap(),
    )
    .unwrap();
    listen(&listener, Backlog::new(1).unwrap()).unwrap();
    let (control_parent, control_child) = socketpair(
        AddressFamily::Unix,
        SockType::SeqPacket,
        None,
        SockFlag::SOCK_CLOEXEC,
    )
    .unwrap();
    nix::fcntl::fcntl(
        control_child.as_raw_fd(),
        nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::empty()),
    )
    .unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "broker_credential_peer_helper", "--nocapture"])
        .env("D2B_BROKER_CREDENTIAL_HELPER", "1")
        .env("D2B_BROKER_CREDENTIAL_SOCKET", &path)
        .env(
            "D2B_BROKER_CREDENTIAL_CONTROL_FD",
            control_child.as_raw_fd().to_string(),
        )
        .spawn()
        .unwrap();
    drop(control_child);
    let accepted = accept4(
        listener.as_raw_fd(),
        SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
    )
    .map(d2b_priv_broker::sys::owned_fd_from_raw)
    .unwrap();
    let (_, mut fds) = d2b_priv_broker::fd_passing::recv_fds(control_parent.as_raw_fd()).unwrap();
    assert_eq!(fds.len(), 1);
    let child_endpoint = d2b_priv_broker::sys::owned_fd_from_raw(fds.remove(0));
    setsockopt(&accepted, PassCred, &true).unwrap();
    send(accepted.as_raw_fd(), b"go", MsgFlags::MSG_NOSIGNAL).unwrap();
    (accepted, child_endpoint, child, root)
}

fn cleanup_credential_helpers(mut helpers: Vec<Child>, roots: Vec<tempfile::TempDir>) {
    for helper in &mut helpers {
        let _ = helper.kill();
        let _ = helper.wait();
    }
    for root in roots {
        root.close().expect("remove broker service socket tempdir");
    }
}

fn spawn_binding(
    role: d2b_contracts::v2_services::broker::RealmChildRole,
    kind: d2b_contracts::v2_services::broker::RealmChildFdKind,
    index: u32,
) -> d2b_contracts::v2_services::broker::RealmChildFd {
    let mut binding = d2b_contracts::v2_services::broker::RealmChildFd::new();
    binding.role = role.into();
    binding.kind = kind.into();
    binding.attachment_index = index;
    binding
}

fn controller_static_identity() -> OwnedFd {
    let fd = rustix::fs::memfd_create(
        "controller-static-identity",
        rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
    )
    .unwrap();
    let mut writer = File::from(fd);
    writer.write_all(&[7; 32]).unwrap();
    writer.seek(SeekFrom::Start(0)).unwrap();
    nix::fcntl::fcntl(
        writer.as_raw_fd(),
        nix::fcntl::FcntlArg::F_ADD_SEALS(
            nix::fcntl::SealFlag::F_SEAL_WRITE
                | nix::fcntl::SealFlag::F_SEAL_GROW
                | nix::fcntl::SealFlag::F_SEAL_SHRINK
                | nix::fcntl::SealFlag::F_SEAL_SEAL,
        ),
    )
    .unwrap();
    let readonly = rustix::fs::open(
        format!("/proc/self/fd/{}", writer.as_raw_fd()),
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .unwrap();
    drop(writer);
    readonly
}

fn spawn_descriptor(
    context: &BrokerCallContext,
    index: u16,
    object_type: KernelObjectType,
    access: AttachmentAccess,
) -> AttachmentDescriptor {
    let method = SERVICE_INVENTORY
        .iter()
        .find(|service| service.package == "d2b.broker.v2")
        .and_then(|service| service.methods.iter().find(|method| method.name == "Spawn"))
        .unwrap();
    AttachmentDescriptor {
        index,
        kind: AttachmentKind::FileDescriptor,
        object_type,
        access,
        purpose: AttachmentPurpose::RequestInput,
        service: ServicePackage::BrokerV2,
        method_id: method.method_id("d2b.broker.v2", "BrokerService"),
        request_id: context.request_id.clone(),
        operation_id: None,
        packet_sequence: 1,
        reconnect_generation: context.session_generation,
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

fn transport_credits(limit: usize) -> CreditScopeSet {
    CreditScopeSet::new(
        CreditPool::new(limit).unwrap(),
        CreditPool::new(limit).unwrap(),
        CreditPool::new(limit).unwrap(),
        CreditPool::new(limit).unwrap(),
        CreditPool::new(limit).unwrap(),
        CreditPool::new(limit).unwrap(),
    )
}

#[tokio::test]
async fn production_spawn_enforces_authorization_and_correlates_response_fds() {
    use d2b_contracts::v2_services::broker::{RealmChildFdKind as K, RealmChildRole as R};

    let registry = Arc::new(AuthorizedFdRegistry::new());
    let (controller_parent, controller_session, controller_helper, controller_root) =
        bootstrap_endpoint_process();
    let (broker_parent, broker_session, broker_helper, broker_root) = bootstrap_endpoint_process();
    let helper_pids = (controller_helper.id(), broker_helper.id());
    let sources = vec![
        (
            R::REALM_CHILD_ROLE_CONTROLLER,
            K::REALM_CHILD_FD_KIND_PUBLIC_LISTENER,
            listener(b"broker-service-controller"),
            None,
            KernelObjectType::UnixSeqpacketSocket,
            AttachmentAccess::ReadWrite,
        ),
        (
            R::REALM_CHILD_ROLE_BROKER,
            K::REALM_CHILD_FD_KIND_BROKER_LISTENER,
            listener(b"broker-service-broker"),
            None,
            KernelObjectType::UnixSeqpacketSocket,
            AttachmentAccess::ReadWrite,
        ),
        (
            R::REALM_CHILD_ROLE_CONTROLLER,
            K::REALM_CHILD_FD_KIND_BOOTSTRAP_SESSION,
            controller_session,
            Some(controller_parent),
            KernelObjectType::UnixSeqpacketSocket,
            AttachmentAccess::ReadWrite,
        ),
        (
            R::REALM_CHILD_ROLE_BROKER,
            K::REALM_CHILD_FD_KIND_BOOTSTRAP_SESSION,
            broker_session,
            Some(broker_parent),
            KernelObjectType::UnixSeqpacketSocket,
            AttachmentAccess::ReadWrite,
        ),
        (
            R::REALM_CHILD_ROLE_CONTROLLER,
            K::REALM_CHILD_FD_KIND_CGROUP_LEAF,
            File::open("/sys/fs/cgroup").unwrap().into(),
            None,
            KernelObjectType::Directory,
            AttachmentAccess::ReadOnly,
        ),
        (
            R::REALM_CHILD_ROLE_BROKER,
            K::REALM_CHILD_FD_KIND_CGROUP_LEAF,
            File::open("/sys/fs/cgroup").unwrap().into(),
            None,
            KernelObjectType::Directory,
            AttachmentAccess::ReadOnly,
        ),
        (
            R::REALM_CHILD_ROLE_CONTROLLER,
            K::REALM_CHILD_FD_KIND_RESOURCE,
            controller_static_identity(),
            None,
            KernelObjectType::Memfd,
            AttachmentAccess::ReadOnly,
        ),
    ];
    let context = call_context(0x41);
    let mut attachments = Vec::new();
    let mut request = SpawnRealmChildrenRequest::new();
    request.metadata = Some(allocator_metadata(0x41)).into();
    request.scope = Some(common::IdentityScope {
        realm_id: "aaaaaaaaaaaaaaaaaaaa".into(),
        ..Default::default()
    })
    .into();
    request.operation_id = "spawn-1".into();
    request.realm_id = "aaaaaaaaaaaaaaaaaaaa".into();
    request.controller_generation_id = "generation-1".into();
    request.controller_process_id = "controller-1".into();
    request.broker_process_id = "broker-1".into();
    request.launch_record_digest = vec![7; 32];
    for (index, (role, kind, fd, parent_fd, object_type, access)) in sources.into_iter().enumerate()
    {
        let outbound = fd.try_clone().unwrap();
        if let Some(parent_fd) = parent_fd {
            registry
                .register_spawn_bootstrap_pair(role.value(), fd, parent_fd)
                .unwrap();
        } else if kind == K::REALM_CHILD_FD_KIND_RESOURCE {
            registry
                .register_resource(
                    d2b_host::guest_runtime::CONTROLLER_STATIC_IDENTITY_RESOURCE_ID,
                    fd,
                    object_type,
                    access,
                    AttachmentPurpose::RequestInput,
                )
                .unwrap();
        } else {
            registry
                .register_spawn_binding(
                    role.value(),
                    kind.value(),
                    fd,
                    object_type,
                    access,
                    AttachmentPurpose::RequestInput,
                )
                .unwrap();
        }
        let mut binding = spawn_binding(role, kind, index as u32);
        if kind == K::REALM_CHILD_FD_KIND_RESOURCE {
            binding.resource_id =
                Some(d2b_host::guest_runtime::CONTROLLER_STATIC_IDENTITY_RESOURCE_ID.to_owned());
        }
        request.fds.push(binding);
        let descriptor = spawn_descriptor(&context, index as u16, object_type, access);
        let policy = DescriptorPolicy::File(
            ObjectIdentity::from_trusted(&outbound, object_type, access).unwrap(),
        );
        attachments.push(OwnedUnixAttachment::file(descriptor, outbound, policy).unwrap());
    }
    let handler = production_handler(Arc::clone(&registry), helper_pids);
    handler
        .prepare_spawn_attachments(&request, &context)
        .unwrap();
    let reply = handler
        .spawn(request.clone(), attachments, &context)
        .await
        .unwrap();
    handler.finish_spawn_attachments(&request, &context);
    assert_eq!(reply.message.operation_id, "spawn-1");
    assert_eq!(reply.message.children.len(), 2);
    assert_eq!(reply.attachments.len(), 2);
    assert_eq!(
        reply.attachments[0].descriptor().unwrap().request_id,
        context.request_id
    );
    assert_eq!(reply.attachments[1].descriptor().unwrap().index, 1);
    for attachment in &reply.attachments {
        assert_eq!(
            attachment.descriptor().unwrap().object_type,
            KernelObjectType::Pidfd
        );
        assert_ne!(
            nix::fcntl::fcntl(
                attachment
                    .payload()
                    .and_then(|payload| payload.downcast_ref::<UnixAttachmentPayload>())
                    .and_then(UnixAttachmentPayload::file)
                    .unwrap()
                    .as_raw_fd(),
                nix::fcntl::FcntlArg::F_GETFD,
            )
            .unwrap()
                & libc::FD_CLOEXEC,
            0
        );
    }

    let (sender_fd, receiver_fd) = socketpair(
        AddressFamily::Unix,
        SockType::SeqPacket,
        None,
        SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
    )
    .unwrap();
    let sender_socket = SeqpacketSocket::from_owned(sender_fd).unwrap();
    let sender_peer = sender_socket.acceptor_peer_credentials().unwrap();
    let endpoint = d2b_priv_broker::service_v2::broker_endpoint_policy(
        BrokerPeerRole::LocalRootController,
        d2b_contracts::v2_component_session::EndpointRole::LocalRootBroker,
        GENERATION,
        [1; 32],
    )
    .unwrap();
    let mut sender = UnixSeqpacketTransport::new(
        sender_socket,
        d2b_contracts::v2_component_session::Locality::HostLocal,
        endpoint.limits,
        endpoint.attachment_policy,
        transport_credits(16),
        Arc::new(|_| Err(d2b_session_unix::UnixSessionError::DescriptorMismatch)),
        PeerIdentityPolicy::accepted(sender_peer),
    )
    .unwrap();
    sender
        .send(TransportPacket::with_attachments(
            b"spawn-response".to_vec(),
            reply.attachments,
        ))
        .await
        .unwrap();
    let (_, received_fds) = d2b_priv_broker::fd_passing::recv_fds(receiver_fd.as_raw_fd()).unwrap();
    assert_eq!(received_fds.len(), 2);
    for raw_fd in received_fds {
        let fd = d2b_priv_broker::sys::owned_fd_from_raw(raw_fd);
        assert_ne!(
            nix::fcntl::fcntl(fd.as_raw_fd(), nix::fcntl::FcntlArg::F_GETFD).unwrap()
                & libc::FD_CLOEXEC,
            0
        );
    }

    let denied_context = call_context(0x42);
    let mut denied = request;
    denied.metadata = Some(allocator_metadata(0x42)).into();
    denied.fds[0].kind = K::REALM_CHILD_FD_KIND_RESOURCE.into();
    assert_eq!(
        handler.prepare_spawn_attachments(&denied, &denied_context),
        Err(BrokerServiceFailure::PermissionDenied)
    );
    cleanup_credential_helpers(
        vec![controller_helper, broker_helper],
        vec![controller_root, broker_root],
    );
}

#[tokio::test]
async fn production_allocator_observes_pre_cancelled_context() {
    let registry = Arc::new(AuthorizedFdRegistry::new());
    let handler = production_handler(registry, (1, 2));
    let context = call_context(0x51);
    assert!(context.cancellation.cancel());
    assert_eq!(
        handler
            .allocate(AllocateRequest::new(), &context)
            .await
            .unwrap_err(),
        BrokerServiceFailure::Cancelled
    );
}
