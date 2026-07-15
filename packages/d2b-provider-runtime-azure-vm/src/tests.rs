use std::sync::Arc;

use d2b_azure_vm_fake_sdk::{
    CallDisposition, ConfiguredOutcome, DeploymentHandle, FakeAzureVmSdk, InfrastructureHandle,
    ResourceGeneration, ResourceId, SdkAxis, SdkOperation,
};
use d2b_contracts::{
    v2_component_session::{BoundedVec, EndpointRole, ServicePackage},
    v2_identity::{ProviderType, RealmId, WorkloadId},
    v2_provider::{
        AdoptionRequest, AuthorizedProviderScope, Fingerprint, Generation, HandleId,
        IdempotencyKey, InfrastructurePowerState, MutationState, OperationId, PlanId,
        PlannedResourceClass, ProviderCallContext, ProviderCapability, ProviderFailureKind,
        ProviderHandleKind, ProviderHealthState, ProviderMethod, ProviderOperationInput,
        ProviderTarget, RuntimeProvider,
    },
};
use d2b_provider_toolkit::{Fixture, ProviderValues, check_descriptor_conformance};

use super::*;

fn infrastructure_binding(realm_fixture: &Fixture) -> BoundInfrastructureHandle {
    let base = Fixture::new(ProviderType::Infrastructure, 9).unwrap_or_else(|_| unreachable!());
    let fixture = Fixture::from_descriptor(
        base.descriptor,
        ProviderTarget::Realm {
            realm_id: realm_fixture.descriptor.placement.realm_id().clone(),
        },
        realm_fixture.now_unix_ms,
    )
    .unwrap_or_else(|_| unreachable!());
    let request = fixture
        .request(ProviderMethod::InfrastructurePlan)
        .unwrap_or_else(|_| unreachable!());
    let values = ProviderValues::new(&fixture.descriptor, fixture.now_unix_ms)
        .unwrap_or_else(|_| unreachable!());
    let plan = values
        .plan(
            &request,
            PlanId::parse("runtime-test-infrastructure-plan").unwrap_or_else(|_| unreachable!()),
            fixture.now_unix_ms + 30_000,
            BoundedVec::new(vec![PlannedResourceClass::Infrastructure])
                .unwrap_or_else(|_| unreachable!()),
        )
        .unwrap_or_else(|_| unreachable!());
    let provider = values
        .handle_from_plan(
            &plan,
            HandleId::parse("runtime-test-infrastructure").unwrap_or_else(|_| unreachable!()),
            values.provider_owner(&plan.realm_id),
            Generation::new(1).unwrap_or_else(|_| unreachable!()),
            None,
        )
        .unwrap_or_else(|_| unreachable!());
    let sdk = InfrastructureHandle::new(
        ResourceId::new(501).unwrap_or_else(|_| unreachable!()),
        ResourceGeneration::new(1).unwrap_or_else(|_| unreachable!()),
    );
    BoundInfrastructureHandle::new(provider, sdk).unwrap_or_else(|_| unreachable!())
}

fn runtime_fixture(infrastructure: &BoundInfrastructureHandle) -> Fixture {
    let base = Fixture::new(ProviderType::Runtime, 4).unwrap_or_else(|_| unreachable!());
    let sample = base
        .request(ProviderMethod::RuntimePlan)
        .unwrap_or_else(|_| unreachable!());
    let workload_id = sample
        .target
        .workload_id()
        .cloned()
        .unwrap_or_else(|| unreachable!());
    let mut descriptor = base.descriptor;
    descriptor.implementation_id =
        d2b_contracts::v2_provider::ImplementationId::parse(IMPLEMENTATION_ID)
            .unwrap_or_else(|_| unreachable!());
    Fixture::from_descriptor(
        descriptor,
        ProviderTarget::Handle {
            realm_id: infrastructure.provider.realm_id.clone(),
            workload_id: Some(workload_id),
            handle_id: infrastructure.provider.handle_id.clone(),
            handle_generation: infrastructure.provider.resource_generation,
        },
        base.now_unix_ms,
    )
    .unwrap_or_else(|_| unreachable!())
}

fn scaffold() -> (
    Arc<AzureVmRuntimeProvider>,
    Arc<FakeAzureVmSdk>,
    Fixture,
    BoundInfrastructureHandle,
) {
    let base = Fixture::new(ProviderType::Runtime, 4).unwrap_or_else(|_| unreachable!());
    let infrastructure = infrastructure_binding(&base);
    let fixture = runtime_fixture(&infrastructure);
    let sdk = Arc::new(FakeAzureVmSdk::new());
    let provider = Arc::new(
        AzureVmRuntimeProvider::new_for_conformance(
            fixture.descriptor.clone(),
            sdk.clone(),
            fixture.now_unix_ms,
        )
        .unwrap_or_else(|_| unreachable!()),
    );
    (provider, sdk, fixture, infrastructure)
}

async fn deployment_plan(
    provider: &AzureVmRuntimeProvider,
    fixture: &Fixture,
    infrastructure: &BoundInfrastructureHandle,
) -> AzureVmRuntimePlan {
    let request = fixture
        .request(ProviderMethod::RuntimePlan)
        .unwrap_or_else(|_| unreachable!());
    provider
        .plan_deployment(
            &fixture.call_context(&request.context),
            &request,
            infrastructure,
        )
        .await
        .unwrap_or_else(|_| unreachable!())
}

async fn runtime_handle(
    provider: &AzureVmRuntimeProvider,
    fixture: &Fixture,
    infrastructure: &BoundInfrastructureHandle,
) -> AzureVmRuntimeHandle {
    let plan = deployment_plan(provider, fixture, infrastructure).await;
    let operation = fixture
        .operation(ProviderMethod::RuntimeEnsure)
        .unwrap_or_else(|_| unreachable!());
    provider
        .deploy(&fixture.call_context(&operation), &plan)
        .await
        .unwrap_or_else(|_| unreachable!())
}

async fn assert_deploy_rejected(
    provider: &AzureVmRuntimeProvider,
    context: &ProviderCallContext<'_>,
    plan: &AzureVmRuntimePlan,
) {
    let failure = provider
        .deploy(context, plan)
        .await
        .expect_err("mismatched runtime plan must fail");
    assert_eq!(failure.kind, ProviderFailureKind::InvalidRequest);
}

fn handle_fixture(handle: &AzureVmRuntimeHandle) -> Fixture {
    let mut descriptor = Fixture::new(ProviderType::Runtime, 4)
        .unwrap_or_else(|_| unreachable!())
        .descriptor;
    descriptor.implementation_id =
        d2b_contracts::v2_provider::ImplementationId::parse(IMPLEMENTATION_ID)
            .unwrap_or_else(|_| unreachable!());
    Fixture::from_descriptor(
        descriptor,
        ProviderTarget::Handle {
            realm_id: handle.provider.realm_id.clone(),
            workload_id: handle.provider.workload_id.clone(),
            handle_id: handle.provider.handle_id.clone(),
            handle_generation: handle.provider.resource_generation,
        },
        1_700_000_000_000,
    )
    .unwrap_or_else(|_| unreachable!())
}

fn adoption(fixture: &Fixture, handle: &AzureVmRuntimeHandle) -> AdoptionRequest {
    AdoptionRequest {
        context: fixture
            .operation(ProviderMethod::RuntimeAdopt)
            .unwrap_or_else(|_| unreachable!()),
        handle: handle.provider.clone(),
        expected_owner: handle.provider.owner.clone(),
        expected_configuration_fingerprint: handle.provider.configuration_fingerprint.clone(),
        expected_resource_generation: handle.provider.resource_generation,
    }
}

fn synthetic_runtime_handle(
    provider: &AzureVmRuntimeProvider,
    fixture: &Fixture,
    plan: &AzureVmRuntimePlan,
) -> AzureVmRuntimeHandle {
    let sdk = DeploymentHandle::new(
        plan.infrastructure.sdk,
        ResourceId::new(601).unwrap_or_else(|_| unreachable!()),
        ResourceGeneration::new(1).unwrap_or_else(|_| unreachable!()),
    );
    let values = ProviderValues::new(&provider.descriptor, fixture.now_unix_ms)
        .unwrap_or_else(|_| unreachable!());
    let provider_handle = values
        .handle_from_plan(
            &plan.plan,
            HandleId::parse("azure-vm-runtime-synthetic").unwrap_or_else(|_| unreachable!()),
            values.provider_owner(&plan.plan.realm_id),
            Generation::new(1).unwrap_or_else(|_| unreachable!()),
            None,
        )
        .unwrap_or_else(|_| unreachable!());
    AzureVmRuntimeHandle {
        provider: provider_handle,
        infrastructure: plan.infrastructure.clone(),
        sdk,
    }
}

#[test]
fn descriptor_inventory_is_runtime_only_and_production_unavailable() {
    assert_eq!(
        AzureVmRuntimeProvider::PRODUCTION_AVAILABLE,
        AzureVmRuntimeProvider::production_registration().is_ok()
    );
    assert!(AzureVmRuntimeProvider::LIVE_PRODUCTION_CAPABILITIES.is_empty());
    assert_eq!(AzureVmRuntimeProvider::CONTRACT_METHODS.len(), 7);
    assert!(
        AzureVmRuntimeProvider::CONTRACT_METHODS
            .iter()
            .all(|method| method.provider_type() == ProviderType::Runtime)
    );
    assert!(!AzureVmRuntimeProvider::CONTRACT_METHODS.contains(&ProviderMethod::RuntimeExecute));
    assert!(matches!(
        AzureVmRuntimeProvider::production_registration(),
        Err(ScaffoldUnavailable::CapabilityUnavailable)
    ));

    let (provider, _, _, _) = scaffold();
    let instance = provider.conformance_instance();
    let descriptor = check_descriptor_conformance(&instance).unwrap_or_else(|_| unreachable!());
    assert_eq!(descriptor.provider_type(), ProviderType::Runtime);
    assert_eq!(
        descriptor.capabilities.as_slice().len(),
        AzureVmRuntimeProvider::CONTRACT_METHODS.len()
    );
}

#[tokio::test]
async fn normal_dispatch_denies_every_method_without_sdk_work() {
    let (provider, sdk, fixture, infrastructure) = scaffold();
    let plan = deployment_plan(&provider, &fixture, &infrastructure).await;
    let handle = synthetic_runtime_handle(&provider, &fixture, &plan);
    let handle_fixture = handle_fixture(&handle);

    let plan_request = fixture
        .request(ProviderMethod::RuntimePlan)
        .unwrap_or_else(|_| unreachable!());
    let plan_context = fixture.call_context(&plan_request.context);
    let ensure_operation = fixture
        .operation(ProviderMethod::RuntimeEnsure)
        .unwrap_or_else(|_| unreachable!());
    let ensure_context = fixture.call_context(&ensure_operation);
    let start_request = handle_fixture
        .request(ProviderMethod::RuntimeStart)
        .unwrap_or_else(|_| unreachable!());
    let start_context = handle_fixture.call_context(&start_request.context);
    let stop_request = handle_fixture
        .request(ProviderMethod::RuntimeStop)
        .unwrap_or_else(|_| unreachable!());
    let stop_context = handle_fixture.call_context(&stop_request.context);
    let inspect_request = handle_fixture
        .request(ProviderMethod::RuntimeInspect)
        .unwrap_or_else(|_| unreachable!());
    let inspect_context = handle_fixture.call_context(&inspect_request.context);
    let adoption = adoption(&handle_fixture, &handle);
    let adoption_context = handle_fixture.call_context(&adoption.context);
    let destroy_request = handle_fixture
        .request(ProviderMethod::RuntimeDestroy)
        .unwrap_or_else(|_| unreachable!());
    let destroy_context = handle_fixture.call_context(&destroy_request.context);

    let failures = [
        RuntimeProvider::plan(&*provider, &plan_context, &plan_request)
            .await
            .expect_err("plan dispatch must be unavailable"),
        RuntimeProvider::ensure(&*provider, &ensure_context, &plan.plan)
            .await
            .expect_err("ensure dispatch must be unavailable"),
        RuntimeProvider::start(&*provider, &start_context, &start_request)
            .await
            .expect_err("start dispatch must be unavailable"),
        RuntimeProvider::stop(&*provider, &stop_context, &stop_request)
            .await
            .expect_err("stop dispatch must be unavailable"),
        RuntimeProvider::inspect(&*provider, &inspect_context, &inspect_request)
            .await
            .expect_err("inspect dispatch must be unavailable"),
        RuntimeProvider::adopt(&*provider, &adoption_context, &adoption)
            .await
            .expect_err("adopt dispatch must be unavailable"),
        RuntimeProvider::destroy(&*provider, &destroy_context, &destroy_request)
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
async fn wrong_infrastructure_method_and_input_fail_before_sdk_work() {
    let (provider, sdk, fixture, infrastructure) = scaffold();
    let plan = deployment_plan(&provider, &fixture, &infrastructure).await;
    let handle = synthetic_runtime_handle(&provider, &fixture, &plan);
    let handle_fixture = handle_fixture(&handle);

    let mut wrong_method = handle_fixture
        .request(ProviderMethod::RuntimeStart)
        .unwrap_or_else(|_| unreachable!());
    wrong_method.context.method = ProviderMethod::InfrastructureSetPowerState;
    wrong_method.context.capability =
        ProviderCapability(ProviderMethod::InfrastructureSetPowerState);
    wrong_method.input = ProviderOperationInput::InfrastructurePowerState {
        state: InfrastructurePowerState::Running,
    };
    let wrong_method_context = handle_fixture.call_context(&wrong_method.context);
    let failure = provider
        .start_direct(&wrong_method_context, &wrong_method, &handle)
        .await
        .expect_err("infrastructure method must be denied");
    assert_eq!(failure.kind, ProviderFailureKind::CapabilityDenied);

    let mut wrong_input = handle_fixture
        .request(ProviderMethod::RuntimeStart)
        .unwrap_or_else(|_| unreachable!());
    wrong_input.input = ProviderOperationInput::InfrastructurePowerState {
        state: InfrastructurePowerState::Stopped,
    };
    let wrong_input_context = handle_fixture.call_context(&wrong_input.context);
    let failure = provider
        .start_direct(&wrong_input_context, &wrong_input, &handle)
        .await
        .expect_err("infrastructure input must be denied");
    assert_eq!(failure.kind, ProviderFailureKind::CapabilityDenied);
    assert_eq!(sdk.snapshot().await.total_calls(), 0);
}

#[tokio::test]
async fn direct_lifecycle_uses_only_runtime_sdk_operations() {
    let (provider, sdk, fixture, infrastructure) = scaffold();
    let handle = runtime_handle(&provider, &fixture, &infrastructure).await;
    let handle_fixture = handle_fixture(&handle);

    let start_request = handle_fixture
        .request(ProviderMethod::RuntimeStart)
        .unwrap_or_else(|_| unreachable!());
    provider
        .start_direct(
            &handle_fixture.call_context(&start_request.context),
            &start_request,
            &handle,
        )
        .await
        .unwrap_or_else(|_| unreachable!());

    let stop_request = handle_fixture
        .request(ProviderMethod::RuntimeStop)
        .unwrap_or_else(|_| unreachable!());
    provider
        .stop_direct(
            &handle_fixture.call_context(&stop_request.context),
            &stop_request,
            &handle,
        )
        .await
        .unwrap_or_else(|_| unreachable!());

    let inspect_request = handle_fixture
        .request(ProviderMethod::RuntimeInspect)
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

    let destroy_request = handle_fixture
        .request(ProviderMethod::RuntimeDestroy)
        .unwrap_or_else(|_| unreachable!());
    let receipt = provider
        .remove_deployment_direct(
            &handle_fixture.call_context(&destroy_request.context),
            &destroy_request,
            &handle,
        )
        .await
        .unwrap_or_else(|_| unreachable!());
    assert_eq!(receipt.state, MutationState::Applied);

    let snapshot = sdk.snapshot().await;
    for operation in [
        SdkOperation::RuntimeDeploy,
        SdkOperation::RuntimeStart,
        SdkOperation::RuntimeStop,
        SdkOperation::RuntimeInspect,
        SdkOperation::RuntimeAdopt,
        SdkOperation::RuntimeRemoveDeployment,
    ] {
        assert_eq!(snapshot.calls(operation), 1, "{operation:?}");
    }
    assert!(
        SdkOperation::ALL
            .iter()
            .filter(|operation| operation.axis() == SdkAxis::Infrastructure)
            .all(|operation| snapshot.calls(*operation) == 0)
    );
}

#[tokio::test]
async fn deployment_is_idempotent_and_sdk_counters_record_replay() {
    let (provider, sdk, fixture, infrastructure) = scaffold();
    sdk.configure_outcomes(
        SdkOperation::RuntimeDeploy,
        [ConfiguredOutcome::AlreadyApplied],
    )
    .await
    .unwrap_or_else(|_| unreachable!());
    let plan = deployment_plan(&provider, &fixture, &infrastructure).await;
    let operation = fixture
        .operation(ProviderMethod::RuntimeEnsure)
        .unwrap_or_else(|_| unreachable!());
    let context = fixture.call_context(&operation);
    let first = provider
        .deploy(&context, &plan)
        .await
        .unwrap_or_else(|_| unreachable!());
    let replay = provider
        .deploy(&context, &plan)
        .await
        .unwrap_or_else(|_| unreachable!());
    assert_eq!(first, replay);

    let snapshot = sdk.snapshot().await;
    assert_eq!(snapshot.calls(SdkOperation::RuntimeDeploy), 2);
    assert_eq!(
        snapshot.log()[0].disposition(),
        CallDisposition::AlreadyApplied
    );
    assert!(snapshot.log()[1].replayed());
}

#[tokio::test]
async fn ensure_rejects_cross_operation_cross_workload_and_stale_resource_plans() {
    let (provider, sdk, fixture, infrastructure) = scaffold();
    let plan = deployment_plan(&provider, &fixture, &infrastructure).await;
    let operation = fixture
        .operation(ProviderMethod::RuntimeEnsure)
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
    value.plan.binding.provider_id = Fixture::new(ProviderType::Runtime, 5)
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
    value.plan.method = ProviderMethod::InfrastructurePlan;
    corruptions.push(value);

    let mut value = plan.clone();
    value.plan.resources = BoundedVec::new(vec![PlannedResourceClass::Infrastructure])
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
    value.desired = DeploymentHandle::new(
        value.desired.infrastructure(),
        value.desired.identity(),
        ResourceGeneration::new(value.desired.generation().get() + 1)
            .unwrap_or_else(|_| unreachable!()),
    );
    corruptions.push(value);

    let mut value = plan.clone();
    value.infrastructure.sdk = InfrastructureHandle::new(
        ResourceId::new(502).unwrap_or_else(|_| unreachable!()),
        value.infrastructure.sdk.generation(),
    );
    corruptions.push(value);

    let mut value = plan.clone();
    value.infrastructure.provider.realm_id =
        RealmId::parse("eeeeeeeeeeeeeeeeeeea").unwrap_or_else(|_| unreachable!());
    corruptions.push(value);

    let mut value = plan.clone();
    value.infrastructure.provider.expires_at_unix_ms = Some(fixture.now_unix_ms);
    corruptions.push(value);

    for corrupted in &corruptions {
        assert_deploy_rejected(&provider, &context, corrupted).await;
    }

    let mut cross_workload = operation.clone();
    cross_workload.scope = AuthorizedProviderScope::Workload {
        realm_id: fixture.descriptor.placement.realm_id().clone(),
        workload_id: WorkloadId::parse("fffffffffffffffffffa").unwrap_or_else(|_| unreachable!()),
    };
    assert_deploy_rejected(&provider, &fixture.call_context(&cross_workload), &plan).await;

    assert_eq!(sdk.snapshot().await.total_calls(), 0);
}

#[tokio::test]
async fn adoption_identity_and_generation_mismatches_stop_before_sdk() {
    let (provider, sdk, fixture, infrastructure) = scaffold();
    let handle = runtime_handle(&provider, &fixture, &infrastructure).await;
    let handle_fixture = handle_fixture(&handle);
    let calls_before = sdk.snapshot().await.total_calls();

    let mut generation_mismatch = adoption(&handle_fixture, &handle);
    generation_mismatch.expected_resource_generation = generation_mismatch
        .expected_resource_generation
        .next()
        .unwrap_or_else(|_| unreachable!());
    let failure = provider
        .adopt_direct(
            &handle_fixture.call_context(&generation_mismatch.context),
            &generation_mismatch,
            &handle,
        )
        .await
        .expect_err("generation mismatch must fail");
    assert_eq!(failure.kind, ProviderFailureKind::AdoptionRejected);
    assert_eq!(failure.reason, ProviderHealthReason::GenerationMismatch);

    let mut identity_mismatch = adoption(&handle_fixture, &handle);
    identity_mismatch.handle.handle_id =
        HandleId::parse("different-runtime").unwrap_or_else(|_| unreachable!());
    let failure = provider
        .adopt_direct(
            &handle_fixture.call_context(&identity_mismatch.context),
            &identity_mismatch,
            &handle,
        )
        .await
        .expect_err("identity mismatch must fail");
    assert_eq!(failure.kind, ProviderFailureKind::AdoptionRejected);
    assert_eq!(failure.reason, ProviderHealthReason::IdentityMismatch);
    assert_eq!(sdk.snapshot().await.total_calls(), calls_before);
}

#[tokio::test]
async fn cancellation_deadline_and_bad_infrastructure_binding_do_no_sdk_work() {
    let (provider, sdk, fixture, infrastructure) = scaffold();
    let handle = runtime_handle(&provider, &fixture, &infrastructure).await;
    let handle_fixture = handle_fixture(&handle);
    let request = handle_fixture
        .request(ProviderMethod::RuntimeInspect)
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

    let mut wrong_generation = infrastructure.provider.clone();
    wrong_generation.resource_generation = wrong_generation
        .resource_generation
        .next()
        .unwrap_or_else(|_| unreachable!());
    assert!(matches!(
        BoundInfrastructureHandle::new(wrong_generation, infrastructure.sdk),
        Err(InfrastructureBindingError::GenerationMismatch)
    ));
    assert_eq!(sdk.snapshot().await.total_calls(), calls_before);
}

#[tokio::test]
async fn debug_and_errors_redact_operation_identity_and_path_canaries() {
    let (provider, _, fixture, infrastructure) = scaffold();
    let mut request = fixture
        .request(ProviderMethod::RuntimePlan)
        .unwrap_or_else(|_| unreachable!());
    request.context.operation_id = d2b_contracts::v2_provider::OperationId::parse("secret-canary")
        .unwrap_or_else(|_| unreachable!());
    request.context.correlation_id =
        d2b_contracts::v2_provider::CorrelationId::parse("home-alice-private")
            .unwrap_or_else(|_| unreachable!());
    let context = fixture.call_context(&request.context);
    let plan = provider
        .plan_deployment(&context, &request, &infrastructure)
        .await
        .unwrap_or_else(|_| unreachable!());
    let failure = RuntimeProvider::plan(&*provider, &context, &request)
        .await
        .expect_err("normal dispatch must fail closed");

    for rendered in [
        format!("{provider:?}"),
        format!("{infrastructure:?}"),
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
fn bound_handle_kinds_preserve_axis_split() {
    let (provider, _, fixture, infrastructure) = scaffold();
    assert_eq!(
        infrastructure.provider_handle().kind,
        ProviderHandleKind::Infrastructure
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap_or_else(|_| unreachable!());
    let handle = runtime.block_on(runtime_handle(&provider, &fixture, &infrastructure));
    assert_eq!(handle.provider_handle().kind, ProviderHandleKind::Runtime);
    assert_eq!(
        handle.infrastructure().sdk_handle(),
        infrastructure.sdk_handle()
    );
}
