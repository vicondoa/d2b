//! Execution-parent and domain cells for `system-systemd`.
//!
//! `ADR-046-provider-model-and-packaging`, section "Provider/system-systemd",
//! implements Process and EphemeralProcess for systemd-capable Hosts *and*
//! Guests. The ResourceType and its status do not change with the execution
//! parent, so the non-Host parent is not a variant to special-case: these
//! cells hold Host and Guest to the same status shape, and hold the user
//! domain to the same exact-identity requirement under both parents.

use d2b_contracts::v3::ResourceRef;
use d2b_contracts::v3::execution_policy::ExecutionDomain;
use d2b_process_conformance::testing::{PortCall, ScriptedEffectPort, block_on, fixtures};
use d2b_process_conformance::{
    AdoptionCondition, AdoptionOutcome, IdentityBinding, ProcessConformanceError, ProcessProvider,
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

fn host_ref() -> ResourceRef {
    ResourceRef::parse("Host/host-system").expect("valid reference")
}

fn guest_ref() -> ResourceRef {
    ResourceRef::parse("Guest/dev-vm").expect("valid reference")
}

fn user_ref() -> ResourceRef {
    ResourceRef::parse("User/alice").expect("valid reference")
}

#[test]
fn a_non_host_execution_parent_yields_the_same_status_shape() {
    let provider = launching();
    let build = |execution_ref: ResourceRef| {
        fixtures::ticket_builder()
            .selected_provider(PROVIDER_NAME)
            .expected_identity(required())
            .execution_ref(execution_ref)
            .build()
            .expect("conformant ticket")
    };
    let on_host = block_on(provider.launch(&build(host_ref()))).expect("host launch");
    let on_guest = block_on(provider.launch(&build(guest_ref()))).expect("guest launch");

    assert_eq!(on_host.execution_ref, host_ref());
    assert_eq!(on_guest.execution_ref, guest_ref());
    // Every other field is identical: the execution parent is the only
    // difference a non-Host launch may produce.
    assert_eq!(on_host.provider, on_guest.provider);
    assert_eq!(on_host.phase, on_guest.phase);
    assert_eq!(on_host.adoption, on_guest.adoption);
    assert_eq!(on_host.wait_reap_owner, on_guest.wait_reap_owner);
    assert_eq!(on_host.domain, on_guest.domain);
    assert_eq!(on_host.user_ref, on_guest.user_ref);
    assert_eq!(on_host.digests, on_guest.digests);
    assert_eq!(on_host.identity, on_guest.identity);
}

#[test]
fn a_user_domain_launch_requires_an_exact_user_under_either_parent() {
    // A transient user scope is created through the fixed user supervisor
    // for one exact identity. The launch ticket refuses to be constructed
    // without one, under a Host and under a Guest alike, so the identity is
    // never defaulted; the controller's own check is defence in depth
    // rather than the only guard.
    for execution_ref in [host_ref(), guest_ref()] {
        assert_eq!(
            fixtures::ticket_builder()
                .selected_provider(PROVIDER_NAME)
                .expected_identity(required())
                .execution_ref(execution_ref)
                .domain(ExecutionDomain::User)
                .user_ref(None)
                .build()
                .unwrap_err(),
            ProcessConformanceError::UserRefRequired
        );
    }
}

#[test]
fn a_user_domain_launch_on_a_guest_carries_the_exact_user_through() {
    let ticket = fixtures::ticket_builder()
        .selected_provider(PROVIDER_NAME)
        .expected_identity(required())
        .execution_ref(guest_ref())
        .domain(ExecutionDomain::User)
        .user_ref(Some(user_ref()))
        .build()
        .expect("conformant ticket");
    let report = block_on(launching().launch(&ticket)).expect("guest user-domain launch");
    assert_eq!(report.domain, ExecutionDomain::User);
    assert_eq!(report.user_ref.as_ref(), Some(&user_ref()));
    assert_eq!(report.execution_ref, guest_ref());
}

#[test]
fn adoption_on_a_non_host_parent_verifies_identity_the_same_way() {
    let provider = provider(
        ScriptedEffectPort::launching(required(), WaitReapOwner::ServiceManager)
            .with_candidate(required(), WaitReapOwner::ServiceManager),
    );
    let ticket = fixtures::ticket_builder()
        .selected_provider(PROVIDER_NAME)
        .expected_identity(required())
        .execution_ref(guest_ref())
        .build()
        .expect("conformant ticket");
    match block_on(provider.adopt(&ticket)).expect("adoption reports") {
        AdoptionOutcome::Adopted(report) => {
            assert_eq!(report.adoption, AdoptionCondition::Adopted);
            assert_eq!(report.execution_ref, guest_ref());
        }
        other => panic!("expected adoption, observed {other:?}"),
    }
    assert_eq!(
        provider.port().calls(),
        vec![PortCall::Observe, PortCall::OpenPidfd]
    );
}

#[test]
fn a_candidate_whose_wait_owner_disagrees_is_quarantined() {
    // Every identity binding is verified, but the candidate is not owned by
    // the service manager. That disagreement is ambiguity about whose
    // process this is, so it quarantines and opens no pidfd.
    let provider = provider(
        ScriptedEffectPort::launching(required(), WaitReapOwner::ServiceManager)
            .with_candidate(required(), WaitReapOwner::Local),
    );
    let ticket = fixtures::ticket_builder()
        .selected_provider(PROVIDER_NAME)
        .expected_identity(required())
        .build()
        .expect("conformant ticket");
    assert!(matches!(
        block_on(provider.adopt(&ticket)).expect("adoption reports"),
        AdoptionOutcome::Quarantined(_)
    ));
    assert!(!provider.port().calls().contains(&PortCall::OpenPidfd));
}
