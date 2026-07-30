//! The shared Process conformance suite run against `system-minijail`,
//! plus the minijail-specific pidfd, wait-ownership, and adoption
//! obligations.

use d2b_process_conformance::suite;
use d2b_process_conformance::testing::{PortCall, ScriptedEffectPort, block_on, fixtures};
use d2b_process_conformance::{
    AdoptionOutcome, IdentityBinding, ProcessConformanceError, ProcessProvider, WaitReapOwner,
};
use d2b_provider_system_minijail::{MinijailProcessProvider, PROVIDER_NAME};

fn required() -> Vec<IdentityBinding> {
    vec![
        IdentityBinding::Pid,
        IdentityBinding::ProcessStartTime,
        IdentityBinding::Cgroup,
        IdentityBinding::Executable,
        IdentityBinding::Template,
        IdentityBinding::Generation,
    ]
}

fn provider(port: ScriptedEffectPort) -> MinijailProcessProvider<ScriptedEffectPort> {
    MinijailProcessProvider::new(port)
}

fn launching() -> MinijailProcessProvider<ScriptedEffectPort> {
    provider(ScriptedEffectPort::launching(
        required(),
        WaitReapOwner::Local,
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
fn d2b_owns_wait_and_reap() {
    assert_eq!(
        launching().profile().wait_reap_owner(),
        WaitReapOwner::Local
    );
    let mismatched = provider(ScriptedEffectPort::launching(
        required(),
        WaitReapOwner::ServiceManager,
    ));
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

#[test]
fn the_user_domain_is_admitted_only_by_the_descriptor() {
    let ticket = |provider_name: &str| {
        fixtures::ticket_builder()
            .selected_provider(provider_name)
            .expected_identity(required())
            .domain(d2b_contracts::v3::execution_policy::ExecutionDomain::User)
            .user_ref(Some(
                d2b_contracts::v3::ResourceRef::parse("User/alice").expect("valid reference"),
            ))
            .build()
            .expect("conformant ticket")
    };

    let default = launching();
    assert_eq!(
        block_on(default.launch(&ticket(PROVIDER_NAME))).unwrap_err(),
        ProcessConformanceError::DomainNotSupported
    );

    let admitted = MinijailProcessProvider::with_user_domain(
        ScriptedEffectPort::launching(required(), WaitReapOwner::Local),
        true,
    );
    assert!(block_on(admitted.launch(&ticket(PROVIDER_NAME))).is_ok());
}

#[test]
fn a_reused_pid_without_a_matching_start_time_is_quarantined() {
    // The daemon's pidfd table already treats a pid whose start time does
    // not match as a different process. The same rule is an adoption
    // ambiguity here: quarantine, and never open a pidfd.
    let stale: Vec<IdentityBinding> = required()
        .into_iter()
        .filter(|binding| *binding != IdentityBinding::ProcessStartTime)
        .collect();
    let provider = provider(
        ScriptedEffectPort::launching(required(), WaitReapOwner::Local)
            .with_candidate(stale, WaitReapOwner::Local),
    );
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
fn a_fully_verified_candidate_is_adopted_after_observation() {
    let provider = provider(
        ScriptedEffectPort::launching(required(), WaitReapOwner::Local)
            .with_candidate(required(), WaitReapOwner::Local),
    );
    let ticket = fixtures::ticket_builder()
        .selected_provider(PROVIDER_NAME)
        .expected_identity(required())
        .build()
        .expect("conformant ticket");
    assert!(matches!(
        block_on(provider.adopt(&ticket)).expect("adoption reports"),
        AdoptionOutcome::Adopted(_)
    ));
    let calls = provider.port().calls();
    assert_eq!(calls, vec![PortCall::Observe, PortCall::OpenPidfd]);
    suite::assert_pidfd_open_follows_verification(&calls);
}
