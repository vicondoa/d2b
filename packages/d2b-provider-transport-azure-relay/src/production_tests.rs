use std::{
    future::pending,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_identity::{ProviderId, ProviderType},
    v2_provider::{
        Fingerprint, Generation, HandleId, LeaseId, OperationBinding, OperationId, ProviderMethod,
        TransportBindingId,
    },
};
use d2b_provider::{ProviderFactory, ProviderInstance};
use d2b_provider_toolkit::Fixture;

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
    open: AtomicBool,
    close_calls: AtomicUsize,
}

impl FakeSocket {
    fn new() -> Self {
        Self {
            open: AtomicBool::new(true),
            close_calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl RelaySocket for FakeSocket {
    fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }

    async fn receive(&self) -> Result<RelaySocketEvent, RelaySocketFailure> {
        pending::<Result<RelaySocketEvent, RelaySocketFailure>>().await
    }

    async fn send_binary(&self, bytes: &[u8]) -> Result<(), RelaySocketFailure> {
        if bytes.len() > usize::try_from(RELAY_MAX_FRAME_BYTES).expect("frame bound fits usize") {
            return Err(RelaySocketFailure::FrameTooLarge);
        }
        Ok(())
    }

    async fn send_pong(&self, _bytes: &[u8]) -> Result<(), RelaySocketFailure> {
        Ok(())
    }

    async fn close(&self) -> Result<(), RelaySocketFailure> {
        self.close_calls.fetch_add(1, Ordering::AcqRel);
        self.open.store(false, Ordering::Release);
        Ok(())
    }
}

struct DropSignal(Arc<AtomicUsize>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::AcqRel);
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
}

impl FakeSocketConnector {
    fn ready() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            listener_not_ready: AtomicUsize::new(0),
            hang: AtomicBool::new(false),
            dropped_connects: Arc::new(AtomicUsize::new(0)),
            roles: Mutex::new(Vec::new()),
            request_debug: Mutex::new(Vec::new()),
            sockets: Mutex::new(Vec::new()),
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
        if self.take_listener_not_ready() {
            return Err(RelaySocketFailure::ListenerNotReady);
        }
        let socket = Arc::new(FakeSocket::new());
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
    lease_id: LeaseId,
}

fn production_harness(credentials: FakeCredentialSource) -> ProductionHarness {
    let provider_id = operation(ProviderMethod::TransportConnect).provider_id;
    let binding_id = TransportBindingId::parse("transport-binding").expect("binding");
    let rendezvous_id = RelayRendezvousId::parse("relay-rendezvous").expect("rendezvous");
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
    let credentials = Arc::new(credentials);
    let connector = Arc::new(FakeSocketConnector::ready());
    let credential_port: Arc<dyn RelayCredentialSource> = credentials.clone();
    let socket_port: Arc<dyn RelaySocketConnector> = connector.clone();
    let port = Arc::new(
        ProductionRelayControlPort::with_socket_connector([binding], credential_port, socket_port)
            .expect("production port"),
    );
    ProductionHarness {
        port,
        credentials,
        connector,
        binding_id,
        rendezvous_id,
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

fn close_request(harness: &ProductionHarness, operation: OperationBinding) -> RelayCloseRequest {
    RelayCloseRequest::new(
        operation,
        harness.binding_id.clone(),
        harness.rendezvous_id.clone(),
        5_000,
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
