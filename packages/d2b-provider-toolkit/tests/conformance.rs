use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{EndpointRole, ServicePackage},
    v2_identity::{ProviderId, ProviderType},
    v2_provider::{
        AdoptionRequest, AdoptionState, Fingerprint, Generation, PrincipalRef, ProviderFailureKind,
        ProviderMethod, RuntimeProvider,
    },
    v2_services::{
        StrictWireMessage, common, decode_strict, encode_strict, provider_credential_ttrpc,
        provider_runtime_ttrpc,
    },
};
use d2b_provider::{
    CancellationToken, ProviderInstance, ProviderRegistryBuilder, RpcProviderProxy, SessionIdentity,
};
use d2b_provider_toolkit::{
    AuthenticatedSessionState, ClosedProviderMethod, ComponentSessionDriver, DeterministicClock,
    FakeProvider, Fixture, GeneratedProviderServiceServer, OwnedAttachment, ProviderAgentAdapter,
    Redacted, Secret, SessionDriverError, TransportPacket, check_provider_conformance,
    register_exact_instances, sample_lease_request,
};
use protobuf::{EnumOrUnknown, MessageField};

fn proxy_instance(provider_type: ProviderType, proxy: Arc<RpcProviderProxy>) -> ProviderInstance {
    match provider_type {
        ProviderType::Runtime => ProviderInstance::Runtime(proxy),
        ProviderType::Infrastructure => ProviderInstance::Infrastructure(proxy),
        ProviderType::Transport => ProviderInstance::Transport(proxy),
        ProviderType::Substrate => ProviderInstance::Substrate(proxy),
        ProviderType::Credential => ProviderInstance::Credential(proxy),
        ProviderType::Display => ProviderInstance::Display(proxy),
        ProviderType::Network => ProviderInstance::Network(proxy),
        ProviderType::Storage => ProviderInstance::Storage(proxy),
        ProviderType::Device => ProviderInstance::Device(proxy),
        ProviderType::Audio => ProviderInstance::Audio(proxy),
        ProviderType::Observability => ProviderInstance::Observability(proxy),
    }
}

#[tokio::test]
async fn every_axis_passes_identical_in_process_and_rpc_conformance() {
    for (ordinal, provider_type) in ProviderType::ALL.into_iter().enumerate() {
        let fixture = Fixture::new(provider_type, ordinal).unwrap_or_else(|_| unreachable!());
        let clock = Arc::new(DeterministicClock::new(fixture.now_unix_ms));
        let in_process = Arc::new(FakeProvider::new(fixture.clone())).instance();
        check_provider_conformance(&in_process, &fixture)
            .await
            .unwrap_or_else(|_| unreachable!());

        let adapter = Arc::new(
            ProviderAgentAdapter::new(in_process, fixture.session_identity(), clock.clone())
                .unwrap_or_else(|_| unreachable!()),
        );
        let proxy = Arc::new(
            RpcProviderProxy::new(fixture.descriptor.clone(), adapter, clock)
                .unwrap_or_else(|_| unreachable!()),
        );
        check_provider_conformance(&proxy_instance(provider_type, proxy), &fixture)
            .await
            .unwrap_or_else(|_| unreachable!());
    }
}

#[test]
fn exact_registration_supports_all_axes_and_shared_factories() {
    let mut instances = Vec::new();
    for (ordinal, provider_type) in ProviderType::ALL.into_iter().enumerate() {
        let fixture = Fixture::new(provider_type, ordinal).unwrap_or_else(|_| unreachable!());
        instances.push(Arc::new(FakeProvider::new(fixture)).instance());
    }
    let second_runtime = Fixture::new(ProviderType::Runtime, 20)
        .map(FakeProvider::new)
        .map(Arc::new)
        .map(FakeProvider::instance)
        .unwrap_or_else(|_| unreachable!());
    instances.push(second_runtime);

    let mut builder = ProviderRegistryBuilder::new(
        Generation::new(1).unwrap_or_else(|_| unreachable!()),
        Fingerprint::parse(format!("{:064x}", 900)).unwrap_or_else(|_| unreachable!()),
        1_700_000_000_000,
    );
    register_exact_instances(&mut builder, instances).unwrap_or_else(|_| unreachable!());
    let registry = builder.finish().unwrap_or_else(|_| unreachable!());
    let snapshot = registry.snapshot();
    assert_eq!(snapshot.axes.len(), 11);
    assert_eq!(snapshot.providers.len(), 12);
    assert_eq!(snapshot.factories.len(), 11);
}

#[test]
fn adapter_rejects_authenticated_identity_mismatch() {
    let fixture = Fixture::new(ProviderType::Runtime, 0).unwrap_or_else(|_| unreachable!());
    let instance = Arc::new(FakeProvider::new(fixture.clone())).instance();
    let mut identity: SessionIdentity = fixture.session_identity();
    identity.provider_id =
        ProviderId::parse("zzzzzzzzzzzzzzzzzzza").unwrap_or_else(|_| unreachable!());
    assert!(
        ProviderAgentAdapter::new(
            instance,
            identity,
            Arc::new(DeterministicClock::new(fixture.now_unix_ms)),
        )
        .is_err()
    );
}

#[tokio::test]
async fn rpc_proxy_fails_closed_on_cancellation_and_method_mismatch() {
    let fixture = Fixture::new(ProviderType::Runtime, 0).unwrap_or_else(|_| unreachable!());
    let clock = Arc::new(DeterministicClock::new(fixture.now_unix_ms));
    let instance = Arc::new(FakeProvider::new(fixture.clone())).instance();
    let adapter = Arc::new(
        ProviderAgentAdapter::new(instance, fixture.session_identity(), clock.clone())
            .unwrap_or_else(|_| unreachable!()),
    );
    let proxy = RpcProviderProxy::new(fixture.descriptor.clone(), adapter, clock)
        .unwrap_or_else(|_| unreachable!());
    let request = fixture
        .request(ProviderMethod::RuntimeInspect)
        .unwrap_or_else(|_| unreachable!());
    let mut cancelled = fixture.call_context(&request.context);
    cancelled.cancelled = true;
    let failure = proxy
        .inspect(&cancelled, &request)
        .await
        .expect_err("cancelled calls fail closed");
    assert_eq!(failure.kind, ProviderFailureKind::Cancelled);

    let wrong_operation = fixture
        .operation(ProviderMethod::RuntimeStart)
        .unwrap_or_else(|_| unreachable!());
    let wrong_context = fixture.call_context(&wrong_operation);
    let failure = proxy
        .inspect(&wrong_context, &request)
        .await
        .expect_err("method authority cannot be widened");
    assert_eq!(failure.kind, ProviderFailureKind::InvalidRequest);
}

#[tokio::test]
async fn rpc_proxy_preserves_plan_handle_and_adoption_bindings() {
    let fixture = Fixture::new(ProviderType::Runtime, 0).unwrap_or_else(|_| unreachable!());
    let clock = Arc::new(DeterministicClock::new(fixture.now_unix_ms));
    let instance = Arc::new(FakeProvider::new(fixture.clone())).instance();
    let adapter = Arc::new(
        ProviderAgentAdapter::new(instance, fixture.session_identity(), clock.clone())
            .unwrap_or_else(|_| unreachable!()),
    );
    let proxy = RpcProviderProxy::new(fixture.descriptor.clone(), adapter, clock)
        .unwrap_or_else(|_| unreachable!());

    let plan_request = fixture
        .request(ProviderMethod::RuntimePlan)
        .unwrap_or_else(|_| unreachable!());
    let plan_context = fixture.call_context(&plan_request.context);
    let plan = proxy
        .plan(&plan_context, &plan_request)
        .await
        .unwrap_or_else(|_| unreachable!());
    plan.validate(&plan_request, fixture.now_unix_ms)
        .unwrap_or_else(|_| unreachable!());

    let ensure_operation = fixture
        .operation(ProviderMethod::RuntimeEnsure)
        .unwrap_or_else(|_| unreachable!());
    let ensure_context = fixture.call_context(&ensure_operation);
    let handle = proxy
        .ensure(&ensure_context, &plan)
        .await
        .unwrap_or_else(|_| unreachable!());
    handle.validate().unwrap_or_else(|_| unreachable!());
    assert_eq!(handle.created_by, plan.binding);

    let adoption_operation = fixture
        .operation(ProviderMethod::RuntimeAdopt)
        .unwrap_or_else(|_| unreachable!());
    let adoption_context = fixture.call_context(&adoption_operation);
    let adoption = AdoptionRequest {
        context: adoption_operation.clone(),
        handle: handle.clone(),
        expected_owner: handle.owner.clone(),
        expected_configuration_fingerprint: handle.configuration_fingerprint.clone(),
        expected_resource_generation: handle.resource_generation,
    };
    let observation = proxy
        .adopt(&adoption_context, &adoption)
        .await
        .unwrap_or_else(|_| unreachable!());
    assert_eq!(observation.adoption, AdoptionState::Adopted);

    let mut mismatch = adoption;
    mismatch.expected_resource_generation = Generation::new(2).unwrap_or_else(|_| unreachable!());
    assert!(proxy.adopt(&adoption_context, &mismatch).await.is_err());
}

#[test]
fn redaction_wrappers_do_not_expose_canaries() {
    let secret = Secret::new("secret-canary");
    assert_eq!(format!("{secret:?}"), "Secret(<redacted>)");
    assert!(!format!("{:?}", Redacted("/sensitive/provider/path")).contains("/sensitive"));
    assert_eq!(secret.with_exposed(|value| value.len()), 13);
}

struct FakeSessionDriver {
    state: Mutex<AuthenticatedSessionState>,
    cancellation: CancellationToken,
    remaining_nanos: Mutex<u64>,
    attachments: Mutex<Vec<OwnedAttachment>>,
    incoming: Mutex<VecDeque<TransportPacket>>,
    outgoing: Mutex<VecDeque<TransportPacket>>,
}

impl FakeSessionDriver {
    fn new(fixture: &Fixture) -> Self {
        let operation = fixture
            .operation(ProviderMethod::RuntimePlan)
            .unwrap_or_else(|_| unreachable!());
        Self {
            state: Mutex::new(AuthenticatedSessionState {
                local_provider_id: fixture.descriptor.provider_id.clone(),
                local_provider_type: fixture.descriptor.provider_type(),
                local_provider_generation: fixture.descriptor.registry_generation,
                local_role: EndpointRole::ProviderAgent,
                peer_role: EndpointRole::RealmController,
                service: ServicePackage::ProviderV2,
                session_generation: 7,
                principal: PrincipalRef::parse("authenticated-controller")
                    .unwrap_or_else(|_| unreachable!()),
                authorized_scope: operation.scope,
            }),
            cancellation: CancellationToken::new(),
            remaining_nanos: Mutex::new(30_000_000_000),
            attachments: Mutex::new(Vec::new()),
            incoming: Mutex::new(VecDeque::new()),
            outgoing: Mutex::new(VecDeque::new()),
        }
    }

    fn push_packet(&self, packet: TransportPacket) {
        self.incoming
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push_back(packet);
    }

    fn pop_response(&self) -> TransportPacket {
        self.outgoing
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pop_front()
            .unwrap_or_else(|| unreachable!())
    }
}

#[async_trait]
impl ComponentSessionDriver for FakeSessionDriver {
    fn authenticated_state(&self) -> Result<AuthenticatedSessionState, SessionDriverError> {
        Ok(self
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone())
    }

    fn cancellation(&self, _: [u8; 16]) -> CancellationToken {
        self.cancellation.clone()
    }

    fn monotonic_remaining_nanos(&self, _: [u8; 16]) -> Result<u64, SessionDriverError> {
        Ok(*self
            .remaining_nanos
            .lock()
            .unwrap_or_else(|error| error.into_inner()))
    }

    async fn take_attachments(
        &self,
        _: [u8; 16],
        indexes: &[u32],
    ) -> Result<Vec<OwnedAttachment>, SessionDriverError> {
        let mut available = self
            .attachments
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if available
            .iter()
            .map(OwnedAttachment::index)
            .eq(indexes.iter().copied())
        {
            Ok(std::mem::take(&mut *available))
        } else {
            Err(SessionDriverError::AttachmentMismatch)
        }
    }

    async fn receive_packet(&self) -> Result<TransportPacket, SessionDriverError> {
        self.incoming
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pop_front()
            .ok_or(SessionDriverError::Disconnected)
    }

    async fn send_packet(&self, packet: TransportPacket) -> Result<(), SessionDriverError> {
        self.outgoing
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push_back(packet);
        Ok(())
    }
}

fn generated_request(fixture: &Fixture, method: ProviderMethod) -> common::ProviderRequest {
    let operation = fixture.operation(method).unwrap_or_else(|_| unreachable!());
    let scope = match &operation.scope {
        d2b_contracts::v2_provider::AuthorizedProviderScope::Workload {
            realm_id,
            workload_id,
        } => common::IdentityScope {
            realm_id: realm_id.as_str().to_owned(),
            workload_id: workload_id.as_str().to_owned(),
            ..Default::default()
        },
        _ => unreachable!(),
    };
    let mut metadata = common::RequestMetadata::new();
    metadata.request_id = vec![0x11; 16];
    metadata.correlation_id = operation.correlation_id.as_str().to_owned();
    metadata.trace_id = vec![0x22; 16];
    metadata.idempotency_key = vec![0x33; 16];
    metadata.issued_at_unix_ms = operation.issued_at_unix_ms;
    metadata.expires_at_unix_ms = fixture.now_unix_ms + 30_000;
    metadata.session_generation = 7;
    let mut context = common::ProviderOperationContext::new();
    context.metadata = MessageField::some(metadata);
    context.scope = MessageField::some(scope);
    context.operation_id = operation.operation_id.as_str().to_owned();
    context.provider_id = operation.provider_id.as_str().to_owned();
    context.provider_type = EnumOrUnknown::new(match fixture.descriptor.provider_type() {
        ProviderType::Runtime => common::ProviderType::PROVIDER_TYPE_RUNTIME,
        ProviderType::Infrastructure => common::ProviderType::PROVIDER_TYPE_INFRASTRUCTURE,
        ProviderType::Transport => common::ProviderType::PROVIDER_TYPE_TRANSPORT,
        ProviderType::Substrate => common::ProviderType::PROVIDER_TYPE_SUBSTRATE,
        ProviderType::Credential => common::ProviderType::PROVIDER_TYPE_CREDENTIAL,
        ProviderType::Display => common::ProviderType::PROVIDER_TYPE_DISPLAY,
        ProviderType::Network => common::ProviderType::PROVIDER_TYPE_NETWORK,
        ProviderType::Storage => common::ProviderType::PROVIDER_TYPE_STORAGE,
        ProviderType::Device => common::ProviderType::PROVIDER_TYPE_DEVICE,
        ProviderType::Audio => common::ProviderType::PROVIDER_TYPE_AUDIO,
        ProviderType::Observability => common::ProviderType::PROVIDER_TYPE_OBSERVABILITY,
    });
    context.provider_generation = operation.provider_generation.get();
    context.policy_epoch = operation.policy_epoch.get();
    context.authorization_digest = vec![0xc9; 32];
    context.request_digest = vec![0xc8; 32];
    common::ProviderRequest {
        context: MessageField::some(context),
        ..Default::default()
    }
}

#[tokio::test]
async fn generated_server_dispatches_closed_methods_over_authenticated_session() {
    let fixture = Fixture::new(ProviderType::Runtime, 0).unwrap_or_else(|_| unreachable!());
    let driver = Arc::new(FakeSessionDriver::new(&fixture));
    let server = Arc::new(
        GeneratedProviderServiceServer::new(
            Arc::new(FakeProvider::new(fixture.clone())).instance(),
            driver,
            Arc::new(DeterministicClock::new(fixture.now_unix_ms)),
        )
        .unwrap_or_else(|_| unreachable!()),
    );
    let services = server.generated_services();
    assert_eq!(services.len(), 1);
    assert!(services.keys().any(|name| name.contains("RuntimeProvider")));

    let context = ttrpc::r#async::TtrpcContext {
        mh: Default::default(),
        metadata: Default::default(),
        timeout_nano: 30_000_000_000,
    };
    generated_request(&fixture, ProviderMethod::RuntimePlan)
        .validate_wire(false)
        .unwrap_or_else(|error| panic!("{error:?}"));
    let capability_request = common::CapabilityRequest {
        context: generated_request(&fixture, ProviderMethod::RuntimePlan).context,
        ..Default::default()
    };
    let capabilities = provider_runtime_ttrpc::RuntimeProviderService::capabilities(
        server.as_ref(),
        &context,
        capability_request,
    )
    .await
    .unwrap_or_else(|error| panic!("{error:?}"));
    capabilities
        .validate_wire(false)
        .unwrap_or_else(|_| unreachable!());
    assert_eq!(
        capabilities.capabilities.len(),
        fixture.descriptor.capabilities.as_slice().len()
    );
    assert_eq!(
        capabilities.provider_generation,
        fixture.descriptor.registry_generation.get()
    );
    let plan = provider_runtime_ttrpc::RuntimeProviderService::plan(
        server.as_ref(),
        &context,
        generated_request(&fixture, ProviderMethod::RuntimePlan),
    )
    .await
    .unwrap_or_else(|error| panic!("{error:?}"));
    plan.validate_wire(false).unwrap_or_else(|_| unreachable!());
    assert!(!plan.resource_handle.is_empty());

    let mut ensure = generated_request(&fixture, ProviderMethod::RuntimeEnsure);
    ensure.resource_id = plan.resource_handle;
    let handle =
        provider_runtime_ttrpc::RuntimeProviderService::ensure(server.as_ref(), &context, ensure)
            .await
            .unwrap_or_else(|_| unreachable!());
    handle
        .validate_wire(false)
        .unwrap_or_else(|_| unreachable!());
    assert!(!handle.resource_handle.is_empty());
}

#[tokio::test]
async fn packet_proxy_preserves_owned_attachments_and_fails_closed() {
    let fixture = Fixture::new(ProviderType::Runtime, 0).unwrap_or_else(|_| unreachable!());
    let driver = Arc::new(FakeSessionDriver::new(&fixture));
    let server = GeneratedProviderServiceServer::new(
        Arc::new(FakeProvider::new(fixture.clone())).instance(),
        driver.clone(),
        Arc::new(DeterministicClock::new(fixture.now_unix_ms)),
    )
    .unwrap_or_else(|_| unreachable!());
    let mut request = generated_request(&fixture, ProviderMethod::RuntimePlan);
    request.attachment_indexes = vec![4];
    let payload = encode_strict(&request, false).unwrap_or_else(|_| unreachable!());
    driver.push_packet(TransportPacket {
        request_id: [0x11; 16],
        method: ClosedProviderMethod::Invoke(ProviderMethod::RuntimePlan),
        payload,
        attachments: vec![OwnedAttachment::new(4, vec![0x55; 8])],
    });
    server
        .serve_next_packet()
        .await
        .unwrap_or_else(|error| panic!("{error:?}"));
    let response = driver.pop_response();
    let response: common::ProviderResponse =
        decode_strict(&response.payload, false).unwrap_or_else(|_| unreachable!());
    assert!(!response.resource_handle.is_empty());

    driver.cancellation.cancel();
    let request = generated_request(&fixture, ProviderMethod::RuntimeInspect);
    driver.push_packet(TransportPacket {
        request_id: [0x11; 16],
        method: ClosedProviderMethod::Invoke(ProviderMethod::RuntimeInspect),
        payload: encode_strict(&request, false).unwrap_or_else(|_| unreachable!()),
        attachments: Vec::new(),
    });
    assert_eq!(
        server.serve_next_packet().await,
        Err(SessionDriverError::Protocol)
    );
}

#[tokio::test]
async fn generated_credential_service_owns_lease_payloads_and_continuity() {
    let fixture = Fixture::new(ProviderType::Credential, 4).unwrap_or_else(|_| unreachable!());
    let driver = Arc::new(FakeSessionDriver::new(&fixture));
    let server = GeneratedProviderServiceServer::new(
        Arc::new(FakeProvider::new(fixture.clone())).instance(),
        driver.clone(),
        Arc::new(DeterministicClock::new(fixture.now_unix_ms)),
    )
    .unwrap_or_else(|_| unreachable!());
    let context = ttrpc::r#async::TtrpcContext {
        mh: Default::default(),
        metadata: Default::default(),
        timeout_nano: 30_000_000_000,
    };
    let lease_request = sample_lease_request(&fixture).unwrap_or_else(|_| unreachable!());
    driver
        .attachments
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .push(OwnedAttachment::new(
            0,
            serde_json::to_vec(&lease_request).unwrap_or_else(|_| unreachable!()),
        ));
    let mut acquire = generated_request(&fixture, ProviderMethod::CredentialAcquireLease);
    acquire.attachment_indexes = vec![0];
    let acquired = provider_credential_ttrpc::CredentialProviderService::acquire_lease(
        &server, &context, acquire,
    )
    .await
    .unwrap_or_else(|error| panic!("{error:?}"));
    acquired
        .validate_wire(false)
        .unwrap_or_else(|_| unreachable!());
    assert!(!acquired.resource_handle.is_empty());

    let mut refresh = generated_request(&fixture, ProviderMethod::CredentialRefreshLease);
    refresh.resource_id = acquired.resource_handle;
    let refreshed = provider_credential_ttrpc::CredentialProviderService::refresh_lease(
        &server, &context, refresh,
    )
    .await
    .unwrap_or_else(|error| panic!("{error:?}"));
    refreshed
        .validate_wire(false)
        .unwrap_or_else(|_| unreachable!());
    assert!(!refreshed.resource_handle.is_empty());
}

#[tokio::test]
async fn generated_server_reauthenticates_and_rechecks_deadlines_per_request() {
    let fixture = Fixture::new(ProviderType::Runtime, 0).unwrap_or_else(|_| unreachable!());
    let driver = Arc::new(FakeSessionDriver::new(&fixture));
    let server = GeneratedProviderServiceServer::new(
        Arc::new(FakeProvider::new(fixture.clone())).instance(),
        driver.clone(),
        Arc::new(DeterministicClock::new(fixture.now_unix_ms)),
    )
    .unwrap_or_else(|_| unreachable!());
    let context = ttrpc::r#async::TtrpcContext {
        mh: Default::default(),
        metadata: Default::default(),
        timeout_nano: 30_000_000_000,
    };

    driver
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .session_generation = 8;
    assert!(
        provider_runtime_ttrpc::RuntimeProviderService::plan(
            &server,
            &context,
            generated_request(&fixture, ProviderMethod::RuntimePlan),
        )
        .await
        .is_err()
    );

    driver
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .session_generation = 7;
    *driver
        .remaining_nanos
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = 0;
    assert!(
        provider_runtime_ttrpc::RuntimeProviderService::plan(
            &server,
            &context,
            generated_request(&fixture, ProviderMethod::RuntimePlan),
        )
        .await
        .is_err()
    );

    assert!(server.shutdown(std::time::Duration::from_millis(10)).await);
    assert!(
        provider_runtime_ttrpc::RuntimeProviderService::plan(
            &server,
            &context,
            generated_request(&fixture, ProviderMethod::RuntimePlan),
        )
        .await
        .is_err()
    );
}
