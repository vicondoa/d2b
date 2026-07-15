use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::EndpointRole,
    v2_identity::ProviderType,
    v2_provider::{
        Fingerprint, Generation, IdempotencyKey, ImplementationId, MutationState, OperationId,
        Provider, ProviderAuthority, ProviderCallContext, ProviderCapability, ProviderFailureKind,
        ProviderMethod, ProviderOperationRequest, ProviderPlacement, ProviderTarget,
        SubstrateProvider,
    },
};
use d2b_provider::{
    FactoryError, ProviderClock, ProviderFactory, ProviderInstance, ProviderRegistryBuilder,
};
use d2b_provider_substrate_host::{
    HostApplyOutcome, HostCapability, HostCheckKind, HostCheckProfile, HostCheckReport,
    HostCheckRequest, HostDiagnostic, HostEvidenceSource, HostFinding, HostFindingKind,
    HostFindingSeverity, HostKernelModule, HostModelError, HostPlanRequest, HostPortError,
    HostRemediationClass, HostRemediationId, HostRemediationPlan, HostSubstrateConfiguration,
    HostSubstrateKind, HostSubstratePort, HostSubstrateProviderFactory, HostSupportEntry,
    HostSupportEvidence, HostSupportStatus, LINUX_IMPLEMENTATION_ID, LinuxSubstrateProvider,
    MAX_CHECK_FINDINGS, MAX_FINDING_DIAGNOSTICS, MAX_PLAN_FINDINGS, MAX_REPORT_DIAGNOSTICS,
    NIXOS_IMPLEMENTATION_ID, NixOsSubstrateProvider,
};
use d2b_provider_toolkit::{DeterministicClock, Fixture, check_provider_conformance};

const NOW: u64 = 1_700_000_000_000;

#[derive(Clone)]
struct PortBehavior {
    support: HostSupportEvidence,
    findings: Vec<HostFinding>,
    check_fingerprint: Fingerprint,
    check_error: Option<HostPortError>,
    plan_error: Option<HostPortError>,
    apply_error: Option<HostPortError>,
    apply_outcome: HostApplyOutcome,
    authorize_plan: bool,
    plan_lifetime_ms: u64,
    plan_fingerprint: Option<Fingerprint>,
}

impl Default for PortBehavior {
    fn default() -> Self {
        Self {
            support: HostSupportEvidence::default(),
            findings: Vec::new(),
            check_fingerprint: fingerprint(901),
            check_error: None,
            plan_error: None,
            apply_error: None,
            apply_outcome: HostApplyOutcome::Applied,
            authorize_plan: false,
            plan_lifetime_ms: 30_000,
            plan_fingerprint: None,
        }
    }
}

struct FakePort {
    behavior: Mutex<PortBehavior>,
    now: AtomicU64,
    delay_ms: AtomicU64,
    check_completion_advance_ms: AtomicU64,
    check_completion_clock: Mutex<Option<Arc<DeterministicClock>>>,
    check_calls: AtomicUsize,
    plan_calls: AtomicUsize,
    apply_calls: AtomicUsize,
    effects: AtomicUsize,
    checked_configurations: Mutex<Vec<HostSubstrateConfiguration>>,
    applied_ids: Mutex<Vec<String>>,
}

impl FakePort {
    fn new(behavior: PortBehavior) -> Self {
        Self {
            behavior: Mutex::new(behavior),
            now: AtomicU64::new(NOW),
            delay_ms: AtomicU64::new(0),
            check_completion_advance_ms: AtomicU64::new(0),
            check_completion_clock: Mutex::new(None),
            check_calls: AtomicUsize::new(0),
            plan_calls: AtomicUsize::new(0),
            apply_calls: AtomicUsize::new(0),
            effects: AtomicUsize::new(0),
            checked_configurations: Mutex::new(Vec::new()),
            applied_ids: Mutex::new(Vec::new()),
        }
    }

    fn set_now(&self, now: u64) {
        self.now.store(now, Ordering::Release);
    }

    fn set_delay(&self, delay: Duration) {
        self.delay_ms.store(
            u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
            Ordering::Release,
        );
    }

    fn advance_clock_on_check_completion(&self, clock: Arc<DeterministicClock>, advance_ms: u64) {
        *self
            .check_completion_clock
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(clock);
        self.check_completion_advance_ms
            .store(advance_ms, Ordering::Release);
    }

    fn update(&self, update: impl FnOnce(&mut PortBehavior)) {
        update(
            &mut self
                .behavior
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        );
    }

    async fn delay(&self) {
        let delay = self.delay_ms.load(Ordering::Acquire);
        if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
    }
}

#[async_trait]
impl HostSubstratePort for FakePort {
    async fn check(&self, request: HostCheckRequest) -> Result<HostCheckReport, HostPortError> {
        self.check_calls.fetch_add(1, Ordering::AcqRel);
        self.checked_configurations
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(request.configuration());
        self.delay().await;
        let advance_ms = self.check_completion_advance_ms.swap(0, Ordering::AcqRel);
        if advance_ms > 0 {
            let completed_at = self.now.fetch_add(advance_ms, Ordering::AcqRel) + advance_ms;
            if let Some(clock) = self
                .check_completion_clock
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take()
            {
                clock.set(completed_at);
            }
        }
        let behavior = self
            .behavior
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if let Some(error) = behavior.check_error {
            return Err(error);
        }
        HostCheckReport::new(
            request.configuration(),
            request.descriptor().clone(),
            request.owner().clone(),
            request.operation().clone(),
            self.now.load(Ordering::Acquire),
            behavior.check_fingerprint,
            behavior.support,
            behavior.findings,
        )
        .map_err(|_| HostPortError::InvalidResponse)
    }

    async fn plan_remediation(
        &self,
        request: HostPlanRequest,
    ) -> Result<HostRemediationPlan, HostPortError> {
        self.plan_calls.fetch_add(1, Ordering::AcqRel);
        self.delay().await;
        let behavior = self
            .behavior
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if let Some(error) = behavior.plan_error {
            return Err(error);
        }
        let now = self.now.load(Ordering::Acquire);
        let remediation_id = HostRemediationId::parse("remediation-fixture")
            .map_err(|_| HostPortError::InvalidResponse)?;
        let report_fingerprint = behavior.plan_fingerprint.unwrap_or_else(|| {
            request
                .latest_report_fingerprint()
                .cloned()
                .unwrap_or_else(|| fingerprint(902))
        });
        let result = if behavior.authorize_plan {
            HostRemediationPlan::authorized(
                remediation_id,
                request.configuration(),
                request.descriptor().clone(),
                request.owner().clone(),
                request.operation().clone(),
                report_fingerprint,
                behavior.findings,
                now,
                now + behavior.plan_lifetime_ms,
            )
        } else {
            HostRemediationPlan::not_applicable(
                remediation_id,
                request.configuration(),
                request.descriptor().clone(),
                request.owner().clone(),
                request.operation().clone(),
                report_fingerprint,
                behavior.findings,
                now,
                now + behavior.plan_lifetime_ms,
            )
        };
        result.map_err(|_| HostPortError::InvalidResponse)
    }

    async fn apply(
        &self,
        remediation_id: HostRemediationId,
    ) -> Result<HostApplyOutcome, HostPortError> {
        self.apply_calls.fetch_add(1, Ordering::AcqRel);
        self.delay().await;
        let behavior = self
            .behavior
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if let Some(error) = behavior.apply_error {
            return Err(error);
        }
        self.applied_ids
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(remediation_id.as_str().to_owned());
        if matches!(
            behavior.apply_outcome,
            HostApplyOutcome::Applied | HostApplyOutcome::AlreadyApplied
        ) {
            self.effects.fetch_add(1, Ordering::AcqRel);
        }
        Ok(behavior.apply_outcome)
    }
}

fn fingerprint(value: usize) -> Fingerprint {
    Fingerprint::parse(format!("{value:064x}")).expect("test fingerprint")
}

fn automatic_finding() -> HostFinding {
    HostFinding::new(
        HostFindingKind::ConfigurationDrift(HostCheckKind::NetworkPolicy),
        HostFindingSeverity::Blocking,
        HostRemediationClass::DaemonAuthorized,
        vec![HostDiagnostic::ConfigurationMismatch],
        1,
    )
    .expect("test finding")
}

fn explicit_support() -> HostSupportEvidence {
    HostSupportEvidence::new(
        HostCapability::ALL
            .into_iter()
            .map(|capability| {
                HostSupportEntry::new(
                    capability,
                    HostSupportStatus::Confirmed(HostEvidenceSource::DaemonPreflight),
                )
            })
            .collect(),
    )
    .expect("test support evidence")
}

fn fixture(implementation: &str) -> Fixture {
    let base = Fixture::new(ProviderType::Substrate, 3).expect("base fixture");
    let mut descriptor = base.descriptor.clone();
    descriptor.implementation_id =
        ImplementationId::parse(implementation).expect("implementation id");
    descriptor.placement = ProviderPlacement::TrustedFirstPartyInProcess {
        realm_id: descriptor.placement.realm_id().clone(),
        controller_role: EndpointRole::LocalRootController,
    };
    Fixture::from_descriptor(
        descriptor,
        ProviderTarget::Realm {
            realm_id: base.descriptor.placement.realm_id().clone(),
        },
        NOW,
    )
    .expect("host substrate fixture")
}

fn request(fixture: &Fixture, method: ProviderMethod) -> ProviderOperationRequest {
    fixture.request(method).expect("provider request")
}

fn distinct_request(
    fixture: &Fixture,
    method: ProviderMethod,
    identity: &str,
    digest: usize,
) -> ProviderOperationRequest {
    let mut request = request(fixture, method);
    request.context.operation_id =
        OperationId::parse(format!("operation-{identity}")).expect("operation id");
    request.context.idempotency_key =
        IdempotencyKey::parse(format!("idempotency-{identity}")).expect("idempotency key");
    request.context.request_digest = fingerprint(digest);
    request
}

fn context<'a>(
    fixture: &Fixture,
    request: &'a ProviderOperationRequest,
) -> ProviderCallContext<'a> {
    fixture.call_context(&request.context)
}

fn nixos_provider(
    port: Arc<FakePort>,
    clock: Arc<DeterministicClock>,
) -> (Fixture, NixOsSubstrateProvider) {
    let fixture = fixture(NIXOS_IMPLEMENTATION_ID);
    let provider = NixOsSubstrateProvider::with_clock(
        fixture.descriptor.clone(),
        port,
        clock as Arc<dyn ProviderClock>,
    )
    .expect("NixOS provider");
    (fixture, provider)
}

fn linux_provider(
    port: Arc<FakePort>,
    clock: Arc<DeterministicClock>,
) -> (Fixture, LinuxSubstrateProvider) {
    let fixture = fixture(LINUX_IMPLEMENTATION_ID);
    let provider = LinuxSubstrateProvider::with_clock(
        fixture.descriptor.clone(),
        port,
        clock as Arc<dyn ProviderClock>,
    )
    .expect("Linux provider");
    (fixture, provider)
}

#[test]
fn factories_publish_canonical_keys_and_construct_exact_substrate_instances() {
    for (kind, implementation) in [
        (HostSubstrateKind::NixOs, NIXOS_IMPLEMENTATION_ID),
        (HostSubstrateKind::GenericLinux, LINUX_IMPLEMENTATION_ID),
    ] {
        let port = Arc::new(FakePort::new(PortBehavior::default()));
        let factory = HostSubstrateProviderFactory::with_clock(
            kind,
            port,
            Arc::new(DeterministicClock::new(NOW)),
        )
        .expect("host substrate factory");
        assert_eq!(factory.kind(), kind);
        assert_eq!(factory.implementation_id().as_str(), implementation);
        assert_eq!(factory.key(), kind.factory_key().expect("canonical key"));

        let fixture = fixture(implementation);
        let instance = factory
            .construct(&fixture.descriptor)
            .expect("substrate provider instance");
        assert!(matches!(instance, ProviderInstance::Substrate(_)));
        assert_eq!(instance.descriptor(), fixture.descriptor);
    }
}

#[test]
fn factory_rejects_wrong_descriptor_type_and_implementation() {
    let port = Arc::new(FakePort::new(PortBehavior::default()));
    let factory = HostSubstrateProviderFactory::nixos(port).expect("NixOS factory");

    let mut wrong_type = fixture(NIXOS_IMPLEMENTATION_ID).descriptor;
    wrong_type.authority = ProviderAuthority::Storage;
    assert_eq!(
        factory.construct(&wrong_type).err(),
        Some(FactoryError::Rejected)
    );

    let wrong_implementation = fixture(LINUX_IMPLEMENTATION_ID).descriptor;
    assert_eq!(
        factory.construct(&wrong_implementation).err(),
        Some(FactoryError::Rejected)
    );
}

#[test]
fn factory_registers_directly_with_provider_registry_builder() {
    let port = Arc::new(FakePort::new(PortBehavior::default()));
    let factory = HostSubstrateProviderFactory::linux(port).expect("Linux factory");
    let fixture = fixture(LINUX_IMPLEMENTATION_ID);
    let mut builder = ProviderRegistryBuilder::new(
        fixture.descriptor.registry_generation,
        fingerprint(903),
        NOW,
    );
    builder
        .register_factory(factory.key(), Arc::new(factory))
        .expect("register factory");
    builder
        .register_instance(fixture.descriptor.clone())
        .expect("register provider");
    let registry = builder.finish().expect("provider registry");
    assert_eq!(
        registry
            .snapshot()
            .descriptor(&fixture.descriptor.provider_id),
        Some(&fixture.descriptor)
    );
}

#[tokio::test]
async fn nixos_and_linux_use_canonical_profiles_and_conform() {
    let behavior = PortBehavior {
        support: explicit_support(),
        ..PortBehavior::default()
    };
    let nix_port = Arc::new(FakePort::new(behavior.clone()));
    let nix_clock = Arc::new(DeterministicClock::new(NOW));
    let (nix_fixture, nix) = nixos_provider(nix_port.clone(), nix_clock);
    let nix_request = request(&nix_fixture, ProviderMethod::SubstrateCheck);
    let nix_context = context(&nix_fixture, &nix_request);
    let observation = nix
        .check(&nix_context, &nix_request)
        .await
        .expect("NixOS check");
    assert_eq!(
        observation.health.state,
        d2b_contracts::v2_provider::ProviderHealthState::Healthy
    );
    assert_eq!(
        nix.inspect(&nix_context, &nix_request)
            .await
            .expect("healthy NixOS inspection")
            .state(),
        d2b_provider_substrate_host::HostSubstrateState::Ready
    );
    assert_eq!(
        nix_port
            .checked_configurations
            .lock()
            .unwrap_or_else(|error| error.into_inner())[0]
            .check_profile(),
        HostCheckProfile::NixOsFullHost
    );
    check_provider_conformance(
        &ProviderInstance::Substrate(Arc::new(nix.clone())),
        &nix_fixture,
    )
    .await
    .expect("NixOS provider conformance");

    let linux_port = Arc::new(FakePort::new(behavior));
    let linux_clock = Arc::new(DeterministicClock::new(NOW));
    let (linux_fixture, linux) = linux_provider(linux_port.clone(), linux_clock);
    let linux_request = request(&linux_fixture, ProviderMethod::SubstrateCheck);
    let linux_context = context(&linux_fixture, &linux_request);
    linux
        .check(&linux_context, &linux_request)
        .await
        .expect("Linux check");
    assert_eq!(
        linux_port
            .checked_configurations
            .lock()
            .unwrap_or_else(|error| error.into_inner())[0]
            .check_profile(),
        HostCheckProfile::GenericLinuxFullHost
    );
    check_provider_conformance(
        &ProviderInstance::Substrate(Arc::new(linux)),
        &linux_fixture,
    )
    .await
    .expect("Linux provider conformance");
}

#[tokio::test]
async fn zero_findings_do_not_overclaim_unproven_host_support() {
    let port = Arc::new(FakePort::new(PortBehavior::default()));
    let clock = Arc::new(DeterministicClock::new(NOW));
    let (fixture, provider) = nixos_provider(port, clock);
    let unknown_request = request(&fixture, ProviderMethod::SubstrateCheck);
    let unknown_context = context(&fixture, &unknown_request);
    let observation = provider
        .check(&unknown_context, &unknown_request)
        .await
        .expect("zero-finding check");
    assert_eq!(
        observation.health.state,
        d2b_contracts::v2_provider::ProviderHealthState::Degraded
    );
    assert_eq!(
        observation.reason,
        d2b_contracts::v2_provider::ObservationReason::MissingEvidence
    );
    assert_eq!(
        observation.lifecycle,
        d2b_contracts::v2_provider::ObservedLifecycleState::Unknown
    );

    let inspection = provider
        .inspect(&unknown_context, &unknown_request)
        .await
        .expect("cached inspection");
    assert_eq!(
        inspection.state(),
        d2b_provider_substrate_host::HostSubstrateState::Checked
    );
    let report = inspection.report().expect("check report");
    assert!(report.findings().is_empty());
    assert!(report.support().confirmed_capabilities().is_empty());
    for capability in HostCapability::ALL {
        assert_eq!(
            report.support().status(capability),
            HostSupportStatus::Unknown
        );
    }

    let explicit = HostSupportEvidence::new(vec![
        HostSupportEntry::new(HostCapability::CgroupV2, HostSupportStatus::Unsupported),
        HostSupportEntry::new(
            HostCapability::UserNamespaces,
            HostSupportStatus::NotApplicable,
        ),
    ])
    .expect("distinct support states");
    assert_eq!(
        explicit.status(HostCapability::CgroupV2),
        HostSupportStatus::Unsupported
    );
    assert_eq!(
        explicit.status(HostCapability::UserNamespaces),
        HostSupportStatus::NotApplicable
    );
    assert_eq!(
        explicit.status(HostCapability::VhostAcceleration),
        HostSupportStatus::Unknown
    );

    let explicit_non_support = HostSupportEvidence::new(vec![
        HostSupportEntry::new(HostCapability::CgroupV2, HostSupportStatus::Unsupported),
        HostSupportEntry::new(
            HostCapability::UserNamespaces,
            HostSupportStatus::NotApplicable,
        ),
        HostSupportEntry::new(
            HostCapability::VhostAcceleration,
            HostSupportStatus::NotApplicable,
        ),
        HostSupportEntry::new(
            HostCapability::DeviceAccess,
            HostSupportStatus::NotApplicable,
        ),
    ])
    .expect("complete explicit non-support evidence");
    let port = Arc::new(FakePort::new(PortBehavior {
        support: explicit_non_support,
        ..PortBehavior::default()
    }));
    let clock = Arc::new(DeterministicClock::new(NOW));
    let (fixture, provider) = linux_provider(port, clock);
    let non_support_request = request(&fixture, ProviderMethod::SubstrateCheck);
    let non_support_context = context(&fixture, &non_support_request);
    let observation = provider
        .check(&non_support_context, &non_support_request)
        .await
        .expect("explicit non-support report");
    assert_eq!(
        observation.reason,
        d2b_contracts::v2_provider::ObservationReason::ConfigurationMismatch
    );
    assert_eq!(
        observation.health.state,
        d2b_contracts::v2_provider::ProviderHealthState::Degraded
    );
}

#[tokio::test]
async fn completed_checks_accept_fresh_completion_timestamps() {
    let port = Arc::new(FakePort::new(PortBehavior {
        support: explicit_support(),
        ..PortBehavior::default()
    }));
    let clock = Arc::new(DeterministicClock::new(NOW));
    port.advance_clock_on_check_completion(clock.clone(), 5);
    let (fixture, provider) = nixos_provider(port, clock);
    let request = request(&fixture, ProviderMethod::SubstrateCheck);
    let context = context(&fixture, &request);

    let observation = provider
        .check(&context, &request)
        .await
        .expect("check completed after admission");
    assert_eq!(observation.observed_at_unix_ms, NOW + 5);
}

#[tokio::test]
async fn check_preserves_typed_findings_and_bounded_diagnostics() {
    let finding = HostFinding::new(
        HostFindingKind::CheckFailed(HostCheckKind::CgroupV2),
        HostFindingSeverity::Blocking,
        HostRemediationClass::OperatorAction,
        vec![
            HostDiagnostic::EvidenceMissing,
            HostDiagnostic::RequiredComponentMissing,
        ],
        2,
    )
    .expect("typed finding");
    let port = Arc::new(FakePort::new(PortBehavior {
        support: HostSupportEvidence::new(vec![HostSupportEntry::new(
            HostCapability::CgroupV2,
            HostSupportStatus::Unsupported,
        )])
        .expect("support"),
        findings: vec![finding],
        ..PortBehavior::default()
    }));
    let clock = Arc::new(DeterministicClock::new(NOW));
    let (fixture, provider) = nixos_provider(port, clock);
    let request = request(&fixture, ProviderMethod::SubstrateCheck);
    let context = context(&fixture, &request);
    let observation = provider
        .check(&context, &request)
        .await
        .expect("typed check");
    assert_eq!(
        observation.health.remediation,
        d2b_contracts::v2_provider::ProviderRemediation::RepairConfiguration
    );
    let inspection = provider.inspect(&context, &request).await.expect("inspect");
    let report = inspection.report().expect("report");
    assert_eq!(
        report.findings()[0].kind(),
        HostFindingKind::CheckFailed(HostCheckKind::CgroupV2)
    );
    assert_eq!(report.findings()[0].affected_count(), 2);
    assert_eq!(report.summary().blocking, 1);
}

#[tokio::test]
async fn denied_or_tampered_apply_never_reaches_an_effect() {
    let finding = automatic_finding();
    let port = Arc::new(FakePort::new(PortBehavior {
        support: explicit_support(),
        findings: vec![finding],
        authorize_plan: true,
        ..PortBehavior::default()
    }));
    let clock = Arc::new(DeterministicClock::new(NOW));
    let (fixture, provider) = nixos_provider(port.clone(), clock);
    let plan_request = request(&fixture, ProviderMethod::SubstratePlanRemediation);
    let plan_context = context(&fixture, &plan_request);
    let plan = provider
        .plan_remediation(&plan_context, &plan_request)
        .await
        .expect("plan");

    let mut tampered = plan.clone();
    tampered.configuration_fingerprint = fingerprint(999);
    let apply_request = request(&fixture, ProviderMethod::SubstrateApply);
    let apply_context = context(&fixture, &apply_request);
    let failure = provider
        .apply(&apply_context, &tampered)
        .await
        .expect_err("tampered apply must fail");
    assert_eq!(failure.kind, ProviderFailureKind::InvalidRequest);
    assert_eq!(port.apply_calls.load(Ordering::Acquire), 0);
    assert_eq!(port.effects.load(Ordering::Acquire), 0);

    port.update(|behavior| behavior.apply_error = Some(HostPortError::Denied));
    let failure = provider
        .apply(&apply_context, &plan)
        .await
        .expect_err("daemon-owned port denial");
    assert_eq!(failure.kind, ProviderFailureKind::UnauthorizedScope);
    assert_eq!(port.apply_calls.load(Ordering::Acquire), 1);
    assert_eq!(port.effects.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn stale_plan_and_generation_mismatch_are_rejected_before_apply() {
    let port = Arc::new(FakePort::new(PortBehavior {
        findings: vec![automatic_finding()],
        authorize_plan: true,
        plan_lifetime_ms: 1_000,
        ..PortBehavior::default()
    }));
    let clock = Arc::new(DeterministicClock::new(NOW));
    let (fixture, provider) = nixos_provider(port.clone(), clock.clone());
    let plan_request = request(&fixture, ProviderMethod::SubstratePlanRemediation);
    let plan_context = context(&fixture, &plan_request);
    let plan = provider
        .plan_remediation(&plan_context, &plan_request)
        .await
        .expect("plan");

    let mut wrong_generation = plan.clone();
    wrong_generation.binding.provider_generation = Generation::new(2).expect("generation");
    let apply_request = request(&fixture, ProviderMethod::SubstrateApply);
    let apply_context = context(&fixture, &apply_request);
    let failure = provider
        .apply(&apply_context, &wrong_generation)
        .await
        .expect_err("generation mismatch");
    assert_eq!(failure.kind, ProviderFailureKind::InvalidRequest);
    assert_eq!(port.apply_calls.load(Ordering::Acquire), 0);

    clock.set(NOW + 2_000);
    port.set_now(NOW + 2_000);
    let failure = provider
        .apply(&apply_context, &plan)
        .await
        .expect_err("expired plan");
    assert_eq!(failure.kind, ProviderFailureKind::InvalidRequest);
    assert_eq!(port.apply_calls.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn plan_must_bind_the_latest_check_fingerprint() {
    let port = Arc::new(FakePort::new(PortBehavior {
        findings: vec![automatic_finding()],
        authorize_plan: true,
        plan_fingerprint: Some(fingerprint(999)),
        ..PortBehavior::default()
    }));
    let clock = Arc::new(DeterministicClock::new(NOW));
    let (fixture, provider) = nixos_provider(port.clone(), clock);

    let check_request = request(&fixture, ProviderMethod::SubstrateCheck);
    let check_context = context(&fixture, &check_request);
    provider
        .check(&check_context, &check_request)
        .await
        .expect("check establishes report binding");

    let plan_request = request(&fixture, ProviderMethod::SubstratePlanRemediation);
    let plan_context = context(&fixture, &plan_request);
    let failure = provider
        .plan_remediation(&plan_context, &plan_request)
        .await
        .expect_err("mismatched report fingerprint must fail");
    assert_eq!(failure.kind, ProviderFailureKind::InvariantViolation);
    assert_eq!(port.plan_calls.load(Ordering::Acquire), 1);
    assert_eq!(port.apply_calls.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn fresh_check_invalidates_a_plan_bound_to_older_evidence() {
    let port = Arc::new(FakePort::new(PortBehavior {
        findings: vec![automatic_finding()],
        authorize_plan: true,
        ..PortBehavior::default()
    }));
    let clock = Arc::new(DeterministicClock::new(NOW));
    let (fixture, provider) = nixos_provider(port.clone(), clock);

    let initial_check = distinct_request(
        &fixture,
        ProviderMethod::SubstrateCheck,
        "initial-check",
        201,
    );
    let initial_context = context(&fixture, &initial_check);
    provider
        .check(&initial_context, &initial_check)
        .await
        .expect("initial check");

    let plan_request = request(&fixture, ProviderMethod::SubstratePlanRemediation);
    let plan_context = context(&fixture, &plan_request);
    let plan = provider
        .plan_remediation(&plan_context, &plan_request)
        .await
        .expect("plan bound to initial check");

    port.update(|behavior| behavior.check_fingerprint = fingerprint(903));
    let fresh_check =
        distinct_request(&fixture, ProviderMethod::SubstrateCheck, "fresh-check", 202);
    let fresh_context = context(&fixture, &fresh_check);
    provider
        .check(&fresh_context, &fresh_check)
        .await
        .expect("fresh check");
    let retried_plan = provider
        .plan_remediation(&plan_context, &plan_request)
        .await
        .expect("same planning operation remains idempotent");
    assert_eq!(retried_plan, plan);
    assert_eq!(port.plan_calls.load(Ordering::Acquire), 1);

    let apply_request = request(&fixture, ProviderMethod::SubstrateApply);
    let apply_context = context(&fixture, &apply_request);
    let failure = provider
        .apply(&apply_context, &plan)
        .await
        .expect_err("plan backed by older evidence must be stale");
    assert_eq!(failure.kind, ProviderFailureKind::InvalidRequest);
    assert_eq!(port.apply_calls.load(Ordering::Acquire), 0);
}

#[test]
fn findings_plans_support_and_diagnostics_are_bounded() {
    let too_many_diagnostics = vec![HostDiagnostic::EvidenceMissing; MAX_FINDING_DIAGNOSTICS + 1];
    assert_eq!(
        HostFinding::new(
            HostFindingKind::OwnershipDrift,
            HostFindingSeverity::Blocking,
            HostRemediationClass::OperatorAction,
            too_many_diagnostics,
            1,
        ),
        Err(HostModelError::BoundExceeded)
    );
    assert_eq!(
        HostFinding::new(
            HostFindingKind::OwnershipDrift,
            HostFindingSeverity::Blocking,
            HostRemediationClass::OperatorAction,
            Vec::new(),
            0,
        ),
        Err(HostModelError::InvalidBinding)
    );
    assert_eq!(
        HostSupportEvidence::new(vec![
            HostSupportEntry::new(HostCapability::CgroupV2, HostSupportStatus::Unknown),
            HostSupportEntry::new(HostCapability::CgroupV2, HostSupportStatus::Unsupported),
        ]),
        Err(HostModelError::DuplicateEntry)
    );

    let fixture = fixture(NIXOS_IMPLEMENTATION_ID);
    let check_request = request(&fixture, ProviderMethod::SubstrateCheck);
    let owner = d2b_provider_substrate_host::HostOperationOwner {
        realm_id: check_request.context.scope.realm_id().clone(),
        principal: check_request.context.principal.clone(),
    };
    let descriptor =
        d2b_provider_substrate_host::HostDescriptorBinding::from_descriptor(&fixture.descriptor);
    let repeated = HostFinding::new(
        HostFindingKind::OwnershipDrift,
        HostFindingSeverity::Blocking,
        HostRemediationClass::OperatorAction,
        Vec::new(),
        1,
    )
    .expect("finding");
    assert_eq!(
        HostCheckReport::new(
            HostSubstrateConfiguration::nixos(),
            descriptor.clone(),
            owner.clone(),
            check_request.context.binding(),
            NOW,
            fingerprint(100),
            HostSupportEvidence::default(),
            vec![repeated.clone(); MAX_CHECK_FINDINGS + 1],
        ),
        Err(HostModelError::BoundExceeded)
    );

    let kinds = [
        HostFindingKind::CheckUnavailable(HostCheckKind::KernelVersion),
        HostFindingKind::CheckUnavailable(HostCheckKind::CpuVirtualization),
        HostFindingKind::CheckUnavailable(HostCheckKind::CgroupV2),
        HostFindingKind::CheckUnavailable(HostCheckKind::UserNamespaces),
        HostFindingKind::CheckUnavailable(HostCheckKind::KernelModules),
        HostFindingKind::CheckUnavailable(HostCheckKind::DeviceAccess),
        HostFindingKind::CheckUnavailable(HostCheckKind::NetworkPolicy),
        HostFindingKind::CheckUnavailable(HostCheckKind::SysctlPolicy),
        HostFindingKind::CheckUnavailable(HostCheckKind::RunnerParity),
        HostFindingKind::CheckUnavailable(HostCheckKind::StateOwnership),
        HostFindingKind::CheckUnavailable(HostCheckKind::HostIdentity),
        HostFindingKind::KernelModuleMissing(HostKernelModule::KvmIntel),
        HostFindingKind::KernelModuleMissing(HostKernelModule::KvmAmd),
        HostFindingKind::KernelModuleMissing(HostKernelModule::VhostNet),
        HostFindingKind::KernelModuleMissing(HostKernelModule::Tun),
        HostFindingKind::KernelModuleMissing(HostKernelModule::VirtioFs),
        HostFindingKind::KernelModuleMissing(HostKernelModule::BridgeNetfilter),
    ];
    let all_diagnostics = vec![
        HostDiagnostic::EvidenceMissing,
        HostDiagnostic::ProbeUnavailable,
        HostDiagnostic::VersionTooOld,
        HostDiagnostic::RequiredComponentMissing,
        HostDiagnostic::ConfigurationMismatch,
        HostDiagnostic::OwnershipMismatch,
        HostDiagnostic::ConflictingOwner,
        HostDiagnostic::AuthorizationRequired,
    ];
    let diagnostic_heavy_findings: Vec<_> = kinds
        .into_iter()
        .map(|kind| {
            HostFinding::new(
                kind,
                HostFindingSeverity::Degraded,
                HostRemediationClass::OperatorAction,
                all_diagnostics.clone(),
                1,
            )
            .expect("individually bounded finding")
        })
        .collect();
    assert!(
        diagnostic_heavy_findings
            .iter()
            .map(|finding| finding.diagnostics().len())
            .sum::<usize>()
            > MAX_REPORT_DIAGNOSTICS
    );
    assert_eq!(
        HostCheckReport::new(
            HostSubstrateConfiguration::nixos(),
            descriptor.clone(),
            owner.clone(),
            check_request.context.binding(),
            NOW,
            fingerprint(102),
            HostSupportEvidence::default(),
            diagnostic_heavy_findings,
        ),
        Err(HostModelError::BoundExceeded)
    );

    let plan_request = request(&fixture, ProviderMethod::SubstratePlanRemediation);
    let automatic = automatic_finding();
    assert_eq!(
        HostRemediationPlan::authorized(
            HostRemediationId::parse("bounded-plan").expect("plan id"),
            HostSubstrateConfiguration::nixos(),
            descriptor,
            owner,
            plan_request.context.binding(),
            fingerprint(101),
            vec![automatic; MAX_PLAN_FINDINGS + 1],
            NOW,
            NOW + 1_000,
        ),
        Err(HostModelError::BoundExceeded)
    );
}

#[tokio::test]
async fn cancellation_and_deadline_fail_closed() {
    let port = Arc::new(FakePort::new(PortBehavior::default()));
    let clock = Arc::new(DeterministicClock::new(NOW));
    let (fixture, provider) = nixos_provider(port.clone(), clock);
    let request = request(&fixture, ProviderMethod::SubstrateCheck);
    let mut cancelled = context(&fixture, &request);
    cancelled.cancelled = true;
    let failure = provider
        .check(&cancelled, &request)
        .await
        .expect_err("cancelled check");
    assert_eq!(failure.kind, ProviderFailureKind::Cancelled);
    assert_eq!(port.check_calls.load(Ordering::Acquire), 0);

    port.set_delay(Duration::from_millis(30));
    let mut deadline = context(&fixture, &request);
    deadline.monotonic_deadline_remaining_ms = 2;
    let failure = provider
        .check(&deadline, &request)
        .await
        .expect_err("deadline check");
    assert_eq!(failure.kind, ProviderFailureKind::DeadlineExpired);
    assert_eq!(port.check_calls.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn plan_and_apply_are_idempotent_and_apply_passes_only_the_opaque_id() {
    let port = Arc::new(FakePort::new(PortBehavior {
        findings: vec![automatic_finding()],
        authorize_plan: true,
        ..PortBehavior::default()
    }));
    let clock = Arc::new(DeterministicClock::new(NOW));
    let (fixture, provider) = linux_provider(port.clone(), clock);
    port.set_delay(Duration::from_millis(10));

    let check_request = request(&fixture, ProviderMethod::SubstrateCheck);
    let check_context = context(&fixture, &check_request);
    let (first_check, second_check) = tokio::join!(
        provider.check(&check_context, &check_request),
        provider.check(&check_context, &check_request)
    );
    let first_check = first_check.expect("first check");
    let second_check = second_check.expect("concurrent idempotent check");
    assert_eq!(first_check, second_check);
    assert_eq!(port.check_calls.load(Ordering::Acquire), 1);

    let plan_request = request(&fixture, ProviderMethod::SubstratePlanRemediation);
    let plan_context = context(&fixture, &plan_request);
    let (first_plan, second_plan) = tokio::join!(
        provider.plan_remediation(&plan_context, &plan_request),
        provider.plan_remediation(&plan_context, &plan_request)
    );
    let first_plan = first_plan.expect("first plan");
    let second_plan = second_plan.expect("concurrent idempotent plan");
    assert_eq!(first_plan, second_plan);
    assert_eq!(port.plan_calls.load(Ordering::Acquire), 1);

    let apply_request = request(&fixture, ProviderMethod::SubstrateApply);
    let apply_context = context(&fixture, &apply_request);
    let (first_apply, second_apply) = tokio::join!(
        provider.apply(&apply_context, &first_plan),
        provider.apply(&apply_context, &first_plan)
    );
    let first_apply = first_apply.expect("first apply");
    let second_apply = second_apply.expect("concurrent idempotent apply");
    assert_eq!(first_apply, second_apply);
    assert_eq!(first_apply.state, MutationState::Applied);
    assert_eq!(port.apply_calls.load(Ordering::Acquire), 1);
    assert_eq!(port.effects.load(Ordering::Acquire), 1);
    assert_eq!(
        port.applied_ids
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_slice(),
        ["remediation-fixture"]
    );

    let mut tampered_retry = first_plan;
    tampered_retry.configuration_fingerprint = fingerprint(999);
    let failure = provider
        .apply(&apply_context, &tampered_retry)
        .await
        .expect_err("cached apply must still validate the complete plan");
    assert_eq!(failure.kind, ProviderFailureKind::InvalidRequest);
    assert_eq!(port.apply_calls.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn apply_deadline_after_dispatch_is_reported_as_ambiguous() {
    let port = Arc::new(FakePort::new(PortBehavior {
        findings: vec![automatic_finding()],
        authorize_plan: true,
        ..PortBehavior::default()
    }));
    let clock = Arc::new(DeterministicClock::new(NOW));
    let (fixture, provider) = nixos_provider(port.clone(), clock);
    let plan_request = request(&fixture, ProviderMethod::SubstratePlanRemediation);
    let plan_context = context(&fixture, &plan_request);
    let plan = provider
        .plan_remediation(&plan_context, &plan_request)
        .await
        .expect("plan");
    port.set_delay(Duration::from_millis(30));
    let apply_request = request(&fixture, ProviderMethod::SubstrateApply);
    let mut apply_context = context(&fixture, &apply_request);
    apply_context.monotonic_deadline_remaining_ms = 2;
    let receipt = provider
        .apply(&apply_context, &plan)
        .await
        .expect("ambiguous receipt");
    assert_eq!(receipt.state, MutationState::CompletionAmbiguous);
    assert!(receipt.observation_required_before_retry);
    assert_eq!(port.effects.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn non_applicable_plan_never_dispatches_remediation() {
    let port = Arc::new(FakePort::new(PortBehavior::default()));
    let clock = Arc::new(DeterministicClock::new(NOW));
    let (fixture, provider) = linux_provider(port.clone(), clock);
    let plan_request = request(&fixture, ProviderMethod::SubstratePlanRemediation);
    let plan_context = context(&fixture, &plan_request);
    let plan = provider
        .plan_remediation(&plan_context, &plan_request)
        .await
        .expect("not-applicable plan");
    assert!(plan.resources.is_empty());

    let apply_request = request(&fixture, ProviderMethod::SubstrateApply);
    let apply_context = context(&fixture, &apply_request);
    let receipt = provider
        .apply(&apply_context, &plan)
        .await
        .expect("not-applicable apply");
    assert_eq!(receipt.state, MutationState::NotApplicable);
    assert_eq!(port.apply_calls.load(Ordering::Acquire), 0);
}

#[test]
fn descriptors_advertise_exactly_the_three_substrate_methods() {
    let port = Arc::new(FakePort::new(PortBehavior::default()));
    let clock = Arc::new(DeterministicClock::new(NOW));
    let (_, provider) = nixos_provider(port, clock);
    let methods: Vec<_> = provider
        .capabilities()
        .as_slice()
        .iter()
        .map(|ProviderCapability(method)| *method)
        .collect();
    assert_eq!(
        methods,
        vec![
            ProviderMethod::SubstrateCheck,
            ProviderMethod::SubstratePlanRemediation,
            ProviderMethod::SubstrateApply,
        ]
    );
    assert_eq!(provider.descriptor().capabilities, provider.capabilities());
}
