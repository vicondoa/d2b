use std::{
    collections::VecDeque,
    future::pending,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use crate::{
    AZURE_RELAY_IMPLEMENTATION_ID, AzureRelayConfiguration, AzureRelayProviderBuildError,
    AzureRelayTransportProvider, RELAY_ACCEPT_QUEUE_CAPACITY, RELAY_MAX_CREDENTIAL_TTL_SECS,
    RELAY_MAX_FRAME_BYTES, RELAY_MAX_PROLOGUE_BYTES, RELAY_MAX_RECONNECT_BACKOFF_MS,
    RELAY_RECONNECT_STABLE_RESET_MS, RELAY_SENDER_RETRY_DELAY_MS, RELAY_SENDER_RETRY_LIMIT,
    RelayAdoptRequest, RelayCloseOutcome, RelayCloseRequest, RelayControlPort, RelayInspectRequest,
    RelayInspection, RelayOpenRequest, RelayPortFailure, RelayRendezvousId, RelayResource,
    RelayResourceState, RelayTransportLimits, azure_relay_capabilities,
};
use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::EndpointRole,
    v2_identity::{ProviderType, RealmId},
    v2_provider::{
        AdoptionState, AuthorizedProviderScope, Generation, HandleId, HandleOwner,
        ImplementationId, LeaseId, MutationState, ProviderCallContext, ProviderFailureKind,
        ProviderHealthReason, ProviderMethod, ProviderOperationInput, ProviderPlacement,
        ProviderTarget, TransportBindingId, TransportProvider,
    },
};
use d2b_provider::ProviderInstance;
use d2b_provider_toolkit::{DeterministicClock, Fixture, check_provider_conformance};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallKind {
    Connect,
    Listen,
    Inspect,
    Close,
    Adopt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedCall {
    kind: CallKind,
    operation_id: String,
    idempotency_key: String,
    request_digest: String,
    binding_id: String,
    rendezvous_id: String,
    credential_lease_id: Option<String>,
    deadline_remaining_ms: u32,
    limits: RelayTransportLimits,
    expected_handle_id: Option<String>,
    expected_provider_generation: Option<Generation>,
    expected_resource_generation: Option<Generation>,
}

impl RecordedCall {
    fn open(kind: CallKind, request: &RelayOpenRequest) -> Self {
        Self {
            kind,
            operation_id: request.operation().operation_id.as_str().to_owned(),
            idempotency_key: request.operation().idempotency_key.as_str().to_owned(),
            request_digest: request.operation().request_digest.as_str().to_owned(),
            binding_id: request.transport_binding_id().as_str().to_owned(),
            rendezvous_id: request.rendezvous_id().as_str().to_owned(),
            credential_lease_id: Some(request.credential_lease_id().as_str().to_owned()),
            deadline_remaining_ms: request.deadline_remaining_ms(),
            limits: request.limits(),
            expected_handle_id: None,
            expected_provider_generation: None,
            expected_resource_generation: None,
        }
    }

    fn inspect(request: &RelayInspectRequest) -> Self {
        Self {
            kind: CallKind::Inspect,
            operation_id: request.operation().operation_id.as_str().to_owned(),
            idempotency_key: request.operation().idempotency_key.as_str().to_owned(),
            request_digest: request.operation().request_digest.as_str().to_owned(),
            binding_id: request.transport_binding_id().as_str().to_owned(),
            rendezvous_id: request.rendezvous_id().as_str().to_owned(),
            credential_lease_id: None,
            deadline_remaining_ms: request.deadline_remaining_ms(),
            limits: request.limits(),
            expected_handle_id: None,
            expected_provider_generation: None,
            expected_resource_generation: None,
        }
    }

    fn close(request: &RelayCloseRequest) -> Self {
        Self {
            kind: CallKind::Close,
            operation_id: request.operation().operation_id.as_str().to_owned(),
            idempotency_key: request.operation().idempotency_key.as_str().to_owned(),
            request_digest: request.operation().request_digest.as_str().to_owned(),
            binding_id: request.transport_binding_id().as_str().to_owned(),
            rendezvous_id: request.rendezvous_id().as_str().to_owned(),
            credential_lease_id: None,
            deadline_remaining_ms: request.deadline_remaining_ms(),
            limits: request.limits(),
            expected_handle_id: None,
            expected_provider_generation: None,
            expected_resource_generation: None,
        }
    }

    fn adopt(request: &RelayAdoptRequest) -> Self {
        Self {
            kind: CallKind::Adopt,
            operation_id: request.operation().operation_id.as_str().to_owned(),
            idempotency_key: request.operation().idempotency_key.as_str().to_owned(),
            request_digest: request.operation().request_digest.as_str().to_owned(),
            binding_id: request.transport_binding_id().as_str().to_owned(),
            rendezvous_id: request.rendezvous_id().as_str().to_owned(),
            credential_lease_id: None,
            deadline_remaining_ms: request.deadline_remaining_ms(),
            limits: request.limits(),
            expected_handle_id: Some(request.expected().handle_id().as_str().to_owned()),
            expected_provider_generation: Some(request.expected().provider_generation()),
            expected_resource_generation: Some(request.expected().resource_generation()),
        }
    }
}

struct DropSignal(Arc<AtomicUsize>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct RecordingPort {
    calls: Mutex<Vec<RecordedCall>>,
    connect_result: Mutex<Result<RelayResource, RelayPortFailure>>,
    listen_result: Mutex<Result<RelayResource, RelayPortFailure>>,
    inspect_result: Mutex<Result<RelayInspection, RelayPortFailure>>,
    adopt_result: Mutex<Result<RelayResource, RelayPortFailure>>,
    close_results: Mutex<VecDeque<Result<RelayCloseOutcome, RelayPortFailure>>>,
    hang_connect: AtomicBool,
    dropped_connects: Arc<AtomicUsize>,
    _secret_canary: String,
    _token_canary: String,
    _url_canary: String,
}

impl RecordingPort {
    fn new(resource: RelayResource) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            connect_result: Mutex::new(Ok(resource.clone())),
            listen_result: Mutex::new(Ok(RelayResource::new(
                resource.provider_id().clone(),
                resource.transport_binding_id().clone(),
                resource.rendezvous_id().clone(),
                resource.handle_id().clone(),
                resource.provider_generation(),
                resource.resource_generation(),
                RelayResourceState::Listening,
                resource.expires_at_unix_ms(),
            ))),
            inspect_result: Mutex::new(Ok(RelayInspection::Present(resource.clone()))),
            adopt_result: Mutex::new(Ok(resource)),
            close_results: Mutex::new(VecDeque::new()),
            hang_connect: AtomicBool::new(false),
            dropped_connects: Arc::new(AtomicUsize::new(0)),
            _secret_canary: "secret-material-canary".to_owned(),
            _token_canary: "token-material-canary".to_owned(),
            _url_canary: "wss://relay.invalid/path?token=url-canary".to_owned(),
        }
    }

    fn calls(&self) -> Vec<RecordedCall> {
        self.calls.lock().expect("calls lock").clone()
    }

    fn call_count(&self) -> usize {
        self.calls.lock().expect("calls lock").len()
    }

    fn set_connect_result(&self, result: Result<RelayResource, RelayPortFailure>) {
        *self.connect_result.lock().expect("connect result lock") = result;
    }

    fn set_adopt_result(&self, result: Result<RelayResource, RelayPortFailure>) {
        *self.adopt_result.lock().expect("adopt result lock") = result;
    }

    fn push_close_result(&self, result: Result<RelayCloseOutcome, RelayPortFailure>) {
        self.close_results
            .lock()
            .expect("close result lock")
            .push_back(result);
    }
}

#[async_trait]
impl RelayControlPort for RecordingPort {
    async fn connect(&self, request: RelayOpenRequest) -> Result<RelayResource, RelayPortFailure> {
        self.calls
            .lock()
            .expect("calls lock")
            .push(RecordedCall::open(CallKind::Connect, &request));
        if self.hang_connect.load(Ordering::SeqCst) {
            let _drop_signal = DropSignal(self.dropped_connects.clone());
            pending::<()>().await;
            unreachable!("pending connect cannot complete");
        }
        self.connect_result
            .lock()
            .expect("connect result lock")
            .clone()
    }

    async fn listen(&self, request: RelayOpenRequest) -> Result<RelayResource, RelayPortFailure> {
        self.calls
            .lock()
            .expect("calls lock")
            .push(RecordedCall::open(CallKind::Listen, &request));
        self.listen_result
            .lock()
            .expect("listen result lock")
            .clone()
    }

    async fn inspect(
        &self,
        request: RelayInspectRequest,
    ) -> Result<RelayInspection, RelayPortFailure> {
        self.calls
            .lock()
            .expect("calls lock")
            .push(RecordedCall::inspect(&request));
        self.inspect_result
            .lock()
            .expect("inspect result lock")
            .clone()
    }

    async fn close(
        &self,
        request: RelayCloseRequest,
    ) -> Result<RelayCloseOutcome, RelayPortFailure> {
        self.calls
            .lock()
            .expect("calls lock")
            .push(RecordedCall::close(&request));
        self.close_results
            .lock()
            .expect("close result lock")
            .pop_front()
            .unwrap_or(Ok(RelayCloseOutcome::Closed))
    }

    async fn adopt(&self, request: RelayAdoptRequest) -> Result<RelayResource, RelayPortFailure> {
        self.calls
            .lock()
            .expect("calls lock")
            .push(RecordedCall::adopt(&request));
        self.adopt_result.lock().expect("adopt result lock").clone()
    }
}

struct Harness {
    fixture: Fixture,
    provider: Arc<AzureRelayTransportProvider>,
    port: Arc<RecordingPort>,
    resource: RelayResource,
}

fn harness() -> Harness {
    let mut fixture = Fixture::new(ProviderType::Transport, 0).expect("transport fixture");
    fixture.descriptor.implementation_id =
        ImplementationId::parse(AZURE_RELAY_IMPLEMENTATION_ID).expect("implementation id");
    fixture.descriptor.capabilities = azure_relay_capabilities();
    let scope = fixture
        .operation(ProviderMethod::TransportInspect)
        .expect("inspect operation")
        .scope;
    let binding_id = TransportBindingId::parse("transport-binding").expect("binding id");
    let rendezvous_id = RelayRendezvousId::parse("relay-rendezvous").expect("rendezvous id");
    let configuration = AzureRelayConfiguration::new(
        scope,
        binding_id.clone(),
        rendezvous_id.clone(),
        LeaseId::parse("relay-connect-lease").expect("connect lease"),
        LeaseId::parse("relay-listen-lease").expect("listen lease"),
    )
    .expect("configuration");
    let resource = RelayResource::new(
        fixture.descriptor.provider_id.clone(),
        binding_id,
        rendezvous_id,
        HandleId::parse("relay-handle").expect("handle id"),
        fixture.descriptor.registry_generation,
        Generation::new(7).expect("resource generation"),
        RelayResourceState::Connected,
        Some(fixture.now_unix_ms + 30_000),
    );
    let port = Arc::new(RecordingPort::new(resource.clone()));
    let provider = Arc::new(
        AzureRelayTransportProvider::with_clock(
            fixture.descriptor.clone(),
            configuration,
            port.clone(),
            Arc::new(DeterministicClock::new(fixture.now_unix_ms)),
        )
        .expect("provider"),
    );
    Harness {
        fixture,
        provider,
        port,
        resource,
    }
}

fn call_context<'a>(
    fixture: &Fixture,
    request: &'a d2b_contracts::v2_provider::ProviderOperationRequest,
) -> ProviderCallContext<'a> {
    fixture.call_context(&request.context)
}

#[tokio::test]
async fn exact_capabilities_and_toolkit_conformance_match_live_behavior() {
    let harness = harness();
    let capabilities = harness.provider.capabilities();
    assert_eq!(capabilities, azure_relay_capabilities());
    assert!(capabilities.contains_method(ProviderMethod::TransportConnect));
    assert!(capabilities.contains_method(ProviderMethod::TransportListen));
    assert!(capabilities.contains_method(ProviderMethod::TransportRevokeBinding));
    assert!(capabilities.contains_method(ProviderMethod::TransportInspect));
    assert!(!capabilities.contains_method(ProviderMethod::TransportIssueBinding));

    let instance = ProviderInstance::Transport(harness.provider);
    check_provider_conformance(&instance, &harness.fixture)
        .await
        .expect("canonical provider conformance");
}

#[tokio::test]
async fn connect_and_listen_preserve_operation_id_idempotency_and_bounds() {
    let harness = harness();
    let connect = harness
        .fixture
        .request(ProviderMethod::TransportConnect)
        .expect("connect request");
    assert!(matches!(connect.input, ProviderOperationInput::NoInput));
    let context = call_context(&harness.fixture, &connect);
    let handle = harness
        .provider
        .connect(&context, &connect)
        .await
        .expect("connect");
    assert_eq!(handle.created_by.operation_id, connect.context.operation_id);
    assert_eq!(
        handle.created_by.idempotency_key,
        connect.context.idempotency_key
    );
    assert_eq!(
        handle.created_by.request_digest,
        connect.context.request_digest
    );
    assert_eq!(handle.handle_id, *harness.resource.handle_id());
    assert_eq!(handle.resource_generation, Generation::new(7).unwrap());

    let listen = harness
        .fixture
        .request(ProviderMethod::TransportListen)
        .expect("listen request");
    let context = call_context(&harness.fixture, &listen);
    harness
        .provider
        .listen(&context, &listen)
        .await
        .expect("listen");

    let calls = harness.port.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].kind, CallKind::Connect);
    assert_eq!(calls[1].kind, CallKind::Listen);
    assert_eq!(calls[0].operation_id, "operation-fixture");
    assert_eq!(calls[0].idempotency_key, "idempotency-fixture");
    assert_eq!(
        calls[0].request_digest,
        connect.context.request_digest.as_str()
    );
    assert_eq!(
        calls[0].credential_lease_id.as_deref(),
        Some("relay-connect-lease")
    );
    assert_eq!(
        calls[1].credential_lease_id.as_deref(),
        Some("relay-listen-lease")
    );
    for call in calls {
        assert_eq!(
            call.limits.accept_queue_capacity(),
            RELAY_ACCEPT_QUEUE_CAPACITY
        );
        assert_eq!(call.limits.max_frame_bytes(), RELAY_MAX_FRAME_BYTES);
        assert_eq!(call.limits.max_prologue_bytes(), RELAY_MAX_PROLOGUE_BYTES);
        assert_eq!(call.limits.sender_retry_limit(), RELAY_SENDER_RETRY_LIMIT);
        assert_eq!(
            call.limits.sender_retry_delay_ms(),
            RELAY_SENDER_RETRY_DELAY_MS
        );
        assert_eq!(
            call.limits.max_reconnect_backoff_ms(),
            RELAY_MAX_RECONNECT_BACKOFF_MS
        );
        assert_eq!(
            call.limits.reconnect_stable_reset_ms(),
            RELAY_RECONNECT_STABLE_RESET_MS
        );
        assert_eq!(
            call.limits.max_credential_ttl_secs(),
            RELAY_MAX_CREDENTIAL_TTL_SECS
        );
        assert!(call.deadline_remaining_ms > 0);
        assert!(call.deadline_remaining_ms <= 30_000);
    }
}

#[tokio::test]
async fn all_denials_happen_before_the_relay_control_port() {
    let harness = harness();

    let inspect = harness
        .fixture
        .request(ProviderMethod::TransportInspect)
        .expect("inspect request");
    let context = call_context(&harness.fixture, &inspect);
    let error = harness
        .provider
        .connect(&context, &inspect)
        .await
        .expect_err("wrong method must fail");
    assert_eq!(error.kind, ProviderFailureKind::CapabilityDenied);
    assert_eq!(harness.port.call_count(), 0);

    let issue = harness
        .fixture
        .request(ProviderMethod::TransportIssueBinding)
        .expect("issue request");
    let context = call_context(&harness.fixture, &issue);
    let error = harness
        .provider
        .issue_binding(&context, &issue)
        .await
        .expect_err("binding issuance is unsupported");
    assert_eq!(error.kind, ProviderFailureKind::CapabilityDenied);
    assert_eq!(harness.port.call_count(), 0);

    let mut close = harness
        .fixture
        .request(ProviderMethod::TransportRevokeBinding)
        .expect("close request");
    close.input = ProviderOperationInput::TransportBinding {
        transport_binding_id: TransportBindingId::parse("other-binding").unwrap(),
    };
    let context = call_context(&harness.fixture, &close);
    let error = harness
        .provider
        .revoke_binding(&context, &close)
        .await
        .expect_err("wrong binding must fail");
    assert_eq!(error.kind, ProviderFailureKind::UnauthorizedScope);
    assert_eq!(harness.port.call_count(), 0);

    let mut connect = harness
        .fixture
        .request(ProviderMethod::TransportConnect)
        .expect("connect request");
    connect.context.scope = AuthorizedProviderScope::Workload {
        realm_id: RealmId::parse("bbbbbbbbbbbbbbbbbbba").unwrap(),
        workload_id: connect
            .context
            .scope
            .workload_id()
            .expect("workload scope")
            .clone(),
    };
    let context = call_context(&harness.fixture, &connect);
    let error = harness
        .provider
        .connect(&context, &connect)
        .await
        .expect_err("wrong authorization scope must fail");
    assert_eq!(error.kind, ProviderFailureKind::UnauthorizedScope);
    assert_eq!(harness.port.call_count(), 0);
}

#[tokio::test]
async fn relay_authentication_never_substitutes_for_d2b_authorization() {
    let harness = harness();
    let mut request = harness
        .fixture
        .request(ProviderMethod::TransportConnect)
        .expect("connect request");
    request.context.scope = AuthorizedProviderScope::Workload {
        realm_id: RealmId::parse("bbbbbbbbbbbbbbbbbbba").unwrap(),
        workload_id: request
            .context
            .scope
            .workload_id()
            .expect("workload")
            .clone(),
    };
    let context = call_context(&harness.fixture, &request);

    let error = harness
        .provider
        .connect(&context, &request)
        .await
        .expect_err("an authn-capable port cannot authorize a denied d2b scope");
    assert_eq!(error.kind, ProviderFailureKind::UnauthorizedScope);
    assert_eq!(harness.port.call_count(), 0);
}

#[tokio::test]
async fn close_is_idempotent_and_preserves_the_operation_binding() {
    let harness = harness();
    harness
        .port
        .push_close_result(Ok(RelayCloseOutcome::Closed));
    harness
        .port
        .push_close_result(Ok(RelayCloseOutcome::AlreadyClosed));
    harness
        .port
        .push_close_result(Ok(RelayCloseOutcome::NotFound));

    let request = harness
        .fixture
        .request(ProviderMethod::TransportRevokeBinding)
        .expect("close request");
    let context = call_context(&harness.fixture, &request);
    let applied = harness
        .provider
        .revoke_binding(&context, &request)
        .await
        .expect("first close");
    let replay = harness
        .provider
        .revoke_binding(&context, &request)
        .await
        .expect("idempotent replay");
    let absent = harness
        .provider
        .revoke_binding(&context, &request)
        .await
        .expect("already absent");

    assert_eq!(applied.state, MutationState::Applied);
    assert_eq!(replay.state, MutationState::AlreadyApplied);
    assert_eq!(absent.state, MutationState::NotApplicable);
    for receipt in [&applied, &replay, &absent] {
        assert_eq!(receipt.binding.operation_id, request.context.operation_id);
        assert_eq!(
            receipt.binding.idempotency_key,
            request.context.idempotency_key
        );
    }
    assert_eq!(
        harness
            .port
            .calls()
            .into_iter()
            .filter(|call| call.kind == CallKind::Close)
            .count(),
        3
    );
}

#[tokio::test]
async fn cancellation_and_deadlines_fail_closed_and_cancel_in_flight_io() {
    let harness = harness();
    let request = harness
        .fixture
        .request(ProviderMethod::TransportConnect)
        .expect("connect request");

    let mut cancelled = call_context(&harness.fixture, &request);
    cancelled.cancelled = true;
    let error = harness
        .provider
        .connect(&cancelled, &request)
        .await
        .expect_err("cancelled call");
    assert_eq!(error.kind, ProviderFailureKind::Cancelled);
    assert_eq!(harness.port.call_count(), 0);

    let mut expired = call_context(&harness.fixture, &request);
    expired.monotonic_deadline_remaining_ms = 0;
    let error = harness
        .provider
        .connect(&expired, &request)
        .await
        .expect_err("expired call");
    assert_eq!(error.kind, ProviderFailureKind::DeadlineExpired);
    assert_eq!(harness.port.call_count(), 0);

    harness.port.hang_connect.store(true, Ordering::SeqCst);
    let mut bounded = call_context(&harness.fixture, &request);
    bounded.monotonic_deadline_remaining_ms = 5;
    let error = harness
        .provider
        .connect(&bounded, &request)
        .await
        .expect_err("hanging mutation must time out");
    assert_eq!(error.kind, ProviderFailureKind::AmbiguousMutation);
    assert_eq!(harness.port.call_count(), 1);
    assert_eq!(harness.port.dropped_connects.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn queue_frame_and_credential_failures_are_closed_and_have_no_fallback() {
    let harness = harness();
    let request = harness
        .fixture
        .request(ProviderMethod::TransportConnect)
        .expect("connect request");
    let context = call_context(&harness.fixture, &request);

    harness
        .port
        .set_connect_result(Err(RelayPortFailure::QueueFull));
    let error = harness
        .provider
        .connect(&context, &request)
        .await
        .expect_err("queue pressure");
    assert_eq!(error.kind, ProviderFailureKind::Unavailable);
    assert_eq!(error.reason, ProviderHealthReason::QueuePressure);

    harness
        .port
        .set_connect_result(Err(RelayPortFailure::FrameTooLarge));
    let error = harness
        .provider
        .connect(&context, &request)
        .await
        .expect_err("oversize frame");
    assert_eq!(error.kind, ProviderFailureKind::InvalidRequest);

    harness
        .port
        .set_connect_result(Err(RelayPortFailure::CredentialLeaseInvalid));
    let before = harness.port.call_count();
    let error = harness
        .provider
        .connect(&context, &request)
        .await
        .expect_err("invalid opaque lease must not fall back to ambient credentials");
    assert_eq!(error.kind, ProviderFailureKind::CredentialLeaseInvalid);
    assert_eq!(harness.port.call_count(), before + 1);
}

#[tokio::test]
async fn secret_token_and_url_canaries_never_reach_debug_errors_or_results() {
    let harness = harness();
    let request = harness
        .fixture
        .request(ProviderMethod::TransportConnect)
        .expect("connect request");
    let context = call_context(&harness.fixture, &request);

    harness
        .port
        .set_connect_result(Err(RelayPortFailure::AuthenticationFailed));
    let error = harness
        .provider
        .connect(&context, &request)
        .await
        .expect_err("authentication failure");
    let rendered = format!(
        "{:?} {:?} {:?} {}",
        harness.provider,
        harness.provider.configuration(),
        error,
        RelayPortFailure::AuthenticationFailed
    );
    for canary in [
        "secret-material-canary",
        "token-material-canary",
        "wss://relay.invalid/path?token=url-canary",
    ] {
        assert!(
            !rendered.contains(canary),
            "diagnostic leaked canary {canary}: {rendered}"
        );
    }

    harness
        .port
        .set_connect_result(Ok(harness.resource.clone()));
    let handle = harness
        .provider
        .connect(&context, &request)
        .await
        .expect("safe bounded handle");
    let rendered = format!("{handle:?}");
    assert!(!rendered.contains("wss://"));
    assert!(!rendered.contains("token-material-canary"));
    assert!(!rendered.contains("secret-material-canary"));

    let tainted_ids = RelayResource::new(
        harness.resource.provider_id().clone(),
        TransportBindingId::parse("token-material-canary").unwrap(),
        RelayRendezvousId::parse("url-canary").unwrap(),
        HandleId::parse("secret-material-canary").unwrap(),
        harness.resource.provider_generation(),
        harness.resource.resource_generation(),
        RelayResourceState::Connected,
        None,
    );
    let rendered = format!("{tainted_ids:?}");
    assert!(!rendered.contains("token-material-canary"));
    assert!(!rendered.contains("url-canary"));
    assert!(!rendered.contains("secret-material-canary"));
    assert!(RelayRendezvousId::parse("wss://relay.invalid/path?token=canary").is_err());
}

#[tokio::test]
async fn adopt_verifies_handle_identity_and_both_generations() {
    let harness = harness();
    let connect = harness
        .fixture
        .request(ProviderMethod::TransportConnect)
        .expect("connect request");
    let connect_context = call_context(&harness.fixture, &connect);
    let handle = harness
        .provider
        .connect(&connect_context, &connect)
        .await
        .expect("connect");

    let inspect = harness
        .fixture
        .request(ProviderMethod::TransportInspect)
        .expect("inspect request");
    let inspect_context = call_context(&harness.fixture, &inspect);
    let adopted = harness
        .provider
        .adopt(&inspect_context, &handle)
        .await
        .expect("exact adoption");
    assert_eq!(adopted.adoption, AdoptionState::Adopted);
    assert_eq!(adopted.handle_id.as_ref(), Some(&handle.handle_id));
    let adopt_call = harness
        .port
        .calls()
        .into_iter()
        .find(|call| call.kind == CallKind::Adopt)
        .expect("adopt call");
    assert_eq!(
        adopt_call.expected_handle_id.as_deref(),
        Some(handle.handle_id.as_str())
    );
    assert_eq!(
        adopt_call.expected_provider_generation,
        Some(handle.provider_generation)
    );
    assert_eq!(
        adopt_call.expected_resource_generation,
        Some(handle.resource_generation)
    );

    harness.port.set_adopt_result(Ok(RelayResource::new(
        harness.resource.provider_id().clone(),
        harness.resource.transport_binding_id().clone(),
        harness.resource.rendezvous_id().clone(),
        harness.resource.handle_id().clone(),
        harness.resource.provider_generation(),
        Generation::new(8).unwrap(),
        RelayResourceState::Connected,
        harness.resource.expires_at_unix_ms(),
    )));
    let error = harness
        .provider
        .adopt(&inspect_context, &handle)
        .await
        .expect_err("generation mismatch must reject adoption");
    assert_eq!(error.kind, ProviderFailureKind::AdoptionRejected);
    assert_eq!(error.reason, ProviderHealthReason::GenerationMismatch);

    let before = harness.port.call_count();
    let mut wrong_owner = handle;
    wrong_owner.owner = HandleOwner::RealmController {
        realm_id: wrong_owner.realm_id.clone(),
    };
    let error = harness
        .provider
        .adopt(&inspect_context, &wrong_owner)
        .await
        .expect_err("owner mismatch must fail before I/O");
    assert_eq!(error.kind, ProviderFailureKind::AdoptionRejected);
    assert_eq!(harness.port.call_count(), before);
}

#[tokio::test]
async fn canonical_handle_inspection_uses_the_verified_adoption_seam() {
    let harness = harness();
    let mut request = harness
        .fixture
        .request(ProviderMethod::TransportInspect)
        .expect("inspect request");
    request.target = ProviderTarget::Handle {
        realm_id: request.context.scope.realm_id().clone(),
        workload_id: request.context.scope.workload_id().cloned(),
        handle_id: harness.resource.handle_id().clone(),
        handle_generation: harness.resource.resource_generation(),
    };
    let context = call_context(&harness.fixture, &request);
    let observation = harness
        .provider
        .inspect(&context, &request)
        .await
        .expect("handle inspection adopts exact resource");
    assert_eq!(observation.adoption, AdoptionState::Adopted);
    assert_eq!(
        harness.port.calls().last().map(|call| call.kind),
        Some(CallKind::Adopt)
    );
}

#[test]
fn build_rejects_non_agent_placement_and_overlapping_credential_roles() {
    let mut fixture = Fixture::new(ProviderType::Transport, 0).expect("fixture");
    fixture.descriptor.implementation_id =
        ImplementationId::parse(AZURE_RELAY_IMPLEMENTATION_ID).unwrap();
    fixture.descriptor.capabilities = azure_relay_capabilities();
    let scope = fixture
        .operation(ProviderMethod::TransportInspect)
        .unwrap()
        .scope;
    let lease = LeaseId::parse("same-lease").unwrap();
    assert!(
        AzureRelayConfiguration::new(
            scope,
            TransportBindingId::parse("transport-binding").unwrap(),
            RelayRendezvousId::parse("relay-rendezvous").unwrap(),
            lease.clone(),
            lease,
        )
        .is_err()
    );

    let harness = harness();
    let mut descriptor = harness.fixture.descriptor.clone();
    descriptor.placement = ProviderPlacement::TrustedFirstPartyInProcess {
        realm_id: harness.provider.configuration().scope().realm_id().clone(),
        controller_role: EndpointRole::RealmController,
    };
    let error = AzureRelayTransportProvider::new(
        descriptor,
        harness.provider.configuration().clone(),
        harness.port,
    )
    .expect_err("Relay credentials may not move into a controller");
    assert_eq!(error, AzureRelayProviderBuildError::WrongPlacement);
}
