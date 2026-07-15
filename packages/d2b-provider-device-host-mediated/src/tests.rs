use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use crate::{
    DeviceAdoptionOutcome, DeviceAttachOutcome, DeviceCall, DeviceEffectPort,
    DeviceEffectPreparation, DeviceHealth, DeviceInspection, DeviceKind, DevicePlanOutcome,
    DevicePortError, DeviceQueryPort, DeviceSelectorDefinition, DeviceSemanticSelector, Factory,
    FidoCeremonyApproval, FidoClientPinSubcommand, FidoCommandKind, FidoPolicyDecision,
    FidoPolicyIntent, HostMediatedDeviceFactoryEntry, HostMediatedDeviceProvider,
    MAX_FIDO_CLIENT_PIN_CBOR_BYTES, factory_key, implementation_id, live_device_capabilities,
    parse_fido_client_pin_subcommand,
};
use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::EndpointRole,
    v2_identity::{ProviderType, RealmId, WorkloadId},
    v2_provider::{
        AdoptionRequest, DeviceProvider, DeviceSelectorId, Generation, ImplementationId,
        ProviderFailureKind, ProviderMethod, ProviderOperationInput, ProviderPlacement,
        ProviderTarget, RetryClass,
    },
};
use d2b_host::{
    gpu_argv::{GpuArgvInput, GpuContextType, GpuDisplayConfig, GpuParams},
    swtpm_argv::{SwtpmArgvInput, SwtpmIoctlFlushInput},
    usbip_argv::UsbipArgvInput,
    video_argv::{VideoArgvInput, VideoBackend},
};
use d2b_provider::{FactoryError, ProviderFactory, ProviderInstance, ProviderRegistryBuilder};
use d2b_provider_toolkit::{DeterministicClock, Fixture, check_provider_conformance};

const NOW: u64 = 1_700_000_000_000;

#[derive(Default)]
struct FakeEffects {
    plan_calls: AtomicUsize,
    attach_calls: AtomicUsize,
    detach_calls: AtomicUsize,
    slow_plan: AtomicBool,
    slow_attach: AtomicBool,
    slow_detach: AtomicBool,
    observed_kinds: std::sync::Mutex<Vec<DeviceKind>>,
}

#[async_trait]
impl DeviceEffectPort for FakeEffects {
    async fn plan_attach(
        &self,
        _context: DeviceCall,
        selector: DeviceSemanticSelector,
    ) -> Result<DevicePlanOutcome, DevicePortError> {
        self.plan_calls.fetch_add(1, Ordering::Relaxed);
        self.observed_kinds
            .lock()
            .expect("kind lock")
            .push(selector.kind());
        match selector.preparation() {
            DeviceEffectPreparation::Tpm { sidecar, flush } => {
                assert_eq!(sidecar.vm_name, flush.vm_name);
                assert!(sidecar.extra_args.is_empty());
            }
            DeviceEffectPreparation::Usbip(input) => assert_eq!(input.bus_id, "1-2"),
            DeviceEffectPreparation::Fido(policy) => {
                assert_eq!(*policy, FidoPolicyIntent::canonical());
            }
            DeviceEffectPreparation::Gpu(input) => assert!(input.extra_args.is_empty()),
            DeviceEffectPreparation::Video(input) => {
                assert_eq!(input.backend, VideoBackend::Vaapi);
            }
            DeviceEffectPreparation::Mediated => {}
        }
        if self.slow_plan.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        Ok(DevicePlanOutcome {
            plan_id: d2b_contracts::v2_provider::PlanId::parse("device-plan").expect("plan id"),
            expires_at_unix_ms: NOW + 30_000,
        })
    }

    async fn attach(
        &self,
        _context: DeviceCall,
        _plan: d2b_contracts::v2_provider::ProviderPlan,
    ) -> Result<DeviceAttachOutcome, DevicePortError> {
        self.attach_calls.fetch_add(1, Ordering::Relaxed);
        if self.slow_attach.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        Ok(DeviceAttachOutcome {
            handle_id: d2b_contracts::v2_provider::HandleId::parse("device-handle")
                .expect("handle id"),
            resource_generation: Generation::new(1).expect("generation"),
        })
    }

    async fn detach(
        &self,
        _context: DeviceCall,
        _target: ProviderTarget,
    ) -> Result<d2b_contracts::v2_provider::MutationState, DevicePortError> {
        self.detach_calls.fetch_add(1, Ordering::Relaxed);
        if self.slow_detach.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        Ok(d2b_contracts::v2_provider::MutationState::Applied)
    }
}

#[derive(Default)]
struct FakeQueries {
    adopt_calls: AtomicUsize,
}

#[async_trait]
impl DeviceQueryPort for FakeQueries {
    async fn health(&self, _context: DeviceCall) -> Result<DeviceHealth, DevicePortError> {
        Ok(DeviceHealth::healthy())
    }

    async fn inspect(
        &self,
        _context: DeviceCall,
        _target: ProviderTarget,
    ) -> Result<DeviceInspection, DevicePortError> {
        Ok(DeviceInspection::ready(None))
    }

    async fn adopt(
        &self,
        _context: DeviceCall,
        _request: AdoptionRequest,
    ) -> Result<DeviceAdoptionOutcome, DevicePortError> {
        self.adopt_calls.fetch_add(1, Ordering::Relaxed);
        Ok(DeviceAdoptionOutcome::Adopted)
    }
}

fn fixture() -> Fixture {
    let realm_id = RealmId::parse("aaaaaaaaaaaaaaaaaaaa").expect("realm id");
    let workload_id = WorkloadId::parse("ccccccccccccccccccca").expect("workload id");
    let mut descriptor = Fixture::new(ProviderType::Device, 8)
        .expect("base fixture")
        .descriptor;
    descriptor.implementation_id = implementation_id();
    descriptor.capabilities = live_device_capabilities().expect("capabilities");
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

fn selectors() -> Vec<DeviceSelectorDefinition> {
    vec![
        DeviceSelectorDefinition::tpm(
            DeviceSelectorId::parse("tpm-main").expect("selector"),
            SwtpmArgvInput {
                swtpm_binary_path: "/nix/store/aaaaaaaa-swtpm/bin/swtpm".into(),
                vm_name: "corp-vm".into(),
                state_dir: "/var/lib/d2b/vms/corp-vm/tpm".into(),
                ctrl_socket_path: "/var/lib/d2b/vms/corp-vm/tpm/ctrl.sock".into(),
                server_socket_path: "/run/d2b/vms/corp-vm/swtpm.sock".into(),
                uid: 1100,
                gid: 1100,
                log_path: "/var/lib/d2b/vms/corp-vm/tpm/swtpm.log".into(),
                log_level: 10,
                pid_path: "/var/lib/d2b/vms/corp-vm/tpm/swtpm.pid".into(),
                startup_clear: true,
                extra_args: vec![],
            },
            SwtpmIoctlFlushInput {
                swtpm_ioctl_binary_path: "/nix/store/aaaaaaaa-swtpm/bin/swtpm_ioctl".into(),
                vm_name: "corp-vm".into(),
                ctrl_socket_path: "/var/lib/d2b/vms/corp-vm/tpm/ctrl.sock".into(),
            },
        ),
        DeviceSelectorDefinition::usbip(
            DeviceSelectorId::parse("usbip-key").expect("selector"),
            UsbipArgvInput {
                usbip_binary_path: "/nix/store/bbbbbbbb-usbip/bin/usbip".into(),
                bus_id: "1-2".into(),
            },
        ),
        DeviceSelectorDefinition::fido(DeviceSelectorId::parse("fido-key").expect("selector")),
        DeviceSelectorDefinition::gpu(
            DeviceSelectorId::parse("gpu-main").expect("selector"),
            GpuArgvInput {
                crosvm_binary_path: "/nix/store/cccccccc-crosvm/bin/crosvm".into(),
                vm_name: "corp-vm".into(),
                socket_path: "/run/d2b/vms/corp-vm/gpu.sock".into(),
                wayland_sock: "/run/d2b-gpu/corp-vm/wayland-0".into(),
                params: GpuParams {
                    context_types: vec![GpuContextType::Virgl, GpuContextType::CrossDomain],
                    displays: vec![GpuDisplayConfig { hidden: true }],
                    egl: true,
                    vulkan: true,
                },
                extra_args: vec![],
            },
        ),
        DeviceSelectorDefinition::video(
            DeviceSelectorId::parse("video-main").expect("selector"),
            VideoArgvInput {
                crosvm_binary_path: "/nix/store/dddddddd-crosvm/bin/crosvm".into(),
                vm_name: "corp-vm".into(),
                socket_path: "/run/d2b-video/corp-vm/video.sock".into(),
                backend: VideoBackend::Vaapi,
            },
        ),
        DeviceSelectorDefinition::mediated(
            DeviceSelectorId::parse("mediated-main").expect("selector"),
        ),
    ]
}

fn provider(
    fixture: &Fixture,
    effects: Arc<FakeEffects>,
    queries: Arc<FakeQueries>,
) -> HostMediatedDeviceProvider {
    HostMediatedDeviceProvider::with_clock(
        fixture.descriptor.clone(),
        selectors(),
        effects,
        queries,
        Arc::new(DeterministicClock::new(NOW)),
    )
    .expect("provider")
}

fn factory() -> Factory {
    let fixture = fixture();
    let entry = HostMediatedDeviceFactoryEntry::new(
        &fixture.descriptor,
        selectors(),
        Arc::new(FakeEffects::default()),
        Arc::new(FakeQueries::default()),
    )
    .expect("factory entry");
    Factory::with_clock(vec![entry], Arc::new(DeterministicClock::new(NOW))).expect("factory")
}

#[test]
fn factory_key_constructs_only_the_exact_device_implementation() {
    let fixture = fixture();
    assert_eq!(factory_key().provider_type, ProviderType::Device);
    assert_eq!(factory_key().implementation_id, implementation_id());
    let instance = factory()
        .construct(&fixture.descriptor)
        .expect("device instance");
    assert!(matches!(instance, ProviderInstance::Device(_)));
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

    let wrong_type = Fixture::new(ProviderType::Audio, 11).expect("wrong-type fixture");
    assert!(matches!(
        factory().construct(&wrong_type.descriptor),
        Err(FactoryError::Rejected)
    ));

    let mut wrong_implementation = fixture.descriptor.clone();
    wrong_implementation.implementation_id =
        ImplementationId::parse("other-device").expect("implementation");
    assert!(matches!(
        factory().construct(&wrong_implementation),
        Err(FactoryError::Rejected)
    ));

    let alternate = Fixture::new(ProviderType::Device, 44)
        .expect("alternate fixture")
        .descriptor;
    let mut wrong_schema = fixture.descriptor.clone();
    wrong_schema.configuration_schema_fingerprint =
        alternate.configuration_schema_fingerprint.clone();
    assert!(matches!(
        factory().construct(&wrong_schema),
        Err(FactoryError::Rejected)
    ));
    let mut wrong_scope = fixture.descriptor.clone();
    wrong_scope.configured_scope_digest = alternate.configured_scope_digest.clone();
    assert!(matches!(
        factory().construct(&wrong_scope),
        Err(FactoryError::Rejected)
    ));
    let mut wrong_placement = fixture.descriptor.clone();
    wrong_placement.placement = ProviderPlacement::TrustedFirstPartyInProcess {
        realm_id: RealmId::parse("bbbbbbbbbbbbbbbbbbba").expect("alternate realm"),
        controller_role: EndpointRole::RealmController,
    };
    assert!(matches!(
        factory().construct(&wrong_placement),
        Err(FactoryError::Rejected)
    ));
    let mut wrong_provider = fixture.descriptor.clone();
    wrong_provider.provider_id = alternate.provider_id;
    assert!(matches!(
        factory().construct(&wrong_provider),
        Err(FactoryError::Rejected)
    ));

    let entry = HostMediatedDeviceFactoryEntry::new(
        &fixture.descriptor,
        selectors(),
        Arc::new(FakeEffects::default()),
        Arc::new(FakeQueries::default()),
    )
    .expect("entry");
    assert!(Factory::new(vec![entry.clone(), entry]).is_err());
}

#[tokio::test]
async fn every_closed_device_kind_plans_through_the_semantic_port() {
    let fixture = fixture();
    let effects = Arc::new(FakeEffects::default());
    let provider = provider(&fixture, effects.clone(), Arc::new(FakeQueries::default()));
    let cases = [
        ("tpm-main", DeviceKind::Tpm),
        ("usbip-key", DeviceKind::Usbip),
        ("fido-key", DeviceKind::FidoCtaphidUhid),
        ("gpu-main", DeviceKind::Gpu),
        ("video-main", DeviceKind::Video),
        ("mediated-main", DeviceKind::Mediated),
    ];

    for (selector, _) in cases {
        let request = fixture
            .request_with_input(
                ProviderMethod::DevicePlanAttach,
                ProviderOperationInput::DeviceSelector {
                    device_selector_id: DeviceSelectorId::parse(selector).expect("selector"),
                },
            )
            .expect("request");
        provider
            .plan_attach(&fixture.call_context(&request.context), &request)
            .await
            .expect("plan");
    }

    assert_eq!(
        *effects.observed_kinds.lock().expect("kind lock"),
        cases.map(|(_, kind)| kind)
    );
}

#[test]
fn fido_policy_requires_trusted_approval_and_denies_destructive_commands() {
    let policy = FidoPolicyIntent::canonical();
    assert_eq!(
        policy.decide(
            FidoCommandKind::MakeCredential,
            FidoCeremonyApproval::Missing
        ),
        FidoPolicyDecision::DenyApprovalRequired
    );
    assert_eq!(
        policy.decide(
            FidoCommandKind::GetAssertion,
            FidoCeremonyApproval::ApprovedTrustedSource
        ),
        FidoPolicyDecision::AllowApprovedCeremony
    );
    for command in [
        FidoCommandKind::ClientPin,
        FidoCommandKind::LargeBlobs,
        FidoCommandKind::Reset,
        FidoCommandKind::CredentialManagement,
        FidoCommandKind::BioEnrollment,
        FidoCommandKind::AuthenticatorConfiguration,
        FidoCommandKind::Vendor,
        FidoCommandKind::Unknown,
    ] {
        assert_eq!(
            policy.decide(command, FidoCeremonyApproval::ApprovedTrustedSource),
            FidoPolicyDecision::DenyDestructive
        );
    }

    let vectors = [
        (
            1,
            FidoClientPinSubcommand::GetPinRetries,
            FidoPolicyDecision::AllowReadOnly,
        ),
        (
            2,
            FidoClientPinSubcommand::GetKeyAgreement,
            FidoPolicyDecision::AllowReadOnly,
        ),
        (
            3,
            FidoClientPinSubcommand::SetPin,
            FidoPolicyDecision::DenyDestructive,
        ),
        (
            4,
            FidoClientPinSubcommand::ChangePin,
            FidoPolicyDecision::DenyDestructive,
        ),
        (
            5,
            FidoClientPinSubcommand::GetPinToken,
            FidoPolicyDecision::AllowApprovedCeremony,
        ),
        (
            6,
            FidoClientPinSubcommand::GetPinUvAuthTokenUsingUvWithPermissions,
            FidoPolicyDecision::AllowApprovedCeremony,
        ),
        (
            7,
            FidoClientPinSubcommand::GetUvRetries,
            FidoPolicyDecision::AllowReadOnly,
        ),
        (
            9,
            FidoClientPinSubcommand::GetPinUvAuthTokenUsingPinWithPermissions,
            FidoPolicyDecision::AllowApprovedCeremony,
        ),
        (
            8,
            FidoClientPinSubcommand::Unknown,
            FidoPolicyDecision::DenyDestructive,
        ),
    ];
    for (subcommand, parsed, decision) in vectors {
        let request = [0xa2, 0x01, 0x02, 0x02, subcommand];
        assert_eq!(
            parse_fido_client_pin_subcommand(&request),
            Ok(parsed),
            "subcommand {subcommand}"
        );
        assert_eq!(
            policy.decide_client_pin(&request, FidoCeremonyApproval::ApprovedTrustedSource),
            decision,
            "subcommand {subcommand}"
        );
    }
    let token = [0xa2, 0x01, 0x02, 0x02, 0x05];
    assert_eq!(
        policy.decide_client_pin(&token, FidoCeremonyApproval::Missing),
        FidoPolicyDecision::DenyApprovalRequired
    );
    for malformed in [
        vec![0xa1, 0x01, 0x02],
        vec![0xa2, 0x02, 0x03, 0x02, 0x05],
        vec![0xbf, 0x02, 0x03, 0xff],
    ] {
        assert_eq!(
            policy.decide_client_pin(&malformed, FidoCeremonyApproval::ApprovedTrustedSource),
            FidoPolicyDecision::DenyDestructive
        );
    }
    let oversized = vec![0; MAX_FIDO_CLIENT_PIN_CBOR_BYTES + 1];
    assert_eq!(
        policy.decide_client_pin(&oversized, FidoCeremonyApproval::ApprovedTrustedSource),
        FidoPolicyDecision::DenyDestructive
    );
}

#[tokio::test]
async fn wrong_selector_and_cancelled_call_never_reach_effects() {
    let fixture = fixture();
    let effects = Arc::new(FakeEffects::default());
    let provider = provider(&fixture, effects.clone(), Arc::new(FakeQueries::default()));
    let request = fixture
        .request_with_input(
            ProviderMethod::DevicePlanAttach,
            ProviderOperationInput::DeviceSelector {
                device_selector_id: DeviceSelectorId::parse("not-configured").expect("selector"),
            },
        )
        .expect("request");
    let failure = provider
        .plan_attach(&fixture.call_context(&request.context), &request)
        .await
        .expect_err("unknown selector");
    assert_eq!(failure.kind, ProviderFailureKind::InvalidRequest);

    let valid = fixture
        .request_with_input(
            ProviderMethod::DevicePlanAttach,
            ProviderOperationInput::DeviceSelector {
                device_selector_id: DeviceSelectorId::parse("fido-key").expect("selector"),
            },
        )
        .expect("request");
    let mut cancelled = fixture.call_context(&valid.context);
    cancelled.cancelled = true;
    let failure = provider
        .plan_attach(&cancelled, &valid)
        .await
        .expect_err("cancelled");
    assert_eq!(failure.kind, ProviderFailureKind::Cancelled);
    assert_eq!(effects.plan_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn stale_adoption_is_rejected_before_query_port_call() {
    let fixture = fixture();
    let effects = Arc::new(FakeEffects::default());
    let queries = Arc::new(FakeQueries::default());
    let provider = provider(&fixture, effects, queries.clone());
    let plan_request = fixture
        .request_with_input(
            ProviderMethod::DevicePlanAttach,
            ProviderOperationInput::DeviceSelector {
                device_selector_id: DeviceSelectorId::parse("gpu-main").expect("selector"),
            },
        )
        .expect("request");
    let plan = provider
        .plan_attach(&fixture.call_context(&plan_request.context), &plan_request)
        .await
        .expect("plan");
    let attach_operation = fixture
        .operation(ProviderMethod::DeviceAttach)
        .expect("operation");
    let handle = provider
        .attach(&fixture.call_context(&attach_operation), &plan)
        .await
        .expect("handle");
    let adoption_operation = fixture
        .operation(ProviderMethod::DeviceAdopt)
        .expect("operation");
    let adoption = AdoptionRequest {
        context: adoption_operation.clone(),
        handle: handle.clone(),
        expected_owner: handle.owner.clone(),
        expected_configuration_fingerprint: handle.configuration_fingerprint.clone(),
        expected_resource_generation: Generation::new(2).expect("generation"),
    };
    let failure = provider
        .adopt(&fixture.call_context(&adoption_operation), &adoption)
        .await
        .expect_err("stale adoption");
    assert_eq!(failure.kind, ProviderFailureKind::InvalidRequest);
    assert_eq!(queries.adopt_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn monotonic_deadline_bounds_the_effect_future() {
    let fixture = fixture();
    let effects = Arc::new(FakeEffects::default());
    effects.slow_plan.store(true, Ordering::Relaxed);
    let provider = provider(&fixture, effects.clone(), Arc::new(FakeQueries::default()));
    let request = fixture
        .request_with_input(
            ProviderMethod::DevicePlanAttach,
            ProviderOperationInput::DeviceSelector {
                device_selector_id: DeviceSelectorId::parse("tpm-main").expect("selector"),
            },
        )
        .expect("request");
    let mut context = fixture.call_context(&request.context);
    context.monotonic_deadline_remaining_ms = 1;
    let failure = provider
        .plan_attach(&context, &request)
        .await
        .expect_err("deadline");
    assert_eq!(failure.kind, ProviderFailureKind::DeadlineExpired);
    assert_eq!(effects.plan_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn post_dispatch_attach_and_detach_timeouts_are_ambiguous() {
    let fixture = fixture();
    let effects = Arc::new(FakeEffects::default());
    let provider = provider(&fixture, effects.clone(), Arc::new(FakeQueries::default()));
    let plan_request = fixture
        .request_with_input(
            ProviderMethod::DevicePlanAttach,
            ProviderOperationInput::DeviceSelector {
                device_selector_id: DeviceSelectorId::parse("gpu-main").expect("selector"),
            },
        )
        .expect("plan request");
    let plan = provider
        .plan_attach(&fixture.call_context(&plan_request.context), &plan_request)
        .await
        .expect("plan");
    let attach_operation = fixture
        .operation(ProviderMethod::DeviceAttach)
        .expect("attach operation");
    effects.slow_attach.store(true, Ordering::Relaxed);
    let mut attach_context = fixture.call_context(&attach_operation);
    attach_context.monotonic_deadline_remaining_ms = 1;
    let failure = provider
        .attach(&attach_context, &plan)
        .await
        .expect_err("ambiguous attach");
    assert_eq!(failure.kind, ProviderFailureKind::AmbiguousMutation);
    assert_eq!(failure.retry, RetryClass::AfterObservation);

    effects.slow_attach.store(false, Ordering::Relaxed);
    let handle = provider
        .attach(&fixture.call_context(&attach_operation), &plan)
        .await
        .expect("handle");
    let mut detach_request = fixture
        .request(ProviderMethod::DeviceDetach)
        .expect("detach request");
    detach_request.target = ProviderTarget::Handle {
        realm_id: handle.realm_id.clone(),
        workload_id: handle.workload_id.clone(),
        handle_id: handle.handle_id.clone(),
        handle_generation: handle.resource_generation,
    };
    effects.slow_detach.store(true, Ordering::Relaxed);
    let mut detach_context = fixture.call_context(&detach_request.context);
    detach_context.monotonic_deadline_remaining_ms = 1;
    let failure = provider
        .detach(&detach_context, &detach_request)
        .await
        .expect_err("ambiguous detach");
    assert_eq!(failure.kind, ProviderFailureKind::AmbiguousMutation);
    assert_eq!(failure.retry, RetryClass::AfterObservation);
}

#[tokio::test]
async fn canonical_device_conformance_uses_live_capabilities() {
    let fixture = fixture();
    let provider = Arc::new(provider(
        &fixture,
        Arc::new(FakeEffects::default()),
        Arc::new(FakeQueries::default()),
    ));
    check_provider_conformance(&ProviderInstance::Device(provider), &fixture)
        .await
        .expect("conformance");
}
