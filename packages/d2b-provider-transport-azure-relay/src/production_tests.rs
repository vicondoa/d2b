use std::{
    future::pending,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_identity::{ProviderId, ProviderType},
    v2_provider::{
        Fingerprint, Generation, HandleId, LeaseId, MAX_PROVIDER_REGISTRY_ENTRIES,
        OperationBinding, OperationId, ProviderMethod, TransportBindingId,
    },
};
use d2b_provider::{ProviderFactory, ProviderInstance};
use d2b_provider_toolkit::Fixture;
use tokio::sync::Semaphore;

use crate::production::{RELAY_CLOSE_REPLAY_CAPACITY, RELAY_OPEN_REPLAY_CAPACITY};
use crate::{
    AzureRelayBinding, AzureRelayConfiguration, AzureRelayFactoryEntry, AzureRelayProviderFactory,
    ProductionRelayControlPort, RELAY_ACCEPT_QUEUE_CAPACITY, RELAY_MAX_CREDENTIAL_TTL_SECS,
    RELAY_MAX_FRAME_BYTES, RELAY_MAX_PROLOGUE_BYTES, RELAY_MAX_RECONNECT_BACKOFF_MS,
    RELAY_RECONNECT_STABLE_RESET_MS, RELAY_SENDER_RETRY_DELAY_MS, RELAY_SENDER_RETRY_LIMIT,
    RelayAdoptRequest, RelayCloseOutcome, RelayCloseRequest, RelayControlPort,
    RelayCredentialLease, RelayCredentialMaterial, RelayCredentialSource,
    RelayCredentialSourceFailure, RelayCredentialUse, RelayExpectedResource, RelayInspectRequest,
    RelayInspection, RelayOpenRequest, RelayPortCapabilities, RelayPortFailure, RelayRendezvousId,
    RelayResourceState, RelaySecret, RelaySocket, RelaySocketConnectRequest, RelaySocketConnection,
    RelaySocketConnector, RelaySocketEvent, RelaySocketFailure, RelaySocketRole,
    RelayTransportLimits, azure_relay_capabilities, azure_relay_implementation_id,
};

const SECRET_CANARY: &str = "secret-token-canary";
const NAMESPACE_CANARY: &str = "url-canary.servicebus.windows.net";

#[derive(Debug, Clone, PartialEq, Eq)]
struct CredentialCall {
    lease_id: String,
    credential_use: RelayCredentialUse,
}

struct FakeCredentialSource {
    calls: Mutex<Vec<CredentialCall>>,
    failure: Mutex<Option<RelayCredentialSourceFailure>>,
}

impl FakeCredentialSource {
    fn available() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            failure: Mutex::new(None),
        }
    }

    fn failing(failure: RelayCredentialSourceFailure) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            failure: Mutex::new(Some(failure)),
        }
    }

    fn calls(&self) -> Vec<CredentialCall> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl RelayCredentialSource for FakeCredentialSource {
    async fn resolve(
        &self,
        lease_id: &LeaseId,
        credential_use: RelayCredentialUse,
    ) -> Result<RelayCredentialLease, RelayCredentialSourceFailure> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(CredentialCall {
                lease_id: lease_id.as_str().to_owned(),
                credential_use,
            });
        if let Some(failure) = *self
            .failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            return Err(failure);
        }
        Ok(RelayCredentialLease::new(
            RelayCredentialMaterial::SasToken(
                RelaySecret::new(SECRET_CANARY.as_bytes().to_vec()).expect("bounded secret"),
            ),
            now_unix_ms() + 60_000,
        ))
    }
}

struct FakeSocket {
    lifecycle: AtomicU8,
    close_calls: AtomicUsize,
    close_gate: Arc<OneShotGate>,
}

const FAKE_SOCKET_OPEN: u8 = 0;
const FAKE_SOCKET_CLOSING: u8 = 1;
const FAKE_SOCKET_CLOSED: u8 = 2;

impl FakeSocket {
    fn new(close_gate: Arc<OneShotGate>) -> Self {
        Self {
            lifecycle: AtomicU8::new(FAKE_SOCKET_OPEN),
            close_calls: AtomicUsize::new(0),
            close_gate,
        }
    }
}

#[async_trait]
impl RelaySocket for FakeSocket {
    fn is_open(&self) -> bool {
        self.lifecycle.load(Ordering::Acquire) != FAKE_SOCKET_CLOSED
    }

    async fn receive(&self) -> Result<RelaySocketEvent, RelaySocketFailure> {
        pending::<Result<RelaySocketEvent, RelaySocketFailure>>().await
    }

    async fn send_binary(&self, bytes: &[u8]) -> Result<(), RelaySocketFailure> {
        if bytes.len() > usize::try_from(RELAY_MAX_FRAME_BYTES).expect("frame bound fits usize") {
            return Err(RelaySocketFailure::FrameTooLarge);
        }
        if self.lifecycle.load(Ordering::Acquire) != FAKE_SOCKET_OPEN {
            return Err(RelaySocketFailure::Unavailable);
        }
        Ok(())
    }

    async fn send_pong(&self, _bytes: &[u8]) -> Result<(), RelaySocketFailure> {
        if self.lifecycle.load(Ordering::Acquire) == FAKE_SOCKET_OPEN {
            Ok(())
        } else {
            Err(RelaySocketFailure::Unavailable)
        }
    }

    async fn close(&self) -> Result<(), RelaySocketFailure> {
        if self.lifecycle.load(Ordering::Acquire) == FAKE_SOCKET_CLOSED {
            return Ok(());
        }
        self.close_calls.fetch_add(1, Ordering::AcqRel);
        self.lifecycle.store(FAKE_SOCKET_CLOSING, Ordering::Release);
        self.close_gate.block_if_armed().await;
        self.lifecycle.store(FAKE_SOCKET_CLOSED, Ordering::Release);
        Ok(())
    }
}

struct DropSignal(Arc<AtomicUsize>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }
}

struct OneShotGate {
    remaining: AtomicUsize,
    started: Semaphore,
    release: Semaphore,
}

impl OneShotGate {
    fn new() -> Self {
        Self {
            remaining: AtomicUsize::new(0),
            started: Semaphore::new(0),
            release: Semaphore::new(0),
        }
    }

    fn arm(&self) {
        self.remaining.store(1, Ordering::Release);
    }

    async fn block_if_armed(&self) {
        if self
            .remaining
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                remaining.checked_sub(1)
            })
            .is_err()
        {
            return;
        }
        self.started.add_permits(1);
        self.release
            .acquire()
            .await
            .expect("gate remains open")
            .forget();
    }

    async fn wait_started(&self) {
        self.started
            .acquire()
            .await
            .expect("gate remains open")
            .forget();
    }

    fn release(&self) {
        self.release.add_permits(1);
    }
}

struct FakeSocketConnector {
    calls: AtomicUsize,
    listener_not_ready: AtomicUsize,
    hang: AtomicBool,
    dropped_connects: Arc<AtomicUsize>,
    roles: Mutex<Vec<RelaySocketRole>>,
    request_debug: Mutex<Vec<String>>,
    sockets: Mutex<Vec<Arc<FakeSocket>>>,
    connect_gate: Arc<OneShotGate>,
    close_gate: Arc<OneShotGate>,
}

impl FakeSocketConnector {
    fn ready() -> Self {
        let connect_gate = Arc::new(OneShotGate::new());
        let close_gate = Arc::new(OneShotGate::new());
        Self {
            calls: AtomicUsize::new(0),
            listener_not_ready: AtomicUsize::new(0),
            hang: AtomicBool::new(false),
            dropped_connects: Arc::new(AtomicUsize::new(0)),
            roles: Mutex::new(Vec::new()),
            request_debug: Mutex::new(Vec::new()),
            sockets: Mutex::new(Vec::new()),
            connect_gate,
            close_gate,
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::Acquire)
    }

    fn take_listener_not_ready(&self) -> bool {
        self.listener_not_ready
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
    }
}

#[async_trait]
impl RelaySocketConnector for FakeSocketConnector {
    async fn connect(
        &self,
        request: RelaySocketConnectRequest,
    ) -> Result<RelaySocketConnection, RelaySocketFailure> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        self.roles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(request.role());
        self.request_debug
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(format!("{request:?}"));
        if self.hang.load(Ordering::Acquire) {
            let _drop_signal = DropSignal(Arc::clone(&self.dropped_connects));
            pending::<()>().await;
            unreachable!("a hanging connector cannot complete");
        }
        self.connect_gate.block_if_armed().await;
        if self.take_listener_not_ready() {
            return Err(RelaySocketFailure::ListenerNotReady);
        }
        let socket = Arc::new(FakeSocket::new(Arc::clone(&self.close_gate)));
        self.sockets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(Arc::clone(&socket));
        let socket: Arc<dyn RelaySocket> = socket;
        Ok(RelaySocketConnection::connected(socket))
    }
}

struct ProductionHarness {
    port: Arc<ProductionRelayControlPort>,
    credentials: Arc<FakeCredentialSource>,
    connector: Arc<FakeSocketConnector>,
    binding_id: TransportBindingId,
    rendezvous_id: RelayRendezvousId,
    other_binding_id: TransportBindingId,
    other_rendezvous_id: RelayRendezvousId,
    lease_id: LeaseId,
}

fn production_harness(credentials: FakeCredentialSource) -> ProductionHarness {
    let provider_id = operation(ProviderMethod::TransportConnect).provider_id;
    let binding_id = TransportBindingId::parse("transport-binding").expect("binding");
    let rendezvous_id = RelayRendezvousId::parse("relay-rendezvous").expect("rendezvous");
    let other_binding_id = TransportBindingId::parse("other-transport-binding").expect("binding");
    let other_rendezvous_id =
        RelayRendezvousId::parse("other-relay-rendezvous").expect("rendezvous");
    let lease_id = LeaseId::parse("credential-lease").expect("lease");
    let binding = AzureRelayBinding::new(
        provider_id,
        binding_id.clone(),
        rendezvous_id.clone(),
        NAMESPACE_CANARY,
        "hybrid-connection",
        None,
    )
    .expect("private binding");
    let other_binding = AzureRelayBinding::new(
        operation(ProviderMethod::TransportConnect).provider_id,
        other_binding_id.clone(),
        other_rendezvous_id.clone(),
        "other-url-canary.servicebus.windows.net",
        "other-hybrid-connection",
        None,
    )
    .expect("other private binding");
    let credentials = Arc::new(credentials);
    let connector = Arc::new(FakeSocketConnector::ready());
    let credential_port: Arc<dyn RelayCredentialSource> = credentials.clone();
    let socket_port: Arc<dyn RelaySocketConnector> = connector.clone();
    let port = Arc::new(
        ProductionRelayControlPort::with_socket_connector(
            [binding, other_binding],
            credential_port,
            socket_port,
        )
        .expect("production port"),
    );
    ProductionHarness {
        port,
        credentials,
        connector,
        binding_id,
        rendezvous_id,
        other_binding_id,
        other_rendezvous_id,
        lease_id,
    }
}

fn operation(method: ProviderMethod) -> OperationBinding {
    Fixture::new(ProviderType::Transport, 0)
        .expect("fixture")
        .request(method)
        .expect("operation request")
        .context
        .binding()
}

fn unique_operation(method: ProviderMethod, prefix: &str, sequence: usize) -> OperationBinding {
    let mut operation = operation(method);
    operation.operation_id =
        OperationId::parse(format!("{prefix}-{sequence:04}")).expect("unique operation");
    operation.request_digest =
        Fingerprint::parse(format!("{:064x}", sequence + 1_000)).expect("unique request digest");
    operation
}

fn open_request(
    harness: &ProductionHarness,
    operation: OperationBinding,
    deadline_remaining_ms: u32,
) -> RelayOpenRequest {
    RelayOpenRequest::new(
        operation,
        harness.binding_id.clone(),
        harness.rendezvous_id.clone(),
        harness.lease_id.clone(),
        deadline_remaining_ms,
        RelayTransportLimits::production(),
    )
}

fn other_open_request(
    harness: &ProductionHarness,
    operation: OperationBinding,
    deadline_remaining_ms: u32,
) -> RelayOpenRequest {
    RelayOpenRequest::new(
        operation,
        harness.other_binding_id.clone(),
        harness.other_rendezvous_id.clone(),
        harness.lease_id.clone(),
        deadline_remaining_ms,
        RelayTransportLimits::production(),
    )
}

fn inspect_request(
    harness: &ProductionHarness,
    operation: OperationBinding,
) -> RelayInspectRequest {
    RelayInspectRequest::new(
        operation,
        harness.binding_id.clone(),
        harness.rendezvous_id.clone(),
        5_000,
        RelayTransportLimits::production(),
    )
}

fn other_inspect_request(
    harness: &ProductionHarness,
    operation: OperationBinding,
) -> RelayInspectRequest {
    RelayInspectRequest::new(
        operation,
        harness.other_binding_id.clone(),
        harness.other_rendezvous_id.clone(),
        5_000,
        RelayTransportLimits::production(),
    )
}

fn close_request(harness: &ProductionHarness, operation: OperationBinding) -> RelayCloseRequest {
    close_request_with_deadline(harness, operation, 5_000)
}

fn close_request_with_deadline(
    harness: &ProductionHarness,
    operation: OperationBinding,
    deadline_remaining_ms: u32,
) -> RelayCloseRequest {
    RelayCloseRequest::new(
        operation,
        harness.binding_id.clone(),
        harness.rendezvous_id.clone(),
        deadline_remaining_ms,
        RelayTransportLimits::production(),
    )
}

fn now_unix_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_millis(),
    )
    .expect("current Unix time fits u64")
}

#[test]
fn production_constructor_has_exact_capabilities_bounds_and_redacted_debug() {
    let provider_id = operation(ProviderMethod::TransportConnect).provider_id;
    let binding_id = TransportBindingId::parse("token-canary-binding").expect("binding");
    let rendezvous_id = RelayRendezvousId::parse("secret-canary").expect("rendezvous");
    let binding = AzureRelayBinding::new(
        provider_id,
        binding_id,
        rendezvous_id,
        NAMESPACE_CANARY,
        "path-canary",
        Some(RelaySecret::new(b"certificate-canary".to_vec()).expect("CA bytes")),
    )
    .expect("binding");
    let credentials: Arc<dyn RelayCredentialSource> = Arc::new(FakeCredentialSource::available());
    let port = ProductionRelayControlPort::new([binding.clone()], credentials)
        .expect("real production connector constructor");

    assert_eq!(port.capabilities(), RelayPortCapabilities::production());
    let limits = RelayTransportLimits::production();
    assert_eq!(limits.accept_queue_capacity(), RELAY_ACCEPT_QUEUE_CAPACITY);
    assert_eq!(limits.max_frame_bytes(), RELAY_MAX_FRAME_BYTES);
    assert_eq!(limits.max_prologue_bytes(), RELAY_MAX_PROLOGUE_BYTES);
    assert_eq!(limits.sender_retry_limit(), RELAY_SENDER_RETRY_LIMIT);
    assert_eq!(limits.sender_retry_delay_ms(), RELAY_SENDER_RETRY_DELAY_MS);
    assert_eq!(
        limits.max_reconnect_backoff_ms(),
        RELAY_MAX_RECONNECT_BACKOFF_MS
    );
    assert_eq!(
        limits.reconnect_stable_reset_ms(),
        RELAY_RECONNECT_STABLE_RESET_MS
    );
    assert_eq!(
        limits.max_credential_ttl_secs(),
        RELAY_MAX_CREDENTIAL_TTL_SECS
    );

    let rendered = format!("{binding:?} {port:?}");
    for canary in [
        "token-canary-binding",
        "secret-canary",
        NAMESPACE_CANARY,
        "path-canary",
        "certificate-canary",
    ] {
        assert!(!rendered.contains(canary), "diagnostic leaked {canary}");
    }
}

#[test]
fn production_port_constructs_the_canonical_registry_factory_instance() {
    let harness = production_harness(FakeCredentialSource::available());
    let mut fixture = Fixture::new(ProviderType::Transport, 0).expect("fixture");
    fixture.descriptor.implementation_id = azure_relay_implementation_id();
    fixture.descriptor.capabilities = azure_relay_capabilities();
    let scope = fixture
        .operation(ProviderMethod::TransportInspect)
        .expect("scope")
        .scope;
    let configuration = AzureRelayConfiguration::new(
        scope,
        harness.binding_id.clone(),
        harness.rendezvous_id.clone(),
        LeaseId::parse("connect-credential-lease").expect("connect lease"),
        LeaseId::parse("listen-credential-lease").expect("listen lease"),
    )
    .expect("configuration");
    let port: Arc<dyn RelayControlPort> = harness.port;
    let factory = AzureRelayProviderFactory::new(
        port,
        [AzureRelayFactoryEntry::for_descriptor(
            &fixture.descriptor,
            configuration,
        )],
    )
    .expect("factory");
    let instance = factory
        .construct(&fixture.descriptor)
        .expect("canonical instance");
    assert!(matches!(instance, ProviderInstance::Transport(_)));
    assert_eq!(harness.credentials.calls().len(), 0);
    assert_eq!(harness.connector.call_count(), 0);
}

#[tokio::test]
async fn binding_and_credential_denials_stop_before_socket_io_without_fallback() {
    let harness = production_harness(FakeCredentialSource::available());
    let mut wrong_provider_operation = operation(ProviderMethod::TransportConnect);
    wrong_provider_operation.provider_id =
        ProviderId::parse("ddddddddddddddddddda").expect("different provider");
    assert_eq!(
        harness
            .port
            .connect(open_request(&harness, wrong_provider_operation, 5_000))
            .await
            .expect_err("binding ownership is provider-specific"),
        RelayPortFailure::BindingMismatch
    );
    assert!(harness.credentials.calls().is_empty());
    assert_eq!(harness.connector.call_count(), 0);

    let wrong_binding = RelayOpenRequest::new(
        operation(ProviderMethod::TransportConnect),
        TransportBindingId::parse("wrong-binding").expect("wrong binding"),
        harness.rendezvous_id.clone(),
        harness.lease_id.clone(),
        5_000,
        RelayTransportLimits::production(),
    );
    assert_eq!(
        harness
            .port
            .connect(wrong_binding)
            .await
            .expect_err("unknown opaque binding"),
        RelayPortFailure::BindingMismatch
    );
    assert!(harness.credentials.calls().is_empty());
    assert_eq!(harness.connector.call_count(), 0);

    let harness = production_harness(FakeCredentialSource::failing(
        RelayCredentialSourceFailure::LeaseUnknown,
    ));
    let request = open_request(&harness, operation(ProviderMethod::TransportConnect), 5_000);
    assert_eq!(
        harness
            .port
            .connect(request)
            .await
            .expect_err("opaque lease resolution must fail closed"),
        RelayPortFailure::CredentialLeaseInvalid
    );
    assert_eq!(
        harness.credentials.calls(),
        [CredentialCall {
            lease_id: "credential-lease".to_owned(),
            credential_use: RelayCredentialUse::Connect,
        }]
    );
    assert_eq!(harness.connector.call_count(), 0);
}

#[tokio::test]
async fn production_lifecycle_is_idempotent_and_adoption_checks_generations() {
    let harness = production_harness(FakeCredentialSource::available());
    let open_operation = operation(ProviderMethod::TransportConnect);
    let resource = harness
        .port
        .connect(open_request(&harness, open_operation.clone(), 5_000))
        .await
        .expect("connect");
    assert_eq!(
        resource.handle_id().as_str(),
        open_operation.operation_id.as_str()
    );
    assert_eq!(resource.state(), RelayResourceState::Connected);
    assert_eq!(resource.resource_generation(), Generation::new(1).unwrap());

    let replay = harness
        .port
        .connect(open_request(&harness, open_operation.clone(), 5_000))
        .await
        .expect("same operation replay");
    assert_eq!(replay, resource);
    assert_eq!(harness.credentials.calls().len(), 1);
    assert_eq!(harness.connector.call_count(), 1);

    assert_eq!(
        harness
            .port
            .listen(open_request(&harness, open_operation.clone(), 5_000))
            .await
            .expect_err("operation ID cannot change methods"),
        RelayPortFailure::IdentityMismatch
    );
    assert_eq!(harness.credentials.calls().len(), 1);
    assert_eq!(harness.connector.call_count(), 1);

    let mut conflicting_operation = open_operation.clone();
    conflicting_operation.request_digest =
        Fingerprint::parse(format!("{:064x}", 201)).expect("alternate digest");
    assert_eq!(
        harness
            .port
            .connect(open_request(&harness, conflicting_operation, 5_000))
            .await
            .expect_err("operation ID cannot be rebound"),
        RelayPortFailure::IdentityMismatch
    );
    assert_eq!(harness.connector.call_count(), 1);

    assert_eq!(
        harness
            .port
            .inspect(inspect_request(
                &harness,
                operation(ProviderMethod::TransportInspect)
            ))
            .await
            .expect("inspect"),
        RelayInspection::Present(resource.clone())
    );

    let expected = RelayExpectedResource::new(
        resource.provider_id().clone(),
        resource.handle_id().clone(),
        resource.provider_generation(),
        resource.resource_generation(),
    );
    let adopt = RelayAdoptRequest::new(
        operation(ProviderMethod::TransportInspect),
        harness.binding_id.clone(),
        harness.rendezvous_id.clone(),
        expected,
        5_000,
        RelayTransportLimits::production(),
    );
    assert_eq!(harness.port.adopt(adopt).await.expect("adopt"), resource);

    let wrong_generation = RelayExpectedResource::new(
        resource.provider_id().clone(),
        resource.handle_id().clone(),
        resource.provider_generation(),
        Generation::new(2).unwrap(),
    );
    let adopt = RelayAdoptRequest::new(
        operation(ProviderMethod::TransportInspect),
        harness.binding_id.clone(),
        harness.rendezvous_id.clone(),
        wrong_generation,
        5_000,
        RelayTransportLimits::production(),
    );
    assert_eq!(
        harness
            .port
            .adopt(adopt)
            .await
            .expect_err("wrong generation"),
        RelayPortFailure::GenerationMismatch
    );

    let close_operation = operation(ProviderMethod::TransportRevokeBinding);
    assert_eq!(
        harness
            .port
            .close(close_request(&harness, close_operation.clone()))
            .await
            .expect("close"),
        RelayCloseOutcome::Closed
    );
    assert_eq!(
        harness
            .port
            .close(close_request(&harness, close_operation))
            .await
            .expect("close replay"),
        RelayCloseOutcome::Closed
    );
    let mut second_close = operation(ProviderMethod::TransportRevokeBinding);
    second_close.operation_id = OperationId::parse("second-close-operation").expect("operation");
    assert_eq!(
        harness
            .port
            .close(close_request(&harness, second_close))
            .await
            .expect("independent idempotent close"),
        RelayCloseOutcome::AlreadyClosed
    );
    assert_eq!(
        harness
            .port
            .inspect(inspect_request(
                &harness,
                operation(ProviderMethod::TransportInspect)
            ))
            .await
            .expect("inspect after close"),
        RelayInspection::Absent
    );
}

#[tokio::test]
async fn credential_roles_are_exact_and_relay_authentication_grants_no_authority() {
    let harness = production_harness(FakeCredentialSource::available());
    harness
        .port
        .connect(open_request(
            &harness,
            operation(ProviderMethod::TransportConnect),
            5_000,
        ))
        .await
        .expect("sender");
    let mut listener_operation = operation(ProviderMethod::TransportListen);
    listener_operation.operation_id =
        OperationId::parse("listener-operation").expect("listener operation");
    listener_operation.request_digest =
        Fingerprint::parse(format!("{:064x}", 202)).expect("listener digest");
    harness
        .port
        .listen(open_request(&harness, listener_operation, 5_000))
        .await
        .expect("listener");

    assert_eq!(
        harness.credentials.calls(),
        [
            CredentialCall {
                lease_id: "credential-lease".to_owned(),
                credential_use: RelayCredentialUse::Connect,
            },
            CredentialCall {
                lease_id: "credential-lease".to_owned(),
                credential_use: RelayCredentialUse::Listen,
            },
        ]
    );
    assert_eq!(
        *harness
            .connector
            .roles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        [RelaySocketRole::Sender, RelaySocketRole::Listener]
    );
}

#[tokio::test]
async fn slow_connect_keeps_inspection_and_other_bindings_responsive() {
    let harness = production_harness(FakeCredentialSource::available());
    harness.connector.connect_gate.arm();
    let slow_operation = unique_operation(ProviderMethod::TransportConnect, "slow-connect", 0);
    let slow_request = open_request(&harness, slow_operation.clone(), 5_000);
    let slow_port = Arc::clone(&harness.port);
    let slow_connect = tokio::spawn(async move { slow_port.connect(slow_request).await });
    harness.connector.connect_gate.wait_started().await;
    assert_eq!(harness.port.test_state_counts().2, 1);
    let duplicate = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        harness
            .port
            .connect(open_request(&harness, slow_operation, 5_000)),
    )
    .await
    .expect("duplicate admission remained responsive")
    .expect_err("duplicate in-flight operation stays single-flight");
    assert_eq!(duplicate, RelayPortFailure::Unavailable);
    assert_eq!(harness.connector.call_count(), 1);

    let inspection = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        harness.port.inspect(inspect_request(
            &harness,
            unique_operation(
                ProviderMethod::TransportInspect,
                "inspect-during-connect",
                0,
            ),
        )),
    )
    .await
    .expect("inspection remained responsive")
    .expect("inspection");
    assert_eq!(inspection, RelayInspection::Absent);

    let other = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        harness.port.connect(other_open_request(
            &harness,
            unique_operation(ProviderMethod::TransportConnect, "other-connect", 0),
            5_000,
        )),
    )
    .await
    .expect("other binding remained responsive")
    .expect("other binding connect");
    assert_eq!(other.transport_binding_id(), &harness.other_binding_id);

    harness.connector.connect_gate.release();
    slow_connect
        .await
        .expect("slow task")
        .expect("slow connect");
    assert_eq!(harness.port.test_state_counts().2, 0);
}

#[tokio::test]
async fn slow_close_keeps_inspection_and_other_bindings_responsive() {
    let harness = production_harness(FakeCredentialSource::available());
    harness
        .port
        .connect(open_request(
            &harness,
            unique_operation(ProviderMethod::TransportConnect, "close-primary-open", 0),
            5_000,
        ))
        .await
        .expect("primary connect");
    harness
        .port
        .connect(other_open_request(
            &harness,
            unique_operation(ProviderMethod::TransportConnect, "close-other-open", 0),
            5_000,
        ))
        .await
        .expect("other connect");

    harness.connector.close_gate.arm();
    let close = close_request(
        &harness,
        unique_operation(ProviderMethod::TransportRevokeBinding, "slow-close", 0),
    );
    let close_port = Arc::clone(&harness.port);
    let slow_close = tokio::spawn(async move { close_port.close(close).await });
    harness.connector.close_gate.wait_started().await;
    assert_eq!(harness.port.test_state_counts().3, 1);

    let primary = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        harness.port.inspect(inspect_request(
            &harness,
            unique_operation(ProviderMethod::TransportInspect, "inspect-during-close", 0),
        )),
    )
    .await
    .expect("primary inspection remained responsive")
    .expect("primary inspection");
    assert!(matches!(
        primary,
        RelayInspection::Present(ref resource)
            if resource.state() == RelayResourceState::Connected
    ));

    let other = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        harness.port.inspect(other_inspect_request(
            &harness,
            unique_operation(ProviderMethod::TransportInspect, "inspect-other-close", 0),
        )),
    )
    .await
    .expect("other inspection remained responsive")
    .expect("other inspection");
    assert!(matches!(other, RelayInspection::Present(_)));

    harness.connector.close_gate.release();
    assert_eq!(
        slow_close
            .await
            .expect("slow close task")
            .expect("slow close"),
        RelayCloseOutcome::Closed
    );
    assert_eq!(harness.port.test_state_counts().3, 0);
}

#[tokio::test]
async fn cancelled_close_retries_shutdown_and_disables_external_socket_clones() {
    let harness = production_harness(FakeCredentialSource::available());
    let resource = harness
        .port
        .connect(open_request(
            &harness,
            unique_operation(ProviderMethod::TransportConnect, "cancel-close-open", 0),
            5_000,
        ))
        .await
        .expect("connect");
    let external_socket = harness
        .port
        .connected_socket(resource.handle_id())
        .await
        .expect("external socket clone");
    external_socket
        .send_binary(b"before-close")
        .await
        .expect("open socket accepts sends");

    harness.connector.close_gate.arm();
    let close_operation =
        unique_operation(ProviderMethod::TransportRevokeBinding, "cancel-close", 0);
    let close_request = close_request(&harness, close_operation);
    let close_port = Arc::clone(&harness.port);
    let request_to_cancel = close_request.clone();
    let cancelled = tokio::spawn(async move { close_port.close(request_to_cancel).await });
    harness.connector.close_gate.wait_started().await;
    assert!(external_socket.is_open());
    assert_eq!(
        external_socket
            .send_binary(b"during-close")
            .await
            .expect_err("closing socket rejects sends"),
        RelaySocketFailure::Unavailable
    );

    cancelled.abort();
    assert!(
        cancelled
            .await
            .expect_err("close task was cancelled")
            .is_cancelled()
    );
    assert_eq!(harness.port.test_state_counts().3, 0);
    assert!(external_socket.is_open());
    assert_eq!(
        external_socket
            .send_binary(b"after-cancel")
            .await
            .expect_err("cancelled shutdown remains non-writable"),
        RelaySocketFailure::Unavailable
    );

    let inspection = harness
        .port
        .inspect(inspect_request(
            &harness,
            unique_operation(ProviderMethod::TransportInspect, "cancel-close-inspect", 0),
        ))
        .await
        .expect("inspect after cancellation");
    assert!(matches!(
        inspection,
        RelayInspection::Present(ref current)
            if current.state() == RelayResourceState::Connected
    ));

    assert_eq!(
        harness
            .port
            .close(close_request)
            .await
            .expect("retry completes shutdown"),
        RelayCloseOutcome::Closed
    );
    assert!(!external_socket.is_open());
    assert_eq!(
        external_socket
            .send_binary(b"after-close")
            .await
            .expect_err("completed shutdown rejects sends"),
        RelaySocketFailure::Unavailable
    );
    assert!(
        harness
            .port
            .connected_socket(resource.handle_id())
            .await
            .is_none()
    );
    assert_eq!(
        harness
            .connector
            .sockets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)[0]
            .close_calls
            .load(Ordering::Acquire),
        2
    );
}

#[tokio::test]
async fn terminal_replays_evict_and_cleanup_continues_beyond_capacity() {
    let harness = production_harness(FakeCredentialSource::available());
    let operation_count = MAX_PROVIDER_REGISTRY_ENTRIES + 32;
    for sequence in 0..operation_count {
        harness
            .port
            .connect(open_request(
                &harness,
                unique_operation(ProviderMethod::TransportConnect, "evict-open", sequence),
                5_000,
            ))
            .await
            .expect("open beyond replay capacity");
        assert_eq!(
            harness
                .port
                .close(close_request(
                    &harness,
                    unique_operation(
                        ProviderMethod::TransportRevokeBinding,
                        "evict-close",
                        sequence,
                    ),
                ))
                .await
                .expect("cleanup beyond replay capacity"),
            RelayCloseOutcome::Closed
        );
    }

    let (open_replays, close_replays, open_in_flight, close_in_flight) =
        harness.port.test_state_counts();
    assert_eq!(open_replays, RELAY_OPEN_REPLAY_CAPACITY);
    assert_eq!(close_replays, RELAY_CLOSE_REPLAY_CAPACITY);
    assert_eq!(open_in_flight, 0);
    assert_eq!(close_in_flight, 0);
    assert_eq!(harness.connector.call_count(), operation_count);

    harness
        .port
        .connect(open_request(
            &harness,
            unique_operation(ProviderMethod::TransportConnect, "evict-open", 0),
            5_000,
        ))
        .await
        .expect("evicted operation can execute again");
    assert_eq!(harness.connector.call_count(), operation_count + 1);
    assert_eq!(
        harness
            .port
            .close(close_request(
                &harness,
                unique_operation(
                    ProviderMethod::TransportRevokeBinding,
                    "evict-final-cleanup",
                    0,
                ),
            ))
            .await
            .expect("reserved cleanup remains available"),
        RelayCloseOutcome::Closed
    );
}

#[tokio::test]
async fn retries_and_deadlines_are_bounded_and_drop_in_flight_socket_io() {
    let harness = production_harness(FakeCredentialSource::available());
    harness
        .connector
        .listener_not_ready
        .store(2, Ordering::Release);
    harness
        .port
        .connect(open_request(
            &harness,
            operation(ProviderMethod::TransportConnect),
            5_000,
        ))
        .await
        .expect("bounded sender retries");
    assert_eq!(harness.connector.call_count(), 3);

    let harness = production_harness(FakeCredentialSource::available());
    harness.connector.hang.store(true, Ordering::Release);
    let failure = harness
        .port
        .connect(open_request(
            &harness,
            operation(ProviderMethod::TransportConnect),
            5,
        ))
        .await
        .expect_err("deadline must cancel connector");
    assert_eq!(failure, RelayPortFailure::CompletionAmbiguous);
    assert_eq!(harness.connector.call_count(), 1);
    assert_eq!(
        harness.connector.dropped_connects.load(Ordering::Acquire),
        1
    );
    assert_eq!(harness.port.test_state_counts().2, 0);
    harness.connector.hang.store(false, Ordering::Release);
    harness
        .port
        .connect(open_request(
            &harness,
            operation(ProviderMethod::TransportConnect),
            5_000,
        ))
        .await
        .expect("cancelled reservation rolled back");
}

#[tokio::test]
async fn secrets_urls_frames_and_zero_deadlines_remain_closed_and_bounded() {
    let harness = production_harness(FakeCredentialSource::available());
    let resource = harness
        .port
        .connect(open_request(
            &harness,
            operation(ProviderMethod::TransportConnect),
            5_000,
        ))
        .await
        .expect("connect");
    let socket = harness
        .port
        .connected_socket(resource.handle_id())
        .await
        .expect("sender socket");
    let oversized =
        vec![0_u8; usize::try_from(RELAY_MAX_FRAME_BYTES).expect("frame bound fits usize") + 1];
    assert_eq!(
        socket
            .send_binary(&oversized)
            .await
            .expect_err("oversized frame"),
        RelaySocketFailure::FrameTooLarge
    );

    let request_debug = harness
        .connector
        .request_debug
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .join(" ");
    let rendered = format!(
        "{request_debug} {resource:?} {:?} {}",
        RelayCredentialMaterial::SasToken(
            RelaySecret::new(SECRET_CANARY.as_bytes().to_vec()).expect("secret")
        ),
        RelaySocketFailure::AuthenticationFailed
    );
    for canary in [SECRET_CANARY, NAMESPACE_CANARY, "wss://", "sb-hc-token"] {
        assert!(!rendered.contains(canary), "diagnostic leaked {canary}");
    }

    let zero_deadline = open_request(&harness, operation(ProviderMethod::TransportConnect), 0);
    let calls_before = harness.connector.call_count();
    assert_eq!(
        harness
            .port
            .connect(zero_deadline)
            .await
            .expect_err("zero deadline"),
        RelayPortFailure::HandshakeTimeout
    );
    assert_eq!(harness.connector.call_count(), calls_before);

    assert!(RelaySecret::new(vec![b'x'; 16 * 1024]).is_ok());
    assert!(RelaySecret::new(vec![b'x'; 16 * 1024 + 1]).is_err());
    let _bounded_handle = HandleId::parse("safe-handle").expect("handle");
}
