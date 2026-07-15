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
        Fingerprint, Generation, HandleId, HandleOwner, IdempotencyKey, ImplementationId,
        MutationState, ObservationReason, ObservedLifecycleState, OperationId,
        PROVIDER_SCHEMA_VERSION, PrincipalRef, Provider, ProviderApiVersion, ProviderAuthority,
        ProviderCallContext, ProviderCapability, ProviderCapabilitySet, ProviderDescriptor,
        ProviderFailureKind, ProviderHealthState, ProviderMethod, ProviderOperationContext,
        ProviderOperationInput, ProviderOperationRequest, ProviderPlacement, ProviderTarget,
        RetryClass, RuntimeProvider,
    },
};
use d2b_host::{
    ch_argv::{ChArgvInput, ChNetHandoff},
    qemu_media_argv::QemuMediaArgvInput,
};
use d2b_provider::ProviderInstance;
use d2b_provider_runtime_local::{
    LocalRuntimeConfiguration, LocalRuntimeConfigurationError, LocalRuntimeKind,
    LocalRuntimeProvider, LocalRuntimeProviderBuildError, MAX_CONFIGURED_RUNTIME_ITEMS,
    RuntimeAdoptionControl, RuntimeAdoptionOutcome, RuntimeConfiguredItemControl,
    RuntimeControlContext, RuntimeControlError, RuntimeControlPort, RuntimeEnsureControl,
    RuntimeHealth, RuntimeMutationOutcome, RuntimeObservedState, RuntimeOperationControl,
    RuntimePlanDecision, RuntimeResourceIdentity, live_runtime_capabilities,
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
}

struct FakeControl {
    kind: LocalRuntimeKind,
    configuration_fingerprint: Fingerprint,
    calls: Mutex<Vec<SeenCall>>,
    next_error: Mutex<Option<RuntimeControlError>>,
    delay_ms: AtomicU64,
    forge_adopted_generation: AtomicBool,
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
        }
    }

    fn record(&self, method: SeenMethod, context: &RuntimeControlContext) {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(SeenCall {
                method,
                operation: context.operation().clone(),
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

    async fn delay(&self) {
        let delay_ms = self.delay_ms.load(Ordering::Acquire);
        if delay_ms != 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
    }

    fn owner(&self, context: &RuntimeControlContext) -> HandleOwner {
        if self.kind.requires_provider_agent() {
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
        self.delay().await;
        self.take_error()?;
        Ok(RuntimeHealth::healthy())
    }

    async fn plan(
        &self,
        request: RuntimeOperationControl,
    ) -> Result<RuntimePlanDecision, RuntimeControlError> {
        self.record(SeenMethod::Plan, request.context());
        self.delay().await;
        self.take_error()?;
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
        self.delay().await;
        self.take_error()?;
        Ok(self.identity(request.context(), None))
    }

    async fn start(
        &self,
        request: RuntimeOperationControl,
    ) -> Result<RuntimeObservedState, RuntimeControlError> {
        self.record(SeenMethod::Start, request.context());
        self.delay().await;
        self.take_error()?;
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
        self.delay().await;
        self.take_error()?;
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
        self.delay().await;
        self.take_error()?;
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
        self.delay().await;
        self.take_error()?;
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
        self.delay().await;
        self.take_error()?;
        Ok(RuntimeMutationOutcome::new(MutationState::Applied))
    }

    async fn execute_configured_item(
        &self,
        request: RuntimeConfiguredItemControl,
    ) -> Result<RuntimeObservedState, RuntimeControlError> {
        self.record(SeenMethod::ExecuteConfigured, request.context());
        self.delay().await;
        self.take_error()?;
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

fn ch_input() -> ChArgvInput {
    ChArgvInput {
        vm_name: "corp-vm".to_owned(),
        ch_binary_path: "/nix/store/runtime-cloud-hypervisor/bin/cloud-hypervisor".to_owned(),
        cpus: 1,
        watchdog: false,
        kernel_path: "/nix/store/runtime-kernel/vmlinux".to_owned(),
        initramfs_path: None,
        cmdline: "console=ttyS0".to_owned(),
        seccomp: "true".to_owned(),
        memory: "shared=on,size=512M".to_owned(),
        platform_oem_strings: Vec::new(),
        console: "null".to_owned(),
        serial: "tty".to_owned(),
        primary_vsock: None,
        extra_vsock: Vec::new(),
        fs_shares: Vec::new(),
        api_socket_path: "/run/d2b/vms/corp-vm/ch-api.sock".to_owned(),
        net_ifaces: Vec::new(),
        net_handoff: ChNetHandoff::PersistentTap,
        extra_args: Vec::new(),
    }
}

fn qemu_input() -> QemuMediaArgvInput {
    QemuMediaArgvInput {
        qemu_binary_path: "/nix/store/runtime-qemu/bin/qemu-system-x86_64".to_owned(),
        vm_name: "media".to_owned(),
        qmp_socket_path: "/run/d2b/vms/media/qmp.sock".to_owned(),
        mac_address: "02:00:00:00:00:10".to_owned(),
        tap_fd: 10,
        memory_mib: 1024,
        vcpu: 1,
        lock_memory: false,
        exclude_memory_from_core_dump: true,
        disable_memory_merge: true,
        console_fd: None,
    }
}

fn configuration(kind: LocalRuntimeKind) -> LocalRuntimeConfiguration {
    match kind {
        LocalRuntimeKind::CloudHypervisor => {
            LocalRuntimeConfiguration::cloud_hypervisor(ch_input())
                .unwrap_or_else(|_| unreachable!())
        }
        LocalRuntimeKind::QemuMedia => {
            LocalRuntimeConfiguration::qemu_media(qemu_input()).unwrap_or_else(|_| unreachable!())
        }
        LocalRuntimeKind::SystemdUser => LocalRuntimeConfiguration::systemd_user(vec![
            ConfiguredItemId::parse("configured-editor").unwrap_or_else(|_| unreachable!()),
        ])
        .unwrap_or_else(|_| unreachable!()),
    }
}

fn descriptor(kind: LocalRuntimeKind) -> ProviderDescriptor {
    ProviderDescriptor {
        schema_version: PROVIDER_SCHEMA_VERSION,
        provider_id: provider_id(),
        authority: ProviderAuthority::Runtime {
            posture: kind.authority_posture(),
        },
        implementation_id: ImplementationId::parse(kind.implementation_id())
            .unwrap_or_else(|_| unreachable!()),
        api_version: ProviderApiVersion::V2,
        capabilities: live_runtime_capabilities().unwrap_or_else(|_| unreachable!()),
        configuration_schema_fingerprint: fingerprint(1),
        configured_scope_digest: fingerprint(2),
        registry_generation: Generation::new(4).unwrap_or_else(|_| unreachable!()),
        placement: if kind.requires_provider_agent() {
            ProviderPlacement::ProviderAgent {
                realm_id: realm_id(),
                workload_id: workload_id(),
                role_id: role_id(),
                endpoint_role: EndpointRole::ProviderAgent,
                service: ServicePackage::ProviderV2,
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
    provider: LocalRuntimeProvider,
    control: Arc<FakeControl>,
    fixture: Fixture,
}

fn harness(kind: LocalRuntimeKind) -> Harness {
    let descriptor = descriptor(kind);
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
    let provider =
        LocalRuntimeProvider::with_clock(descriptor, configuration(kind), control.clone(), clock)
            .unwrap_or_else(|_| unreachable!());
    Harness {
        provider,
        control,
        fixture,
    }
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
            matches!(actual.placement, ProviderPlacement::ProviderAgent { .. }),
            kind.requires_provider_agent()
        );
        assert_eq!(harness.provider.kind(), kind);
    }
}

#[tokio::test]
async fn all_closed_runtime_kinds_pass_common_conformance() {
    for kind in [
        LocalRuntimeKind::CloudHypervisor,
        LocalRuntimeKind::QemuMedia,
        LocalRuntimeKind::SystemdUser,
    ] {
        let harness = harness(kind);
        let instance = ProviderInstance::Runtime(Arc::new(harness.provider.clone()));
        check_provider_conformance(&instance, &harness.fixture)
            .await
            .unwrap_or_else(|_| unreachable!());
    }
}

#[test]
fn closed_configurations_validate_typed_builders_and_configured_items() {
    let configured_item =
        ConfiguredItemId::parse("configured-editor").unwrap_or_else(|_| unreachable!());
    let systemd = LocalRuntimeConfiguration::systemd_user(vec![configured_item.clone()])
        .unwrap_or_else(|_| unreachable!());
    assert!(systemd.validates_configured_item(&configured_item));
    assert!(
        !configuration(LocalRuntimeKind::CloudHypervisor)
            .validates_configured_item(&configured_item)
    );
    assert!(matches!(
        LocalRuntimeConfiguration::systemd_user(vec![configured_item.clone(), configured_item]),
        Err(LocalRuntimeConfigurationError::DuplicateConfiguredItem)
    ));
    let too_many = (0..=MAX_CONFIGURED_RUNTIME_ITEMS)
        .map(|index| {
            ConfiguredItemId::parse(format!("configured-{index}"))
                .unwrap_or_else(|_| unreachable!())
        })
        .collect();
    assert!(matches!(
        LocalRuntimeConfiguration::systemd_user(too_many),
        Err(LocalRuntimeConfigurationError::ConfiguredItemBoundExceeded)
    ));

    let mut invalid_ch = ch_input();
    invalid_ch.cpus = 0;
    assert!(matches!(
        LocalRuntimeConfiguration::cloud_hypervisor(invalid_ch),
        Err(LocalRuntimeConfigurationError::BackendConfigurationInvalid)
    ));
    let mut invalid_qemu = qemu_input();
    invalid_qemu.tap_fd = 2;
    assert!(matches!(
        LocalRuntimeConfiguration::qemu_media(invalid_qemu),
        Err(LocalRuntimeConfigurationError::BackendConfigurationInvalid)
    ));
}

#[test]
fn constructor_rejects_capability_and_placement_drift() {
    let kind = LocalRuntimeKind::SystemdUser;
    let control = Arc::new(FakeControl::new(kind, fingerprint(1)));
    let mut wrong_capabilities = descriptor(kind);
    let capabilities = live_runtime_capabilities().unwrap_or_else(|_| unreachable!());
    let mut capabilities = capabilities.as_slice().to_vec();
    capabilities.push(ProviderCapability(ProviderMethod::RuntimeExecute));
    wrong_capabilities.capabilities =
        ProviderCapabilitySet::new(capabilities).unwrap_or_else(|_| unreachable!());
    assert!(matches!(
        LocalRuntimeProvider::with_clock(
            wrong_capabilities,
            configuration(kind),
            control.clone(),
            Arc::new(DeterministicClock::new(NOW)),
        ),
        Err(LocalRuntimeProviderBuildError::CapabilityMismatch)
    ));

    let mut wrong_placement = descriptor(kind);
    wrong_placement.placement = ProviderPlacement::TrustedFirstPartyInProcess {
        realm_id: realm_id(),
        controller_role: EndpointRole::RealmController,
    };
    assert!(matches!(
        LocalRuntimeProvider::with_clock(
            wrong_placement,
            configuration(kind),
            control,
            Arc::new(DeterministicClock::new(NOW)),
        ),
        Err(LocalRuntimeProviderBuildError::PlacementMismatch)
    ));
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
    let request = operation_request(&harness.fixture, ProviderMethod::RuntimeInspect);
    let mut cancelled = context(&harness.fixture, &request.context);
    cancelled.cancelled = true;
    let failure = harness
        .provider
        .inspect(&cancelled, &request)
        .await
        .expect_err("cancelled request must fail");
    assert_eq!(failure.kind, ProviderFailureKind::Cancelled);

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

    let start_request = operation_request(&harness.fixture, ProviderMethod::RuntimeStart);
    let mut mutation_timeout = context(&harness.fixture, &start_request.context);
    mutation_timeout.monotonic_deadline_remaining_ms = 1;
    let failure = harness
        .provider
        .start(&mutation_timeout, &start_request)
        .await
        .expect_err("mutation timeout must be ambiguous");
    assert_eq!(failure.kind, ProviderFailureKind::AmbiguousMutation);
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
    let configuration_debug = format!("{:?}", harness.provider.configuration());
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
        include_str!("../src/provider.rs"),
    ]
    .concat();
    for forbidden in ["Command::new", "std::process"] {
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
