use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{EndpointRole, ServicePackage},
    v2_identity::{ProviderId, ProviderType, RealmId, RoleId, WorkloadId},
    v2_provider::{
        AdoptionRequest, AdoptionState, AuthorizedProviderScope, ConfiguredItemId, CorrelationId,
        Fingerprint, Generation, HandleId, HandleOwner, IdempotencyKey, MutationState,
        ObservationReason, ObservedLifecycleState, OperationId, PROVIDER_SCHEMA_VERSION,
        PrincipalRef, ProviderApiVersion, ProviderAuthority, ProviderCallContext,
        ProviderCapability, ProviderCapabilitySet, ProviderDescriptor, ProviderFailureKind,
        ProviderHealthState, ProviderMethod, ProviderOperationContext, ProviderOperationInput,
        ProviderOperationRequest, ProviderPlacement, ProviderTarget, RetryClass, RuntimeProvider,
    },
};
use d2b_provider::{
    CancellationToken, FactoryError, ProviderFactory, ProviderInstance, ProviderRegistryBuilder,
};
use d2b_provider_runtime_local::{
    LocalRuntimeConfiguration, LocalRuntimeConfigurationError, LocalRuntimeKind,
    LocalRuntimeProviderBuildError, LocalRuntimeProviderFactory, LocalRuntimeProviderFactoryEntry,
    MAX_RUNTIME_OPAQUE_ID_BYTES, RuntimeAdoptionControl, RuntimeAdoptionOutcome,
    RuntimeBundleIntentId, RuntimeConfiguredItemControl, RuntimeControlContext,
    RuntimeControlError, RuntimeControlPort, RuntimeEnsureControl, RuntimeHealth,
    RuntimeMutationOutcome, RuntimeObservedState, RuntimeOperationControl, RuntimePlanDecision,
    RuntimeResourceIdentity, RuntimeRunnerId, live_runtime_capabilities,
};
use d2b_provider_toolkit::{DeterministicClock, Fixture, check_provider_conformance};

const NOW: u64 = 1_700_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SeenMethod {
    Health,
    Plan,
    Ensure,
    Start,
    Stop,
    Inspect,
    Adopt,
    Destroy,
    ExecuteConfigured,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SeenCall {
    method: SeenMethod,
    operation: ProviderOperationContext,
    effective_deadline_remaining_ms: u32,
}

struct FakeControl {
    kind: LocalRuntimeKind,
    configuration_fingerprint: Fingerprint,
    calls: Mutex<Vec<SeenCall>>,
    next_error: Mutex<Option<RuntimeControlError>>,
    delay_ms: AtomicU64,
    forge_adopted_generation: AtomicBool,
    last_cancellation: Mutex<Option<CancellationToken>>,
    mutation_count: AtomicU64,
}

impl FakeControl {
    fn new(kind: LocalRuntimeKind, configuration_fingerprint: Fingerprint) -> Self {
        Self {
            kind,
            configuration_fingerprint,
            calls: Mutex::new(Vec::new()),
            next_error: Mutex::new(None),
            delay_ms: AtomicU64::new(0),
            forge_adopted_generation: AtomicBool::new(false),
            last_cancellation: Mutex::new(None),
            mutation_count: AtomicU64::new(0),
        }
    }

    fn record(&self, method: SeenMethod, context: &RuntimeControlContext) {
        *self
            .last_cancellation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some(context.cancellation().clone());
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(SeenCall {
                method,
                operation: context.operation().clone(),
                effective_deadline_remaining_ms: context.effective_deadline_remaining_ms(),
            });
    }

    fn calls(&self) -> Vec<SeenCall> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn call_count(&self) -> usize {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    fn set_delay_ms(&self, delay_ms: u64) {
        self.delay_ms.store(delay_ms, Ordering::Release);
    }

    fn last_cancellation(&self) -> Option<CancellationToken> {
        self.last_cancellation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn mutation_count(&self) -> u64 {
        self.mutation_count.load(Ordering::Acquire)
    }

    fn fail_next(&self, error: RuntimeControlError) {
        *self
            .next_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(error);
    }

    fn take_error(&self) -> Result<(), RuntimeControlError> {
        self.next_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .map_or(Ok(()), Err)
    }

    async fn before_result(
        &self,
        context: &RuntimeControlContext,
        mutation: bool,
    ) -> Result<(), RuntimeControlError> {
        let delay_ms = self.delay_ms.load(Ordering::Acquire);
        if delay_ms != 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
        if context.is_cancelled() {
            return Err(RuntimeControlError::CancelledBeforeMutation);
        }
        self.take_error()?;
        if mutation {
            self.mutation_count.fetch_add(1, Ordering::AcqRel);
        }
        Ok(())
    }

    fn owner(&self, context: &RuntimeControlContext) -> HandleOwner {
        if self.kind.uses_user_agent() {
            HandleOwner::Provider {
                realm_id: context.operation().scope.realm_id().clone(),
                provider_id: context.operation().provider_id.clone(),
            }
        } else {
            HandleOwner::RealmController {
                realm_id: context.operation().scope.realm_id().clone(),
            }
        }
    }

    fn identity(
        &self,
        context: &RuntimeControlContext,
        target: Option<&ProviderTarget>,
    ) -> RuntimeResourceIdentity {
        let (handle_id, resource_generation) = match target {
            Some(ProviderTarget::Handle {
                handle_id,
                handle_generation,
                ..
            }) => (handle_id.clone(), *handle_generation),
            Some(ProviderTarget::Workload { .. }) | Some(ProviderTarget::Realm { .. }) | None => (
                HandleId::parse("runtime-handle").unwrap_or_else(|_| unreachable!()),
                Generation::new(1).unwrap_or_else(|_| unreachable!()),
            ),
        };
        RuntimeResourceIdentity::new(
            self.kind,
            context.operation().provider_id.clone(),
            context.operation().provider_generation,
            context.operation().scope.clone(),
            handle_id,
            self.owner(context),
            resource_generation,
            self.configuration_fingerprint.clone(),
        )
    }

    fn observed(
        &self,
        context: &RuntimeControlContext,
        target: &ProviderTarget,
        lifecycle: ObservedLifecycleState,
    ) -> RuntimeObservedState {
        RuntimeObservedState::new(
            Some(self.identity(context, Some(target))),
            lifecycle,
            ObservationReason::None,
            RuntimeHealth::healthy(),
        )
        .unwrap_or_else(|_| unreachable!())
    }
}

#[async_trait]
impl RuntimeControlPort for FakeControl {
    async fn health(
        &self,
        request: RuntimeControlContext,
    ) -> Result<RuntimeHealth, RuntimeControlError> {
        self.record(SeenMethod::Health, &request);
        self.before_result(&request, false).await?;
        Ok(RuntimeHealth::healthy())
    }

    async fn plan(
        &self,
        request: RuntimeOperationControl,
    ) -> Result<RuntimePlanDecision, RuntimeControlError> {
        self.record(SeenMethod::Plan, request.context());
        self.before_result(request.context(), false).await?;
        Ok(RuntimePlanDecision::new(
            d2b_contracts::v2_provider::PlanId::parse("runtime-plan")
                .unwrap_or_else(|_| unreachable!()),
            request.context().operation().expires_at_unix_ms,
        ))
    }

    async fn ensure(
        &self,
        request: RuntimeEnsureControl,
    ) -> Result<RuntimeResourceIdentity, RuntimeControlError> {
        self.record(SeenMethod::Ensure, request.context());
        self.before_result(request.context(), true).await?;
        Ok(self.identity(request.context(), None))
    }

    async fn start(
        &self,
        request: RuntimeOperationControl,
    ) -> Result<RuntimeObservedState, RuntimeControlError> {
        self.record(SeenMethod::Start, request.context());
        self.before_result(request.context(), true).await?;
        Ok(self.observed(
            request.context(),
            request.target(),
            ObservedLifecycleState::Running,
        ))
    }

    async fn stop(
        &self,
        request: RuntimeOperationControl,
    ) -> Result<RuntimeObservedState, RuntimeControlError> {
        self.record(SeenMethod::Stop, request.context());
        self.before_result(request.context(), true).await?;
        Ok(self.observed(
            request.context(),
            request.target(),
            ObservedLifecycleState::Stopped,
        ))
    }

    async fn inspect(
        &self,
        request: RuntimeOperationControl,
    ) -> Result<RuntimeObservedState, RuntimeControlError> {
        self.record(SeenMethod::Inspect, request.context());
        self.before_result(request.context(), false).await?;
        Ok(self.observed(
            request.context(),
            request.target(),
            ObservedLifecycleState::Ready,
        ))
    }

    async fn adopt(
        &self,
        request: RuntimeAdoptionControl,
    ) -> Result<RuntimeAdoptionOutcome, RuntimeControlError> {
        self.record(SeenMethod::Adopt, request.context());
        self.before_result(request.context(), true).await?;
        let expected = request.expected();
        let generation = if self.forge_adopted_generation.load(Ordering::Acquire) {
            expected
                .resource_generation()
                .next()
                .unwrap_or_else(|_| unreachable!())
        } else {
            expected.resource_generation()
        };
        let identity = RuntimeResourceIdentity::new(
            expected.kind(),
            expected.provider_id().clone(),
            expected.provider_generation(),
            expected.scope().clone(),
            expected.handle_id().clone(),
            expected.owner().clone(),
            generation,
            expected.configuration_fingerprint().clone(),
        );
        Ok(RuntimeAdoptionOutcome::Adopted(Box::new(
            RuntimeObservedState::new(
                Some(identity),
                ObservedLifecycleState::Running,
                ObservationReason::None,
                RuntimeHealth::healthy(),
            )
            .unwrap_or_else(|_| unreachable!()),
        )))
    }

    async fn destroy(
        &self,
        request: RuntimeOperationControl,
    ) -> Result<RuntimeMutationOutcome, RuntimeControlError> {
        self.record(SeenMethod::Destroy, request.context());
        self.before_result(request.context(), true).await?;
        Ok(RuntimeMutationOutcome::new(MutationState::Applied))
    }

    async fn execute_configured_item(
        &self,
        request: RuntimeConfiguredItemControl,
    ) -> Result<RuntimeObservedState, RuntimeControlError> {
        self.record(SeenMethod::ExecuteConfigured, request.context());
        self.before_result(request.context(), true).await?;
        RuntimeObservedState::new(
            Some(self.identity(request.context(), None)),
            ObservedLifecycleState::Running,
            ObservationReason::None,
            RuntimeHealth::healthy(),
        )
        .map_err(|_| RuntimeControlError::InvariantViolation)
    }
}

fn fingerprint(value: u8) -> Fingerprint {
    Fingerprint::parse(format!("{value:064x}")).unwrap_or_else(|_| unreachable!())
}

fn realm_id() -> RealmId {
    RealmId::parse("aaaaaaaaaaaaaaaaaaaa").unwrap_or_else(|_| unreachable!())
}

fn workload_id() -> WorkloadId {
    WorkloadId::parse("bbbbbbbbbbbbbbbbbbba").unwrap_or_else(|_| unreachable!())
}

fn role_id() -> RoleId {
    RoleId::parse("ccccccccccccccccccca").unwrap_or_else(|_| unreachable!())
}

fn provider_id() -> ProviderId {
    ProviderId::parse("ddddddddddddddddddda").unwrap_or_else(|_| unreachable!())
}

fn second_provider_id() -> ProviderId {
    ProviderId::parse("eeeeeeeeeeeeeeeeeeea").unwrap_or_else(|_| unreachable!())
}

fn bundle_intent_id(kind: LocalRuntimeKind) -> RuntimeBundleIntentId {
    RuntimeBundleIntentId::parse(format!("intent:{}", kind.implementation_id()))
        .unwrap_or_else(|_| unreachable!())
}

fn runner_id(kind: LocalRuntimeKind) -> RuntimeRunnerId {
    RuntimeRunnerId::parse(format!("runner:{}", kind.implementation_id()))
        .unwrap_or_else(|_| unreachable!())
}

fn configuration(kind: LocalRuntimeKind) -> LocalRuntimeConfiguration {
    match kind {
        LocalRuntimeKind::CloudHypervisor => {
            LocalRuntimeConfiguration::cloud_hypervisor(bundle_intent_id(kind), runner_id(kind))
        }
        LocalRuntimeKind::QemuMedia => {
            LocalRuntimeConfiguration::qemu_media(bundle_intent_id(kind), runner_id(kind))
        }
        LocalRuntimeKind::SystemdUser => {
            LocalRuntimeConfiguration::systemd_user(bundle_intent_id(kind), runner_id(kind))
        }
    }
}

fn descriptor(kind: LocalRuntimeKind) -> ProviderDescriptor {
    ProviderDescriptor {
        schema_version: PROVIDER_SCHEMA_VERSION,
        provider_id: provider_id(),
        authority: ProviderAuthority::Runtime {
            posture: kind.authority_posture(),
        },
        implementation_id: kind
            .canonical_implementation_id()
            .unwrap_or_else(|_| unreachable!()),
        api_version: ProviderApiVersion::V2,
        capabilities: live_runtime_capabilities().unwrap_or_else(|_| unreachable!()),
        configuration_schema_fingerprint: fingerprint(1),
        configured_scope_digest: fingerprint(2),
        registry_generation: Generation::new(4).unwrap_or_else(|_| unreachable!()),
        placement: if kind.uses_user_agent() {
            ProviderPlacement::UserAgent {
                realm_id: realm_id(),
                role_id: role_id(),
                endpoint_role: EndpointRole::UserAgent,
                service: ServicePackage::UserV2,
                agent_generation: Generation::new(8).unwrap_or_else(|_| unreachable!()),
            }
        } else {
            ProviderPlacement::TrustedFirstPartyInProcess {
                realm_id: realm_id(),
                controller_role: EndpointRole::RealmController,
            }
        },
    }
}

struct Harness {
    provider: Arc<dyn RuntimeProvider>,
    control: Arc<FakeControl>,
    fixture: Fixture,
    configuration: LocalRuntimeConfiguration,
    clock: Arc<DeterministicClock>,
}

fn harness(kind: LocalRuntimeKind) -> Harness {
    let descriptor = descriptor(kind);
    let configuration = configuration(kind);
    let target = ProviderTarget::Workload {
        realm_id: realm_id(),
        workload_id: workload_id(),
    };
    let fixture = Fixture::from_descriptor(descriptor.clone(), target, NOW)
        .unwrap_or_else(|_| unreachable!());
    let control = Arc::new(FakeControl::new(
        kind,
        descriptor.configuration_schema_fingerprint.clone(),
    ));
    let clock = Arc::new(DeterministicClock::new(NOW));
    let entry = factory_entry(&descriptor, configuration.clone(), control.clone());
    let factory = LocalRuntimeProviderFactory::with_clock(kind, vec![entry], clock.clone())
        .unwrap_or_else(|_| unreachable!());
    let provider = match factory
        .construct(&descriptor)
        .unwrap_or_else(|_| unreachable!())
    {
        ProviderInstance::Runtime(provider) => provider,
        _ => unreachable!(),
    };
    Harness {
        provider,
        control,
        fixture,
        configuration,
        clock,
    }
}

fn named_factory(
    kind: LocalRuntimeKind,
    entries: Vec<LocalRuntimeProviderFactoryEntry>,
) -> LocalRuntimeProviderFactory {
    match kind {
        LocalRuntimeKind::CloudHypervisor => LocalRuntimeProviderFactory::cloud_hypervisor(entries),
        LocalRuntimeKind::QemuMedia => LocalRuntimeProviderFactory::qemu_media(entries),
        LocalRuntimeKind::SystemdUser => LocalRuntimeProviderFactory::systemd_user(entries),
    }
    .unwrap_or_else(|_| unreachable!())
}

fn factory_entry(
    descriptor: &ProviderDescriptor,
    configuration: LocalRuntimeConfiguration,
    control: Arc<dyn RuntimeControlPort>,
) -> LocalRuntimeProviderFactoryEntry {
    LocalRuntimeProviderFactoryEntry::new(
        descriptor.provider_id.clone(),
        configuration,
        descriptor.configuration_schema_fingerprint.clone(),
        descriptor.configured_scope_digest.clone(),
        control,
    )
}

fn operation_request(fixture: &Fixture, method: ProviderMethod) -> ProviderOperationRequest {
    fixture.request(method).unwrap_or_else(|_| unreachable!())
}

fn context<'a>(
    fixture: &Fixture,
    operation: &'a ProviderOperationContext,
) -> ProviderCallContext<'a> {
    fixture.call_context(operation)
}

async fn ensured_handle(harness: &Harness) -> d2b_contracts::v2_provider::ProviderHandle {
    let plan_request = operation_request(&harness.fixture, ProviderMethod::RuntimePlan);
    let plan_context = context(&harness.fixture, &plan_request.context);
    let plan = harness
        .provider
        .plan(&plan_context, &plan_request)
        .await
        .unwrap_or_else(|_| unreachable!());
    let ensure_operation = harness
        .fixture
        .operation(ProviderMethod::RuntimeEnsure)
        .unwrap_or_else(|_| unreachable!());
    let ensure_context = context(&harness.fixture, &ensure_operation);
    harness
        .provider
        .ensure(&ensure_context, &plan)
        .await
        .unwrap_or_else(|_| unreachable!())
}

#[test]
fn descriptors_are_exact_for_all_closed_runtime_kinds() {
    for kind in [
        LocalRuntimeKind::CloudHypervisor,
        LocalRuntimeKind::QemuMedia,
        LocalRuntimeKind::SystemdUser,
    ] {
        let harness = harness(kind);
        let actual = harness.provider.descriptor();
        assert_eq!(actual.implementation_id.as_str(), kind.implementation_id());
        assert!(
            actual.authority
                == ProviderAuthority::Runtime {
                    posture: kind.authority_posture()
                }
        );
        assert_eq!(
            harness.provider.capabilities(),
            live_runtime_capabilities().unwrap_or_else(|_| unreachable!())
        );
        assert!(
            !actual
                .capabilities
                .contains_method(ProviderMethod::RuntimeExecute)
        );
        assert_eq!(
            matches!(actual.placement, ProviderPlacement::UserAgent { .. }),
            kind.uses_user_agent()
        );
        assert_eq!(harness.configuration.kind(), kind);
    }
}

#[test]
fn named_factories_publish_canonical_keys_and_runtime_instances() {
    for kind in [
        LocalRuntimeKind::CloudHypervisor,
        LocalRuntimeKind::QemuMedia,
        LocalRuntimeKind::SystemdUser,
    ] {
        let control = Arc::new(FakeControl::new(kind, fingerprint(1)));
        let expected_descriptor = descriptor(kind);
        let entry = factory_entry(&expected_descriptor, configuration(kind), control);
        let factory = named_factory(kind, vec![entry]);
        let expected_key = kind.factory_key().unwrap_or_else(|_| unreachable!());
        assert_eq!(factory.key(), expected_key);
        assert_eq!(
            factory.implementation_id(),
            &kind
                .canonical_implementation_id()
                .unwrap_or_else(|_| unreachable!())
        );
        let instance = factory
            .construct(&expected_descriptor)
            .unwrap_or_else(|_| unreachable!());
        assert!(matches!(instance, ProviderInstance::Runtime(_)));
        assert_eq!(instance.descriptor(), expected_descriptor);
    }
}

#[test]
fn factory_rejects_wrong_descriptor_type_and_implementation() {
    let kind = LocalRuntimeKind::CloudHypervisor;
    let control = Arc::new(FakeControl::new(kind, fingerprint(1)));
    let expected = descriptor(kind);
    let factory = named_factory(
        kind,
        vec![factory_entry(&expected, configuration(kind), control)],
    );

    let mut wrong_type = descriptor(kind);
    wrong_type.authority = ProviderAuthority::Storage;
    assert_eq!(
        factory.construct(&wrong_type).err(),
        Some(FactoryError::Rejected)
    );

    let mut wrong_implementation = descriptor(kind);
    wrong_implementation.implementation_id = LocalRuntimeKind::QemuMedia
        .canonical_implementation_id()
        .unwrap_or_else(|_| unreachable!());
    assert_eq!(
        factory.construct(&wrong_implementation).err(),
        Some(FactoryError::Rejected)
    );
}

#[test]
fn factory_entries_bind_provider_id_fingerprints_scope_and_runtime_kind() {
    let kind = LocalRuntimeKind::CloudHypervisor;
    let expected = descriptor(kind);
    let control = Arc::new(FakeControl::new(kind, fingerprint(1)));
    let entry = factory_entry(&expected, configuration(kind), control);
    let factory = named_factory(kind, vec![entry.clone()]);

    let mut unknown_provider = expected.clone();
    unknown_provider.provider_id = second_provider_id();
    assert_eq!(
        factory.construct(&unknown_provider).err(),
        Some(FactoryError::Rejected)
    );

    let mut wrong_configuration_fingerprint = expected.clone();
    wrong_configuration_fingerprint.configuration_schema_fingerprint = fingerprint(8);
    assert_eq!(
        factory.construct(&wrong_configuration_fingerprint).err(),
        Some(FactoryError::Rejected)
    );

    let mut wrong_scope_digest = expected.clone();
    wrong_scope_digest.configured_scope_digest = fingerprint(8);
    assert_eq!(
        factory.construct(&wrong_scope_digest).err(),
        Some(FactoryError::Rejected)
    );

    assert!(matches!(
        LocalRuntimeProviderFactory::cloud_hypervisor(Vec::new()),
        Err(LocalRuntimeProviderBuildError::FactoryEntriesEmpty)
    ));
    assert!(matches!(
        LocalRuntimeProviderFactory::cloud_hypervisor(vec![entry.clone(), entry]),
        Err(LocalRuntimeProviderBuildError::DuplicateProviderEntry)
    ));

    let wrong_kind_entry = factory_entry(
        &expected,
        configuration(LocalRuntimeKind::QemuMedia),
        Arc::new(FakeControl::new(
            LocalRuntimeKind::QemuMedia,
            fingerprint(1),
        )),
    );
    assert!(matches!(
        LocalRuntimeProviderFactory::cloud_hypervisor(vec![wrong_kind_entry]),
        Err(LocalRuntimeProviderBuildError::RuntimeKindMismatch)
    ));
}

#[tokio::test]
async fn one_factory_routes_each_provider_id_to_its_bound_control_port() {
    let kind = LocalRuntimeKind::CloudHypervisor;
    let first_descriptor = descriptor(kind);
    let mut second_descriptor = descriptor(kind);
    second_descriptor.provider_id = second_provider_id();
    second_descriptor.configuration_schema_fingerprint = fingerprint(3);
    second_descriptor.configured_scope_digest = fingerprint(4);

    let first_control = Arc::new(FakeControl::new(kind, fingerprint(1)));
    let second_control = Arc::new(FakeControl::new(kind, fingerprint(3)));
    let factory = LocalRuntimeProviderFactory::with_clock(
        kind,
        vec![
            factory_entry(
                &first_descriptor,
                configuration(kind),
                first_control.clone(),
            ),
            factory_entry(
                &second_descriptor,
                configuration(kind),
                second_control.clone(),
            ),
        ],
        Arc::new(DeterministicClock::new(NOW)),
    )
    .unwrap_or_else(|_| unreachable!());

    for descriptor in [&first_descriptor, &second_descriptor] {
        let provider = match factory
            .construct(descriptor)
            .unwrap_or_else(|_| unreachable!())
        {
            ProviderInstance::Runtime(provider) => provider,
            _ => unreachable!(),
        };
        let fixture = Fixture::from_descriptor(
            descriptor.clone(),
            ProviderTarget::Workload {
                realm_id: realm_id(),
                workload_id: workload_id(),
            },
            NOW,
        )
        .unwrap_or_else(|_| unreachable!());
        let request = operation_request(&fixture, ProviderMethod::RuntimeInspect);
        let call_context = context(&fixture, &request.context);
        provider
            .inspect(&call_context, &request)
            .await
            .unwrap_or_else(|_| unreachable!());
    }

    assert_eq!(first_control.call_count(), 1);
    assert_eq!(second_control.call_count(), 1);
}

#[test]
fn factory_registers_directly_with_provider_registry_builder() {
    let kind = LocalRuntimeKind::CloudHypervisor;
    let control = Arc::new(FakeControl::new(kind, fingerprint(1)));
    let expected_descriptor = descriptor(kind);
    let factory = named_factory(
        kind,
        vec![factory_entry(
            &expected_descriptor,
            configuration(kind),
            control,
        )],
    );
    let key = factory.key();
    let mut builder =
        ProviderRegistryBuilder::new(expected_descriptor.registry_generation, fingerprint(9), NOW);
    builder
        .register_factory(key, Arc::new(factory))
        .unwrap_or_else(|_| unreachable!());
    builder
        .register_instance(expected_descriptor.clone())
        .unwrap_or_else(|_| unreachable!());
    let registry = builder.finish().unwrap_or_else(|_| unreachable!());
    assert_eq!(
        registry
            .snapshot()
            .descriptor(&expected_descriptor.provider_id),
        Some(&expected_descriptor)
    );
}

#[tokio::test]
async fn all_closed_runtime_kinds_pass_common_conformance() {
    for kind in [
        LocalRuntimeKind::CloudHypervisor,
        LocalRuntimeKind::QemuMedia,
        LocalRuntimeKind::SystemdUser,
    ] {
        let harness = harness(kind);
        let instance = ProviderInstance::Runtime(harness.provider.clone());
        check_provider_conformance(&instance, &harness.fixture)
            .await
            .unwrap_or_else(|_| unreachable!());
    }
}

#[test]
fn closed_configurations_bind_only_opaque_bundle_intents_and_runners() {
    for kind in [
        LocalRuntimeKind::CloudHypervisor,
        LocalRuntimeKind::QemuMedia,
        LocalRuntimeKind::SystemdUser,
    ] {
        let configuration = configuration(kind);
        assert_eq!(configuration.kind(), kind);
        assert_eq!(
            configuration.intent_binding().bundle_intent_id().as_str(),
            format!("intent:{}", kind.implementation_id())
        );
        assert_eq!(
            configuration.intent_binding().runner_id().as_str(),
            format!("runner:{}", kind.implementation_id())
        );
        let debug = format!("{configuration:?}");
        assert!(!debug.contains("intent:"));
        assert!(!debug.contains("runner:"));
    }

    for invalid in ["", "UPPERCASE", "/host/path", "contains space"] {
        assert_eq!(
            RuntimeBundleIntentId::parse(invalid).err(),
            Some(LocalRuntimeConfigurationError::InvalidOpaqueIdentifier)
        );
        assert_eq!(
            RuntimeRunnerId::parse(invalid).err(),
            Some(LocalRuntimeConfigurationError::InvalidOpaqueIdentifier)
        );
    }
    assert_eq!(
        RuntimeRunnerId::parse("a".repeat(MAX_RUNTIME_OPAQUE_ID_BYTES + 1)).err(),
        Some(LocalRuntimeConfigurationError::InvalidOpaqueIdentifier)
    );
    assert!(RuntimeRunnerId::parse("7runner").is_ok());
}

#[test]
fn factories_accept_exact_placement_and_reject_cross_placement_or_capability_drift() {
    for kind in [
        LocalRuntimeKind::CloudHypervisor,
        LocalRuntimeKind::QemuMedia,
        LocalRuntimeKind::SystemdUser,
    ] {
        let exact = descriptor(kind);
        let control = Arc::new(FakeControl::new(kind, fingerprint(1)));
        let factory = named_factory(
            kind,
            vec![factory_entry(&exact, configuration(kind), control.clone())],
        );
        assert!(factory.construct(&exact).is_ok());

        let mut wrong_placement = exact.clone();
        wrong_placement.placement = if kind.uses_user_agent() {
            ProviderPlacement::TrustedFirstPartyInProcess {
                realm_id: realm_id(),
                controller_role: EndpointRole::RealmController,
            }
        } else {
            ProviderPlacement::UserAgent {
                realm_id: realm_id(),
                role_id: role_id(),
                endpoint_role: EndpointRole::UserAgent,
                service: ServicePackage::UserV2,
                agent_generation: Generation::new(8).unwrap_or_else(|_| unreachable!()),
            }
        };
        assert_eq!(
            factory.construct(&wrong_placement).err(),
            Some(FactoryError::Rejected)
        );
        if kind.uses_user_agent() {
            let mut provider_agent = exact.clone();
            provider_agent.placement = ProviderPlacement::ProviderAgent {
                realm_id: realm_id(),
                workload_id: workload_id(),
                role_id: role_id(),
                endpoint_role: EndpointRole::ProviderAgent,
                service: ServicePackage::ProviderV2,
                agent_generation: Generation::new(8).unwrap_or_else(|_| unreachable!()),
            };
            assert_eq!(
                factory.construct(&provider_agent).err(),
                Some(FactoryError::Rejected)
            );
        }
    }

    let kind = LocalRuntimeKind::SystemdUser;
    let exact = descriptor(kind);
    let factory = named_factory(
        kind,
        vec![factory_entry(
            &exact,
            configuration(kind),
            Arc::new(FakeControl::new(kind, fingerprint(1))),
        )],
    );
    let mut wrong_capabilities = exact;
    let capabilities = live_runtime_capabilities().unwrap_or_else(|_| unreachable!());
    let mut capabilities = capabilities.as_slice().to_vec();
    capabilities.push(ProviderCapability(ProviderMethod::RuntimeExecute));
    wrong_capabilities.capabilities =
        ProviderCapabilitySet::new(capabilities).unwrap_or_else(|_| unreachable!());
    assert_eq!(
        factory.construct(&wrong_capabilities).err(),
        Some(FactoryError::Rejected)
    );
}

#[tokio::test]
async fn every_advertised_method_maps_through_the_semantic_port() {
    let harness = harness(LocalRuntimeKind::SystemdUser);

    let health_operation = harness
        .fixture
        .operation(ProviderMethod::RuntimeInspect)
        .unwrap_or_else(|_| unreachable!());
    let health_context = context(&harness.fixture, &health_operation);
    let health = harness
        .provider
        .health(&health_context)
        .await
        .unwrap_or_else(|_| unreachable!());
    assert_eq!(health.state, ProviderHealthState::Healthy);

    let handle = ensured_handle(&harness).await;

    for (method, lifecycle) in [
        (
            ProviderMethod::RuntimeStart,
            ObservedLifecycleState::Running,
        ),
        (ProviderMethod::RuntimeStop, ObservedLifecycleState::Stopped),
        (
            ProviderMethod::RuntimeInspect,
            ObservedLifecycleState::Ready,
        ),
    ] {
        let request = operation_request(&harness.fixture, method);
        let call_context = context(&harness.fixture, &request.context);
        let observation = match method {
            ProviderMethod::RuntimeStart => harness.provider.start(&call_context, &request).await,
            ProviderMethod::RuntimeStop => harness.provider.stop(&call_context, &request).await,
            ProviderMethod::RuntimeInspect => {
                harness.provider.inspect(&call_context, &request).await
            }
            _ => unreachable!(),
        }
        .unwrap_or_else(|_| unreachable!());
        assert_eq!(observation.lifecycle, lifecycle);
        assert_eq!(observation.adoption, AdoptionState::NotAttempted);
        assert!(observation.handle_id.is_some());
    }

    let adopt_operation = harness
        .fixture
        .operation(ProviderMethod::RuntimeAdopt)
        .unwrap_or_else(|_| unreachable!());
    let adopt_request = AdoptionRequest {
        context: adopt_operation.clone(),
        expected_owner: handle.owner.clone(),
        expected_configuration_fingerprint: handle.configuration_fingerprint.clone(),
        expected_resource_generation: handle.resource_generation,
        handle,
    };
    let adopt_context = context(&harness.fixture, &adopt_operation);
    let adopted = harness
        .provider
        .adopt(&adopt_context, &adopt_request)
        .await
        .unwrap_or_else(|_| unreachable!());
    assert_eq!(adopted.adoption, AdoptionState::Adopted);

    let destroy_request = operation_request(&harness.fixture, ProviderMethod::RuntimeDestroy);
    let destroy_context = context(&harness.fixture, &destroy_request.context);
    let receipt = harness
        .provider
        .destroy(&destroy_context, &destroy_request)
        .await
        .unwrap_or_else(|_| unreachable!());
    assert_eq!(receipt.state, MutationState::Applied);

    assert_eq!(
        harness
            .control
            .calls()
            .into_iter()
            .map(|call| call.method)
            .collect::<Vec<_>>(),
        vec![
            SeenMethod::Health,
            SeenMethod::Plan,
            SeenMethod::Ensure,
            SeenMethod::Start,
            SeenMethod::Stop,
            SeenMethod::Inspect,
            SeenMethod::Adopt,
            SeenMethod::Destroy,
        ]
    );
}

#[tokio::test]
async fn validation_denials_never_invoke_control() {
    let harness = harness(LocalRuntimeKind::CloudHypervisor);
    let mut wrong_input = operation_request(&harness.fixture, ProviderMethod::RuntimeStart);
    wrong_input.input = ProviderOperationInput::ConfiguredRuntimeExecution {
        configured_item_id: ConfiguredItemId::parse("configured-editor")
            .unwrap_or_else(|_| unreachable!()),
    };
    let wrong_input_context = context(&harness.fixture, &wrong_input.context);
    let failure = harness
        .provider
        .start(&wrong_input_context, &wrong_input)
        .await
        .expect_err("wrong input must fail");
    assert_eq!(failure.kind, ProviderFailureKind::InvalidRequest);

    let wrong_method = operation_request(&harness.fixture, ProviderMethod::RuntimeStop);
    let wrong_method_context = context(&harness.fixture, &wrong_method.context);
    let failure = harness
        .provider
        .start(&wrong_method_context, &wrong_method)
        .await
        .expect_err("wrong method must fail");
    assert_eq!(failure.kind, ProviderFailureKind::CapabilityDenied);

    let execute = operation_request(&harness.fixture, ProviderMethod::RuntimeExecute);
    let execute_context = context(&harness.fixture, &execute.context);
    let failure = harness
        .provider
        .start(&execute_context, &execute)
        .await
        .expect_err("unadvertised execution must fail");
    assert_eq!(failure.kind, ProviderFailureKind::CapabilityDenied);
    assert_eq!(harness.control.call_count(), 0);
}

#[tokio::test]
async fn runtime_kinds_reject_cross_placement_callers_before_control() {
    for kind in [
        LocalRuntimeKind::CloudHypervisor,
        LocalRuntimeKind::QemuMedia,
        LocalRuntimeKind::SystemdUser,
    ] {
        let harness = harness(kind);
        let request = operation_request(&harness.fixture, ProviderMethod::RuntimeInspect);
        let mut call_context = context(&harness.fixture, &request.context);
        (call_context.peer_role, call_context.service) = if kind.uses_user_agent() {
            (EndpointRole::RealmController, ServicePackage::ProviderV2)
        } else {
            (EndpointRole::UserAgent, ServicePackage::UserV2)
        };
        let failure = harness
            .provider
            .inspect(&call_context, &request)
            .await
            .expect_err("cross-placement caller must fail");
        assert_eq!(failure.kind, ProviderFailureKind::UnauthorizedScope);
        assert_eq!(harness.control.call_count(), 0);
    }

    let harness = harness(LocalRuntimeKind::SystemdUser);
    let request = operation_request(&harness.fixture, ProviderMethod::RuntimeInspect);
    let mut provider_agent = context(&harness.fixture, &request.context);
    provider_agent.peer_role = EndpointRole::ProviderAgent;
    provider_agent.service = ServicePackage::ProviderV2;
    let failure = harness
        .provider
        .inspect(&provider_agent, &request)
        .await
        .expect_err("systemd-user must not require the provider agent");
    assert_eq!(failure.kind, ProviderFailureKind::UnauthorizedScope);
    assert_eq!(harness.control.call_count(), 0);
}

#[tokio::test]
async fn ensure_rejects_a_plan_from_another_operation_binding() {
    let harness = harness(LocalRuntimeKind::CloudHypervisor);
    let plan_request = operation_request(&harness.fixture, ProviderMethod::RuntimePlan);
    let plan_context = context(&harness.fixture, &plan_request.context);
    let plan = harness
        .provider
        .plan(&plan_context, &plan_request)
        .await
        .unwrap_or_else(|_| unreachable!());
    assert_eq!(harness.control.call_count(), 1);

    let mut ensure_operation = harness
        .fixture
        .operation(ProviderMethod::RuntimeEnsure)
        .unwrap_or_else(|_| unreachable!());
    ensure_operation.operation_id =
        OperationId::parse("different-operation").unwrap_or_else(|_| unreachable!());
    let ensure_context = context(&harness.fixture, &ensure_operation);
    let failure = harness
        .provider
        .ensure(&ensure_context, &plan)
        .await
        .expect_err("foreign plan binding must fail");
    assert_eq!(failure.kind, ProviderFailureKind::InvalidRequest);
    assert_eq!(harness.control.call_count(), 1);
}

#[tokio::test]
async fn authorized_role_scope_and_idempotency_are_preserved_exactly() {
    let harness = harness(LocalRuntimeKind::SystemdUser);
    let mut request = operation_request(&harness.fixture, ProviderMethod::RuntimeInspect);
    request.context.scope = AuthorizedProviderScope::WorkloadRole {
        realm_id: realm_id(),
        workload_id: workload_id(),
        role_id: role_id(),
    };
    let operation = request.context.clone();
    let call_context = context(&harness.fixture, &operation);
    for _ in 0..2 {
        harness
            .provider
            .inspect(&call_context, &request)
            .await
            .unwrap_or_else(|_| unreachable!());
    }
    let calls = harness.control.calls();
    assert_eq!(calls.len(), 2);
    for call in &calls {
        assert!(call.operation.scope == operation.scope);
        assert_eq!(call.operation.operation_id, operation.operation_id);
        assert_eq!(call.operation.idempotency_key, operation.idempotency_key);
        assert_eq!(call.operation.request_digest, operation.request_digest);
    }
    assert_eq!(calls[0].operation, calls[1].operation);
}

#[tokio::test]
async fn semantic_control_failures_map_to_bounded_canonical_failures() {
    let harness = harness(LocalRuntimeKind::SystemdUser);
    harness.control.fail_next(RuntimeControlError::Unavailable);
    let request = operation_request(&harness.fixture, ProviderMethod::RuntimeInspect);
    let call_context = context(&harness.fixture, &request.context);
    let failure = harness
        .provider
        .inspect(&call_context, &request)
        .await
        .expect_err("unavailable control must fail");
    assert_eq!(failure.kind, ProviderFailureKind::Unavailable);
    assert_eq!(failure.retry, RetryClass::SameOperation);
    failure
        .validate_against(&harness.provider.descriptor())
        .unwrap_or_else(|_| unreachable!());
}

#[tokio::test]
async fn cancellation_and_deadlines_are_typed_and_fail_closed() {
    let harness = harness(LocalRuntimeKind::QemuMedia);
    let cancelled_request = operation_request(&harness.fixture, ProviderMethod::RuntimeStart);
    let mut cancelled = context(&harness.fixture, &cancelled_request.context);
    cancelled.cancelled = true;
    let failure = harness
        .provider
        .start(&cancelled, &cancelled_request)
        .await
        .expect_err("cancelled mutation must fail");
    assert_eq!(failure.kind, ProviderFailureKind::Cancelled);
    assert_eq!(harness.control.call_count(), 0);
    assert_eq!(harness.control.mutation_count(), 0);

    let request = operation_request(&harness.fixture, ProviderMethod::RuntimeInspect);
    let mut expired = context(&harness.fixture, &request.context);
    expired.monotonic_deadline_remaining_ms = 0;
    let failure = harness
        .provider
        .inspect(&expired, &request)
        .await
        .expect_err("expired request must fail");
    assert_eq!(failure.kind, ProviderFailureKind::DeadlineExpired);
    assert_eq!(harness.control.call_count(), 0);

    harness.control.set_delay_ms(20);
    let mut read_timeout = context(&harness.fixture, &request.context);
    read_timeout.monotonic_deadline_remaining_ms = 1;
    let failure = harness
        .provider
        .inspect(&read_timeout, &request)
        .await
        .expect_err("read timeout must fail");
    assert_eq!(failure.kind, ProviderFailureKind::DeadlineExpired);
    assert!(
        harness
            .control
            .last_cancellation()
            .is_some_and(|token| token.is_cancelled())
    );
    assert_eq!(
        harness
            .control
            .calls()
            .last()
            .map(|call| call.effective_deadline_remaining_ms),
        Some(1)
    );
    assert_eq!(harness.control.mutation_count(), 0);

    let start_request = operation_request(&harness.fixture, ProviderMethod::RuntimeStart);
    let mut mutation_timeout = context(&harness.fixture, &start_request.context);
    mutation_timeout.monotonic_deadline_remaining_ms = 1;
    let failure = harness
        .provider
        .start(&mutation_timeout, &start_request)
        .await
        .expect_err("mutation timeout must be ambiguous");
    assert_eq!(failure.kind, ProviderFailureKind::AmbiguousMutation);
    assert!(
        harness
            .control
            .last_cancellation()
            .is_some_and(|token| token.is_cancelled())
    );
    assert_eq!(harness.control.mutation_count(), 0);

    let mut wall_bounded = operation_request(&harness.fixture, ProviderMethod::RuntimeInspect);
    wall_bounded.context.expires_at_unix_ms = NOW + 5;
    let mut wall_bounded_context = context(&harness.fixture, &wall_bounded.context);
    wall_bounded_context.monotonic_deadline_remaining_ms = 100;
    let failure = harness
        .provider
        .inspect(&wall_bounded_context, &wall_bounded)
        .await
        .expect_err("wall-clock deadline must bound the control call");
    assert_eq!(failure.kind, ProviderFailureKind::DeadlineExpired);
    assert_eq!(
        harness
            .control
            .calls()
            .last()
            .map(|call| call.effective_deadline_remaining_ms),
        Some(5)
    );
    assert!(
        harness
            .control
            .last_cancellation()
            .is_some_and(|token| token.is_cancelled())
    );
    assert_eq!(harness.control.mutation_count(), 0);

    let expired_request = operation_request(&harness.fixture, ProviderMethod::RuntimeStart);
    harness
        .clock
        .set(expired_request.context.expires_at_unix_ms);
    let expired_context = context(&harness.fixture, &expired_request.context);
    let calls_before_expiry = harness.control.call_count();
    let failure = harness
        .provider
        .start(&expired_context, &expired_request)
        .await
        .expect_err("wall-clock-expired mutation must fail before control");
    assert_eq!(failure.kind, ProviderFailureKind::DeadlineExpired);
    assert_eq!(harness.control.call_count(), calls_before_expiry);
    assert_eq!(harness.control.mutation_count(), 0);
}

#[tokio::test]
async fn adoption_rejects_forged_resource_generation() {
    let harness = harness(LocalRuntimeKind::CloudHypervisor);
    let handle = ensured_handle(&harness).await;
    harness
        .control
        .forge_adopted_generation
        .store(true, Ordering::Release);
    let adopt_operation = harness
        .fixture
        .operation(ProviderMethod::RuntimeAdopt)
        .unwrap_or_else(|_| unreachable!());
    let request = AdoptionRequest {
        context: adopt_operation.clone(),
        expected_owner: handle.owner.clone(),
        expected_configuration_fingerprint: handle.configuration_fingerprint.clone(),
        expected_resource_generation: handle.resource_generation,
        handle,
    };
    let call_context = context(&harness.fixture, &adopt_operation);
    let failure = harness
        .provider
        .adopt(&call_context, &request)
        .await
        .expect_err("forged generation must not be adopted");
    assert_eq!(failure.kind, ProviderFailureKind::AdoptionRejected);
    assert_eq!(
        failure.reason,
        d2b_contracts::v2_provider::ProviderHealthReason::GenerationMismatch
    );
}

#[tokio::test]
async fn diagnostics_are_closed_and_bounded() {
    let harness = harness(LocalRuntimeKind::CloudHypervisor);
    let configuration_debug = format!("{:?}", harness.configuration);
    assert!(!configuration_debug.contains("/nix/store"));
    assert!(!configuration_debug.contains("/run/d2b"));

    let mut operation = harness
        .fixture
        .operation(ProviderMethod::RuntimeInspect)
        .unwrap_or_else(|_| unreachable!());
    operation.operation_id =
        OperationId::parse("operation-sensitive").unwrap_or_else(|_| unreachable!());
    operation.idempotency_key =
        IdempotencyKey::parse("idempotency-sensitive").unwrap_or_else(|_| unreachable!());
    operation.principal =
        PrincipalRef::parse("principal-sensitive").unwrap_or_else(|_| unreachable!());
    operation.correlation_id =
        CorrelationId::parse("correlation-sensitive").unwrap_or_else(|_| unreachable!());
    let request = ProviderOperationRequest {
        context: operation.clone(),
        target: ProviderTarget::Workload {
            realm_id: realm_id(),
            workload_id: workload_id(),
        },
        expected_configuration_fingerprint: fingerprint(1),
        input: ProviderOperationInput::NoInput,
    };
    let mut call_context = context(&harness.fixture, &operation);
    call_context.peer_role = EndpointRole::ProviderAgent;
    let failure = harness
        .provider
        .inspect(&call_context, &request)
        .await
        .expect_err("wrong peer must fail");
    failure
        .validate_against(&harness.provider.descriptor())
        .unwrap_or_else(|_| unreachable!());
    let debug = format!("{failure:?}");
    for sensitive in [
        "operation-sensitive",
        "idempotency-sensitive",
        "principal-sensitive",
        "correlation-sensitive",
    ] {
        assert!(!debug.contains(sensitive));
    }
    assert_eq!(harness.control.call_count(), 0);
}

#[test]
fn operation_input_has_no_argv_path_endpoint_or_json_variant() {
    let control_source = include_str!("../src/control.rs");
    for forbidden in [
        "argv",
        "host_path",
        "endpoint:",
        "credential:",
        "serde_json",
    ] {
        assert!(!control_source.contains(forbidden), "{forbidden}");
    }
    let all_source = [
        include_str!("../src/config.rs"),
        include_str!("../src/control.rs"),
        include_str!("../src/factory.rs"),
        include_str!("../src/provider.rs"),
        include_str!("../Cargo.toml"),
    ]
    .concat();
    for forbidden in [
        "Command::new",
        "std::process",
        "ChArgvInput",
        "QemuMediaArgvInput",
        "d2b-host",
        "extra_args",
    ] {
        assert!(!all_source.contains(forbidden), "{forbidden}");
    }
    assert!(
        !live_runtime_capabilities()
            .unwrap_or_else(|_| unreachable!())
            .contains_method(ProviderMethod::RuntimeExecute)
    );
    assert_eq!(
        ProviderType::Runtime,
        descriptor(LocalRuntimeKind::QemuMedia).provider_type()
    );
}
