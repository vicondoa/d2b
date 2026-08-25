//! The shared Process conformance suite run against `system-systemd`,
//! plus the systemd-specific identity and adoption obligations.

use d2b_contracts_resource::v3::ResourceGeneration;
use d2b_contracts_resource::v3::identity::ReconnectGeneration;
use d2b_process_conformance::suite;
use d2b_process_conformance::testing::{PortCall, ScriptedEffectPort, block_on, fixtures};
use d2b_process_conformance::{
    AdoptionOutcome, ConfigurationDigest, IdentityBinding, ProcessConformanceError,
    ProcessIdentityDigest, ProcessPhaseClass, ProcessProvider, ReadinessExpectation, StopClass,
    WaitReapOwner,
};
use d2b_provider_system_systemd::{PROVIDER_NAME, SystemdProcessProvider};

fn required() -> Vec<IdentityBinding> {
    vec![
        IdentityBinding::UnitInvocationId,
        IdentityBinding::Cgroup,
        IdentityBinding::UnitMainPid,
        IdentityBinding::ProcessStartTime,
        IdentityBinding::Template,
        IdentityBinding::Generation,
    ]
}

fn provider(port: ScriptedEffectPort) -> SystemdProcessProvider<ScriptedEffectPort> {
    SystemdProcessProvider::new(port)
}

fn launching() -> SystemdProcessProvider<ScriptedEffectPort> {
    provider(ScriptedEffectPort::launching(
        required(),
        WaitReapOwner::ServiceManager,
    ))
}

#[test]
fn shared_conformance_holds() {
    suite::assert_launch_is_locality_neutral(&launching(), PROVIDER_NAME);
    suite::assert_foreign_provider_selection_is_rejected(&launching());
    suite::assert_domain_support_matches_the_profile(&launching(), PROVIDER_NAME);
    suite::assert_status_is_redacted(&launching(), PROVIDER_NAME);
    suite::assert_incomplete_launch_identity_fails_closed(provider, PROVIDER_NAME);
    suite::assert_adoption_verifies_identity_before_opening_a_pidfd(provider, PROVIDER_NAME);
    suite::assert_finalizer_requires_verified_stop(WaitReapOwner::ServiceManager);
}

#[test]
fn systemd_owns_wait_and_reap() {
    let provider = launching();
    assert_eq!(
        provider.profile().wait_reap_owner(),
        WaitReapOwner::ServiceManager
    );
    let mismatched = provider_with_wait_owner(WaitReapOwner::Local);
    let ticket = fixtures::ticket_builder()
        .selected_provider(PROVIDER_NAME)
        .expected_identity(required())
        .build()
        .expect("conformant ticket");
    assert_eq!(
        block_on(mismatched.launch(&ticket)).unwrap_err(),
        ProcessConformanceError::WaitOwnerMismatch
    );
    assert_eq!(mismatched.port().calls(), vec![PortCall::Launch]);
}

fn provider_with_wait_owner(owner: WaitReapOwner) -> SystemdProcessProvider<ScriptedEffectPort> {
    provider(ScriptedEffectPort::launching(required(), owner))
}

#[test]
fn a_unit_name_alone_is_not_identity() {
    // The InvocationID, cgroup, unit main process, its start time, the
    // template, and the generation are all required; nothing here is a
    // unit name, and dropping any single binding fails closed.
    let profile_bindings = launching();
    let full = profile_bindings.profile().required_identity_bindings();
    assert_eq!(full.len(), required().len());
    for dropped in required() {
        let partial: Vec<IdentityBinding> =
            required().into_iter().filter(|b| *b != dropped).collect();
        let provider = provider(ScriptedEffectPort::launching(
            partial,
            WaitReapOwner::ServiceManager,
        ));
        let ticket = fixtures::ticket_builder()
            .selected_provider(PROVIDER_NAME)
            .expected_identity(required())
            .build()
            .expect("conformant ticket");
        assert_eq!(
            block_on(provider.launch(&ticket)).unwrap_err(),
            ProcessConformanceError::IdentityUnverified
        );
    }
}

#[test]
fn adoption_never_opens_a_pidfd_for_an_ambiguous_scope() {
    let port = ScriptedEffectPort::launching(required(), WaitReapOwner::ServiceManager)
        .with_candidate(
            [IdentityBinding::UnitInvocationId],
            WaitReapOwner::ServiceManager,
        );
    let provider = provider(port);
    let ticket = fixtures::ticket_builder()
        .selected_provider(PROVIDER_NAME)
        .expected_identity(required())
        .build()
        .expect("conformant ticket");
    let outcome = block_on(provider.adopt(&ticket)).expect("adoption reports");
    assert!(matches!(outcome, AdoptionOutcome::Quarantined(_)));
    let calls = provider.port().calls();
    assert!(!calls.contains(&PortCall::OpenPidfd));
    suite::assert_pidfd_open_follows_verification(&calls);
}

#[test]
fn malformed_readiness_is_rejected_before_effect_dispatch() {
    let provider = provider(ScriptedEffectPort::launching(
        required(),
        WaitReapOwner::ServiceManager,
    ));
    let ticket = fixtures::ticket_builder()
        .selected_provider(PROVIDER_NAME)
        .expected_identity(required())
        .build()
        .expect("conformant ticket")
        .with_readiness(ReadinessExpectation::Condition { timeout_ms: 0 });

    assert_eq!(
        block_on(provider.launch(&ticket)).unwrap_err(),
        ProcessConformanceError::InvalidTicket
    );
    assert!(provider.port().calls().is_empty());
}

#[test]
fn readiness_is_verified_before_reporting_ready() {
    let provider = provider(
        ScriptedEffectPort::launching(required(), WaitReapOwner::ServiceManager)
            .with_candidate(required(), WaitReapOwner::ServiceManager),
    );
    let ticket = fixtures::ticket_builder()
        .selected_provider(PROVIDER_NAME)
        .expected_identity(required())
        .build()
        .expect("conformant ticket")
        .with_readiness(ReadinessExpectation::condition(1_000).expect("bounded readiness"));

    let report = block_on(provider.launch(&ticket)).expect("ready launch");
    assert_eq!(report.phase, ProcessPhaseClass::Ready);
    assert_eq!(
        provider.port().calls(),
        vec![PortCall::Launch, PortCall::Observe]
    );
}

#[test]
fn a_readiness_timeout_stops_the_exact_launched_identity() {
    let provider = launching();
    let ticket = fixtures::ticket_builder()
        .selected_provider(PROVIDER_NAME)
        .expected_identity(required())
        .build()
        .expect("conformant ticket")
        .with_readiness(ReadinessExpectation::condition(1_000).expect("bounded readiness"));

    assert_eq!(
        block_on(provider.launch(&ticket)).unwrap_err(),
        ProcessConformanceError::DeadlineExceeded
    );
    assert_eq!(
        provider.port().calls(),
        vec![
            PortCall::Launch,
            PortCall::Observe,
            PortCall::Stop(StopClass::Terminate)
        ]
    );
}

#[test]
fn an_adoption_identity_seal_mismatch_is_quarantined_before_pidfd_open() {
    let provider = provider(
        ScriptedEffectPort::launching(required(), WaitReapOwner::ServiceManager)
            .with_candidate(required(), WaitReapOwner::ServiceManager),
    );
    let ticket = fixtures::ticket_builder()
        .selected_provider(PROVIDER_NAME)
        .expected_identity(required())
        .build()
        .expect("conformant ticket")
        .with_expected_identity_digest(ProcessIdentityDigest::from_bytes([0x22; 32]))
        .expect("nonzero identity seal");

    assert!(matches!(
        block_on(provider.adopt(&ticket)).expect("adoption result"),
        AdoptionOutcome::Quarantined(report)
            if report.phase == ProcessPhaseClass::Unknown
    ));
    assert_eq!(provider.port().calls(), vec![PortCall::Observe]);
}

#[test]
fn a_launch_identity_seal_mismatch_fails_closed_without_signalling() {
    let provider = launching();
    let ticket = fixtures::ticket_builder()
        .selected_provider(PROVIDER_NAME)
        .expected_identity(required())
        .build()
        .expect("conformant ticket")
        .with_expected_identity_digest(ProcessIdentityDigest::from_bytes([0x22; 32]))
        .expect("nonzero identity seal");

    assert_eq!(
        block_on(provider.launch(&ticket)).unwrap_err(),
        ProcessConformanceError::TerminalEvidenceMismatch
    );
    assert_eq!(provider.port().calls(), vec![PortCall::Launch]);
}

#[test]
fn stopping_a_zero_identity_is_rejected_without_an_effect() {
    let provider = launching();
    let identity = ProcessIdentityDigest::from_bytes([0; 32]);

    assert_eq!(
        block_on(provider.stop(&identity, StopClass::Terminate)).unwrap_err(),
        ProcessConformanceError::IdentityUnverified
    );
    assert!(provider.port().calls().is_empty());
}

#[test]
fn controller_authority_requires_a_committed_revision_before_launch() {
    let controller_ticket = fixtures::ticket_builder()
        .selected_provider(PROVIDER_NAME)
        .expected_identity(required())
        .build()
        .expect("conformant ticket")
        .with_controller_launch_binding(
            ResourceGeneration::new(2).expect("provider generation"),
            ReconnectGeneration::new(4).expect("session generation"),
            ConfigurationDigest::from_bytes([1; 32]),
            ConfigurationDigest::from_bytes([2; 32]),
        )
        .expect("controller binding");
    let provider = launching();
    assert_eq!(
        block_on(provider.launch(&controller_ticket)).unwrap_err(),
        ProcessConformanceError::InvalidTicket
    );
    assert!(provider.port().calls().is_empty());

    let assignment_ticket = fixtures::ticket_builder()
        .selected_provider(PROVIDER_NAME)
        .expected_identity(required())
        .build()
        .expect("conformant ticket")
        .with_assignment_binding(
            ResourceGeneration::new(2).expect("provider generation"),
            ReconnectGeneration::new(4).expect("session generation"),
            9,
            ConfigurationDigest::from_bytes([3; 32]),
        )
        .expect("assignment binding");
    let provider = launching();
    assert_eq!(
        block_on(provider.launch(&assignment_ticket)).unwrap_err(),
        ProcessConformanceError::InvalidTicket
    );
    assert!(provider.port().calls().is_empty());
}

#[test]
fn a_user_domain_process_is_placed_in_a_verified_user_scope() {
    let provider = launching();
    let user_ref = d2b_contracts_resource::v3::ResourceRef::parse("User/alice")
        .expect("valid fixture reference");
    let ticket = fixtures::ticket_builder()
        .selected_provider(PROVIDER_NAME)
        .expected_identity(required())
        .domain(d2b_contracts_resource::v3::execution_policy::ExecutionDomain::User)
        .user_ref(Some(user_ref.clone()))
        .build()
        .expect("conformant ticket");
    let report = block_on(provider.launch(&ticket)).expect("user-domain launch");
    assert_eq!(report.user_ref.as_ref(), Some(&user_ref));
}
