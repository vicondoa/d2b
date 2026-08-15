use d2b_contracts::v3::{ResourceRef, execution_policy::ExecutionDomain};
use d2b_process_conformance::testing::{ScriptedEffectPort, block_on, fixtures};
use d2b_process_conformance::{AdoptionOutcome, IdentityBinding, ProcessProvider, WaitReapOwner};
use d2b_provider_shell_terminal::SupervisorProcessLifecycle;

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

fn ticket() -> d2b_process_conformance::LaunchTicket {
    fixtures::ticket_builder()
        .selected_provider("system-systemd")
        .expected_identity(required())
        .domain(ExecutionDomain::User)
        .user_ref(Some(ResourceRef::parse("User/alice").unwrap()))
        .build()
        .unwrap()
}

#[test]
fn supervisor_lifecycle_uses_typed_user_domain_process_conformance() {
    let lifecycle = SupervisorProcessLifecycle::new(ScriptedEffectPort::launching(
        required(),
        WaitReapOwner::ServiceManager,
    ));
    assert!(block_on(lifecycle.launch(&ticket())).is_ok());

    let stale: Vec<_> = required()
        .into_iter()
        .filter(|binding| *binding != IdentityBinding::UnitInvocationId)
        .collect();
    let lifecycle = SupervisorProcessLifecycle::new(
        ScriptedEffectPort::launching(required(), WaitReapOwner::ServiceManager)
            .with_candidate(stale, WaitReapOwner::ServiceManager),
    );
    assert!(matches!(
        block_on(lifecycle.adopt(&ticket())).unwrap(),
        AdoptionOutcome::Quarantined(_)
    ));
}
