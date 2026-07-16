use std::sync::{
    Arc,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

use d2b_contracts::{
    v2_identity::{ProviderType, WorkloadId},
    v2_provider::{
        CredentialProvider, Fingerprint, ImplementationId, MutationState, Provider,
        ProviderFailureKind, ProviderMethod, ProviderOperationInput, ProviderPlacement,
        SdkOperationClass,
    },
};
use d2b_provider::{
    FactoryError, ProviderClock, ProviderFactory, ProviderInstance, ProviderRegistryBuilder,
};
use d2b_provider_toolkit::{DeterministicClock, Fixture, check_provider_conformance};

use super::*;

const NOW: u64 = 1_700_000_000_000;

struct FakeEntraClient {
    clock: Arc<DeterministicClock>,
    state: Mutex<EntraClientState>,
    lease: Mutex<Option<EntraLeaseInspection>>,
    acquired_by: Mutex<Option<OperationBinding>>,
    state_calls: AtomicUsize,
    issue_calls: AtomicUsize,
    inspect_calls: AtomicUsize,
    refresh_calls: AtomicUsize,
    revoke_calls: AtomicUsize,
    state_delay_ms: AtomicU64,
    credential_canary: String,
    endpoint_canary: String,
}

impl FakeEntraClient {
    fn new(clock: Arc<DeterministicClock>) -> Self {
        Self {
            clock,
            state: Mutex::new(EntraClientState::Ready),
            lease: Mutex::new(None),
            acquired_by: Mutex::new(None),
            state_calls: AtomicUsize::new(0),
            issue_calls: AtomicUsize::new(0),
            inspect_calls: AtomicUsize::new(0),
            refresh_calls: AtomicUsize::new(0),
            revoke_calls: AtomicUsize::new(0),
            state_delay_ms: AtomicU64::new(0),
            credential_canary: "entra-token-canary-remains-in-exact-consumer".to_owned(),
            endpoint_canary: "https://synthetic.invalid/entra".to_owned(),
        }
    }

    fn inspection(&self) -> EntraLeaseInspection {
        self.lease
            .lock()
            .expect("lease lock")
            .clone()
            .expect("issued lease")
    }
}

#[async_trait]
impl EntraCredentialClient for FakeEntraClient {
    async fn state(&self) -> Result<EntraClientState, EntraClientError> {
        self.state_calls.fetch_add(1, Ordering::Relaxed);
        let delay = self.state_delay_ms.load(Ordering::Relaxed);
        if delay != 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        Ok(*self.state.lock().expect("state lock"))
    }

    async fn issue_lease(
        &self,
        request: &EntraLeaseRequest,
    ) -> Result<EntraLeaseGrant, EntraClientError> {
        self.issue_calls.fetch_add(1, Ordering::Relaxed);
        assert!(matches!(
            &request.placement_binding,
            CredentialPlacementBinding::ProviderAgent { .. }
        ));
        if *self.state.lock().expect("state lock") == EntraClientState::InteractionRequired {
            return Err(EntraClientError::InteractionRequired);
        }
        assert!(!self.credential_canary.is_empty());
        assert!(!self.endpoint_canary.is_empty());
        let grant = EntraLeaseGrant {
            lease_id: LeaseId::parse("entra-lease").expect("lease id"),
            source_version: SourceVersion::parse("entra-source-one").expect("source version"),
            rotation_generation: Generation::new(1).expect("generation"),
            expires_at_unix_ms: request.requested_expiry_unix_ms,
        };
        *self.acquired_by.lock().expect("operation lock") = Some(request.operation.clone());
        *self.lease.lock().expect("lease lock") = Some(EntraLeaseInspection {
            state: EntraLeaseState::Active,
            source_version: grant.source_version.clone(),
            rotation_generation: grant.rotation_generation,
            expires_at_unix_ms: grant.expires_at_unix_ms,
            revoked_at_unix_ms: None,
        });
        Ok(grant)
    }

    async fn inspect_lease(
        &self,
        lease: &EntraLeaseRef,
    ) -> Result<EntraLeaseInspection, EntraClientError> {
        self.inspect_calls.fetch_add(1, Ordering::Relaxed);
        assert!(matches!(
            &lease.placement_binding,
            CredentialPlacementBinding::ProviderAgent { .. }
        ));
        assert_eq!(
            self.acquired_by.lock().expect("operation lock").as_ref(),
            Some(&lease.acquired_by)
        );
        Ok(self.inspection())
    }

    async fn refresh_lease(
        &self,
        lease: &EntraLeaseRef,
    ) -> Result<EntraLeaseRenewal, EntraClientError> {
        self.refresh_calls.fetch_add(1, Ordering::Relaxed);
        let renewal = EntraLeaseRenewal {
            source_version: SourceVersion::parse("entra-source-two").expect("source version"),
            rotation_generation: Generation::new(2).expect("generation"),
            expires_at_unix_ms: lease.requested_expiry_unix_ms,
        };
        *self.lease.lock().expect("lease lock") = Some(EntraLeaseInspection {
            state: EntraLeaseState::Active,
            source_version: renewal.source_version.clone(),
            rotation_generation: renewal.rotation_generation,
            expires_at_unix_ms: renewal.expires_at_unix_ms,
            revoked_at_unix_ms: None,
        });
        Ok(renewal)
    }

    async fn revoke_lease(
        &self,
        _lease: &EntraLeaseRef,
    ) -> Result<EntraLeaseRevocation, EntraClientError> {
        self.revoke_calls.fetch_add(1, Ordering::Relaxed);
        let revoked_at = self.clock.now_unix_ms();
        let mut inspection = self.inspection();
        inspection.state = EntraLeaseState::Revoked;
        inspection.revoked_at_unix_ms = Some(revoked_at);
        *self.lease.lock().expect("lease lock") = Some(inspection);
        Ok(EntraLeaseRevocation::Revoked {
            revoked_at_unix_ms: revoked_at,
        })
    }
}

fn descriptors() -> (Fixture, ProviderDescriptor, ProviderDescriptor) {
    let fixture = Fixture::new(ProviderType::Credential, 0).expect("credential fixture");
    let mut descriptor = fixture.descriptor.clone();
    descriptor.implementation_id = implementation_id();
    let consumer = Fixture::new(ProviderType::Runtime, 1)
        .expect("consumer fixture")
        .descriptor;
    (fixture, descriptor, consumer)
}

fn setup() -> (
    EntraCredentialProvider,
    Fixture,
    Arc<FakeEntraClient>,
    Arc<DeterministicClock>,
) {
    let (fixture, descriptor, consumer) = descriptors();
    let clock = Arc::new(DeterministicClock::new(NOW));
    let client = Arc::new(FakeEntraClient::new(clock.clone()));
    let provider = EntraCredentialProvider::new_colocated_with_clock(
        descriptor,
        consumer,
        vec![
            SdkOperationClass::Authenticate,
            SdkOperationClass::Read,
            SdkOperationClass::Create,
        ],
        client.clone(),
        clock.clone(),
    )
    .expect("provider");
    (provider, fixture, client, clock)
}

fn lease_request(provider: &EntraCredentialProvider, fixture: &Fixture) -> CredentialLeaseRequest {
    CredentialLeaseRequest {
        context: fixture
            .operation(ProviderMethod::CredentialAcquireLease)
            .expect("operation"),
        consumer_provider_id: provider.consumer.provider_id.clone(),
        placement_binding: provider.placement_binding(),
        allowed_operations: BoundedVec::new(vec![
            SdkOperationClass::Authenticate,
            SdkOperationClass::Read,
        ])
        .expect("operations"),
        requested_expiry_unix_ms: NOW + 30_000,
    }
}

#[test]
fn factory_registers_and_rejects_wrong_type_or_implementation() {
    let (_, descriptor, consumer) = descriptors();
    let clock = Arc::new(DeterministicClock::new(NOW));
    let client = Arc::new(FakeEntraClient::new(clock.clone()));
    let factory = EntraCredentialProviderFactory::new_with_clock(
        consumer,
        vec![SdkOperationClass::Authenticate, SdkOperationClass::Read],
        client.clone(),
        clock,
    )
    .expect("factory");

    assert_eq!(
        EntraCredentialProviderFactory::key(),
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
        ImplementationId::parse("other-entra").expect("implementation id");
    assert!(matches!(
        factory.construct(&wrong_implementation),
        Err(FactoryError::Rejected)
    ));
    assert_eq!(client.state_calls.load(Ordering::Relaxed), 0);
    assert_eq!(client.issue_calls.load(Ordering::Relaxed), 0);

    let mut builder = ProviderRegistryBuilder::new(
        descriptor.registry_generation,
        Fingerprint::parse("e".repeat(64)).expect("fingerprint"),
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
    let mut production_context = fixture.call_context(&request.context);
    production_context.peer_role = EndpointRole::RealmController;
    provider
        .status(&production_context, &request)
        .await
        .expect("provider-agent server caller");
    let instance = ProviderInstance::Credential(Arc::new(provider));
    check_provider_conformance(&instance, &fixture)
        .await
        .expect("conformance");
}

#[test]
fn construction_requires_exact_consumer_colocation_and_capabilities() {
    let (_, descriptor, mut consumer) = descriptors();
    let ProviderPlacement::ProviderAgent { workload_id, .. } = &mut consumer.placement else {
        panic!("provider agent");
    };
    *workload_id = WorkloadId::parse("ddddddddddddddddddda").expect("other workload");
    let clock = Arc::new(DeterministicClock::new(NOW));
    let client = Arc::new(FakeEntraClient::new(clock.clone()));
    let result = EntraCredentialProvider::new_colocated_with_clock(
        descriptor,
        consumer,
        vec![SdkOperationClass::Read],
        client,
        clock,
    );
    assert!(matches!(result, Err(EntraProviderError::NotColocated)));

    let (_, descriptor, _) = descriptors();
    let consumer = Fixture::new(ProviderType::Audio, 2)
        .expect("audio fixture")
        .descriptor;
    let clock = Arc::new(DeterministicClock::new(NOW));
    let client = Arc::new(FakeEntraClient::new(clock.clone()));
    let result = EntraCredentialProvider::new_colocated_with_clock(
        descriptor,
        consumer,
        vec![SdkOperationClass::Read],
        client,
        clock,
    );
    assert!(matches!(result, Err(EntraProviderError::InvalidConsumer)));

    let (provider, _, _, _) = setup();
    assert_eq!(provider.owner(), EntraCredentialOwner::ExactConsumer);
    assert_eq!(provider.descriptor().capabilities, exact_capabilities());
}

#[tokio::test]
async fn interaction_state_is_typed_and_does_not_return_a_token() {
    let (provider, fixture, client, _) = setup();
    *client.state.lock().expect("state lock") = EntraClientState::InteractionRequired;
    let status_request = fixture
        .request(ProviderMethod::CredentialStatus)
        .expect("status request");
    let status_context = fixture.call_context(&status_request.context);
    let status = provider
        .status(&status_context, &status_request)
        .await
        .expect("status");
    assert_eq!(status.health.state, ProviderHealthState::Degraded);

    let request = lease_request(&provider, &fixture);
    let context = fixture.call_context(&request.context);
    let error = provider
        .acquire_lease(&context, &request)
        .await
        .expect_err("interaction required");
    assert_eq!(error.kind, ProviderFailureKind::Unavailable);
    assert_eq!(error.retry, RetryClass::AfterInteraction);
}

#[tokio::test]
async fn lease_lifecycle_is_exact_consumer_and_operation_bound() {
    let (provider, fixture, client, _) = setup();
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
    assert_eq!(client.issue_calls.load(Ordering::Relaxed), 1);
    assert_eq!(lease.consumer_provider_id, provider.consumer.provider_id);
    assert_eq!(lease.placement_binding, provider.placement_binding());
    assert!(matches!(
        &lease.placement_binding,
        CredentialPlacementBinding::ProviderAgent { .. }
    ));

    let refresh_operation = fixture
        .operation(ProviderMethod::CredentialRefreshLease)
        .expect("refresh operation");
    let refresh_context = fixture.call_context(&refresh_operation);
    let refreshed = provider
        .refresh_lease(&refresh_context, &lease)
        .await
        .expect("refresh");
    assert_eq!(refreshed.rotation_generation.get(), 2);
    assert_eq!(client.inspect_calls.load(Ordering::Relaxed), 1);

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

    let repeated = provider
        .revoke_lease(&revoke_context, &refreshed)
        .await
        .expect("idempotent revoke");
    assert_eq!(repeated.state, MutationState::AlreadyApplied);
    assert_eq!(client.revoke_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn owner_mismatch_expiry_and_wrong_input_stop_before_client_use() {
    let (provider, fixture, client, clock) = setup();
    let wrong_input = fixture
        .request_with_input(
            ProviderMethod::CredentialStatus,
            ProviderOperationInput::StorageSnapshot {
                snapshot_id: d2b_contracts::v2_provider::StorageSnapshotId::parse(
                    "not-a-credential-input",
                )
                .expect("snapshot id"),
            },
        )
        .expect("request");
    let context = fixture.call_context(&wrong_input.context);
    assert!(provider.status(&context, &wrong_input).await.is_err());
    assert_eq!(client.state_calls.load(Ordering::Relaxed), 0);

    let mut request = lease_request(&provider, &fixture);
    request.consumer_provider_id = Fixture::new(ProviderType::Audio, 2)
        .expect("other provider")
        .descriptor
        .provider_id;
    let context = fixture.call_context(&request.context);
    let error = provider
        .acquire_lease(&context, &request)
        .await
        .expect_err("wrong owner");
    assert_eq!(error.kind, ProviderFailureKind::UnauthorizedScope);
    assert_eq!(client.issue_calls.load(Ordering::Relaxed), 0);

    let mut request = lease_request(&provider, &fixture);
    request.allowed_operations =
        BoundedVec::new(vec![SdkOperationClass::Delete]).expect("operations");
    let context = fixture.call_context(&request.context);
    assert!(provider.acquire_lease(&context, &request).await.is_err());
    assert_eq!(client.issue_calls.load(Ordering::Relaxed), 0);

    let mut request = lease_request(&provider, &fixture);
    request.context.method = ProviderMethod::CredentialStatus;
    request.context.capability = ProviderCapability(ProviderMethod::CredentialStatus);
    let context = fixture.call_context(&request.context);
    assert!(provider.acquire_lease(&context, &request).await.is_err());
    assert_eq!(client.issue_calls.load(Ordering::Relaxed), 0);

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
        .expect_err("expired");
    assert_eq!(error.kind, ProviderFailureKind::CredentialLeaseInvalid);
    assert_eq!(client.inspect_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn local_lease_table_is_bounded_before_client_use() {
    let (provider, fixture, client, _) = setup();
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
    assert_eq!(client.issue_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn cancellation_deadline_and_errors_have_no_success_fallback() {
    let (provider, fixture, client, _) = setup();
    let request = fixture
        .request(ProviderMethod::CredentialStatus)
        .expect("status request");

    let mut context = fixture.call_context(&request.context);
    context.cancelled = true;
    let error = provider
        .status(&context, &request)
        .await
        .expect_err("cancelled");
    assert_eq!(error.kind, ProviderFailureKind::Cancelled);
    assert_eq!(client.state_calls.load(Ordering::Relaxed), 0);

    client.state_delay_ms.store(20, Ordering::Relaxed);
    let mut context = fixture.call_context(&request.context);
    context.monotonic_deadline_remaining_ms = 1;
    let error = provider
        .status(&context, &request)
        .await
        .expect_err("deadline");
    assert_eq!(error.kind, ProviderFailureKind::DeadlineExpired);
    assert_eq!(error.retry, RetryClass::SameOperation);
}

#[tokio::test]
async fn token_canary_never_crosses_the_provider_surface() {
    let (provider, fixture, client, _) = setup();
    let request = lease_request(&provider, &fixture);
    let context = fixture.call_context(&request.context);
    let lease = provider
        .acquire_lease(&context, &request)
        .await
        .expect("lease");
    let error = EntraClientError::Denied;
    let rendered = format!(
        "{provider:?} {lease:?} {:?} {error:?} {error}",
        provider.descriptor()
    );
    assert!(!rendered.contains(&client.credential_canary));
    assert!(!rendered.contains(&client.endpoint_canary));
    assert!(!rendered.contains("password"));
}
