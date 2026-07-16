use std::sync::{
    Arc,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

use d2b_contracts::{
    v2_component_session::ServicePackage,
    v2_identity::{ProviderType, WorkloadId},
    v2_provider::{
        AuthorizedProviderScope, CredentialProvider, Fingerprint, ImplementationId, MutationState,
        Provider, ProviderFailureKind, ProviderMethod, ProviderOperationInput, ProviderPlacement,
        ProviderTarget, SdkOperationClass,
    },
};
use d2b_provider::{
    FactoryError, ProviderClock, ProviderFactory, ProviderInstance, ProviderRegistryBuilder,
};
use d2b_provider_toolkit::{DeterministicClock, Fixture, check_provider_conformance};

use super::*;

const NOW: u64 = 1_700_000_000_000;

struct FakeOo7Port {
    clock: Arc<DeterministicClock>,
    state: Mutex<SecretServiceState>,
    lease: Mutex<Option<SecretServiceLeaseInspection>>,
    acquired_by: Mutex<Option<OperationBinding>>,
    state_calls: AtomicUsize,
    issue_calls: AtomicUsize,
    inspect_calls: AtomicUsize,
    refresh_calls: AtomicUsize,
    revoke_calls: AtomicUsize,
    state_delay_ms: AtomicU64,
    credential_canary: String,
    object_path_canary: String,
}

impl FakeOo7Port {
    fn new(clock: Arc<DeterministicClock>) -> Self {
        Self {
            clock,
            state: Mutex::new(SecretServiceState::Unlocked),
            lease: Mutex::new(None),
            acquired_by: Mutex::new(None),
            state_calls: AtomicUsize::new(0),
            issue_calls: AtomicUsize::new(0),
            inspect_calls: AtomicUsize::new(0),
            refresh_calls: AtomicUsize::new(0),
            revoke_calls: AtomicUsize::new(0),
            state_delay_ms: AtomicU64::new(0),
            credential_canary: "secret-canary-must-remain-inside-fake-oo7".to_owned(),
            object_path_canary: "/synthetic/secret-service/object".to_owned(),
        }
    }

    fn set_state(&self, state: SecretServiceState) {
        *self.state.lock().expect("state lock") = state;
    }

    fn inspection(&self) -> SecretServiceLeaseInspection {
        self.lease
            .lock()
            .expect("lease lock")
            .clone()
            .expect("issued lease")
    }
}

#[async_trait]
impl Oo7SecretServicePort for FakeOo7Port {
    async fn state(&self) -> Result<SecretServiceState, SecretServicePortError> {
        self.state_calls.fetch_add(1, Ordering::Relaxed);
        let delay = self.state_delay_ms.load(Ordering::Relaxed);
        if delay != 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        Ok(*self.state.lock().expect("state lock"))
    }

    async fn issue_lease(
        &self,
        request: &SecretServiceLeaseRequest,
    ) -> Result<SecretServiceLeaseGrant, SecretServicePortError> {
        self.issue_calls.fetch_add(1, Ordering::Relaxed);
        assert!(matches!(
            &request.placement_binding,
            CredentialPlacementBinding::UserAgent { .. }
        ));
        if *self.state.lock().expect("state lock") == SecretServiceState::Locked {
            return Err(SecretServicePortError::Locked);
        }
        assert!(!self.credential_canary.is_empty());
        assert!(!self.object_path_canary.is_empty());
        let grant = SecretServiceLeaseGrant {
            lease_id: LeaseId::parse("secret-service-lease").expect("lease id"),
            source_version: SourceVersion::parse("source-one").expect("source version"),
            rotation_generation: Generation::new(1).expect("generation"),
            expires_at_unix_ms: request.requested_expiry_unix_ms,
        };
        *self.acquired_by.lock().expect("operation lock") = Some(request.operation.clone());
        *self.lease.lock().expect("lease lock") = Some(SecretServiceLeaseInspection {
            state: SecretServiceLeaseState::Active,
            source_version: grant.source_version.clone(),
            rotation_generation: grant.rotation_generation,
            expires_at_unix_ms: grant.expires_at_unix_ms,
            revoked_at_unix_ms: None,
        });
        Ok(grant)
    }

    async fn inspect_lease(
        &self,
        lease: &SecretServiceLeaseRef,
    ) -> Result<SecretServiceLeaseInspection, SecretServicePortError> {
        self.inspect_calls.fetch_add(1, Ordering::Relaxed);
        assert!(matches!(
            &lease.placement_binding,
            CredentialPlacementBinding::UserAgent { .. }
        ));
        assert_eq!(
            self.acquired_by.lock().expect("operation lock").as_ref(),
            Some(&lease.acquired_by)
        );
        Ok(self.inspection())
    }

    async fn refresh_lease(
        &self,
        lease: &SecretServiceLeaseRef,
    ) -> Result<SecretServiceLeaseRenewal, SecretServicePortError> {
        self.refresh_calls.fetch_add(1, Ordering::Relaxed);
        let renewal = SecretServiceLeaseRenewal {
            source_version: SourceVersion::parse("source-two").expect("source version"),
            rotation_generation: Generation::new(2).expect("generation"),
            expires_at_unix_ms: lease.requested_expiry_unix_ms,
        };
        *self.lease.lock().expect("lease lock") = Some(SecretServiceLeaseInspection {
            state: SecretServiceLeaseState::Active,
            source_version: renewal.source_version.clone(),
            rotation_generation: renewal.rotation_generation,
            expires_at_unix_ms: renewal.expires_at_unix_ms,
            revoked_at_unix_ms: None,
        });
        Ok(renewal)
    }

    async fn revoke_lease(
        &self,
        _lease: &SecretServiceLeaseRef,
    ) -> Result<SecretServiceLeaseRevocation, SecretServicePortError> {
        self.revoke_calls.fetch_add(1, Ordering::Relaxed);
        let revoked_at = self.clock.now_unix_ms();
        let mut inspection = self.inspection();
        inspection.state = SecretServiceLeaseState::Revoked;
        inspection.revoked_at_unix_ms = Some(revoked_at);
        *self.lease.lock().expect("lease lock") = Some(inspection);
        Ok(SecretServiceLeaseRevocation::Revoked {
            revoked_at_unix_ms: revoked_at,
        })
    }
}

fn descriptors() -> (Fixture, ProviderDescriptor, ProviderDescriptor) {
    let base = Fixture::new(ProviderType::Credential, 0).expect("credential fixture");
    let mut descriptor = base.descriptor;
    descriptor.implementation_id = implementation_id();
    let ProviderPlacement::ProviderAgent {
        realm_id,
        role_id,
        agent_generation,
        ..
    } = &descriptor.placement
    else {
        panic!("provider-agent fixture");
    };
    let realm_id = realm_id.clone();
    let placement = ProviderPlacement::UserAgent {
        realm_id: realm_id.clone(),
        role_id: role_id.clone(),
        endpoint_role: EndpointRole::UserAgent,
        service: ServicePackage::UserV2,
        agent_generation: *agent_generation,
    };
    descriptor.placement = placement.clone();
    let fixture =
        Fixture::from_descriptor(descriptor.clone(), ProviderTarget::Realm { realm_id }, NOW)
            .expect("user-agent fixture");
    let mut consumer = Fixture::new(ProviderType::Transport, 1)
        .expect("consumer fixture")
        .descriptor;
    consumer.placement = placement;
    (fixture, descriptor, consumer)
}

fn setup() -> (
    SecretServiceCredentialProvider,
    Fixture,
    Arc<FakeOo7Port>,
    Arc<DeterministicClock>,
) {
    let (fixture, descriptor, consumer) = descriptors();
    let clock = Arc::new(DeterministicClock::new(NOW));
    let port = Arc::new(FakeOo7Port::new(Arc::clone(&clock)));
    let provider = SecretServiceCredentialProvider::new_userd_with_clock(
        descriptor,
        consumer,
        vec![SdkOperationClass::Read, SdkOperationClass::Update],
        port.clone(),
        clock.clone(),
    )
    .expect("provider");
    (provider, fixture, port, clock)
}

fn lease_request(
    provider: &SecretServiceCredentialProvider,
    fixture: &Fixture,
) -> CredentialLeaseRequest {
    CredentialLeaseRequest {
        context: fixture
            .operation(ProviderMethod::CredentialAcquireLease)
            .expect("operation"),
        consumer_provider_id: provider.consumer.provider_id.clone(),
        placement_binding: provider.placement_binding.clone(),
        allowed_operations: BoundedVec::new(vec![SdkOperationClass::Read]).expect("operations"),
        requested_expiry_unix_ms: NOW + 30_000,
    }
}

#[test]
fn construction_requires_user_agent_and_a_consuming_provider_type() {
    let (_, descriptor, _) = descriptors();
    let mut consumer = Fixture::new(ProviderType::Audio, 1)
        .expect("audio fixture")
        .descriptor;
    consumer.placement = descriptor.placement.clone();
    let clock = Arc::new(DeterministicClock::new(NOW));
    let port = Arc::new(FakeOo7Port::new(clock.clone()));
    let result = SecretServiceCredentialProvider::new_userd_with_clock(
        descriptor,
        consumer,
        vec![SdkOperationClass::Read],
        port,
        clock,
    );
    assert!(matches!(
        result,
        Err(SecretServiceProviderError::InvalidConsumer)
    ));

    let fixture = Fixture::new(ProviderType::Credential, 0).expect("credential fixture");
    let mut descriptor = fixture.descriptor;
    descriptor.implementation_id = implementation_id();
    let consumer = Fixture::new(ProviderType::Transport, 1)
        .expect("consumer fixture")
        .descriptor;
    let clock = Arc::new(DeterministicClock::new(NOW));
    let port = Arc::new(FakeOo7Port::new(clock.clone()));
    let result = SecretServiceCredentialProvider::new_userd_with_clock(
        descriptor,
        consumer,
        vec![SdkOperationClass::Read],
        port,
        clock,
    );
    assert!(matches!(
        result,
        Err(SecretServiceProviderError::InvalidDescriptor)
    ));
}

#[test]
fn factory_registers_and_rejects_wrong_type_or_implementation() {
    let (_, descriptor, consumer) = descriptors();
    let clock = Arc::new(DeterministicClock::new(NOW));
    let port = Arc::new(FakeOo7Port::new(clock.clone()));
    let factory = SecretServiceCredentialProviderFactory::new_with_clock(
        consumer,
        vec![SdkOperationClass::Read],
        port.clone(),
        clock,
    )
    .expect("factory");

    assert_eq!(
        SecretServiceCredentialProviderFactory::key(),
        provider_factory_key()
    );
    assert_eq!(
        provider_factory_key().implementation_id,
        implementation_id()
    );

    let mut wrong_type = Fixture::new(ProviderType::Runtime, 2)
        .expect("runtime fixture")
        .descriptor;
    wrong_type.implementation_id = implementation_id();
    assert!(matches!(
        factory.construct(&wrong_type),
        Err(FactoryError::Rejected)
    ));

    let mut wrong_implementation = descriptor.clone();
    wrong_implementation.implementation_id =
        ImplementationId::parse("other-secret-service").expect("implementation id");
    assert!(matches!(
        factory.construct(&wrong_implementation),
        Err(FactoryError::Rejected)
    ));
    assert_eq!(port.state_calls.load(Ordering::Relaxed), 0);
    assert_eq!(port.issue_calls.load(Ordering::Relaxed), 0);

    let mut builder = ProviderRegistryBuilder::new(
        descriptor.registry_generation,
        Fingerprint::parse("f".repeat(64)).expect("fingerprint"),
        NOW,
    );
    builder
        .register_factory(provider_factory_key(), Arc::new(factory))
        .expect("register factory");
    builder
        .register_instance(descriptor)
        .expect("construct provider");
    let registry = builder.finish().expect("registry");
    assert_eq!(
        registry.snapshot().factories.as_slice(),
        &[provider_factory_key()]
    );
}

#[tokio::test]
async fn passes_common_provider_conformance() {
    let (provider, fixture, _, _) = setup();
    let target = fixture
        .request(ProviderMethod::CredentialStatus)
        .expect("status request")
        .target;
    let fixture =
        Fixture::from_descriptor(provider.descriptor(), target, NOW).expect("canonical fixture");
    let request = fixture
        .request(ProviderMethod::CredentialStatus)
        .expect("status request");
    let production_context = fixture.call_context(&request.context);
    provider
        .status(&production_context, &request)
        .await
        .expect("user-agent caller");
    let instance = ProviderInstance::Credential(Arc::new(provider));
    check_provider_conformance(&instance, &fixture)
        .await
        .expect("conformance");
}

#[tokio::test]
async fn userd_owner_reports_locked_and_unlocked_without_secret_output() {
    let (provider, fixture, port, _) = setup();
    assert_eq!(provider.owner(), SecretServiceOwner::Userd);
    assert_eq!(provider.descriptor().capabilities, exact_capabilities());

    port.set_state(SecretServiceState::Locked);
    let request = fixture
        .request(ProviderMethod::CredentialStatus)
        .expect("status request");
    let context = fixture.call_context(&request.context);
    let locked = provider
        .status(&context, &request)
        .await
        .expect("locked status is observable");
    assert_eq!(locked.health.state, ProviderHealthState::Degraded);
    assert_eq!(
        locked.health.remediation,
        ProviderRemediation::OperatorInteraction
    );

    let lease_request = lease_request(&provider, &fixture);
    let lease_context = fixture.call_context(&lease_request.context);
    let error = provider
        .acquire_lease(&lease_context, &lease_request)
        .await
        .expect_err("locked keyring must deny lease");
    assert_eq!(error.kind, ProviderFailureKind::Unavailable);
    assert_eq!(error.retry, RetryClass::AfterInteraction);

    port.set_state(SecretServiceState::Unlocked);
    let unlocked = provider
        .status(&context, &request)
        .await
        .expect("unlocked status");
    assert_eq!(unlocked.health.state, ProviderHealthState::Healthy);
}

#[tokio::test]
async fn opaque_lease_refresh_revoke_and_inspection_are_bound() {
    let (provider, fixture, port, _) = setup();
    let request = lease_request(&provider, &fixture);
    let context = fixture.call_context(&request.context);
    let lease = provider
        .acquire_lease(&context, &request)
        .await
        .expect("lease");
    let replay = provider
        .acquire_lease(&context, &request)
        .await
        .expect("idempotent acquisition");
    assert_eq!(replay, lease);
    assert_eq!(port.issue_calls.load(Ordering::Relaxed), 1);
    assert_eq!(lease.consumer_provider_id, provider.consumer.provider_id);
    assert_eq!(
        lease.consumer_provider_generation,
        provider.consumer.registry_generation
    );
    assert_eq!(lease.placement_binding, provider.placement_binding);
    assert!(matches!(
        &lease.placement_binding,
        CredentialPlacementBinding::UserAgent { .. }
    ));
    assert_eq!(lease.state, CredentialLeaseState::Active);

    let refresh_operation = fixture
        .operation(ProviderMethod::CredentialRefreshLease)
        .expect("refresh operation");
    let refresh_context = fixture.call_context(&refresh_operation);
    let refreshed = provider
        .refresh_lease(&refresh_context, &lease)
        .await
        .expect("refresh");
    assert_eq!(refreshed.rotation_generation.get(), 2);
    assert_eq!(port.inspect_calls.load(Ordering::Relaxed), 1);
    assert_eq!(port.refresh_calls.load(Ordering::Relaxed), 1);

    let revoke_operation = fixture
        .operation(ProviderMethod::CredentialRevokeLease)
        .expect("revoke operation");
    let revoke_context = fixture.call_context(&revoke_operation);
    let receipt = provider
        .revoke_lease(&revoke_context, &refreshed)
        .await
        .expect("revoke");
    assert_eq!(receipt.binding, revoke_operation.binding());
    assert_eq!(receipt.state, MutationState::Applied);
    assert_eq!(port.revoke_calls.load(Ordering::Relaxed), 1);

    let repeated = provider
        .revoke_lease(&revoke_context, &refreshed)
        .await
        .expect("idempotent revoke");
    assert_eq!(repeated.state, MutationState::AlreadyApplied);
    assert_eq!(port.revoke_calls.load(Ordering::Relaxed), 1);

    let error = provider
        .refresh_lease(&refresh_context, &refreshed)
        .await
        .expect_err("revoked lease cannot refresh");
    assert_eq!(error.kind, ProviderFailureKind::CredentialLeaseInvalid);
    assert_eq!(port.refresh_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn denial_and_wrong_input_make_zero_client_calls() {
    let (provider, fixture, port, _) = setup();
    let wrong_input = fixture
        .request_with_input(
            ProviderMethod::CredentialStatus,
            ProviderOperationInput::AudioState {
                channel: d2b_contracts::v2_provider::AudioChannel::Speaker,
                direction: d2b_contracts::v2_provider::AudioDirection::Output,
                mute: Some(false),
                volume: None,
            },
        )
        .expect("request");
    let wrong_input_context = fixture.call_context(&wrong_input.context);
    assert!(
        provider
            .status(&wrong_input_context, &wrong_input)
            .await
            .is_err()
    );
    assert_eq!(port.state_calls.load(Ordering::Relaxed), 0);

    let mut request = lease_request(&provider, &fixture);
    request.consumer_provider_id = Fixture::new(ProviderType::Audio, 2)
        .expect("other provider")
        .descriptor
        .provider_id;
    let context = fixture.call_context(&request.context);
    let error = provider
        .acquire_lease(&context, &request)
        .await
        .expect_err("wrong consumer");
    assert_eq!(error.kind, ProviderFailureKind::UnauthorizedScope);
    assert_eq!(port.issue_calls.load(Ordering::Relaxed), 0);

    let mut request = lease_request(&provider, &fixture);
    request.allowed_operations =
        BoundedVec::new(vec![SdkOperationClass::Delete]).expect("operations");
    let context = fixture.call_context(&request.context);
    assert!(provider.acquire_lease(&context, &request).await.is_err());
    assert_eq!(port.issue_calls.load(Ordering::Relaxed), 0);

    let mut request = lease_request(&provider, &fixture);
    request.context.scope = AuthorizedProviderScope::Workload {
        realm_id: provider.descriptor.placement.realm_id().clone(),
        workload_id: WorkloadId::parse("ddddddddddddddddddda").expect("other workload"),
    };
    let context = fixture.call_context(&request.context);
    assert!(provider.acquire_lease(&context, &request).await.is_err());
    assert_eq!(port.issue_calls.load(Ordering::Relaxed), 0);

    let mut wrong_method = lease_request(&provider, &fixture);
    wrong_method.context.method = ProviderMethod::CredentialStatus;
    wrong_method.context.capability = ProviderCapability(ProviderMethod::CredentialStatus);
    let context = fixture.call_context(&wrong_method.context);
    assert!(
        provider
            .acquire_lease(&context, &wrong_method)
            .await
            .is_err()
    );
    assert_eq!(port.issue_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn expiry_is_terminal_without_backend_fallback() {
    let (provider, fixture, port, clock) = setup();
    let request = lease_request(&provider, &fixture);
    let context = fixture.call_context(&request.context);
    let lease = provider
        .acquire_lease(&context, &request)
        .await
        .expect("lease");
    clock.set(lease.expires_at_unix_ms);

    let operation = fixture
        .operation(ProviderMethod::CredentialRefreshLease)
        .expect("refresh operation");
    let context = fixture.call_context(&operation);
    let error = provider
        .refresh_lease(&context, &lease)
        .await
        .expect_err("expired lease");
    assert_eq!(error.kind, ProviderFailureKind::CredentialLeaseInvalid);
    assert_eq!(port.inspect_calls.load(Ordering::Relaxed), 0);
    assert_eq!(port.refresh_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn local_lease_table_is_bounded_before_port_use() {
    let (provider, fixture, port, _) = setup();
    let request = lease_request(&provider, &fixture);
    let context = fixture.call_context(&request.context);
    provider
        .acquire_lease(&context, &request)
        .await
        .expect("seed lease");
    {
        let mut leases = provider.leases.lock().expect("lease lock");
        let seed = leases.values().next().expect("seed record").clone();
        for index in 1..MAX_LOCAL_LEASES {
            let mut record = seed.clone();
            record.lease.lease_id =
                LeaseId::parse(format!("capacity-{index}")).expect("capacity lease id");
            leases.insert(record.lease.lease_id.clone(), record);
        }
    }
    let mut request = lease_request(&provider, &fixture);
    request.context.operation_id =
        d2b_contracts::v2_provider::OperationId::parse("operation-capacity").expect("operation id");
    request.context.idempotency_key =
        d2b_contracts::v2_provider::IdempotencyKey::parse("idempotency-capacity")
            .expect("idempotency key");
    request.context.request_digest =
        d2b_contracts::v2_provider::Fingerprint::parse("f".repeat(64)).expect("digest");
    let context = fixture.call_context(&request.context);
    let error = provider
        .acquire_lease(&context, &request)
        .await
        .expect_err("bounded table");
    assert_eq!(error.kind, ProviderFailureKind::Unavailable);
    assert_eq!(error.reason, ProviderHealthReason::QueuePressure);
    assert_eq!(port.issue_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn cancellation_and_deadline_fail_closed() {
    let (provider, fixture, port, _) = setup();
    let request = fixture
        .request(ProviderMethod::CredentialStatus)
        .expect("status request");

    let mut cancelled = fixture.call_context(&request.context);
    cancelled.cancelled = true;
    let error = provider
        .status(&cancelled, &request)
        .await
        .expect_err("cancelled");
    assert_eq!(error.kind, ProviderFailureKind::Cancelled);
    assert_eq!(port.state_calls.load(Ordering::Relaxed), 0);

    port.state_delay_ms.store(20, Ordering::Relaxed);
    let mut deadline = fixture.call_context(&request.context);
    deadline.monotonic_deadline_remaining_ms = 1;
    let error = provider
        .status(&deadline, &request)
        .await
        .expect_err("deadline");
    assert_eq!(error.kind, ProviderFailureKind::DeadlineExpired);
    assert_eq!(error.retry, RetryClass::SameOperation);
}

#[tokio::test]
async fn client_held_secret_canary_is_absent_from_all_outputs() {
    let (provider, fixture, port, _) = setup();
    let request = lease_request(&provider, &fixture);
    let context = fixture.call_context(&request.context);
    let lease = provider
        .acquire_lease(&context, &request)
        .await
        .expect("lease");
    let denied = SecretServicePortError::Denied;
    let rendered = format!(
        "{provider:?} {lease:?} {:?} {denied:?} {denied}",
        provider.descriptor()
    );
    assert!(!rendered.contains(&port.credential_canary));
    assert!(!rendered.contains(&port.object_path_canary));
    assert!(!rendered.contains("password"));
}
