use std::{
    future::pending,
    os::fd::OwnedFd,
    os::unix::net::UnixStream,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{EndpointRole, Locality, ServicePackage, TransportClass},
    v2_identity::{ProviderId, ProviderType, RealmId, WorkloadId},
    v2_provider::{
        AdoptionState, AuthorizedProviderScope, CorrelationId, Fingerprint, Generation,
        IdempotencyKey, ImplementationId, MutationState, ObservationReason, ObservedLifecycleState,
        OperationId, PROVIDER_SCHEMA_VERSION, PrincipalRef, ProviderApiVersion, ProviderAuthority,
        ProviderCallContext, ProviderCapability, ProviderCapabilitySet, ProviderDescriptor,
        ProviderFailure, ProviderFailureKind, ProviderHandle, ProviderHandleKind, ProviderMethod,
        ProviderObservation, ProviderOperationContext, ProviderOperationInput,
        ProviderOperationRequest, ProviderPlacement, ProviderTarget, TransportBindingId,
        TransportProvider,
    },
};
use tokio::sync::Notify;

use crate::{
    AttachmentCapability, AuthenticationOwner, BundleEndpointId, EndpointCloseRequest,
    EndpointCloseResult, EndpointCloseState, EndpointConnectRequest, EndpointConnection,
    EndpointInspectRequest, EndpointLeaseId, EndpointObservation, EndpointObservationState,
    EndpointPortError, EndpointProvenance, EndpointSource, LocalEndpointPort, LocalTransportClock,
    LocalTransportConfigurationError, LocalTransportKind, LocalTransportLimits,
    LocalTransportProvider, OwnedEndpointDescriptor, ReachabilityEvidence, TransportBinding,
    TransportCapabilityProfile, local_transport_capabilities,
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
    connect_identity: Option<Fingerprint>,
    connect_generation: Option<Generation>,
    inspect_error: Option<EndpointPortError>,
    inspect_identity: Option<Fingerprint>,
    inspect_generation: Option<Generation>,
    inspect_state: EndpointObservationState,
    close_error: Option<EndpointPortError>,
    close_identity: Option<Fingerprint>,
    close_generation: Option<Generation>,
    close_state: EndpointCloseState,
}

impl Default for FakeBehavior {
    fn default() -> Self {
        Self {
            pending_connect: false,
            connect_error: None,
            connect_identity: None,
            connect_generation: None,
            inspect_error: None,
            inspect_identity: None,
            inspect_generation: None,
            inspect_state: EndpointObservationState::Connected,
            close_error: None,
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
    connect_started: Notify,
    pending_connect_dropped: Arc<AtomicBool>,
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
        Ok(EndpointConnection {
            operation_id: request.operation_id,
            handle_id: request.handle_id,
            binding_id: request.binding_id,
            identity: behavior
                .connect_identity
                .unwrap_or(request.expected_identity),
            generation: behavior
                .connect_generation
                .unwrap_or(request.expected_generation),
            kind: request.kind,
            capabilities: request.capabilities,
            reachability: ReachabilityEvidence::ReachableOnly,
        })
    }

    async fn inspect(
        &self,
        request: EndpointInspectRequest,
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
    ) -> Result<EndpointCloseResult, EndpointPortError> {
        lock(&self.close_calls).push(request.clone());
        let behavior = lock(&self.behavior).clone();
        if let Some(error) = behavior.close_error {
            return Err(error);
        }
        Ok(EndpointCloseResult {
            operation_id: request.operation_id,
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
        implementation_id: ImplementationId::parse(kind.implementation_id())
            .expect("valid implementation id"),
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
        scope,
        endpoint_identity,
        endpoint_generation,
        EndpointSource::bundle(kind, BundleEndpointId::new(binding_id)),
    )
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

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
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
        let operation_id = format!("connect-{}", kind.implementation_id());
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
        MutationState::Applied
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
    );
    let duplicate = descriptor.duplicate().expect("close-on-exec duplicate");
    drop(duplicate);
    let owned = EndpointSource::owned(descriptor);
    assert_eq!(owned.provenance(), EndpointProvenance::OwnedDescriptor);
    assert!(!format!("{owned:?}").contains("descriptor"));
}
