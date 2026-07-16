use std::{
    collections::VecDeque,
    future::pending,
    io::{Read, Write},
    os::fd::{AsRawFd, OwnedFd},
    os::unix::net::{UnixDatagram, UnixStream},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{EndpointRole, Locality, ServicePackage, TransportClass},
    v2_identity::{ProviderId, ProviderType, RealmId, WorkloadId},
    v2_provider::{
        AdoptionState, AuthorizedProviderScope, CorrelationId, Fingerprint, Generation, HandleId,
        IdempotencyKey, ImplementationId, MutationState, ObservationReason, ObservedLifecycleState,
        OperationId, PROVIDER_SCHEMA_VERSION, PrincipalRef, ProviderApiVersion, ProviderAuthority,
        ProviderCallContext, ProviderCapability, ProviderCapabilitySet, ProviderDescriptor,
        ProviderFailure, ProviderFailureKind, ProviderHandle, ProviderHandleKind, ProviderMethod,
        ProviderObservation, ProviderOperationContext, ProviderOperationInput,
        ProviderOperationRequest, ProviderPlacement, ProviderTarget, TransportBindingId,
        TransportProvider,
    },
};
use d2b_provider::{FactoryError, ProviderFactory, ProviderInstance, ProviderRegistryBuilder};
use tokio::sync::Notify;

use crate::{
    AttachmentCapability, AuthenticationOwner, BundleEndpointId,
    CLOUD_HYPERVISOR_VSOCK_FACTORY_KEY, CLOUD_HYPERVISOR_VSOCK_IMPLEMENTATION_ID,
    CloudHypervisorVsockPort, EndpointCapabilityId, EndpointCloseRequest, EndpointCloseResult,
    EndpointCloseState, EndpointConnectRequest, EndpointConnection, EndpointConnectionMetadata,
    EndpointInspectRequest, EndpointLeaseId, EndpointObservation, EndpointObservationState,
    EndpointPortError, EndpointProvenance, EndpointResolveRequest, EndpointSource,
    LocalEndpointPort, LocalEndpointResolver, LocalTransportClock,
    LocalTransportConfigurationError, LocalTransportFactory, LocalTransportFactoryError,
    LocalTransportKind, LocalTransportLimits, LocalTransportProvider, NATIVE_VSOCK_FACTORY_KEY,
    NATIVE_VSOCK_IMPLEMENTATION_ID, OwnedEndpointConnection, OwnedEndpointDescriptor,
    OwnedLocalTransport, ReachabilityEvidence, TokioLocalEndpointPort, TransportBinding,
    TransportCapabilityProfile, TransportHandoffError, UNIX_SEQPACKET_FACTORY_KEY,
    UNIX_SEQPACKET_IMPLEMENTATION_ID, UNIX_STREAM_FACTORY_KEY, UNIX_STREAM_IMPLEMENTATION_ID,
    local_transport_capabilities,
};

const NOW_UNIX_MS: u64 = 1_700_000_000_000;
const KINDS: [LocalTransportKind; 4] = [
    LocalTransportKind::UnixStream,
    LocalTransportKind::UnixSeqpacket,
    LocalTransportKind::NativeVsock,
    LocalTransportKind::CloudHypervisorVsock,
];

#[derive(Clone)]
struct FakeBehavior {
    pending_connect: bool,
    connect_error: Option<EndpointPortError>,
    connect_operation_id: Option<OperationId>,
    connect_identity: Option<Fingerprint>,
    connect_generation: Option<Generation>,
    connect_preclaimed: bool,
    inspect_error: Option<EndpointPortError>,
    inspect_identity: Option<Fingerprint>,
    inspect_generation: Option<Generation>,
    inspect_state: EndpointObservationState,
    pending_close: bool,
    close_error: Option<EndpointPortError>,
    close_operation_id: Option<OperationId>,
    close_identity: Option<Fingerprint>,
    close_generation: Option<Generation>,
    close_state: EndpointCloseState,
}

impl Default for FakeBehavior {
    fn default() -> Self {
        Self {
            pending_connect: false,
            connect_error: None,
            connect_operation_id: None,
            connect_identity: None,
            connect_generation: None,
            connect_preclaimed: false,
            inspect_error: None,
            inspect_identity: None,
            inspect_generation: None,
            inspect_state: EndpointObservationState::Connected,
            pending_close: false,
            close_error: None,
            close_operation_id: None,
            close_identity: None,
            close_generation: None,
            close_state: EndpointCloseState::Closed,
        }
    }
}

#[derive(Default)]
struct FakeEndpointPort {
    behavior: Mutex<FakeBehavior>,
    connect_calls: Mutex<Vec<EndpointConnectRequest>>,
    inspect_calls: Mutex<Vec<EndpointInspectRequest>>,
    close_calls: Mutex<Vec<EndpointCloseRequest>>,
    connection_peers: Mutex<Vec<UnixStream>>,
    connect_started: Notify,
    pending_connect_dropped: Arc<AtomicBool>,
    pending_close_dropped: Arc<AtomicBool>,
}

impl FakeEndpointPort {
    fn update(&self, update: impl FnOnce(&mut FakeBehavior)) {
        update(&mut lock(&self.behavior));
    }

    fn connect_calls(&self) -> Vec<EndpointConnectRequest> {
        lock(&self.connect_calls).clone()
    }

    fn inspect_calls(&self) -> Vec<EndpointInspectRequest> {
        lock(&self.inspect_calls).clone()
    }

    fn close_calls(&self) -> Vec<EndpointCloseRequest> {
        lock(&self.close_calls).clone()
    }

    async fn wait_for_connect(&self) {
        self.connect_started.notified().await;
    }

    fn closed_peer_count(&self) -> usize {
        lock(&self.connection_peers)
            .iter_mut()
            .map(|peer| {
                let mut byte = [0_u8; 1];
                matches!(peer.read(&mut byte), Ok(0))
            })
            .filter(|closed| *closed)
            .count()
    }
}

#[derive(Default)]
struct RejectingResolver {
    calls: AtomicUsize,
}

#[async_trait]
impl LocalEndpointResolver for RejectingResolver {
    async fn resolve(
        &self,
        _request: EndpointResolveRequest,
    ) -> Result<OwnedEndpointDescriptor, EndpointPortError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        Err(EndpointPortError::InvariantViolation)
    }
}

struct StaticResolver {
    descriptors: Mutex<VecDeque<OwnedEndpointDescriptor>>,
    calls: Mutex<Vec<EndpointResolveRequest>>,
}

impl StaticResolver {
    fn new(descriptor: OwnedEndpointDescriptor) -> Self {
        Self {
            descriptors: Mutex::new(VecDeque::from([descriptor])),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn from_descriptors(descriptors: impl IntoIterator<Item = OwnedEndpointDescriptor>) -> Self {
        Self {
            descriptors: Mutex::new(descriptors.into_iter().collect()),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<EndpointResolveRequest> {
        lock(&self.calls).clone()
    }
}

#[async_trait]
impl LocalEndpointResolver for StaticResolver {
    async fn resolve(
        &self,
        request: EndpointResolveRequest,
    ) -> Result<OwnedEndpointDescriptor, EndpointPortError> {
        lock(&self.calls).push(request);
        lock(&self.descriptors)
            .pop_front()
            .ok_or(EndpointPortError::Unavailable)
    }
}

struct PendingConnectGuard(Arc<AtomicBool>);

impl Drop for PendingConnectGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[async_trait]
impl LocalEndpointPort for FakeEndpointPort {
    async fn connect(
        &self,
        request: EndpointConnectRequest,
    ) -> Result<EndpointConnection, EndpointPortError> {
        lock(&self.connect_calls).push(request.clone());
        let behavior = lock(&self.behavior).clone();
        self.connect_started.notify_one();
        if behavior.pending_connect {
            let _guard = PendingConnectGuard(self.pending_connect_dropped.clone());
            pending::<()>().await;
        }
        if let Some(error) = behavior.connect_error {
            return Err(error);
        }
        let kind = request.kind;
        let (socket, peer) = UnixStream::pair().expect("fake connection pair");
        socket
            .set_nonblocking(true)
            .expect("nonblocking fake connection");
        peer.set_nonblocking(true)
            .expect("nonblocking fake connection peer");
        lock(&self.connection_peers).push(peer);
        let connection = EndpointConnection::new(
            EndpointConnectionMetadata {
                operation_id: behavior
                    .connect_operation_id
                    .unwrap_or(request.operation_id),
                handle_id: request.handle_id,
                binding_id: request.binding_id,
                identity: behavior
                    .connect_identity
                    .unwrap_or(request.expected_identity),
                generation: behavior
                    .connect_generation
                    .unwrap_or(request.expected_generation),
                kind,
                capabilities: request.capabilities,
                reachability: ReachabilityEvidence::ReachableOnly,
            },
            OwnedLocalTransport::from_connected(kind, OwnedFd::from(socket)),
        )?;
        if behavior.connect_preclaimed {
            drop(
                connection
                    .owned()
                    .take_transport()
                    .map_err(|_| EndpointPortError::InvariantViolation)?,
            );
        }
        Ok(connection)
    }

    async fn inspect(
        &self,
        request: EndpointInspectRequest,
        _connection: &OwnedEndpointConnection,
    ) -> Result<EndpointObservation, EndpointPortError> {
        lock(&self.inspect_calls).push(request.clone());
        let behavior = lock(&self.behavior).clone();
        if let Some(error) = behavior.inspect_error {
            return Err(error);
        }
        Ok(EndpointObservation {
            operation_id: request.operation_id,
            handle_id: request.handle_id,
            binding_id: request.binding_id,
            identity: behavior
                .inspect_identity
                .unwrap_or(request.expected_identity),
            generation: behavior
                .inspect_generation
                .unwrap_or(request.expected_generation),
            kind: request.kind,
            capabilities: request.capabilities,
            state: behavior.inspect_state,
        })
    }

    async fn close(
        &self,
        request: EndpointCloseRequest,
        _connection: &OwnedEndpointConnection,
    ) -> Result<EndpointCloseResult, EndpointPortError> {
        lock(&self.close_calls).push(request.clone());
        let behavior = lock(&self.behavior).clone();
        if behavior.pending_close {
            let _guard = PendingConnectGuard(self.pending_close_dropped.clone());
            pending::<()>().await;
        }
        if let Some(error) = behavior.close_error {
            return Err(error);
        }
        Ok(EndpointCloseResult {
            operation_id: behavior.close_operation_id.unwrap_or(request.operation_id),
            handle_id: request.handle_id,
            binding_id: request.binding_id,
            identity: behavior.close_identity.unwrap_or(request.expected_identity),
            generation: behavior
                .close_generation
                .unwrap_or(request.expected_generation),
            state: behavior.close_state,
        })
    }
}

#[derive(Debug)]
struct FixedClock;

impl LocalTransportClock for FixedClock {
    fn now_unix_ms(&self) -> u64 {
        NOW_UNIX_MS
    }
}

struct Harness {
    descriptor: ProviderDescriptor,
    realm_id: RealmId,
    scope: AuthorizedProviderScope,
    binding_id: TransportBindingId,
    endpoint_generation: Generation,
    provider: Arc<LocalTransportProvider>,
    port: Arc<FakeEndpointPort>,
}

impl Harness {
    fn new(kind: LocalTransportKind) -> Self {
        Self::with_limits(kind, LocalTransportLimits::default())
    }

    fn with_limits(kind: LocalTransportKind, limits: LocalTransportLimits) -> Self {
        let descriptor = descriptor(kind);
        let realm_id = descriptor.placement.realm_id().clone();
        let scope = AuthorizedProviderScope::Realm {
            realm_id: realm_id.clone(),
        };
        let binding_id = transport_binding_id("binding-primary");
        let endpoint_identity = fingerprint(0x41);
        let endpoint_generation = generation(9);
        let binding = binding(
            &descriptor,
            kind,
            binding_id.clone(),
            scope.clone(),
            endpoint_identity.clone(),
            endpoint_generation,
        );
        let port = Arc::new(FakeEndpointPort::default());
        let provider = Arc::new(
            LocalTransportProvider::with_clock_and_limits(
                descriptor.clone(),
                kind,
                vec![binding],
                port.clone(),
                Arc::new(FixedClock),
                limits,
            )
            .expect("valid local transport provider"),
        );
        Self {
            descriptor,
            realm_id,
            scope,
            binding_id,
            endpoint_generation,
            provider,
            port,
        }
    }

    fn operation(&self, method: ProviderMethod, operation_id: &str) -> ProviderOperationContext {
        ProviderOperationContext {
            schema_version: PROVIDER_SCHEMA_VERSION,
            operation_id: OperationId::parse(operation_id).expect("valid operation id"),
            idempotency_key: IdempotencyKey::parse(format!("idem-{operation_id}"))
                .expect("valid idempotency key"),
            request_digest: fingerprint(0x51),
            scope: self.scope.clone(),
            principal: PrincipalRef::parse("principal-local-controller").expect("valid principal"),
            provider_id: self.descriptor.provider_id.clone(),
            provider_type: ProviderType::Transport,
            provider_generation: self.descriptor.registry_generation,
            capability: ProviderCapability(method),
            method,
            policy_epoch: generation(3),
            authorization_decision_digest: fingerprint(0x52),
            issued_at_unix_ms: NOW_UNIX_MS - 1_000,
            expires_at_unix_ms: NOW_UNIX_MS + 60_000,
            correlation_id: CorrelationId::parse("correlation-local-transport")
                .expect("valid correlation id"),
            trace_id: fingerprint(0x53),
        }
    }

    fn connect_request(&self, operation_id: &str) -> ProviderOperationRequest {
        ProviderOperationRequest {
            context: self.operation(ProviderMethod::TransportConnect, operation_id),
            target: ProviderTarget::Realm {
                realm_id: self.realm_id.clone(),
            },
            expected_configuration_fingerprint: self
                .descriptor
                .configuration_schema_fingerprint
                .clone(),
            input: ProviderOperationInput::TransportBinding {
                transport_binding_id: self.binding_id.clone(),
            },
        }
    }

    fn handle_request(
        &self,
        method: ProviderMethod,
        operation_id: &str,
        handle: &ProviderHandle,
    ) -> ProviderOperationRequest {
        ProviderOperationRequest {
            context: self.operation(method, operation_id),
            target: ProviderTarget::Handle {
                realm_id: self.realm_id.clone(),
                workload_id: None,
                handle_id: handle.handle_id.clone(),
                handle_generation: handle.resource_generation,
            },
            expected_configuration_fingerprint: self
                .descriptor
                .configuration_schema_fingerprint
                .clone(),
            input: if method == ProviderMethod::TransportRevokeBinding {
                ProviderOperationInput::TransportBinding {
                    transport_binding_id: self.binding_id.clone(),
                }
            } else {
                ProviderOperationInput::NoInput
            },
        }
    }

    fn call_context<'a>(&self, operation: &'a ProviderOperationContext) -> ProviderCallContext<'a> {
        ProviderCallContext {
            operation,
            peer_role: EndpointRole::RealmController,
            service: ServicePackage::ProviderV2,
            monotonic_deadline_remaining_ms: 30_000,
            cancelled: false,
        }
    }
}

fn descriptor(kind: LocalTransportKind) -> ProviderDescriptor {
    let provider_id = match kind {
        LocalTransportKind::UnixStream => "bbbbbbbbbbbbbbbbbbba",
        LocalTransportKind::UnixSeqpacket => "ccccccccccccccccccca",
        LocalTransportKind::NativeVsock => "ddddddddddddddddddda",
        LocalTransportKind::CloudHypervisorVsock => "eeeeeeeeeeeeeeeeeeea",
    };
    ProviderDescriptor {
        schema_version: PROVIDER_SCHEMA_VERSION,
        provider_id: ProviderId::parse(provider_id).expect("valid provider id"),
        authority: ProviderAuthority::Transport,
        implementation_id: kind.implementation_id().clone(),
        api_version: ProviderApiVersion::V2,
        capabilities: local_transport_capabilities(),
        configuration_schema_fingerprint: fingerprint(0x11),
        configured_scope_digest: fingerprint(0x12),
        registry_generation: generation(4),
        placement: ProviderPlacement::TrustedFirstPartyInProcess {
            realm_id: RealmId::parse("aaaaaaaaaaaaaaaaaaaa").expect("valid realm id"),
            controller_role: EndpointRole::RealmController,
        },
    }
}

fn binding(
    descriptor: &ProviderDescriptor,
    kind: LocalTransportKind,
    binding_id: TransportBindingId,
    scope: AuthorizedProviderScope,
    endpoint_identity: Fingerprint,
    endpoint_generation: Generation,
) -> TransportBinding {
    TransportBinding::new(
        binding_id.clone(),
        descriptor.provider_id.clone(),
        descriptor.registry_generation,
        descriptor.configuration_schema_fingerprint.clone(),
        descriptor.configured_scope_digest.clone(),
        scope,
        endpoint_identity,
        endpoint_generation,
        EndpointSource::bundle(kind, BundleEndpointId::new(binding_id)),
    )
}

fn factory(
    kind: LocalTransportKind,
    endpoint_port: Arc<dyn LocalEndpointPort>,
    bindings: Vec<TransportBinding>,
) -> LocalTransportFactory {
    let result = match kind {
        LocalTransportKind::UnixStream => {
            LocalTransportFactory::unix_stream(endpoint_port, bindings)
        }
        LocalTransportKind::UnixSeqpacket => {
            LocalTransportFactory::unix_seqpacket(endpoint_port, bindings)
        }
        LocalTransportKind::NativeVsock => {
            LocalTransportFactory::native_vsock(endpoint_port, bindings)
        }
        LocalTransportKind::CloudHypervisorVsock => {
            LocalTransportFactory::cloud_hypervisor_vsock(endpoint_port, bindings)
        }
    };
    result.expect("valid local transport factory")
}

fn fingerprint(value: u8) -> Fingerprint {
    Fingerprint::parse(format!("{value:02x}").repeat(32)).expect("valid fingerprint")
}

fn generation(value: u64) -> Generation {
    Generation::new(value).expect("valid generation")
}

fn transport_binding_id(value: &str) -> TransportBindingId {
    TransportBindingId::parse(value).expect("valid transport binding id")
}

fn endpoint_connect_request(
    kind: LocalTransportKind,
    operation_id: &str,
    endpoint: EndpointSource,
    identity: Fingerprint,
    generation: Generation,
    deadline: Duration,
) -> EndpointConnectRequest {
    EndpointConnectRequest {
        operation_id: OperationId::parse(operation_id).expect("valid operation id"),
        handle_id: HandleId::parse(operation_id).expect("valid handle id"),
        binding_id: transport_binding_id("production-binding"),
        endpoint,
        expected_identity: identity,
        expected_generation: generation,
        kind,
        capabilities: kind.capability_profile(),
        deadline,
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

#[test]
fn every_live_implementation_exposes_an_exact_registry_factory() {
    for kind in KINDS {
        let descriptor = descriptor(kind);
        let scope = AuthorizedProviderScope::Realm {
            realm_id: descriptor.placement.realm_id().clone(),
        };
        let binding = binding(
            &descriptor,
            kind,
            transport_binding_id("factory-binding"),
            scope,
            fingerprint(0x31),
            generation(5),
        );
        let factory = factory(kind, Arc::new(FakeEndpointPort::default()), vec![binding]);
        let (exported_id, exported_key) = match kind {
            LocalTransportKind::UnixStream => {
                (&*UNIX_STREAM_IMPLEMENTATION_ID, &*UNIX_STREAM_FACTORY_KEY)
            }
            LocalTransportKind::UnixSeqpacket => (
                &*UNIX_SEQPACKET_IMPLEMENTATION_ID,
                &*UNIX_SEQPACKET_FACTORY_KEY,
            ),
            LocalTransportKind::NativeVsock => {
                (&*NATIVE_VSOCK_IMPLEMENTATION_ID, &*NATIVE_VSOCK_FACTORY_KEY)
            }
            LocalTransportKind::CloudHypervisorVsock => (
                &*CLOUD_HYPERVISOR_VSOCK_IMPLEMENTATION_ID,
                &*CLOUD_HYPERVISOR_VSOCK_FACTORY_KEY,
            ),
        };
        assert_eq!(factory.kind(), kind);
        assert_eq!(factory.implementation_id(), exported_id);
        assert_eq!(factory.key(), (*exported_key).clone());
        assert_eq!(factory.registered_provider_count(), 1);
        let instance = factory
            .construct(&descriptor)
            .expect("matching descriptor constructs");
        assert!(matches!(&instance, ProviderInstance::Transport(_)));
        assert_eq!(instance.descriptor(), descriptor);
    }
}

#[test]
fn factory_rejects_wrong_descriptor_type_and_implementation() {
    let kind = LocalTransportKind::UnixStream;
    let descriptor = descriptor(kind);
    let binding = binding(
        &descriptor,
        kind,
        transport_binding_id("factory-rejection-binding"),
        AuthorizedProviderScope::Realm {
            realm_id: descriptor.placement.realm_id().clone(),
        },
        fingerprint(0x32),
        generation(6),
    );
    let port = Arc::new(FakeEndpointPort::default());
    let factory = LocalTransportFactory::unix_stream(port.clone(), vec![binding])
        .expect("valid unix-stream factory");

    let mut wrong_type = descriptor.clone();
    wrong_type.authority = ProviderAuthority::Storage;
    wrong_type.capabilities = ProviderCapabilitySet::new(vec![
        ProviderCapability(ProviderMethod::StoragePlan),
        ProviderCapability(ProviderMethod::StorageEnsure),
        ProviderCapability(ProviderMethod::StorageInspect),
        ProviderCapability(ProviderMethod::StorageAdopt),
        ProviderCapability(ProviderMethod::StorageDestroy),
    ])
    .expect("valid storage capabilities");
    wrong_type.validate().expect("valid wrong-type descriptor");
    assert!(matches!(
        factory.construct(&wrong_type),
        Err(FactoryError::Rejected)
    ));

    let mut wrong_implementation = descriptor;
    wrong_implementation.implementation_id =
        ImplementationId::parse("other-local-transport").expect("valid implementation id");
    wrong_implementation
        .validate()
        .expect("valid wrong-implementation descriptor");
    assert!(matches!(
        factory.construct(&wrong_implementation),
        Err(FactoryError::Rejected)
    ));
    assert!(port.connect_calls().is_empty());
}

#[test]
fn factory_binds_exact_provider_configuration_and_scope_digest() {
    let kind = LocalTransportKind::UnixStream;
    let descriptor = descriptor(kind);
    let configured_binding = binding(
        &descriptor,
        kind,
        transport_binding_id("factory-exact-entry"),
        AuthorizedProviderScope::Realm {
            realm_id: descriptor.placement.realm_id().clone(),
        },
        fingerprint(0x35),
        generation(8),
    );
    let port = Arc::new(FakeEndpointPort::default());
    let factory =
        LocalTransportFactory::unix_stream(port.clone(), vec![configured_binding.clone()])
            .expect("valid exact factory");

    let mut wrong_scope_digest = descriptor.clone();
    wrong_scope_digest.configured_scope_digest = fingerprint(0x36);
    assert!(matches!(
        factory.construct(&wrong_scope_digest),
        Err(FactoryError::Rejected)
    ));
    let mixed_scope_binding = binding(
        &wrong_scope_digest,
        kind,
        transport_binding_id("factory-mixed-scope"),
        AuthorizedProviderScope::Realm {
            realm_id: wrong_scope_digest.placement.realm_id().clone(),
        },
        fingerprint(0x38),
        generation(9),
    );
    assert!(matches!(
        LocalTransportFactory::unix_stream(
            Arc::new(FakeEndpointPort::default()),
            vec![configured_binding.clone(), mixed_scope_binding],
        ),
        Err(LocalTransportFactoryError::BindingDescriptorMismatch)
    ));

    let mut wrong_configuration = descriptor.clone();
    wrong_configuration.configuration_schema_fingerprint = fingerprint(0x37);
    assert!(matches!(
        factory.construct(&wrong_configuration),
        Err(FactoryError::Rejected)
    ));

    let mut wrong_generation = descriptor.clone();
    wrong_generation.registry_generation = generation(81);
    assert!(matches!(
        factory.construct(&wrong_generation),
        Err(FactoryError::Rejected)
    ));

    let mut wrong_provider = descriptor;
    wrong_provider.provider_id =
        ProviderId::parse("fffffffffffffffffffa").expect("valid other provider");
    assert!(matches!(
        factory.construct(&wrong_provider),
        Err(FactoryError::Rejected)
    ));
    assert!(port.connect_calls().is_empty());

    assert!(matches!(
        LocalTransportProvider::new(
            wrong_scope_digest,
            kind,
            vec![configured_binding],
            Arc::new(FakeEndpointPort::default()),
        ),
        Err(LocalTransportConfigurationError::BindingMismatch)
    ));
}

#[test]
fn registry_builder_registers_explicit_construction_and_exact_handoff() {
    let kind = LocalTransportKind::UnixSeqpacket;
    let descriptor = descriptor(kind);
    let binding = binding(
        &descriptor,
        kind,
        transport_binding_id("registry-factory-binding"),
        AuthorizedProviderScope::Realm {
            realm_id: descriptor.placement.realm_id().clone(),
        },
        fingerprint(0x33),
        generation(7),
    );
    let factory = Arc::new(
        LocalTransportFactory::unix_seqpacket(Arc::new(FakeEndpointPort::default()), vec![binding])
            .expect("valid seqpacket factory"),
    );
    let mut builder = ProviderRegistryBuilder::new(
        descriptor.registry_generation,
        fingerprint(0x34),
        NOW_UNIX_MS,
    );
    let construction = factory
        .construct_with_handoff(&descriptor)
        .expect("matching transport constructs with an isolated handoff");
    let (instance, handoffs) = construction.into_parts();
    builder
        .register_factory(factory.key(), factory.clone())
        .expect("factory registers");
    builder
        .register_constructed(factory.key(), instance)
        .expect("explicit construction registers");
    let registry = builder.finish().expect("registry builds");
    let instance = registry
        .instance(&descriptor.provider_id)
        .expect("constructed transport registered");
    assert!(matches!(&instance, ProviderInstance::Transport(_)));
    assert_eq!(instance.descriptor(), descriptor);
    assert_eq!(
        handoffs
            .take_transport(&HandleId::parse("not-connected").expect("valid handle id"))
            .expect_err("exact handoff starts empty"),
        TransportHandoffError::UnknownHandle
    );
}

#[tokio::test]
async fn retained_factory_handoff_claims_transport_after_registry_type_erasure() {
    let harness = Harness::new(LocalTransportKind::UnixStream);
    let configured_binding = binding(
        &harness.descriptor,
        LocalTransportKind::UnixStream,
        harness.binding_id.clone(),
        harness.scope.clone(),
        fingerprint(0x41),
        harness.endpoint_generation,
    );
    let port = Arc::new(FakeEndpointPort::default());
    let factory = Arc::new(
        LocalTransportFactory::with_clock_and_limits(
            LocalTransportKind::UnixStream,
            port.clone(),
            vec![configured_binding],
            Arc::new(FixedClock),
            LocalTransportLimits::default(),
        )
        .expect("valid handoff factory"),
    );
    let mut builder = ProviderRegistryBuilder::new(
        harness.descriptor.registry_generation,
        fingerprint(0x39),
        NOW_UNIX_MS,
    );
    builder
        .register_factory(factory.key(), factory.clone())
        .expect("handoff factory registers");
    builder
        .register_instance(harness.descriptor.clone())
        .expect("handoff provider constructs");
    let registry = builder.finish().expect("handoff registry builds");
    let ProviderInstance::Transport(provider) = registry
        .instance(&harness.descriptor.provider_id)
        .expect("erased transport registered")
    else {
        panic!("registered provider remains a transport");
    };
    let request = harness.connect_request("factory-handoff-connect");
    let context = harness.call_context(&request.context);
    let handle = provider
        .connect(&context, &request)
        .await
        .expect("erased provider connects");
    let transport = factory
        .take_transport(&harness.descriptor.provider_id, &handle.handle_id)
        .expect("retained factory exposes typed handoff");
    assert_eq!(transport.kind(), LocalTransportKind::UnixStream);
    assert_eq!(
        factory
            .take_transport(&harness.descriptor.provider_id, &handle.handle_id)
            .expect_err("factory handoff remains single use"),
        TransportHandoffError::AlreadyClaimed
    );
    let unclaimed_request = harness.connect_request("factory-unclaimed-connect");
    let unclaimed_context = harness.call_context(&unclaimed_request.context);
    provider
        .connect(&unclaimed_context, &unclaimed_request)
        .await
        .expect("second erased provider connection remains unclaimed");
    assert_eq!(port.closed_peer_count(), 0);
    drop(provider);
    drop(registry);
    assert_eq!(port.closed_peer_count(), 1);
    drop(transport);
    assert_eq!(port.closed_peer_count(), 2);
}

#[test]
fn trait_factory_construction_is_single_live_instance() {
    let kind = LocalTransportKind::UnixStream;
    let descriptor = descriptor(kind);
    let configured_binding = binding(
        &descriptor,
        kind,
        transport_binding_id("single-live-instance"),
        AuthorizedProviderScope::Realm {
            realm_id: descriptor.placement.realm_id().clone(),
        },
        fingerprint(0x42),
        generation(10),
    );
    let factory = LocalTransportFactory::unix_stream(
        Arc::new(FakeEndpointPort::default()),
        [configured_binding],
    )
    .expect("valid factory");

    let first = factory
        .construct(&descriptor)
        .expect("first implicit instance constructs");
    let retained_handoff = factory
        .handoff_registry(&descriptor.provider_id)
        .expect("factory exposes first instance handoff");
    assert!(matches!(
        factory.construct(&descriptor),
        Err(FactoryError::Rejected)
    ));

    drop(first);
    assert!(matches!(
        factory.construct(&descriptor),
        Err(FactoryError::Rejected)
    ));
    drop(retained_handoff);
    assert!(
        factory
            .construct(&descriptor)
            .is_ok_and(|instance| matches!(instance, ProviderInstance::Transport(_)))
    );
}

#[tokio::test]
async fn dropping_one_constructed_instance_does_not_clear_another_handoff() {
    let harness = Harness::new(LocalTransportKind::UnixStream);
    let configured_binding = binding(
        &harness.descriptor,
        LocalTransportKind::UnixStream,
        harness.binding_id.clone(),
        harness.scope.clone(),
        fingerprint(0x43),
        harness.endpoint_generation,
    );
    let port = Arc::new(FakeEndpointPort::default());
    let factory = LocalTransportFactory::with_clock_and_limits(
        LocalTransportKind::UnixStream,
        port.clone(),
        [configured_binding],
        Arc::new(FixedClock),
        LocalTransportLimits::default(),
    )
    .expect("valid factory");

    let (first_instance, first_handoffs) = factory
        .construct_with_handoff(&harness.descriptor)
        .expect("first isolated instance constructs")
        .into_parts();
    let (second_instance, second_handoffs) = factory
        .construct_with_handoff(&harness.descriptor)
        .expect("second isolated instance constructs")
        .into_parts();
    let ProviderInstance::Transport(first_provider) = first_instance else {
        panic!("first instance is transport");
    };
    let ProviderInstance::Transport(second_provider) = second_instance else {
        panic!("second instance is transport");
    };

    let request = harness.connect_request("isolated-shared-handle");
    let context = harness.call_context(&request.context);
    let first_handle = first_provider
        .connect(&context, &request)
        .await
        .expect("first instance connects");
    let second_handle = second_provider
        .connect(&context, &request)
        .await
        .expect("second instance connects");
    assert_eq!(first_handle.handle_id, second_handle.handle_id);
    assert_eq!(port.closed_peer_count(), 0);

    drop(first_provider);
    assert_eq!(port.closed_peer_count(), 1);
    assert_eq!(
        first_handoffs
            .take_transport(&first_handle.handle_id)
            .expect_err("dropped instance clears only its own handoff"),
        TransportHandoffError::UnknownHandle
    );

    let second_transport = second_handoffs
        .take_transport(&second_handle.handle_id)
        .expect("second instance handoff remains claimable");
    let mut second_socket = UnixStream::from(second_transport.into_owned_fd());
    {
        let mut peers = lock(&port.connection_peers);
        peers[1]
            .write_all(b"x")
            .expect("second peer writes independently");
    }
    let mut byte = [0_u8; 1];
    second_socket
        .read_exact(&mut byte)
        .expect("second transport remains live");
    assert_eq!(byte, *b"x");

    drop(second_provider);
    assert_eq!(port.closed_peer_count(), 1);
    drop(second_socket);
    assert_eq!(port.closed_peer_count(), 2);
}

#[allow(clippy::result_large_err)]
async fn connect(harness: &Harness, operation_id: &str) -> Result<ProviderHandle, ProviderFailure> {
    let request = harness.connect_request(operation_id);
    let context = harness.call_context(&request.context);
    harness.provider.connect(&context, &request).await
}

#[allow(clippy::result_large_err)]
async fn inspect(
    harness: &Harness,
    operation_id: &str,
    handle: &ProviderHandle,
) -> Result<ProviderObservation, ProviderFailure> {
    let request = harness.handle_request(ProviderMethod::TransportInspect, operation_id, handle);
    let context = harness.call_context(&request.context);
    harness.provider.inspect(&context, &request).await
}

#[test]
fn advertises_only_live_methods_and_exact_transport_metadata() {
    let expected = ProviderCapabilitySet::new(vec![
        ProviderCapability(ProviderMethod::TransportConnect),
        ProviderCapability(ProviderMethod::TransportRevokeBinding),
        ProviderCapability(ProviderMethod::TransportInspect),
    ])
    .expect("valid exact capability set");
    assert_eq!(local_transport_capabilities(), expected);

    let expected_profiles = [
        TransportCapabilityProfile {
            transport_class: TransportClass::UnixStream,
            locality: Locality::HostLocal,
            packet_atomic: false,
            attachments: AttachmentCapability::Disabled,
            authentication: AuthenticationOwner::ComponentSession,
        },
        TransportCapabilityProfile {
            transport_class: TransportClass::UnixSeqpacket,
            locality: Locality::HostLocal,
            packet_atomic: true,
            attachments: AttachmentCapability::ComponentSessionNegotiatedPacketAtomic,
            authentication: AuthenticationOwner::ComponentSession,
        },
        TransportCapabilityProfile {
            transport_class: TransportClass::NativeVsock,
            locality: Locality::GuestLocal,
            packet_atomic: false,
            attachments: AttachmentCapability::Disabled,
            authentication: AuthenticationOwner::ComponentSession,
        },
        TransportCapabilityProfile {
            transport_class: TransportClass::CloudHypervisorVsock,
            locality: Locality::GuestLocal,
            packet_atomic: false,
            attachments: AttachmentCapability::Disabled,
            authentication: AuthenticationOwner::ComponentSession,
        },
    ];
    for (kind, expected_profile) in KINDS.into_iter().zip(expected_profiles) {
        assert_eq!(kind.capability_profile(), expected_profile);
    }
}

#[tokio::test]
async fn connects_all_four_kinds_over_closed_bindings() {
    for kind in KINDS {
        let harness = Harness::new(kind);
        let operation_id = format!("connect-{}", kind.implementation_id().as_str());
        let handle = connect(&harness, &operation_id)
            .await
            .expect("transport connects");
        assert_eq!(handle.kind, ProviderHandleKind::Transport);
        assert_eq!(handle.handle_id.as_str(), operation_id);
        assert_eq!(handle.resource_generation, harness.endpoint_generation);
        assert_eq!(handle.created_by.operation_id.as_str(), operation_id);

        let calls = harness.port.connect_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].operation_id.as_str(), operation_id);
        assert_eq!(calls[0].binding_id, harness.binding_id);
        assert_eq!(calls[0].kind, kind);
        assert_eq!(calls[0].capabilities, kind.capability_profile());
        assert_eq!(
            calls[0].endpoint.provenance(),
            EndpointProvenance::VerifiedBundle
        );
    }
}

#[tokio::test]
async fn canonical_handle_handoff_is_single_use_and_unclaimed_fds_are_raii_owned() {
    let harness = Harness::new(LocalTransportKind::UnixSeqpacket);
    let unknown = HandleId::parse("unknown-transport-handle").expect("valid handle id");
    assert_eq!(
        harness
            .provider
            .take_transport(&unknown)
            .expect_err("unknown handle rejected"),
        TransportHandoffError::UnknownHandle
    );

    let handle = connect(&harness, "handoff-connect")
        .await
        .expect("transport connects");
    let transport = harness
        .provider
        .take_transport(&handle.handle_id)
        .expect("canonical handle claims transport");
    assert_eq!(transport.kind(), LocalTransportKind::UnixSeqpacket);
    assert_eq!(
        transport.capabilities(),
        LocalTransportKind::UnixSeqpacket.capability_profile()
    );
    assert_eq!(
        harness
            .provider
            .take_transport(&handle.handle_id)
            .expect_err("transport handoff is single use"),
        TransportHandoffError::AlreadyClaimed
    );
    drop(transport);
    assert_eq!(harness.port.closed_peer_count(), 1);

    let unclaimed = Harness::new(LocalTransportKind::UnixStream);
    let port = unclaimed.port.clone();
    connect(&unclaimed, "unclaimed-connect")
        .await
        .expect("unclaimed transport connects");
    assert_eq!(port.closed_peer_count(), 0);
    drop(unclaimed);
    assert_eq!(port.closed_peer_count(), 1);
}

#[tokio::test]
async fn denies_wrong_binding_method_provider_and_scope_before_port_call() {
    let harness = Harness::new(LocalTransportKind::UnixStream);

    let mut unknown_binding = harness.connect_request("unknown-binding");
    unknown_binding.input = ProviderOperationInput::TransportBinding {
        transport_binding_id: transport_binding_id("binding-unknown"),
    };
    let context = harness.call_context(&unknown_binding.context);
    assert_eq!(
        harness
            .provider
            .connect(&context, &unknown_binding)
            .await
            .expect_err("unknown binding denied")
            .kind,
        ProviderFailureKind::InvalidRequest
    );

    let mut wrong_method = harness.connect_request("wrong-method");
    wrong_method.context.method = ProviderMethod::TransportInspect;
    wrong_method.context.capability = ProviderCapability(ProviderMethod::TransportInspect);
    let context = harness.call_context(&wrong_method.context);
    assert_eq!(
        harness
            .provider
            .connect(&context, &wrong_method)
            .await
            .expect_err("wrong method denied")
            .kind,
        ProviderFailureKind::CapabilityDenied
    );

    let mut wrong_provider = harness.connect_request("wrong-provider");
    wrong_provider.context.provider_id =
        ProviderId::parse("fffffffffffffffffffa").expect("valid other provider");
    let context = harness.call_context(&wrong_provider.context);
    assert_eq!(
        harness
            .provider
            .connect(&context, &wrong_provider)
            .await
            .expect_err("wrong provider denied")
            .kind,
        ProviderFailureKind::UnauthorizedScope
    );

    let workload_id = WorkloadId::parse("ggggggggggggggggggga").expect("valid workload id");
    let mut wrong_scope = harness.connect_request("wrong-scope");
    wrong_scope.context.scope = AuthorizedProviderScope::Workload {
        realm_id: harness.realm_id.clone(),
        workload_id: workload_id.clone(),
    };
    wrong_scope.target = ProviderTarget::Workload {
        realm_id: harness.realm_id.clone(),
        workload_id,
    };
    let context = harness.call_context(&wrong_scope.context);
    assert_eq!(
        harness
            .provider
            .connect(&context, &wrong_scope)
            .await
            .expect_err("wrong binding scope denied")
            .kind,
        ProviderFailureKind::UnauthorizedScope
    );

    let request = harness.connect_request("wrong-operation-binding");
    let other_operation = harness.operation(ProviderMethod::TransportConnect, "other-operation");
    let context = harness.call_context(&other_operation);
    assert_eq!(
        harness
            .provider
            .connect(&context, &request)
            .await
            .expect_err("mismatched operation denied")
            .kind,
        ProviderFailureKind::UnauthorizedScope
    );
    assert!(harness.port.connect_calls().is_empty());
}

#[tokio::test]
async fn reachability_never_claims_authentication_or_fd_policy() {
    let harness = Harness::new(LocalTransportKind::UnixSeqpacket);
    connect(&harness, "reachable-only")
        .await
        .expect("reachable endpoint connects");
    let request = harness
        .port
        .connect_calls()
        .pop()
        .expect("connector request recorded");
    assert_eq!(
        request.capabilities.authentication,
        AuthenticationOwner::ComponentSession
    );
    assert_eq!(
        request.capabilities.attachments,
        AttachmentCapability::ComponentSessionNegotiatedPacketAtomic
    );
    assert!(request.capabilities.packet_atomic);
}

#[tokio::test]
async fn propagates_operation_ids_through_connect_inspect_and_close() {
    let harness = Harness::new(LocalTransportKind::NativeVsock);
    let handle = connect(&harness, "operation-connect")
        .await
        .expect("connect succeeds");
    let observation = inspect(&harness, "operation-inspect", &handle)
        .await
        .expect("inspect succeeds");
    assert_eq!(observation.handle_id.as_ref(), Some(&handle.handle_id));
    assert_eq!(observation.lifecycle, ObservedLifecycleState::Running);

    let close_request = harness.handle_request(
        ProviderMethod::TransportRevokeBinding,
        "operation-close",
        &handle,
    );
    let close_context = harness.call_context(&close_request.context);
    let receipt = harness
        .provider
        .revoke_binding(&close_context, &close_request)
        .await
        .expect("close succeeds");
    assert_eq!(receipt.binding.operation_id.as_str(), "operation-close");
    assert_eq!(receipt.state, MutationState::Applied);

    assert_eq!(
        harness.port.connect_calls()[0].operation_id.as_str(),
        "operation-connect"
    );
    assert_eq!(
        harness.port.inspect_calls()[0].operation_id.as_str(),
        "operation-inspect"
    );
    assert_eq!(
        harness.port.close_calls()[0].operation_id.as_str(),
        "operation-close"
    );
    assert_eq!(harness.port.closed_peer_count(), 1);
}

#[tokio::test]
async fn realm_scoped_revoke_closes_every_handle_and_disables_binding() {
    let harness = Harness::new(LocalTransportKind::UnixStream);
    connect(&harness, "revoke-first")
        .await
        .expect("first connection succeeds");
    connect(&harness, "revoke-second")
        .await
        .expect("second connection succeeds");
    let request = ProviderOperationRequest {
        context: harness.operation(ProviderMethod::TransportRevokeBinding, "revoke-realm"),
        target: ProviderTarget::Realm {
            realm_id: harness.realm_id.clone(),
        },
        expected_configuration_fingerprint: harness
            .descriptor
            .configuration_schema_fingerprint
            .clone(),
        input: ProviderOperationInput::TransportBinding {
            transport_binding_id: harness.binding_id.clone(),
        },
    };
    let context = harness.call_context(&request.context);
    assert_eq!(
        harness
            .provider
            .revoke_binding(&context, &request)
            .await
            .expect("realm revoke succeeds")
            .state,
        MutationState::Applied
    );
    let calls = harness.port.close_calls();
    assert_eq!(calls.len(), 2);
    assert!(
        calls
            .iter()
            .all(|call| call.operation_id.as_str() == "revoke-realm")
    );
    assert_eq!(
        connect(&harness, "revoke-reconnect")
            .await
            .expect_err("revoked binding remains denied")
            .kind,
        ProviderFailureKind::UnauthorizedScope
    );
}

#[tokio::test]
async fn revoke_close_error_cannot_block_local_detach_and_cleanup() {
    let harness = Harness::new(LocalTransportKind::UnixStream);
    let first = connect(&harness, "cleanup-error-first")
        .await
        .expect("first connection");
    connect(&harness, "cleanup-error-second")
        .await
        .expect("second connection");
    harness.port.update(|behavior| {
        behavior.close_error = Some(EndpointPortError::Unavailable);
    });
    let request = harness.handle_request(
        ProviderMethod::TransportRevokeBinding,
        "cleanup-close-error",
        &first,
    );
    let context = harness.call_context(&request.context);
    assert_eq!(
        harness
            .provider
            .revoke_binding(&context, &request)
            .await
            .expect_err("external close error remains visible")
            .kind,
        ProviderFailureKind::Unavailable
    );
    assert_eq!(harness.port.closed_peer_count(), 2);
    assert_eq!(harness.port.close_calls().len(), 2);
    assert_eq!(
        harness
            .provider
            .take_transport(&first.handle_id)
            .expect_err("detached handle no longer claimable"),
        TransportHandoffError::UnknownHandle
    );
}

#[tokio::test]
async fn revoke_malformed_response_cannot_preserve_provider_owned_fd() {
    let harness = Harness::new(LocalTransportKind::UnixStream);
    let handle = connect(&harness, "cleanup-malformed-connect")
        .await
        .expect("connection");
    harness.port.update(|behavior| {
        behavior.close_operation_id =
            Some(OperationId::parse("substituted-close").expect("valid operation id"));
    });
    let request = harness.handle_request(
        ProviderMethod::TransportRevokeBinding,
        "cleanup-malformed-close",
        &handle,
    );
    let context = harness.call_context(&request.context);
    assert_eq!(
        harness
            .provider
            .revoke_binding(&context, &request)
            .await
            .expect_err("malformed close response remains visible")
            .kind,
        ProviderFailureKind::InvariantViolation
    );
    assert_eq!(harness.port.closed_peer_count(), 1);
    assert_eq!(
        harness
            .provider
            .take_transport(&handle.handle_id)
            .expect_err("malformed response cannot restore detached handle"),
        TransportHandoffError::UnknownHandle
    );
}

#[tokio::test]
async fn revoke_deadline_cannot_block_local_detach_and_cleanup() {
    let harness = Harness::new(LocalTransportKind::UnixStream);
    let handle = connect(&harness, "cleanup-deadline-connect")
        .await
        .expect("connection");
    harness.port.update(|behavior| {
        behavior.pending_close = true;
    });
    let request = harness.handle_request(
        ProviderMethod::TransportRevokeBinding,
        "cleanup-deadline-close",
        &handle,
    );
    let mut context = harness.call_context(&request.context);
    context.monotonic_deadline_remaining_ms = 5;
    assert_eq!(
        harness
            .provider
            .revoke_binding(&context, &request)
            .await
            .expect_err("external close deadline remains visible")
            .kind,
        ProviderFailureKind::DeadlineExpired
    );
    assert_eq!(harness.port.closed_peer_count(), 1);
    assert!(harness.port.pending_close_dropped.load(Ordering::Acquire));
    assert_eq!(
        harness
            .provider
            .take_transport(&handle.handle_id)
            .expect_err("deadline cannot restore detached handle"),
        TransportHandoffError::UnknownHandle
    );
}

#[tokio::test]
async fn revoke_is_not_blocked_by_an_inflight_external_connect() {
    let harness = Harness::new(LocalTransportKind::UnixStream);
    harness.port.update(|behavior| {
        behavior.pending_connect = true;
    });
    let connect_request = harness.connect_request("inflight-connect");
    let connect_context = harness.call_context(&connect_request.context);
    let mut connect_future = Box::pin(harness.provider.connect(&connect_context, &connect_request));
    tokio::select! {
        _ = harness.port.wait_for_connect() => {}
        result = &mut connect_future => panic!("connect unexpectedly completed: {result:?}"),
    }

    let revoke_request = ProviderOperationRequest {
        context: harness.operation(
            ProviderMethod::TransportRevokeBinding,
            "revoke-during-connect",
        ),
        target: ProviderTarget::Realm {
            realm_id: harness.realm_id.clone(),
        },
        expected_configuration_fingerprint: harness
            .descriptor
            .configuration_schema_fingerprint
            .clone(),
        input: ProviderOperationInput::TransportBinding {
            transport_binding_id: harness.binding_id.clone(),
        },
    };
    let revoke_context = harness.call_context(&revoke_request.context);
    let receipt = tokio::time::timeout(
        Duration::from_millis(50),
        harness
            .provider
            .revoke_binding(&revoke_context, &revoke_request),
    )
    .await
    .expect("local revoke is independent of connector")
    .expect("binding revokes while connect is pending");
    assert_eq!(receipt.state, MutationState::Applied);
    drop(connect_future);
    assert!(harness.port.pending_connect_dropped.load(Ordering::Acquire));

    harness.port.update(|behavior| {
        behavior.pending_connect = false;
    });
    assert_eq!(
        connect(&harness, "after-inflight-revoke")
            .await
            .expect_err("revoked binding rejects future connect")
            .kind,
        ProviderFailureKind::UnauthorizedScope
    );
    assert_eq!(harness.port.connect_calls().len(), 1);
}

#[tokio::test]
async fn enforces_active_bound_until_close_releases_slot() {
    let harness = Harness::with_limits(
        LocalTransportKind::CloudHypervisorVsock,
        LocalTransportLimits {
            max_bindings: 1,
            max_active: 1,
        },
    );
    let first = connect(&harness, "bound-first")
        .await
        .expect("first connection succeeds");
    let repeated = connect(&harness, "bound-first")
        .await
        .expect("idempotent active handle bypasses slot pressure");
    assert_eq!(repeated, first);
    let failure = connect(&harness, "bound-second")
        .await
        .expect_err("second active connection denied");
    assert_eq!(failure.kind, ProviderFailureKind::Unavailable);
    assert_eq!(harness.port.connect_calls().len(), 1);

    harness.port.update(|behavior| {
        behavior.inspect_state = EndpointObservationState::Closed;
    });
    inspect(&harness, "bound-close-observed", &first)
        .await
        .expect("closed observation releases slot");
    connect(&harness, "bound-second")
        .await
        .expect("connection succeeds after released handle");
    assert_eq!(harness.port.connect_calls().len(), 2);
}

#[test]
fn enforces_binding_and_configuration_bounds() {
    let kind = LocalTransportKind::UnixStream;
    let descriptor = descriptor(kind);
    let realm_id = descriptor.placement.realm_id().clone();
    let scope = AuthorizedProviderScope::Realm { realm_id };
    let first = binding(
        &descriptor,
        kind,
        transport_binding_id("binding-one"),
        scope.clone(),
        fingerprint(0x61),
        generation(1),
    );
    let second = binding(
        &descriptor,
        kind,
        transport_binding_id("binding-two"),
        scope,
        fingerprint(0x62),
        generation(2),
    );
    let port = Arc::new(FakeEndpointPort::default());
    let result = LocalTransportProvider::with_clock_and_limits(
        descriptor,
        kind,
        vec![first, second],
        port,
        Arc::new(FixedClock),
        LocalTransportLimits {
            max_bindings: 1,
            max_active: 1,
        },
    );
    assert!(matches!(
        result,
        Err(LocalTransportConfigurationError::BindingLimit)
    ));
}

#[tokio::test]
async fn cancellation_and_deadline_never_leave_connector_work_active() {
    let cancelled = Harness::new(LocalTransportKind::UnixStream);
    let request = cancelled.connect_request("cancelled-before-call");
    let mut context = cancelled.call_context(&request.context);
    context.cancelled = true;
    assert_eq!(
        cancelled
            .provider
            .connect(&context, &request)
            .await
            .expect_err("cancelled call denied")
            .kind,
        ProviderFailureKind::Cancelled
    );
    assert!(cancelled.port.connect_calls().is_empty());

    let timed_out = Harness::new(LocalTransportKind::UnixStream);
    timed_out.port.update(|behavior| {
        behavior.pending_connect = true;
    });
    let request = timed_out.connect_request("deadline-connect");
    let mut context = timed_out.call_context(&request.context);
    context.monotonic_deadline_remaining_ms = 5;
    assert_eq!(
        timed_out
            .provider
            .connect(&context, &request)
            .await
            .expect_err("pending connector times out")
            .kind,
        ProviderFailureKind::DeadlineExpired
    );
    assert!(
        timed_out
            .port
            .pending_connect_dropped
            .load(Ordering::Acquire)
    );
}

#[tokio::test]
async fn dropping_connect_future_cancels_injected_connector() {
    let harness = Harness::new(LocalTransportKind::UnixSeqpacket);
    harness.port.update(|behavior| {
        behavior.pending_connect = true;
    });
    let provider = harness.provider.clone();
    let descriptor = harness.descriptor.clone();
    let realm_id = harness.realm_id.clone();
    let scope = harness.scope.clone();
    let binding_id = harness.binding_id.clone();
    let task = tokio::spawn(async move {
        let context_value = ProviderOperationContext {
            schema_version: PROVIDER_SCHEMA_VERSION,
            operation_id: OperationId::parse("abort-connect").expect("valid operation id"),
            idempotency_key: IdempotencyKey::parse("idem-abort-connect")
                .expect("valid idempotency key"),
            request_digest: fingerprint(0x71),
            scope,
            principal: PrincipalRef::parse("principal-local-controller").expect("valid principal"),
            provider_id: descriptor.provider_id,
            provider_type: ProviderType::Transport,
            provider_generation: descriptor.registry_generation,
            capability: ProviderCapability(ProviderMethod::TransportConnect),
            method: ProviderMethod::TransportConnect,
            policy_epoch: generation(3),
            authorization_decision_digest: fingerprint(0x72),
            issued_at_unix_ms: NOW_UNIX_MS - 1_000,
            expires_at_unix_ms: NOW_UNIX_MS + 60_000,
            correlation_id: CorrelationId::parse("correlation-abort")
                .expect("valid correlation id"),
            trace_id: fingerprint(0x73),
        };
        let request = ProviderOperationRequest {
            context: context_value,
            target: ProviderTarget::Realm { realm_id },
            expected_configuration_fingerprint: descriptor.configuration_schema_fingerprint,
            input: ProviderOperationInput::TransportBinding {
                transport_binding_id: binding_id,
            },
        };
        let call = ProviderCallContext {
            operation: &request.context,
            peer_role: EndpointRole::RealmController,
            service: ServicePackage::ProviderV2,
            monotonic_deadline_remaining_ms: 30_000,
            cancelled: false,
        };
        provider.connect(&call, &request).await
    });
    harness.port.wait_for_connect().await;
    task.abort();
    assert!(task.await.expect_err("task aborted").is_cancelled());
    assert!(harness.port.pending_connect_dropped.load(Ordering::Acquire));
    harness.port.update(|behavior| {
        behavior.pending_connect = false;
    });
    connect(&harness, "abort-connect")
        .await
        .expect("cancelled pending reservation is released");
}

#[tokio::test]
async fn invalid_post_connect_output_rolls_back_owned_connection_exactly_once() {
    let harness = Harness::new(LocalTransportKind::UnixStream);
    let port = harness.port.clone();
    harness.port.update(|behavior| {
        behavior.connect_operation_id =
            Some(OperationId::parse("substituted-operation").expect("valid operation id"));
    });

    assert_eq!(
        connect(&harness, "rollback-connect")
            .await
            .expect_err("substituted connector envelope rejected")
            .kind,
        ProviderFailureKind::InvariantViolation
    );
    assert_eq!(port.closed_peer_count(), 1);
    drop(harness);
    assert_eq!(port.closed_peer_count(), 1);
}

#[tokio::test]
async fn provider_refuses_to_publish_a_preclaimed_connection() {
    let harness = Harness::new(LocalTransportKind::UnixStream);
    harness.port.update(|behavior| {
        behavior.connect_preclaimed = true;
    });
    assert_eq!(
        connect(&harness, "preclaimed-connect")
            .await
            .expect_err("preclaimed connector result rejected")
            .kind,
        ProviderFailureKind::InvariantViolation
    );
    assert_eq!(harness.port.closed_peer_count(), 1);
}

#[tokio::test]
async fn rejects_substituted_identity_and_generation_evidence() {
    for (identity, generation_override, expected_kind) in [
        (
            Some(fingerprint(0x81)),
            None,
            ProviderFailureKind::AdoptionRejected,
        ),
        (
            None,
            Some(generation(81)),
            ProviderFailureKind::AdoptionRejected,
        ),
    ] {
        let harness = Harness::new(LocalTransportKind::NativeVsock);
        harness.port.update(|behavior| {
            behavior.connect_identity = identity;
            behavior.connect_generation = generation_override;
        });
        assert_eq!(
            connect(&harness, "substituted-connect")
                .await
                .expect_err("substituted endpoint rejected")
                .kind,
            expected_kind
        );
    }

    let harness = Harness::new(LocalTransportKind::NativeVsock);
    let handle = connect(&harness, "verified-connect")
        .await
        .expect("connect succeeds");
    harness.port.update(|behavior| {
        behavior.inspect_identity = Some(fingerprint(0x82));
    });
    let identity_observation = inspect(&harness, "inspect-identity", &handle)
        .await
        .expect("identity mismatch is typed observation");
    assert_eq!(
        identity_observation.lifecycle,
        ObservedLifecycleState::Quarantined
    );
    assert_eq!(identity_observation.adoption, AdoptionState::Rejected);
    assert_eq!(
        identity_observation.reason,
        ObservationReason::IdentityMismatch
    );

    harness.port.update(|behavior| {
        behavior.inspect_identity = None;
        behavior.inspect_generation = Some(generation(82));
    });
    let generation_observation = inspect(&harness, "inspect-generation", &handle)
        .await
        .expect("generation mismatch is typed observation");
    assert_eq!(
        generation_observation.reason,
        ObservationReason::GenerationMismatch
    );

    harness.port.update(|behavior| {
        behavior.inspect_generation = None;
        behavior.close_identity = Some(fingerprint(0x83));
    });
    let request = harness.handle_request(
        ProviderMethod::TransportRevokeBinding,
        "close-identity",
        &handle,
    );
    let context = harness.call_context(&request.context);
    assert_eq!(
        harness
            .provider
            .revoke_binding(&context, &request)
            .await
            .expect_err("close identity mismatch rejected")
            .kind,
        ProviderFailureKind::AdoptionRejected
    );

    harness.port.update(|behavior| {
        behavior.close_identity = None;
    });
    let request = harness.handle_request(
        ProviderMethod::TransportRevokeBinding,
        "close-verified",
        &handle,
    );
    let context = harness.call_context(&request.context);
    assert_eq!(
        harness
            .provider
            .revoke_binding(&context, &request)
            .await
            .expect("verified close succeeds")
            .state,
        MutationState::AlreadyApplied
    );
}

#[tokio::test]
async fn inspect_closed_releases_handle_and_revoke_is_idempotent() {
    let harness = Harness::new(LocalTransportKind::CloudHypervisorVsock);
    let handle = connect(&harness, "closed-connect")
        .await
        .expect("connect succeeds");
    harness.port.update(|behavior| {
        behavior.inspect_state = EndpointObservationState::Closed;
    });
    let observation = inspect(&harness, "closed-inspect", &handle)
        .await
        .expect("closed state observed");
    assert_eq!(observation.lifecycle, ObservedLifecycleState::Released);

    let request = harness.handle_request(
        ProviderMethod::TransportRevokeBinding,
        "closed-revoke",
        &handle,
    );
    let context = harness.call_context(&request.context);
    assert_eq!(
        harness
            .provider
            .revoke_binding(&context, &request)
            .await
            .expect("binding revocation succeeds without active handle")
            .state,
        MutationState::Applied
    );
    assert!(harness.port.close_calls().is_empty());

    let request = harness.handle_request(
        ProviderMethod::TransportRevokeBinding,
        "closed-revoke-repeat",
        &handle,
    );
    let context = harness.call_context(&request.context);
    assert_eq!(
        harness
            .provider
            .revoke_binding(&context, &request)
            .await
            .expect("binding revocation is idempotent")
            .state,
        MutationState::AlreadyApplied
    );
    assert_eq!(
        connect(&harness, "closed-reconnect")
            .await
            .expect_err("revoked binding cannot reconnect")
            .kind,
        ProviderFailureKind::UnauthorizedScope
    );
}

#[tokio::test]
async fn production_port_adapts_every_owned_transport_and_closes_idempotently() {
    for (index, kind) in KINDS.into_iter().enumerate() {
        let identity = fingerprint(0xa0 + u8::try_from(index).expect("small index"));
        let endpoint_generation = generation(100 + u64::try_from(index).expect("small index"));
        let (descriptor, _peer, responder): (
            OwnedEndpointDescriptor,
            Option<Box<dyn Send>>,
            Option<thread::JoinHandle<()>>,
        ) = match kind {
            LocalTransportKind::UnixSeqpacket => {
                let (socket, peer) = UnixDatagram::pair().expect("packet socket pair");
                socket
                    .set_nonblocking(true)
                    .expect("nonblocking packet endpoint");
                (
                    OwnedEndpointDescriptor::from_pre_authorized(
                        kind,
                        OwnedFd::from(socket),
                        identity.clone(),
                        endpoint_generation,
                    ),
                    Some(Box::new(peer)),
                    None,
                )
            }
            LocalTransportKind::CloudHypervisorVsock => {
                let (socket, mut peer) = UnixStream::pair().expect("CH stream pair");
                socket
                    .set_nonblocking(true)
                    .expect("nonblocking CH endpoint");
                let responder = thread::spawn(move || {
                    let mut line = Vec::new();
                    let mut byte = [0_u8; 1];
                    loop {
                        peer.read_exact(&mut byte).expect("read CONNECT byte");
                        line.push(byte[0]);
                        if byte[0] == b'\n' {
                            break;
                        }
                    }
                    assert_eq!(line, b"CONNECT 14318\n");
                    peer.write_all(b"OK 7\n").expect("write bounded CH ack");
                });
                (
                    OwnedEndpointDescriptor::from_pre_authorized_cloud_hypervisor(
                        OwnedFd::from(socket),
                        CloudHypervisorVsockPort::new(14_318).expect("nonzero CH port"),
                        identity.clone(),
                        endpoint_generation,
                    ),
                    None,
                    Some(responder),
                )
            }
            LocalTransportKind::UnixStream | LocalTransportKind::NativeVsock => {
                let (socket, peer) = UnixStream::pair().expect("stream socket pair");
                socket
                    .set_nonblocking(true)
                    .expect("nonblocking stream endpoint");
                (
                    OwnedEndpointDescriptor::from_pre_authorized(
                        kind,
                        OwnedFd::from(socket),
                        identity.clone(),
                        endpoint_generation,
                    ),
                    Some(Box::new(peer)),
                    None,
                )
            }
        };
        let resolver = Arc::new(RejectingResolver::default());
        let port = TokioLocalEndpointPort::new(resolver.clone());
        let operation_id = format!("production-owned-{index}");
        let connection = port
            .connect(endpoint_connect_request(
                kind,
                &operation_id,
                EndpointSource::owned(descriptor),
                identity.clone(),
                endpoint_generation,
                Duration::from_secs(1),
            ))
            .await
            .expect("owned production endpoint connects");
        assert_eq!(connection.operation_id.as_str(), operation_id);
        assert_eq!(connection.identity, identity);
        assert_eq!(connection.generation, endpoint_generation);
        assert_eq!(connection.kind, kind);
        assert_eq!(connection.capabilities, kind.capability_profile());
        assert_eq!(connection.reachability, ReachabilityEvidence::ReachableOnly);
        assert!(connection.owned().is_open());
        assert_eq!(resolver.calls.load(Ordering::Acquire), 0);

        let inspect = port
            .inspect(
                EndpointInspectRequest {
                    operation_id: OperationId::parse(format!("inspect-{index}"))
                        .expect("valid inspect operation"),
                    handle_id: connection.handle_id.clone(),
                    binding_id: connection.binding_id.clone(),
                    expected_identity: identity.clone(),
                    expected_generation: endpoint_generation,
                    kind,
                    capabilities: kind.capability_profile(),
                    deadline: Duration::from_secs(1),
                },
                connection.owned(),
            )
            .await
            .expect("owned endpoint inspects");
        assert_eq!(inspect.state, EndpointObservationState::Connected);

        let close_request = EndpointCloseRequest {
            operation_id: OperationId::parse(format!("close-{index}"))
                .expect("valid close operation"),
            handle_id: connection.handle_id.clone(),
            binding_id: connection.binding_id.clone(),
            expected_identity: identity,
            expected_generation: endpoint_generation,
            kind,
            deadline: Duration::from_secs(1),
        };
        assert_eq!(
            port.close(close_request.clone(), connection.owned())
                .await
                .expect("first close succeeds")
                .state,
            EndpointCloseState::Closed
        );
        assert_eq!(
            port.close(close_request, connection.owned())
                .await
                .expect("repeat close succeeds")
                .state,
            EndpointCloseState::AlreadyClosed
        );
        assert!(!connection.owned().is_open());
        if let Some(responder) = responder {
            responder.join().expect("CH responder completes");
        }
    }
}

#[tokio::test]
async fn concurrent_connects_resolve_exclusive_descriptors_without_cross_consumption() {
    let harness = Harness::new(LocalTransportKind::UnixStream);
    let endpoint_identity = fingerprint(0x41);
    let mut descriptors = Vec::new();
    let mut peers = Vec::new();
    for _ in 0..2 {
        let (socket, peer) = UnixStream::pair().expect("exclusive stream pair");
        socket
            .set_nonblocking(true)
            .expect("nonblocking exclusive endpoint");
        peer.set_read_timeout(Some(Duration::from_secs(1)))
            .expect("bounded peer read");
        descriptors.push(OwnedEndpointDescriptor::from_pre_authorized(
            LocalTransportKind::UnixStream,
            OwnedFd::from(socket),
            endpoint_identity.clone(),
            harness.endpoint_generation,
        ));
        peers.push(peer);
    }
    let resolver = Arc::new(StaticResolver::from_descriptors(descriptors));
    let endpoint_port: Arc<dyn LocalEndpointPort> =
        Arc::new(TokioLocalEndpointPort::new(resolver.clone()));
    let configured_binding = binding(
        &harness.descriptor,
        LocalTransportKind::UnixStream,
        harness.binding_id.clone(),
        harness.scope.clone(),
        endpoint_identity,
        harness.endpoint_generation,
    );
    let provider = Arc::new(
        LocalTransportProvider::with_clock_and_limits(
            harness.descriptor.clone(),
            LocalTransportKind::UnixStream,
            vec![configured_binding],
            endpoint_port,
            Arc::new(FixedClock),
            LocalTransportLimits::default(),
        )
        .expect("exclusive transport provider"),
    );
    let first_request = harness.connect_request("exclusive-first");
    let second_request = harness.connect_request("exclusive-second");
    let first_context = harness.call_context(&first_request.context);
    let second_context = harness.call_context(&second_request.context);
    let (first, second) = tokio::join!(
        provider.connect(&first_context, &first_request),
        provider.connect(&second_context, &second_request),
    );
    let first = first.expect("first exclusive connect");
    let second = second.expect("second exclusive connect");
    assert_eq!(resolver.calls().len(), 2);

    let mut first_stream = UnixStream::from(
        provider
            .take_transport(&first.handle_id)
            .expect("first transport handoff")
            .into_owned_fd(),
    );
    let mut second_stream = UnixStream::from(
        provider
            .take_transport(&second.handle_id)
            .expect("second transport handoff")
            .into_owned_fd(),
    );
    assert_ne!(first_stream.as_raw_fd(), second_stream.as_raw_fd());
    first_stream.write_all(b"A").expect("write first transport");
    second_stream
        .write_all(b"B")
        .expect("write second transport");

    let mut received = Vec::new();
    for mut peer in peers {
        let mut byte = [0_u8; 1];
        peer.read_exact(&mut byte)
            .expect("each independent peer receives traffic");
        received.push(byte[0]);
    }
    received.sort_unstable();
    assert_eq!(received, b"AB");
}

#[tokio::test]
async fn already_owned_connected_capability_is_claimed_by_only_one_connect() {
    let (socket, _peer) = UnixStream::pair().expect("single-use owned pair");
    socket
        .set_nonblocking(true)
        .expect("nonblocking single-use endpoint");
    let identity = fingerprint(0xa9);
    let endpoint_generation = generation(109);
    let source = EndpointSource::owned(OwnedEndpointDescriptor::from_pre_authorized(
        LocalTransportKind::UnixStream,
        OwnedFd::from(socket),
        identity.clone(),
        endpoint_generation,
    ));
    let port = TokioLocalEndpointPort::new(Arc::new(RejectingResolver::default()));
    let (first, second) = tokio::join!(
        port.connect(endpoint_connect_request(
            LocalTransportKind::UnixStream,
            "single-use-first",
            source.clone(),
            identity.clone(),
            endpoint_generation,
            Duration::from_secs(1),
        )),
        port.connect(endpoint_connect_request(
            LocalTransportKind::UnixStream,
            "single-use-second",
            source,
            identity,
            endpoint_generation,
            Duration::from_secs(1),
        )),
    );
    let connection = match (first, second) {
        (Ok(connection), Err(EndpointPortError::Unavailable))
        | (Err(EndpointPortError::Unavailable), Ok(connection)) => connection,
        _ => panic!("exactly one owned endpoint claim must succeed"),
    };
    assert_eq!(
        connection
            .owned()
            .take_transport()
            .expect("successful claim has actual transport")
            .kind(),
        LocalTransportKind::UnixStream
    );
}

#[tokio::test]
async fn production_resolver_accepts_only_opaque_capabilities_and_propagates_operation_id() {
    for use_lease in [false, true] {
        let (socket, _peer) = UnixStream::pair().expect("resolver stream pair");
        socket
            .set_nonblocking(true)
            .expect("nonblocking resolver endpoint");
        let identity = fingerprint(if use_lease { 0xb1 } else { 0xb0 });
        let endpoint_generation = generation(if use_lease { 111 } else { 110 });
        let descriptor = OwnedEndpointDescriptor::from_pre_authorized(
            LocalTransportKind::UnixStream,
            OwnedFd::from(socket),
            identity.clone(),
            endpoint_generation,
        );
        let resolver = Arc::new(StaticResolver::new(descriptor));
        let port = TokioLocalEndpointPort::new(resolver.clone());
        let capability_binding = transport_binding_id(if use_lease {
            "authorized-lease"
        } else {
            "verified-bundle"
        });
        let endpoint = if use_lease {
            EndpointSource::lease(
                LocalTransportKind::UnixStream,
                EndpointLeaseId::new(capability_binding.clone()),
            )
        } else {
            EndpointSource::bundle(
                LocalTransportKind::UnixStream,
                BundleEndpointId::new(capability_binding.clone()),
            )
        };
        let operation_id = if use_lease {
            "resolve-authorized-lease"
        } else {
            "resolve-verified-bundle"
        };
        let connection = port
            .connect(endpoint_connect_request(
                LocalTransportKind::UnixStream,
                operation_id,
                endpoint,
                identity,
                endpoint_generation,
                Duration::from_secs(1),
            ))
            .await
            .expect("opaque capability resolves");
        let calls = resolver.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].operation_id.as_str(), operation_id);
        assert_eq!(calls[0].kind, LocalTransportKind::UnixStream);
        match &calls[0].capability_id {
            EndpointCapabilityId::VerifiedBundle(id) if !use_lease => {
                assert_eq!(id.as_binding_id(), &capability_binding);
            }
            EndpointCapabilityId::AuthorizedLease(id) if use_lease => {
                assert_eq!(id.as_binding_id(), &capability_binding);
            }
            _ => panic!("resolver capability provenance changed"),
        }
        assert_eq!(connection.owned().close(), EndpointCloseState::Closed);
    }
}

#[tokio::test]
async fn production_resolver_identity_and_generation_are_verified_before_publication() {
    for (identity, endpoint_generation, expected) in [
        (
            fingerprint(0xc1),
            generation(120),
            EndpointPortError::IdentityMismatch,
        ),
        (
            fingerprint(0xc0),
            generation(121),
            EndpointPortError::GenerationMismatch,
        ),
    ] {
        let (socket, _peer) = UnixStream::pair().expect("verification stream pair");
        socket
            .set_nonblocking(true)
            .expect("nonblocking verification endpoint");
        let descriptor = OwnedEndpointDescriptor::from_pre_authorized(
            LocalTransportKind::UnixStream,
            OwnedFd::from(socket),
            identity,
            endpoint_generation,
        );
        let resolver = Arc::new(StaticResolver::new(descriptor));
        let port = TokioLocalEndpointPort::new(resolver);
        let error = port
            .connect(endpoint_connect_request(
                LocalTransportKind::UnixStream,
                "resolver-evidence-mismatch",
                EndpointSource::bundle(
                    LocalTransportKind::UnixStream,
                    BundleEndpointId::new(transport_binding_id("resolver-evidence")),
                ),
                fingerprint(0xc0),
                generation(120),
                Duration::from_secs(1),
            ))
            .await
            .expect_err("substituted resolver evidence rejected");
        assert_eq!(error, expected);
    }
}

#[tokio::test]
async fn cloud_hypervisor_handshake_is_bounded_and_deadlined() {
    let identity = fingerprint(0xd0);
    let endpoint_generation = generation(130);
    let (socket, mut peer) = UnixStream::pair().expect("bounded CH pair");
    socket
        .set_nonblocking(true)
        .expect("nonblocking bounded CH endpoint");
    let responder = thread::spawn(move || {
        let mut line = [0_u8; 14];
        peer.read_exact(&mut line).expect("read bounded CONNECT");
        assert_eq!(&line, b"CONNECT 14318\n");
        peer.write_all(&[b'7'; 64]).expect("write oversized ack");
    });
    let descriptor = OwnedEndpointDescriptor::from_pre_authorized_cloud_hypervisor(
        OwnedFd::from(socket),
        CloudHypervisorVsockPort::new(14_318).expect("nonzero CH port"),
        identity.clone(),
        endpoint_generation,
    );
    let port = TokioLocalEndpointPort::new(Arc::new(RejectingResolver::default()));
    assert_eq!(
        port.connect(endpoint_connect_request(
            LocalTransportKind::CloudHypervisorVsock,
            "bounded-ch-ack",
            EndpointSource::owned(descriptor),
            identity.clone(),
            endpoint_generation,
            Duration::from_secs(1),
        ))
        .await
        .expect_err("oversized CH ack rejected"),
        EndpointPortError::BoundExceeded
    );
    responder.join().expect("bounded CH responder completes");

    let (socket, mut peer) = UnixStream::pair().expect("deadline CH pair");
    socket
        .set_nonblocking(true)
        .expect("nonblocking deadline CH endpoint");
    let responder = thread::spawn(move || {
        let mut line = [0_u8; 14];
        peer.read_exact(&mut line).expect("read deadline CONNECT");
        assert_eq!(&line, b"CONNECT 14318\n");
        thread::sleep(Duration::from_millis(50));
    });
    let descriptor = OwnedEndpointDescriptor::from_pre_authorized_cloud_hypervisor(
        OwnedFd::from(socket),
        CloudHypervisorVsockPort::new(14_318).expect("nonzero CH port"),
        identity.clone(),
        endpoint_generation,
    );
    assert_eq!(
        port.connect(endpoint_connect_request(
            LocalTransportKind::CloudHypervisorVsock,
            "deadline-ch-ack",
            EndpointSource::owned(descriptor),
            identity,
            endpoint_generation,
            Duration::from_millis(5),
        ))
        .await
        .expect_err("silent CH ack times out"),
        EndpointPortError::DeadlineExpired
    );
    responder.join().expect("deadline CH responder completes");
}

#[test]
fn endpoint_sources_are_opaque_or_owned_and_debug_redacted() {
    let binding_id = transport_binding_id("opaque-endpoint-id");
    let bundle = EndpointSource::bundle(
        LocalTransportKind::UnixStream,
        BundleEndpointId::new(binding_id.clone()),
    );
    let lease = EndpointSource::lease(
        LocalTransportKind::NativeVsock,
        EndpointLeaseId::new(binding_id),
    );
    assert_eq!(bundle.provenance(), EndpointProvenance::VerifiedBundle);
    assert_eq!(lease.provenance(), EndpointProvenance::AuthorizedLease);
    assert!(!format!("{bundle:?}").contains("opaque-endpoint-id"));
    assert!(!format!("{lease:?}").contains("opaque-endpoint-id"));

    let (stream, _peer) = UnixStream::pair().expect("unix pair");
    stream.set_nonblocking(true).expect("nonblocking endpoint");
    let descriptor = OwnedEndpointDescriptor::from_pre_authorized(
        LocalTransportKind::UnixStream,
        OwnedFd::from(stream),
        fingerprint(0x91),
        generation(91),
    );
    let owned = EndpointSource::owned(descriptor);
    assert_eq!(owned.provenance(), EndpointProvenance::OwnedDescriptor);
    assert!(!format!("{owned:?}").contains("descriptor"));
    let capability = owned
        .owned_capability()
        .expect("owned capability remains closed typed");
    let claimed = capability.claim().expect("first claim transfers ownership");
    assert!(capability.claim().is_err());
    drop(claimed);
}
