use d2b_contracts::v3::{ResourceRef, execution_policy::ExecutionDomain};
use d2b_process_conformance::testing::{ScriptedEffectPort, block_on, fixtures};
use d2b_process_conformance::{
    AdoptionOutcome, IdentityBinding, ProcessIdentityDigest, ProcessProvider, WaitReapOwner,
};
use d2b_provider_shell_terminal::{
    ExecutionTarget, PoolSpec, ShellPool, ShellSession, SupervisorProcessLifecycle,
};

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
        .execution_ref(ResourceRef::parse("Guest/work").unwrap())
        .domain(ExecutionDomain::User)
        .user_ref(Some(ResourceRef::parse("User/alice").unwrap()))
        .build()
        .unwrap()
}

fn session() -> ShellSession {
    let pool = ShellPool::new(
        "guest-alice",
        "dev",
        PoolSpec::new(
            ExecutionTarget::guest("work"),
            "alice",
            "artifact://shells/bash-login",
            1,
            1,
            4096,
        )
        .unwrap(),
    )
    .unwrap();
    ShellSession::from_pool(&pool, "guest-alice-main", "main", None).unwrap()
}

#[test]
fn supervisor_lifecycle_uses_typed_user_domain_process_conformance() {
    let lifecycle = SupervisorProcessLifecycle::for_session(
        ScriptedEffectPort::launching(required(), WaitReapOwner::ServiceManager),
        &session(),
    );
    assert!(block_on(lifecycle.launch(&ticket())).is_ok());
    assert_eq!(
        format!("{lifecycle:?}"),
        "SupervisorProcessLifecycle(<redacted>)"
    );

    let lifecycle = SupervisorProcessLifecycle::for_session(
        ScriptedEffectPort::launching(required(), WaitReapOwner::Local),
        &session(),
    );
    assert!(matches!(
        block_on(lifecycle.launch(&ticket())),
        Err(d2b_process_conformance::ProcessConformanceError::WaitOwnerMismatch)
    ));

    let stale: Vec<_> = required()
        .into_iter()
        .filter(|binding| *binding != IdentityBinding::UnitInvocationId)
        .collect();
    let lifecycle = SupervisorProcessLifecycle::for_session(
        ScriptedEffectPort::launching(required(), WaitReapOwner::ServiceManager)
            .with_candidate(stale, WaitReapOwner::ServiceManager),
        &session(),
    );
    assert!(matches!(
        block_on(lifecycle.adopt(&ticket())).unwrap(),
        AdoptionOutcome::Quarantined(_)
    ));

    let mismatched_identity = ticket()
        .with_expected_identity_digest(ProcessIdentityDigest::from_bytes([0x22; 32]))
        .unwrap();
    let lifecycle = SupervisorProcessLifecycle::for_session(
        ScriptedEffectPort::launching(required(), WaitReapOwner::ServiceManager)
            .with_candidate(required(), WaitReapOwner::ServiceManager),
        &session(),
    );
    assert!(matches!(
        block_on(lifecycle.adopt(&mismatched_identity)).unwrap(),
        AdoptionOutcome::Quarantined(_)
    ));

    let wrong_user = fixtures::ticket_builder()
        .selected_provider("system-systemd")
        .expected_identity(required())
        .execution_ref(ResourceRef::parse("Guest/work").unwrap())
        .domain(ExecutionDomain::User)
        .user_ref(Some(ResourceRef::parse("User/bob").unwrap()))
        .build()
        .unwrap();
    let lifecycle = SupervisorProcessLifecycle::for_session(
        ScriptedEffectPort::launching(required(), WaitReapOwner::ServiceManager),
        &session(),
    );
    assert!(block_on(lifecycle.launch(&wrong_user)).is_err());
}
