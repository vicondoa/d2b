use std::sync::Arc;

use d2b_azure_vm_fake_sdk::{
    CallDisposition, ConfiguredOutcome, FakeAzureVmSdk, InfrastructureHandle, ResourceGeneration,
    ResourceId, SdkOperation,
};
use d2b_contracts::{
    v2_component_session::{BoundedVec, EndpointRole, ServicePackage},
    v2_identity::{ProviderType, RealmId, WorkloadId},
    v2_provider::{
        AdoptionRequest, AuthorizedProviderScope, ConfiguredItemId, Fingerprint, Generation,
        HandleId, IdempotencyKey, InfrastructureProvider, MutationState, OperationId,
        PlannedResourceClass, ProviderCallContext, ProviderCapability, ProviderFailureKind,
        ProviderHandleKind, ProviderHealthState, ProviderMethod, ProviderOperationInput,
        ProviderTarget,
    },
};
use d2b_provider_toolkit::{Fixture, ProviderValues, check_descriptor_conformance};

use super::*;

fn fixture_for(target: ProviderTarget) -> Fixture {
    let base = Fixture::new(ProviderType::Infrastructure, 3).unwrap_or_else(|_| unreachable!());
    let mut descriptor = base.descriptor;
    descriptor.implementation_id =
        d2b_contracts::v2_provider::ImplementationId::parse(IMPLEMENTATION_ID)
            .unwrap_or_else(|_| unreachable!());
    Fixture::from_descriptor(descriptor, target, base.now_unix_ms)
        .unwrap_or_else(|_| unreachable!())
}

fn realm_fixture() -> Fixture {
    let base = Fixture::new(ProviderType::Infrastructure, 3).unwrap_or_else(|_| unreachable!());
    fixture_for(ProviderTarget::Realm {
        realm_id: base.descriptor.placement.realm_id().clone(),
    })
}

fn scaffold() -> (
    Arc<AzureVmInfrastructureProvider>,
    Arc<FakeAzureVmSdk>,
    Fixture,
) {
    let fixture = realm_fixture();
    let sdk = Arc::new(FakeAzureVmSdk::new());
    let provider = Arc::new(
        AzureVmInfrastructureProvider::new_for_conformance(
            fixture.descriptor.clone(),
            sdk.clone(),
            fixture.now_unix_ms,
        )
        .unwrap_or_else(|_| unreachable!()),
    );
    (provider, sdk, fixture)
}

async fn create_plan(
    provider: &AzureVmInfrastructureProvider,
    fixture: &Fixture,
) -> AzureVmInfrastructurePlan {
    let request = fixture
        .request(ProviderMethod::InfrastructurePlan)
        .unwrap_or_else(|_| unreachable!());
    let context = fixture.call_context(&request.context);
    provider
        .plan_create(&context, &request)
        .await
        .unwrap_or_else(|_| unreachable!())
}

async fn create_handle(
    provider: &AzureVmInfrastructureProvider,
    fixture: &Fixture,
) -> AzureVmInfrastructureHandle {
    let plan = create_plan(provider, fixture).await;
    let operation = fixture
        .operation(ProviderMethod::InfrastructureApply)
        .unwrap_or_else(|_| unreachable!());
    let context = fixture.call_context(&operation);
    provider
        .create(&context, &plan)
        .await
        .unwrap_or_else(|_| unreachable!())
}

async fn assert_create_rejected(
    provider: &AzureVmInfrastructureProvider,
    context: &ProviderCallContext<'_>,
    plan: &AzureVmInfrastructurePlan,
) {
    let failure = provider
        .create(context, plan)
        .await
        .expect_err("mismatched infrastructure plan must fail");
    assert_eq!(failure.kind, ProviderFailureKind::InvalidRequest);
}

fn handle_fixture(handle: &AzureVmInfrastructureHandle) -> Fixture {
    fixture_for(ProviderTarget::Handle {
        realm_id: handle.provider.realm_id.clone(),
        workload_id: None,
        handle_id: handle.provider.handle_id.clone(),
        handle_generation: handle.provider.resource_generation,
    })
}

fn adoption(fixture: &Fixture, handle: &AzureVmInfrastructureHandle) -> AdoptionRequest {
    AdoptionRequest {
        context: fixture
            .operation(ProviderMethod::InfrastructureAdopt)
            .unwrap_or_else(|_| unreachable!()),
        handle: handle.provider.clone(),
        expected_owner: handle.provider.owner.clone(),
        expected_configuration_fingerprint: handle.provider.configuration_fingerprint.clone(),
        expected_resource_generation: handle.provider.resource_generation,
    }
}

fn synthetic_handle(
    provider: &AzureVmInfrastructureProvider,
    fixture: &Fixture,
    plan: &AzureVmInfrastructurePlan,
) -> AzureVmInfrastructureHandle {
    let sdk = InfrastructureHandle::new(
        ResourceId::new(91).unwrap_or_else(|_| unreachable!()),
        ResourceGeneration::new(1).unwrap_or_else(|_| unreachable!()),
    );
    let values = ProviderValues::new(&provider.descriptor, fixture.now_unix_ms)
        .unwrap_or_else(|_| unreachable!());
    let provider_handle = values
        .handle_from_plan(
            &plan.plan,
            HandleId::parse("azure-vm-infrastructure-synthetic").unwrap_or_else(|_| unreachable!()),
            values.provider_owner(&plan.plan.realm_id),
            Generation::new(1).unwrap_or_else(|_| unreachable!()),
            None,
        )
        .unwrap_or_else(|_| unreachable!());
    AzureVmInfrastructureHandle {
        provider: provider_handle,
        sdk,
    }
}

#[test]
fn descriptor_inventory_is_infrastructure_only_and_production_unavailable() {
    assert_eq!(
        AzureVmInfrastructureProvider::PRODUCTION_AVAILABLE,
        AzureVmInfrastructureProvider::production_registration().is_ok()
    );
    assert!(AzureVmInfrastructureProvider::LIVE_PRODUCTION_CAPABILITIES.is_empty());
    assert_eq!(AzureVmInfrastructureProvider::CONTRACT_METHODS.len(), 7);
    assert!(
        AzureVmInfrastructureProvider::CONTRACT_METHODS
            .iter()
            .all(|method| method.provider_type() == ProviderType::Infrastructure)
    );
    assert!(matches!(
        AzureVmInfrastructureProvider::production_registration(),
        Err(ScaffoldUnavailable::CapabilityUnavailable)
    ));

    let (provider, _, _) = scaffold();
    let instance = provider.conformance_instance();
    let descriptor = check_descriptor_conformance(&instance).unwrap_or_else(|_| unreachable!());
    assert_eq!(descriptor.provider_type(), ProviderType::Infrastructure);
    assert_eq!(
        descriptor.capabilities.as_slice().len(),
        AzureVmInfrastructureProvider::CONTRACT_METHODS.len()
    );
}

#[tokio::test]
async fn normal_dispatch_denies_every_method_without_sdk_work() {
    let (provider, sdk, fixture) = scaffold();
    let plan = create_plan(&provider, &fixture).await;
    let handle = synthetic_handle(&provider, &fixture, &plan);
    let handle_fixture = handle_fixture(&handle);

    let plan_request = fixture
        .request(ProviderMethod::InfrastructurePlan)
        .unwrap_or_else(|_| unreachable!());
    let plan_context = fixture.call_context(&plan_request.context);
    let apply_operation = fixture
        .operation(ProviderMethod::InfrastructureApply)
        .unwrap_or_else(|_| unreachable!());
    let apply_context = fixture.call_context(&apply_operation);
    let power_request = handle_fixture
        .request(ProviderMethod::InfrastructureSetPowerState)
        .unwrap_or_else(|_| unreachable!());
    let power_context = handle_fixture.call_context(&power_request.context);
    let inspect_request = handle_fixture
        .request(ProviderMethod::InfrastructureInspect)
        .unwrap_or_else(|_| unreachable!());
    let inspect_context = handle_fixture.call_context(&inspect_request.context);
    let adoption = adoption(&handle_fixture, &handle);
    let adoption_context = handle_fixture.call_context(&adoption.context);
    let bootstrap_request = handle_fixture
        .request(ProviderMethod::InfrastructureBootstrapBinding)
        .unwrap_or_else(|_| unreachable!());
    let bootstrap_context = handle_fixture.call_context(&bootstrap_request.context);
    let destroy_request = handle_fixture
        .request(ProviderMethod::InfrastructureDestroy)
        .unwrap_or_else(|_| unreachable!());
    let destroy_context = handle_fixture.call_context(&destroy_request.context);

    let failures = [
        InfrastructureProvider::plan(&*provider, &plan_context, &plan_request)
            .await
            .expect_err("plan dispatch must be unavailable"),
        InfrastructureProvider::apply(&*provider, &apply_context, &plan.plan)
            .await
            .expect_err("apply dispatch must be unavailable"),
        InfrastructureProvider::set_power_state(&*provider, &power_context, &power_request)
            .await
            .expect_err("power dispatch must be unavailable"),
        InfrastructureProvider::inspect(&*provider, &inspect_context, &inspect_request)
            .await
            .expect_err("inspect dispatch must be unavailable"),
        InfrastructureProvider::adopt(&*provider, &adoption_context, &adoption)
            .await
            .expect_err("adopt dispatch must be unavailable"),
        InfrastructureProvider::bootstrap_binding(
            &*provider,
            &bootstrap_context,
            &bootstrap_request,
        )
        .await
        .expect_err("bootstrap dispatch must be unavailable"),
        InfrastructureProvider::destroy(&*provider, &destroy_context, &destroy_request)
            .await
            .expect_err("destroy dispatch must be unavailable"),
    ];
    assert!(
        failures
            .iter()
            .all(|failure| failure.kind == ProviderFailureKind::CapabilityDenied)
    );
    let health = provider
        .health(&inspect_context)
        .await
        .unwrap_or_else(|_| unreachable!());
    assert_eq!(health.state, ProviderHealthState::Unavailable);
    assert_eq!(sdk.snapshot().await.total_calls(), 0);
}

#[tokio::test]
async fn wrong_runtime_method_and_input_fail_before_sdk_work() {
    let (provider, sdk, fixture) = scaffold();
    let plan = create_plan(&provider, &fixture).await;
    let handle = synthetic_handle(&provider, &fixture, &plan);
    let handle_fixture = handle_fixture(&handle);

    let mut wrong_method = handle_fixture
        .request(ProviderMethod::InfrastructureInspect)
        .unwrap_or_else(|_| unreachable!());
    wrong_method.context.method = ProviderMethod::RuntimeStart;
    wrong_method.context.capability = ProviderCapability(ProviderMethod::RuntimeStart);
    let wrong_method_context = handle_fixture.call_context(&wrong_method.context);
    let failure = provider
        .inspect_direct(&wrong_method_context, &wrong_method, &handle)
        .await
        .expect_err("runtime method must be denied");
    assert_eq!(failure.kind, ProviderFailureKind::CapabilityDenied);

    let mut wrong_input = handle_fixture
        .request(ProviderMethod::InfrastructureSetPowerState)
        .unwrap_or_else(|_| unreachable!());
    wrong_input.input = ProviderOperationInput::ConfiguredRuntimeExecution {
        configured_item_id: ConfiguredItemId::parse("runtime-item")
            .unwrap_or_else(|_| unreachable!()),
    };
    let wrong_input_context = handle_fixture.call_context(&wrong_input.context);
    let failure = provider
        .set_power_state_direct(&wrong_input_context, &wrong_input, &handle)
        .await
        .expect_err("runtime input must be denied");
    assert_eq!(failure.kind, ProviderFailureKind::CapabilityDenied);
    assert_eq!(sdk.snapshot().await.total_calls(), 0);
}

#[tokio::test]
async fn direct_lifecycle_uses_only_infrastructure_sdk_operations() {
    let (provider, sdk, fixture) = scaffold();
    let handle = create_handle(&provider, &fixture).await;
    let handle_fixture = handle_fixture(&handle);

    let power_request = handle_fixture
        .request(ProviderMethod::InfrastructureSetPowerState)
        .unwrap_or_else(|_| unreachable!());
    provider
        .set_power_state_direct(
            &handle_fixture.call_context(&power_request.context),
            &power_request,
            &handle,
        )
        .await
        .unwrap_or_else(|_| unreachable!());

    let inspect_request = handle_fixture
        .request(ProviderMethod::InfrastructureInspect)
        .unwrap_or_else(|_| unreachable!());
    provider
        .inspect_direct(
            &handle_fixture.call_context(&inspect_request.context),
            &inspect_request,
            &handle,
        )
        .await
        .unwrap_or_else(|_| unreachable!());

    let adoption = adoption(&handle_fixture, &handle);
    provider
        .adopt_direct(
            &handle_fixture.call_context(&adoption.context),
            &adoption,
            &handle,
        )
        .await
        .unwrap_or_else(|_| unreachable!());

    let bootstrap_request = handle_fixture
        .request(ProviderMethod::InfrastructureBootstrapBinding)
        .unwrap_or_else(|_| unreachable!());
    let bootstrap = provider
        .bootstrap_direct(
            &handle_fixture.call_context(&bootstrap_request.context),
            &bootstrap_request,
            &handle,
        )
        .await
        .unwrap_or_else(|_| unreachable!());
    assert_eq!(bootstrap.binding().infrastructure(), handle.sdk_handle());

    let delete_request = handle_fixture
        .request(ProviderMethod::InfrastructureDestroy)
        .unwrap_or_else(|_| unreachable!());
    let receipt = provider
        .delete_direct(
            &handle_fixture.call_context(&delete_request.context),
            &delete_request,
            &handle,
        )
        .await
        .unwrap_or_else(|_| unreachable!());
    assert_eq!(receipt.state, MutationState::Applied);

    let snapshot = sdk.snapshot().await;
    for operation in [
        SdkOperation::InfrastructureCreate,
        SdkOperation::InfrastructureSetPowerState,
        SdkOperation::InfrastructureInspect,
        SdkOperation::InfrastructureAdopt,
        SdkOperation::InfrastructureBootstrap,
        SdkOperation::InfrastructureDelete,
    ] {
        assert_eq!(snapshot.calls(operation), 1, "{operation:?}");
    }
    assert!(
        SdkOperation::ALL
            .iter()
            .filter(|operation| operation.axis() == d2b_azure_vm_fake_sdk::SdkAxis::Runtime)
            .all(|operation| snapshot.calls(*operation) == 0)
    );
}

#[tokio::test]
async fn create_is_idempotent_and_sdk_counters_record_replay() {
    let (provider, sdk, fixture) = scaffold();
    sdk.configure_outcomes(
        SdkOperation::InfrastructureCreate,
        [ConfiguredOutcome::AlreadyApplied],
    )
    .await
    .unwrap_or_else(|_| unreachable!());
    let plan = create_plan(&provider, &fixture).await;
    let operation = fixture
        .operation(ProviderMethod::InfrastructureApply)
        .unwrap_or_else(|_| unreachable!());
    let context = fixture.call_context(&operation);
    let first = provider
        .create(&context, &plan)
        .await
        .unwrap_or_else(|_| unreachable!());
    let replay = provider
        .create(&context, &plan)
        .await
        .unwrap_or_else(|_| unreachable!());
    assert_eq!(first, replay);

    let snapshot = sdk.snapshot().await;
    assert_eq!(snapshot.calls(SdkOperation::InfrastructureCreate), 2);
    assert_eq!(snapshot.log().len(), 2);
    assert_eq!(
        snapshot.log()[0].disposition(),
        CallDisposition::AlreadyApplied
    );
    assert!(snapshot.log()[1].replayed());
}

#[tokio::test]
async fn apply_rejects_cross_operation_cross_workload_and_stale_resource_plans() {
    let (provider, sdk, fixture) = scaffold();
    let plan = create_plan(&provider, &fixture).await;
    let operation = fixture
        .operation(ProviderMethod::InfrastructureApply)
        .unwrap_or_else(|_| unreachable!());
    let context = fixture.call_context(&operation);
    assert_eq!(plan.plan.binding, operation.binding());

    let mut corruptions = Vec::new();

    let mut value = plan.clone();
    value.plan.schema_version = 0;
    corruptions.push(value);

    let mut value = plan.clone();
    value.plan.binding.operation_id =
        OperationId::parse("cross-operation").unwrap_or_else(|_| unreachable!());
    corruptions.push(value);

    let mut value = plan.clone();
    value.plan.binding.idempotency_key =
        IdempotencyKey::parse("cross-idempotency").unwrap_or_else(|_| unreachable!());
    corruptions.push(value);

    let mut value = plan.clone();
    value.plan.binding.request_digest =
        Fingerprint::parse("e".repeat(64)).unwrap_or_else(|_| unreachable!());
    corruptions.push(value);

    let mut value = plan.clone();
    value.plan.binding.provider_id = Fixture::new(ProviderType::Infrastructure, 5)
        .unwrap_or_else(|_| unreachable!())
        .descriptor
        .provider_id;
    corruptions.push(value);

    let mut value = plan.clone();
    value.plan.binding.provider_generation = value
        .plan
        .binding
        .provider_generation
        .next()
        .unwrap_or_else(|_| unreachable!());
    corruptions.push(value);

    let mut value = plan.clone();
    value.plan.realm_id = RealmId::parse("eeeeeeeeeeeeeeeeeeea").unwrap_or_else(|_| unreachable!());
    corruptions.push(value);

    let mut value = plan.clone();
    value.plan.workload_id =
        Some(WorkloadId::parse("fffffffffffffffffffa").unwrap_or_else(|_| unreachable!()));
    corruptions.push(value);

    let mut value = plan.clone();
    value.plan.method = ProviderMethod::RuntimePlan;
    corruptions.push(value);

    let mut value = plan.clone();
    value.plan.resources = BoundedVec::new(vec![PlannedResourceClass::WorkloadExecution])
        .unwrap_or_else(|_| unreachable!());
    corruptions.push(value);

    let mut value = plan.clone();
    value.plan.configuration_fingerprint =
        Fingerprint::parse("d".repeat(64)).unwrap_or_else(|_| unreachable!());
    corruptions.push(value);

    let mut value = plan.clone();
    value.plan.created_at_unix_ms = fixture.now_unix_ms + 1;
    corruptions.push(value);

    let mut value = plan.clone();
    value.plan.expires_at_unix_ms = fixture.now_unix_ms;
    corruptions.push(value);

    let mut value = plan.clone();
    value.plan.expires_at_unix_ms = operation.expires_at_unix_ms + 1;
    corruptions.push(value);

    let mut value = plan.clone();
    value.desired = InfrastructureHandle::new(
        value.desired.identity(),
        ResourceGeneration::new(value.desired.generation().get() + 1)
            .unwrap_or_else(|_| unreachable!()),
    );
    corruptions.push(value);

    for corrupted in &corruptions {
        assert_create_rejected(&provider, &context, corrupted).await;
    }

    let mut cross_workload = operation.clone();
    cross_workload.scope = AuthorizedProviderScope::Workload {
        realm_id: fixture.descriptor.placement.realm_id().clone(),
        workload_id: WorkloadId::parse("fffffffffffffffffffa").unwrap_or_else(|_| unreachable!()),
    };
    assert_create_rejected(&provider, &fixture.call_context(&cross_workload), &plan).await;

    assert_eq!(sdk.snapshot().await.total_calls(), 0);
}

#[tokio::test]
async fn adoption_identity_and_generation_mismatches_stop_before_sdk() {
    let (provider, sdk, fixture) = scaffold();
    let handle = create_handle(&provider, &fixture).await;
    let handle_fixture = handle_fixture(&handle);
    let calls_before = sdk.snapshot().await.total_calls();

    let mut generation_mismatch = adoption(&handle_fixture, &handle);
    generation_mismatch.expected_resource_generation = generation_mismatch
        .expected_resource_generation
        .next()
        .unwrap_or_else(|_| unreachable!());
    let generation_context = handle_fixture.call_context(&generation_mismatch.context);
    let failure = provider
        .adopt_direct(&generation_context, &generation_mismatch, &handle)
        .await
        .expect_err("generation mismatch must fail");
    assert_eq!(failure.kind, ProviderFailureKind::AdoptionRejected);
    assert_eq!(failure.reason, ProviderHealthReason::GenerationMismatch);

    let mut identity_mismatch = adoption(&handle_fixture, &handle);
    identity_mismatch.handle.handle_id =
        HandleId::parse("different-infrastructure").unwrap_or_else(|_| unreachable!());
    let identity_context = handle_fixture.call_context(&identity_mismatch.context);
    let failure = provider
        .adopt_direct(&identity_context, &identity_mismatch, &handle)
        .await
        .expect_err("identity mismatch must fail");
    assert_eq!(failure.kind, ProviderFailureKind::AdoptionRejected);
    assert_eq!(failure.reason, ProviderHealthReason::IdentityMismatch);
    assert_eq!(sdk.snapshot().await.total_calls(), calls_before);
}

#[tokio::test]
async fn cancellation_and_deadline_stop_before_sdk() {
    let (provider, sdk, fixture) = scaffold();
    let handle = create_handle(&provider, &fixture).await;
    let handle_fixture = handle_fixture(&handle);
    let request = handle_fixture
        .request(ProviderMethod::InfrastructureInspect)
        .unwrap_or_else(|_| unreachable!());
    let calls_before = sdk.snapshot().await.total_calls();

    let cancelled = ProviderCallContext {
        operation: &request.context,
        peer_role: EndpointRole::ProviderAgent,
        service: ServicePackage::ProviderV2,
        monotonic_deadline_remaining_ms: 1_000,
        cancelled: true,
    };
    let failure = provider
        .inspect_direct(&cancelled, &request, &handle)
        .await
        .expect_err("cancelled call must fail");
    assert_eq!(failure.kind, ProviderFailureKind::Cancelled);

    let deadline = ProviderCallContext {
        operation: &request.context,
        peer_role: EndpointRole::ProviderAgent,
        service: ServicePackage::ProviderV2,
        monotonic_deadline_remaining_ms: 0,
        cancelled: false,
    };
    let failure = provider
        .inspect_direct(&deadline, &request, &handle)
        .await
        .expect_err("expired call must fail");
    assert_eq!(failure.kind, ProviderFailureKind::DeadlineExpired);
    assert_eq!(sdk.snapshot().await.total_calls(), calls_before);
}

#[tokio::test]
async fn debug_and_errors_redact_operation_identity_and_path_canaries() {
    let (provider, _, fixture) = scaffold();
    let mut request = fixture
        .request(ProviderMethod::InfrastructurePlan)
        .unwrap_or_else(|_| unreachable!());
    request.context.operation_id = d2b_contracts::v2_provider::OperationId::parse("secret-canary")
        .unwrap_or_else(|_| unreachable!());
    request.context.correlation_id =
        d2b_contracts::v2_provider::CorrelationId::parse("home-alice-private")
            .unwrap_or_else(|_| unreachable!());
    let context = fixture.call_context(&request.context);
    let plan = provider
        .plan_create(&context, &request)
        .await
        .unwrap_or_else(|_| unreachable!());
    let failure = InfrastructureProvider::plan(&*provider, &context, &request)
        .await
        .expect_err("normal dispatch must fail closed");

    for rendered in [
        format!("{provider:?}"),
        format!("{plan:?}"),
        format!("{failure:?}"),
        ScaffoldUnavailable::CapabilityUnavailable.to_string(),
    ] {
        assert!(!rendered.contains("secret-canary"));
        assert!(!rendered.contains("home-alice-private"));
        assert!(!rendered.contains("/home/alice/private"));
    }
}

#[test]
fn bound_handle_kind_is_infrastructure() {
    let (provider, _, fixture) = scaffold();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap_or_else(|_| unreachable!());
    let handle = runtime.block_on(create_handle(&provider, &fixture));
    assert_eq!(
        handle.provider_handle().kind,
        ProviderHandleKind::Infrastructure
    );
}
