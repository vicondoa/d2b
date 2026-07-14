use std::sync::Arc;

use d2b_contracts::{
    v2_identity::{ProviderId, ProviderType},
    v2_provider::{
        AdoptionRequest, AdoptionState, Fingerprint, Generation, ProviderFailureKind,
        ProviderMethod, RuntimeProvider,
    },
};
use d2b_provider::{ProviderInstance, ProviderRegistryBuilder, RpcProviderProxy, SessionIdentity};
use d2b_provider_toolkit::{
    DeterministicClock, FakeProvider, Fixture, ProviderAgentAdapter, Redacted, Secret,
    check_provider_conformance, register_exact_instances,
};

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
