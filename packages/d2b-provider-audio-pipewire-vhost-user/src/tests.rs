use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use crate::{
    AudioAdoptionOutcome, AudioCall, AudioConfiguration, AudioEffectPort, AudioEnsureOutcome,
    AudioHealth, AudioInspection, AudioPlanOutcome, AudioPortError, AudioQueryPort,
    AudioSessionPlan, AudioState, Factory, PipewireVhostUserAudioProvider, factory_key,
    implementation_id, live_audio_capabilities,
};
use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::EndpointRole,
    v2_identity::{ProviderType, RealmId, WorkloadId},
    v2_provider::{
        AdoptionRequest, AudioChannel, AudioDirection, AudioProvider, Generation, ImplementationId,
        MutationState, ProviderFailureKind, ProviderMethod, ProviderOperationInput,
        ProviderPlacement, ProviderTarget,
    },
};
use d2b_host::audio_argv::{AudioArgvInput, AudioBackend};
use d2b_provider::{FactoryError, ProviderFactory, ProviderInstance, ProviderRegistryBuilder};
use d2b_provider_toolkit::{DeterministicClock, Fixture, check_provider_conformance};

const NOW: u64 = 1_700_000_000_000;

#[derive(Default)]
struct FakeEffects {
    plan_calls: AtomicUsize,
    ensure_calls: AtomicUsize,
    set_calls: AtomicUsize,
    destroy_calls: AtomicUsize,
    slow_plan: AtomicBool,
    plans: Mutex<Vec<AudioSessionPlan>>,
    states: Mutex<Vec<AudioState>>,
}

#[async_trait]
impl AudioEffectPort for FakeEffects {
    async fn plan(
        &self,
        _context: AudioCall,
        plan: AudioSessionPlan,
    ) -> Result<AudioPlanOutcome, AudioPortError> {
        self.plan_calls.fetch_add(1, Ordering::Relaxed);
        self.plans.lock().expect("plans").push(plan);
        if self.slow_plan.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        Ok(AudioPlanOutcome::Planned)
    }

    async fn ensure(
        &self,
        _context: AudioCall,
        _plan: AudioSessionPlan,
    ) -> Result<AudioEnsureOutcome, AudioPortError> {
        self.ensure_calls.fetch_add(1, Ordering::Relaxed);
        Ok(AudioEnsureOutcome {
            handle_id: d2b_contracts::v2_provider::HandleId::parse("audio-handle")
                .expect("handle id"),
            resource_generation: Generation::new(1).expect("generation"),
        })
    }

    async fn set_state(
        &self,
        _context: AudioCall,
        _target: ProviderTarget,
        state: AudioState,
    ) -> Result<AudioInspection, AudioPortError> {
        self.set_calls.fetch_add(1, Ordering::Relaxed);
        self.states.lock().expect("states").push(state);
        Ok(AudioInspection::ready(None, Some(state)))
    }

    async fn destroy(
        &self,
        _context: AudioCall,
        _target: ProviderTarget,
    ) -> Result<MutationState, AudioPortError> {
        self.destroy_calls.fetch_add(1, Ordering::Relaxed);
        Ok(MutationState::Applied)
    }
}

#[derive(Default)]
struct FakeQueries {
    adopt_calls: AtomicUsize,
}

#[async_trait]
impl AudioQueryPort for FakeQueries {
    async fn health(&self, _context: AudioCall) -> Result<AudioHealth, AudioPortError> {
        Ok(AudioHealth::healthy())
    }

    async fn inspect(
        &self,
        _context: AudioCall,
        _target: ProviderTarget,
    ) -> Result<AudioInspection, AudioPortError> {
        Ok(AudioInspection::ready(None, None))
    }

    async fn adopt(
        &self,
        _context: AudioCall,
        _request: AdoptionRequest,
    ) -> Result<AudioAdoptionOutcome, AudioPortError> {
        self.adopt_calls.fetch_add(1, Ordering::Relaxed);
        Ok(AudioAdoptionOutcome::Adopted)
    }
}

fn fixture() -> Fixture {
    let realm_id = RealmId::parse("aaaaaaaaaaaaaaaaaaaa").expect("realm id");
    let workload_id = WorkloadId::parse("ccccccccccccccccccca").expect("workload id");
    let mut descriptor = Fixture::new(ProviderType::Audio, 9)
        .expect("base fixture")
        .descriptor;
    descriptor.implementation_id = implementation_id();
    descriptor.capabilities = live_audio_capabilities().expect("capabilities");
    descriptor.placement = ProviderPlacement::TrustedFirstPartyInProcess {
        realm_id: realm_id.clone(),
        controller_role: EndpointRole::RealmController,
    };
    Fixture::from_descriptor(
        descriptor,
        ProviderTarget::Workload {
            realm_id,
            workload_id,
        },
        NOW,
    )
    .expect("fixture")
}

fn configuration() -> AudioConfiguration {
    AudioConfiguration::new(AudioArgvInput {
        sidecar_binary_path: "/run/d2b/vms/corp-vm/d2b-corp-vm".into(),
        vm_name: "corp-vm".into(),
        socket_path: "/run/d2b/vms/corp-vm/snd.sock".into(),
        backend: AudioBackend::Pipewire,
        extra_args: vec![],
    })
    .expect("configuration")
}

fn provider(
    fixture: &Fixture,
    effects: Arc<FakeEffects>,
    queries: Arc<FakeQueries>,
) -> PipewireVhostUserAudioProvider {
    PipewireVhostUserAudioProvider::with_clock(
        fixture.descriptor.clone(),
        configuration(),
        effects,
        queries,
        Arc::new(DeterministicClock::new(NOW)),
    )
    .expect("provider")
}

fn factory() -> Factory {
    Factory::with_clock(
        configuration(),
        Arc::new(FakeEffects::default()),
        Arc::new(FakeQueries::default()),
        Arc::new(DeterministicClock::new(NOW)),
    )
}

#[test]
fn factory_key_constructs_only_the_exact_audio_implementation() {
    let fixture = fixture();
    assert_eq!(factory_key().provider_type, ProviderType::Audio);
    assert_eq!(factory_key().implementation_id, implementation_id());
    let instance = factory()
        .construct(&fixture.descriptor)
        .expect("audio instance");
    assert!(matches!(instance, ProviderInstance::Audio(_)));
    assert_eq!(instance.descriptor(), fixture.descriptor);
    let mut builder = ProviderRegistryBuilder::new(
        fixture.descriptor.registry_generation,
        fixture.descriptor.configured_scope_digest.clone(),
        NOW,
    );
    builder
        .register_factory(factory_key(), Arc::new(factory()))
        .expect("register factory")
        .register_instance(fixture.descriptor.clone())
        .expect("register instance");
    assert_eq!(
        builder
            .finish()
            .expect("registry")
            .snapshot()
            .providers
            .as_slice(),
        std::slice::from_ref(&fixture.descriptor)
    );

    let wrong_type = Fixture::new(ProviderType::Device, 12).expect("wrong-type fixture");
    assert!(matches!(
        factory().construct(&wrong_type.descriptor),
        Err(FactoryError::Rejected)
    ));

    let mut wrong_implementation = fixture.descriptor.clone();
    wrong_implementation.implementation_id =
        ImplementationId::parse("other-audio").expect("implementation");
    assert!(matches!(
        factory().construct(&wrong_implementation),
        Err(FactoryError::Rejected)
    ));
}

#[tokio::test]
async fn open_plans_and_ensures_with_generated_route_and_role_ids() {
    let fixture = fixture();
    let effects = Arc::new(FakeEffects::default());
    let provider = provider(&fixture, effects.clone(), Arc::new(FakeQueries::default()));
    let request = fixture
        .request(ProviderMethod::AudioOpen)
        .expect("open request");
    provider
        .open(&fixture.call_context(&request.context), &request)
        .await
        .expect("open");

    assert_eq!(effects.plan_calls.load(Ordering::Relaxed), 1);
    assert_eq!(effects.ensure_calls.load(Ordering::Relaxed), 1);
    let plans = effects.plans.lock().expect("plans");
    assert_eq!(plans.len(), 1);
    assert!(plans[0].route_id.as_str().starts_with("audio-route-"));
    assert!(plans[0].role_id.as_str().starts_with("audio-role-"));
    let debug = format!("{:?}", plans[0]);
    assert!(!debug.contains("/run/"));
}

#[tokio::test]
async fn closed_audio_state_is_validated_and_bound_to_the_effect_result() {
    let fixture = fixture();
    let effects = Arc::new(FakeEffects::default());
    let provider = provider(&fixture, effects.clone(), Arc::new(FakeQueries::default()));
    let request = fixture
        .request_with_input(
            ProviderMethod::AudioSetState,
            ProviderOperationInput::AudioState {
                channel: AudioChannel::Microphone,
                direction: AudioDirection::Input,
                mute: Some(true),
                volume: Some(37),
            },
        )
        .expect("state request");
    provider
        .set_state(&fixture.call_context(&request.context), &request)
        .await
        .expect("set state");
    assert_eq!(
        *effects.states.lock().expect("states"),
        [AudioState {
            channel: AudioChannel::Microphone,
            direction: AudioDirection::Input,
            mute: Some(true),
            volume: Some(37),
        }]
    );
}

#[tokio::test]
async fn invalid_audio_state_and_cancellation_make_no_port_calls() {
    let fixture = fixture();
    let effects = Arc::new(FakeEffects::default());
    let provider = provider(&fixture, effects.clone(), Arc::new(FakeQueries::default()));
    let wrong = fixture
        .request_with_input(
            ProviderMethod::AudioSetState,
            ProviderOperationInput::AudioState {
                channel: AudioChannel::Speaker,
                direction: AudioDirection::Input,
                mute: Some(false),
                volume: None,
            },
        )
        .expect("request construction");
    let failure = provider
        .set_state(&fixture.call_context(&wrong.context), &wrong)
        .await
        .expect_err("wrong channel");
    assert_eq!(failure.kind, ProviderFailureKind::InvalidRequest);
    assert_eq!(effects.set_calls.load(Ordering::Relaxed), 0);

    let open = fixture
        .request(ProviderMethod::AudioOpen)
        .expect("open request");
    let mut cancelled = fixture.call_context(&open.context);
    cancelled.cancelled = true;
    let failure = provider
        .open(&cancelled, &open)
        .await
        .expect_err("cancelled");
    assert_eq!(failure.kind, ProviderFailureKind::Cancelled);
    assert_eq!(effects.plan_calls.load(Ordering::Relaxed), 0);
    assert_eq!(effects.ensure_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn stale_adoption_is_rejected_before_query_port_call() {
    let fixture = fixture();
    let effects = Arc::new(FakeEffects::default());
    let queries = Arc::new(FakeQueries::default());
    let provider = provider(&fixture, effects, queries.clone());
    let open = fixture
        .request(ProviderMethod::AudioOpen)
        .expect("open request");
    let handle = provider
        .open(&fixture.call_context(&open.context), &open)
        .await
        .expect("handle");
    let operation = fixture
        .operation(ProviderMethod::AudioAdopt)
        .expect("adopt operation");
    let request = AdoptionRequest {
        context: operation.clone(),
        handle: handle.clone(),
        expected_owner: handle.owner.clone(),
        expected_configuration_fingerprint: handle.configuration_fingerprint.clone(),
        expected_resource_generation: Generation::new(2).expect("generation"),
    };
    let failure = provider
        .adopt(&fixture.call_context(&operation), &request)
        .await
        .expect_err("stale adoption");
    assert_eq!(failure.kind, ProviderFailureKind::InvalidRequest);
    assert_eq!(queries.adopt_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn one_deadline_bounds_both_audio_plan_and_ensure() {
    let fixture = fixture();
    let effects = Arc::new(FakeEffects::default());
    effects.slow_plan.store(true, Ordering::Relaxed);
    let provider = provider(&fixture, effects.clone(), Arc::new(FakeQueries::default()));
    let request = fixture
        .request(ProviderMethod::AudioOpen)
        .expect("open request");
    let mut context = fixture.call_context(&request.context);
    context.monotonic_deadline_remaining_ms = 1;
    let failure = provider
        .open(&context, &request)
        .await
        .expect_err("deadline");
    assert_eq!(failure.kind, ProviderFailureKind::DeadlineExpired);
    assert_eq!(effects.plan_calls.load(Ordering::Relaxed), 1);
    assert_eq!(effects.ensure_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn typed_audio_builder_refuses_free_form_arguments() {
    let mut argv = AudioArgvInput {
        sidecar_binary_path: "/run/d2b/vms/corp-vm/d2b-corp-vm".into(),
        vm_name: "corp-vm".into(),
        socket_path: "/run/d2b/vms/corp-vm/snd.sock".into(),
        backend: AudioBackend::Pipewire,
        extra_args: vec!["--arbitrary".into()],
    };
    assert!(AudioConfiguration::new(argv.clone()).is_err());
    argv.extra_args.clear();
    assert!(AudioConfiguration::new(argv).is_ok());
}

#[tokio::test]
async fn canonical_audio_conformance_uses_live_capabilities() {
    let fixture = fixture();
    let provider = Arc::new(provider(
        &fixture,
        Arc::new(FakeEffects::default()),
        Arc::new(FakeQueries::default()),
    ));
    check_provider_conformance(&ProviderInstance::Audio(provider), &fixture)
        .await
        .expect("conformance");
}
