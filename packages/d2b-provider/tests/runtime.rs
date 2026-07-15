use std::{sync::Arc, time::Duration};

use d2b_contracts::{
    v2_component_session::{EndpointRole, ServicePackage},
    v2_identity::{ProviderId, ProviderType, RealmId},
    v2_provider::{
        AdoptionRequest, AudioProvider, CgroupAuthority, CorrelationId, CredentialProvider,
        DeviceMediationPosture, DeviceProvider, DisplayProvider, Fingerprint, Generation,
        IdempotencyKey, ImplementationId, InfrastructureProvider, MutationReceipt, NetworkPosture,
        NetworkProvider, ObservabilityProvider, OperationId, PROVIDER_SCHEMA_VERSION,
        PersistentIdentityPosture, PrincipalRef, ProcessAuthority, Provider, ProviderApiVersion,
        ProviderAuthority, ProviderCallContext, ProviderCapability, ProviderCapabilitySet,
        ProviderDescriptor, ProviderFailure, ProviderFailureKind, ProviderFuture, ProviderHandle,
        ProviderHealth, ProviderHealthReason, ProviderMethod, ProviderObservation,
        ProviderOperationContext, ProviderPlacement, ProviderPlan, ProviderRemediation,
        RegistryDrainPolicy, RetryClass, RuntimeAuthorityPosture, RuntimeProvider, StorageProvider,
        SubstrateProvider, TransportProvider, UserNamespacePosture,
    },
};
use d2b_provider::{
    AdmissionOptions, CancellationToken, FactoryError, ProviderFactory, ProviderInstance,
    ProviderRegistryBuilder, ProviderRuntimeError, RegistryBuildError, RegistryLimits,
    provider_capabilities_are_dispatchable, provider_inspection_method,
    provider_method_is_dispatchable,
};

const NOW: u64 = 1_700_000_000_000;

fn fingerprint(value: u8) -> Fingerprint {
    Fingerprint::parse(format!("{value:064x}")).unwrap_or_else(|_| unreachable!())
}

fn provider_id(value: char) -> ProviderId {
    ProviderId::parse(format!("{}{value}a", "b".repeat(18))).unwrap_or_else(|_| unreachable!())
}

fn runtime_capabilities() -> ProviderCapabilitySet {
    ProviderCapabilitySet::new(
        ProviderMethod::ALL
            .iter()
            .filter(|method| method.provider_type() == ProviderType::Runtime)
            .filter(|method| **method != ProviderMethod::RuntimeExecute)
            .copied()
            .map(ProviderCapability)
            .collect(),
    )
    .unwrap_or_else(|_| unreachable!())
}

fn descriptor(id: char, generation: u64, configuration: u8) -> ProviderDescriptor {
    ProviderDescriptor {
        schema_version: PROVIDER_SCHEMA_VERSION,
        provider_id: provider_id(id),
        authority: ProviderAuthority::Runtime {
            posture: RuntimeAuthorityPosture {
                process: ProcessAuthority::ProviderOwnedPidfd,
                cgroup: CgroupAuthority::RealmDelegatedLeaf,
                network: NetworkPosture::IsolatedNamespace,
                user_namespace: UserNamespacePosture::BrokerPreestablished,
                persistent_identity: PersistentIdentityPosture::FileBackedCloneable,
                device_mediation: DeviceMediationPosture::BrokerDelegatedTyped,
            },
        },
        implementation_id: ImplementationId::parse("runtime-test")
            .unwrap_or_else(|_| unreachable!()),
        api_version: ProviderApiVersion::V2,
        capabilities: runtime_capabilities(),
        configuration_schema_fingerprint: fingerprint(configuration),
        configured_scope_digest: fingerprint(configuration.saturating_add(20)),
        registry_generation: Generation::new(generation).unwrap_or_else(|_| unreachable!()),
        placement: ProviderPlacement::TrustedFirstPartyInProcess {
            realm_id: RealmId::parse("aaaaaaaaaaaaaaaaaaaa").unwrap_or_else(|_| unreachable!()),
            controller_role: EndpointRole::RealmController,
        },
    }
}

struct StubRuntime(ProviderDescriptor);

fn denied(context: &ProviderCallContext<'_>) -> ProviderFailure {
    ProviderFailure {
        kind: ProviderFailureKind::Unavailable,
        retry: RetryClass::Never,
        provider_type: ProviderType::Runtime,
        binding: context.operation.binding(),
        correlation_id: context.operation.correlation_id.clone(),
        occurred_at_unix_ms: NOW,
        reason: ProviderHealthReason::ProviderDegraded,
        remediation: ProviderRemediation::InspectProvider,
    }
}

impl Provider for StubRuntime {
    fn descriptor(&self) -> ProviderDescriptor {
        self.0.clone()
    }

    fn health<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
    ) -> ProviderFuture<'a, ProviderHealth> {
        Box::pin(async move { Err(denied(context)) })
    }
}

macro_rules! denied_method {
    ($name:ident, $request:ty, $response:ty) => {
        fn $name<'a>(
            &'a self,
            context: &'a ProviderCallContext<'a>,
            _request: &'a $request,
        ) -> ProviderFuture<'a, $response> {
            Box::pin(async move { Err(denied(context)) })
        }
    };
}

impl RuntimeProvider for StubRuntime {
    fn capabilities(&self) -> ProviderCapabilitySet {
        self.0.capabilities.clone()
    }
    denied_method!(
        plan,
        d2b_contracts::v2_provider::ProviderOperationRequest,
        ProviderPlan
    );
    denied_method!(ensure, ProviderPlan, ProviderHandle);
    denied_method!(
        start,
        d2b_contracts::v2_provider::ProviderOperationRequest,
        ProviderObservation
    );
    denied_method!(
        stop,
        d2b_contracts::v2_provider::ProviderOperationRequest,
        ProviderObservation
    );
    denied_method!(
        inspect,
        d2b_contracts::v2_provider::ProviderOperationRequest,
        ProviderObservation
    );
    denied_method!(adopt, AdoptionRequest, ProviderObservation);
    denied_method!(
        destroy,
        d2b_contracts::v2_provider::ProviderOperationRequest,
        MutationReceipt
    );
}

struct StubFactory;

impl ProviderFactory for StubFactory {
    fn construct(&self, descriptor: &ProviderDescriptor) -> Result<ProviderInstance, FactoryError> {
        Ok(ProviderInstance::Runtime(Arc::new(StubRuntime(
            descriptor.clone(),
        ))))
    }
}

fn registry_builder(generation: u64, configuration: u8) -> ProviderRegistryBuilder {
    ProviderRegistryBuilder::new(
        Generation::new(generation).unwrap_or_else(|_| unreachable!()),
        fingerprint(configuration),
        NOW,
    )
}

fn key() -> d2b_contracts::v2_provider::ProviderFactoryKey {
    d2b_contracts::v2_provider::ProviderFactoryKey {
        provider_type: ProviderType::Runtime,
        implementation_id: ImplementationId::parse("runtime-test")
            .unwrap_or_else(|_| unreachable!()),
    }
}

fn operation(descriptor: &ProviderDescriptor) -> ProviderOperationContext {
    ProviderOperationContext {
        schema_version: PROVIDER_SCHEMA_VERSION,
        operation_id: OperationId::parse("operation-test").unwrap_or_else(|_| unreachable!()),
        idempotency_key: IdempotencyKey::parse("idempotency-test")
            .unwrap_or_else(|_| unreachable!()),
        request_digest: fingerprint(40),
        scope: d2b_contracts::v2_provider::AuthorizedProviderScope::Realm {
            realm_id: descriptor.placement.realm_id().clone(),
        },
        principal: PrincipalRef::parse("principal-test").unwrap_or_else(|_| unreachable!()),
        provider_id: descriptor.provider_id.clone(),
        provider_type: ProviderType::Runtime,
        provider_generation: descriptor.registry_generation,
        capability: ProviderCapability(ProviderMethod::RuntimeInspect),
        method: ProviderMethod::RuntimeInspect,
        policy_epoch: Generation::new(1).unwrap_or_else(|_| unreachable!()),
        authorization_decision_digest: fingerprint(41),
        issued_at_unix_ms: NOW - 100,
        expires_at_unix_ms: NOW + 10_000,
        correlation_id: CorrelationId::parse("correlation-test").unwrap_or_else(|_| unreachable!()),
        trace_id: fingerprint(42),
    }
}

#[test]
fn closed_error_context_is_actionable_without_identity_leaks() {
    let contract = RegistryBuildError::Contract(
        d2b_contracts::v2_provider::ProviderContractError::InvalidGeneration,
    );
    assert_eq!(
        contract.to_string(),
        "provider contract validation failed (invalid provider generation)"
    );
    assert!(std::error::Error::source(&contract).is_some());

    let descriptor = descriptor('a', 1, 1);
    let operation = operation(&descriptor);
    let context = ProviderCallContext {
        operation: &operation,
        peer_role: EndpointRole::RealmController,
        service: ServicePackage::ProviderV2,
        monotonic_deadline_remaining_ms: 1,
        cancelled: false,
    };
    let error = ProviderRuntimeError::from(denied(&context));
    let display = error.to_string();
    assert!(display.contains("Unavailable"));
    assert!(display.contains("retry=Never"));
    assert!(display.contains("type=Runtime"));
    assert!(!display.contains("correlation-test"));
    assert!(!display.contains("operation-test"));
}

#[test]
fn health_uses_no_input_inspection_methods_for_every_axis() {
    for provider_type in ProviderType::ALL {
        let method = provider_inspection_method(provider_type);
        assert_eq!(method.provider_type(), provider_type);
        assert!(method.required());
        assert_ne!(method, ProviderMethod::DevicePlanAttach);
    }
}

#[test]
fn all_provider_traits_are_object_safe() {
    fn provider(_: Option<&dyn Provider>) {}
    fn runtime(_: Option<&dyn RuntimeProvider>) {}
    fn infrastructure(_: Option<&dyn InfrastructureProvider>) {}
    fn transport(_: Option<&dyn TransportProvider>) {}
    fn substrate(_: Option<&dyn SubstrateProvider>) {}
    fn credential(_: Option<&dyn CredentialProvider>) {}
    fn display(_: Option<&dyn DisplayProvider>) {}
    fn network(_: Option<&dyn NetworkProvider>) {}
    fn storage(_: Option<&dyn StorageProvider>) {}
    fn device(_: Option<&dyn DeviceProvider>) {}
    fn audio(_: Option<&dyn AudioProvider>) {}
    fn observability(_: Option<&dyn ObservabilityProvider>) {}
    provider(None);
    runtime(None);
    infrastructure(None);
    transport(None);
    substrate(None);
    credential(None);
    display(None);
    network(None);
    storage(None);
    device(None);
    audio(None);
    observability(None);
}

#[test]
fn one_dispatchability_policy_blocks_unimplemented_runtime_execute() {
    assert!(
        ProviderMethod::ALL
            .iter()
            .copied()
            .filter(|method| *method != ProviderMethod::RuntimeExecute)
            .all(provider_method_is_dispatchable)
    );
    assert!(!provider_method_is_dispatchable(
        ProviderMethod::RuntimeExecute
    ));
    assert!(provider_capabilities_are_dispatchable(
        &runtime_capabilities()
    ));

    let mut advertised = descriptor('x', 1, 1);
    advertised.capabilities = ProviderCapabilitySet::new(
        ProviderMethod::ALL
            .iter()
            .copied()
            .filter(|method| method.provider_type() == ProviderType::Runtime)
            .map(ProviderCapability)
            .collect(),
    )
    .unwrap_or_else(|_| unreachable!());
    assert!(!provider_capabilities_are_dispatchable(
        &advertised.capabilities
    ));

    let mut builder = registry_builder(1, 1);
    builder
        .register_factory(key(), Arc::new(StubFactory))
        .unwrap_or_else(|_| unreachable!());
    assert!(matches!(
        builder.register_instance(advertised),
        Err(RegistryBuildError::CapabilityMismatch)
    ));
}

#[test]
fn registry_rejects_duplicates_without_last_registration_wins() {
    let mut builder = registry_builder(1, 1);
    builder
        .register_factory(key(), Arc::new(StubFactory))
        .unwrap_or_else(|_| unreachable!());
    assert!(matches!(
        builder.register_factory(key(), Arc::new(StubFactory)),
        Err(RegistryBuildError::DuplicateFactory)
    ));
    assert!(matches!(
        builder.finish(),
        Err(RegistryBuildError::TransactionAborted)
    ));

    let mut builder = registry_builder(1, 1);
    builder
        .register_factory(key(), Arc::new(StubFactory))
        .unwrap_or_else(|_| unreachable!());
    let first = descriptor('c', 1, 2);
    builder
        .register_instance(first.clone())
        .unwrap_or_else(|_| unreachable!());
    assert!(matches!(
        builder.register_instance(first),
        Err(RegistryBuildError::DuplicateProvider)
    ));
    assert!(matches!(
        builder.finish(),
        Err(RegistryBuildError::TransactionAborted)
    ));

    let mut builder = registry_builder(1, 1);
    builder
        .register_factory(key(), Arc::new(StubFactory))
        .unwrap_or_else(|_| unreachable!());
    builder
        .register_instance(descriptor('c', 1, 2))
        .unwrap_or_else(|_| unreachable!());
    builder
        .register_instance(descriptor('d', 1, 3))
        .unwrap_or_else(|_| unreachable!());
    let registry = builder.finish().unwrap_or_else(|_| unreachable!());
    assert_eq!(registry.snapshot().providers.len(), 2);
}

#[tokio::test]
async fn admission_limits_cancellation_and_shutdown_fail_closed() {
    let descriptor = descriptor('c', 1, 2);
    let mut builder = registry_builder(1, 1);
    builder
        .limits(RegistryLimits {
            total_in_flight: 1,
            per_provider_in_flight: 1,
        })
        .unwrap_or_else(|_| unreachable!());
    builder
        .register_factory(key(), Arc::new(StubFactory))
        .unwrap_or_else(|_| unreachable!());
    builder
        .register_instance(descriptor.clone())
        .unwrap_or_else(|_| unreachable!());
    let registry = builder.finish().unwrap_or_else(|_| unreachable!());
    let caller = CancellationToken::new();
    let admitted = registry
        .admit(
            operation(&descriptor),
            AdmissionOptions {
                expected_method: ProviderMethod::RuntimeInspect,
                peer_role: EndpointRole::RealmController,
                service: ServicePackage::ProviderV2,
                deadline_after: Duration::from_secs(5),
                caller_cancellation: caller.clone(),
                now_unix_ms: NOW,
            },
        )
        .unwrap_or_else(|_| unreachable!());
    assert!(matches!(
        registry.admit(
            operation(&descriptor),
            AdmissionOptions {
                expected_method: ProviderMethod::RuntimeInspect,
                peer_role: EndpointRole::RealmController,
                service: ServicePackage::ProviderV2,
                deadline_after: Duration::from_secs(5),
                caller_cancellation: CancellationToken::new(),
                now_unix_ms: NOW,
            },
        ),
        Err(ProviderRuntimeError::InFlightLimit)
    ));
    caller.cancel();
    assert!(matches!(
        admitted.context.call_context(),
        Err(ProviderRuntimeError::Cancelled)
    ));
    let report = registry
        .shutdown(&RegistryDrainPolicy {
            drain_deadline_ms: 1,
            cancel_in_flight_at_deadline: true,
            revoke_transport_bindings: true,
            revoke_credential_leases: true,
            close_provider_sessions: true,
        })
        .await
        .unwrap_or_else(|_| unreachable!());
    assert!(!report.drained);
    assert_eq!(report.unresolved_in_flight, 1);
    assert_eq!(
        registry.lifecycle(),
        d2b_contracts::v2_provider::RegistryLifecycle::Retired
    );
    drop(admitted);
}
