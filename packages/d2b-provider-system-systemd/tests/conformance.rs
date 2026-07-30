//! The shared Process conformance suite run against `system-systemd`,
//! plus the systemd-specific identity and adoption obligations.

use d2b_process_conformance::suite;
use d2b_process_conformance::testing::{PortCall, ScriptedEffectPort, block_on, fixtures};
use d2b_process_conformance::{
    AdoptionOutcome, IdentityBinding, ProcessConformanceError, ProcessProvider, WaitReapOwner,
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
fn a_user_domain_process_is_placed_in_a_verified_user_scope() {
    let provider = launching();
    let user_ref =
        d2b_contracts::v3::ResourceRef::parse("User/alice").expect("valid fixture reference");
    let ticket = fixtures::ticket_builder()
        .selected_provider(PROVIDER_NAME)
        .expected_identity(required())
        .domain(d2b_contracts::v3::execution_policy::ExecutionDomain::User)
        .user_ref(Some(user_ref.clone()))
        .build()
        .expect("conformant ticket");
    let report = block_on(provider.launch(&ticket)).expect("user-domain launch");
    assert_eq!(report.user_ref.as_ref(), Some(&user_ref));
}
