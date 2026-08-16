use d2b_provider_shell_terminal::{
    AdoptionDecision, CallerOrigin, ExecutionTarget, OpenSessionRequest, PoolSpec, Role, ShellPool,
    ShellSession, ShellTerminalController, Subject, SupervisorCandidate, SupervisorIdentity,
};

fn pool() -> ShellPool {
    ShellPool::new(
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
    .unwrap()
}

#[test]
fn controller_creates_one_pool_derived_session_without_provider_state() {
    assert!(OpenSessionRequest::new("guest_alice", "main", None).is_err());
    let mut controller = ShellTerminalController::default();
    controller.insert_pool(pool()).unwrap();
    let request = Subject::new("dev", CallerOrigin::Local, [Role::ZoneAdmin]);
    let opened = controller
        .open_session(
            &request,
            OpenSessionRequest::new("guest-alice", "main", None).unwrap(),
        )
        .unwrap();

    assert_eq!(opened.session().workload_user(), "alice");
    assert_eq!(opened.supervisor_generation(), 1);
    assert!(controller.provider_state_is_empty());
    assert_eq!(controller.session_count("guest-alice"), 1);
}

#[test]
fn restored_sessions_block_recreation_after_controller_restart() {
    let mut first_controller = ShellTerminalController::default();
    first_controller.insert_pool(pool()).unwrap();
    let admin = Subject::new("dev", CallerOrigin::Local, [Role::ZoneAdmin]);
    let opened = first_controller
        .open_session(
            &admin,
            OpenSessionRequest::new("guest-alice", "main", None).unwrap(),
        )
        .unwrap();
    let identity =
        SupervisorIdentity::new([1; 32], [2; 32], opened.supervisor_generation()).unwrap();
    let mut old_supervisor = opened.start_supervisor(identity.clone()).unwrap();
    let old_attachment = old_supervisor
        .attach(
            &admin,
            d2b_provider_shell_terminal::AttachRequest::new(opened.supervisor_generation(), 0)
                .unwrap(),
        )
        .unwrap()
        .attachment();

    let mut recovered_controller = ShellTerminalController::default();
    recovered_controller
        .restore_pool_with_authority(pool(), opened.pool_attachment_authority())
        .unwrap();
    assert_eq!(
        recovered_controller
            .restore_session(
                opened.session().clone(),
                &identity,
                &[SupervisorCandidate::new(
                    opened.session().name(),
                    identity.clone(),
                )],
                opened.recovery_authority(),
            )
            .unwrap(),
        AdoptionDecision::Adopted
    );
    assert_eq!(recovered_controller.session_count("guest-alice"), 1);
    assert!(matches!(
        recovered_controller.open_session(
            &admin,
            OpenSessionRequest::new("guest-alice", "main", None).unwrap(),
        ),
        Err(d2b_provider_shell_terminal::ShellTerminalError::CapacityExceeded)
    ));
    let restarted = recovered_controller
        .restart_supervisor(&admin, opened.session().name())
        .unwrap();
    let mut new_supervisor = restarted
        .start_supervisor(
            SupervisorIdentity::new([3; 32], [4; 32], restarted.supervisor_generation()).unwrap(),
        )
        .unwrap();
    assert!(matches!(
        old_supervisor.attach(
            &admin,
            d2b_provider_shell_terminal::AttachRequest::new(opened.supervisor_generation(), 0,)
                .unwrap(),
        ),
        Err(d2b_provider_shell_terminal::ShellTerminalError::StaleSessionGeneration)
    ));
    assert!(matches!(
        new_supervisor.attach(
            &admin,
            d2b_provider_shell_terminal::AttachRequest::new(restarted.supervisor_generation(), 0,)
                .unwrap(),
        ),
        Err(d2b_provider_shell_terminal::ShellTerminalError::CapacityExceeded)
    ));
    old_supervisor.detach(&admin, old_attachment).unwrap();
    assert!(
        new_supervisor
            .attach(
                &admin,
                d2b_provider_shell_terminal::AttachRequest::new(
                    restarted.supervisor_generation(),
                    0,
                )
                .unwrap(),
            )
            .is_ok()
    );
}

#[test]
fn restarted_session_advances_generation_and_rejects_old_capability() {
    let mut controller = ShellTerminalController::default();
    controller.insert_pool(pool()).unwrap();
    let admin = Subject::new("dev", CallerOrigin::Local, [Role::ZoneAdmin]);
    let opened = controller
        .open_session(
            &admin,
            OpenSessionRequest::new("guest-alice", "main", None).unwrap(),
        )
        .unwrap();

    let restarted = controller
        .restart_supervisor(&admin, opened.session().name())
        .unwrap();
    assert_eq!(restarted.supervisor_generation(), 2);
    let mut supervisor = restarted
        .start_supervisor(SupervisorIdentity::new([3; 32], [4; 32], 2).unwrap())
        .unwrap();
    assert!(matches!(
        supervisor.attach_with_capability(&admin, opened.capability()),
        Err(d2b_provider_shell_terminal::ShellTerminalError::StaleSessionGeneration)
    ));
}

#[test]
fn restored_pool_attachment_count_blocks_new_attachments() {
    let restored_pool = ShellPool::new(
        "guest-alice",
        "dev",
        PoolSpec::new(
            ExecutionTarget::guest("work"),
            "alice",
            "artifact://shells/bash-login",
            2,
            1,
            4096,
        )
        .unwrap(),
    )
    .unwrap();
    let mut controller = ShellTerminalController::default();
    controller.restore_pool(restored_pool, 1).unwrap();
    let admin = Subject::new("dev", CallerOrigin::Local, [Role::ZoneAdmin]);
    let opened = controller
        .open_session(
            &admin,
            OpenSessionRequest::new("guest-alice", "main", None).unwrap(),
        )
        .unwrap();
    let mut supervisor = opened
        .start_supervisor(
            SupervisorIdentity::new([1; 32], [2; 32], opened.supervisor_generation()).unwrap(),
        )
        .unwrap();

    assert!(matches!(
        supervisor.attach(
            &admin,
            d2b_provider_shell_terminal::AttachRequest::new(opened.supervisor_generation(), 0,)
                .unwrap(),
        ),
        Err(d2b_provider_shell_terminal::ShellTerminalError::CapacityExceeded)
    ));
    controller
        .reconcile_pool_attachments("guest-alice", 0)
        .unwrap();
    assert!(
        supervisor
            .attach(
                &admin,
                d2b_provider_shell_terminal::AttachRequest::new(opened.supervisor_generation(), 0,)
                    .unwrap(),
            )
            .is_ok()
    );
}

#[test]
fn recovery_authorities_refuse_different_pool_or_session() {
    let mut controller = ShellTerminalController::default();
    let source_pool = pool();
    controller.insert_pool(source_pool.clone()).unwrap();
    let admin = Subject::new("dev", CallerOrigin::Local, [Role::ZoneAdmin]);
    let first = controller
        .open_session(
            &admin,
            OpenSessionRequest::new("guest-alice", "main", None).unwrap(),
        )
        .unwrap();
    let different_pool = ShellPool::new(
        "guest-bob",
        "dev",
        PoolSpec::new(
            ExecutionTarget::guest("work"),
            "bob",
            "artifact://shells/bash-login",
            1,
            1,
            4096,
        )
        .unwrap(),
    )
    .unwrap();
    let mut recovered_controller = ShellTerminalController::default();
    assert!(
        recovered_controller
            .restore_pool_with_authority(different_pool, first.pool_attachment_authority())
            .is_err()
    );

    let foreign_session =
        ShellSession::from_pool(&source_pool, "guest-alice-other", "other", None).unwrap();
    let identity = SupervisorIdentity::new([1; 32], [2; 32], 1).unwrap();
    let mut session_recovery = ShellTerminalController::default();
    session_recovery
        .restore_pool_with_authority(source_pool, first.pool_attachment_authority())
        .unwrap();
    assert_eq!(
        session_recovery
            .restore_session(
                foreign_session.clone(),
                &identity,
                &[SupervisorCandidate::new(
                    foreign_session.name(),
                    identity.clone(),
                )],
                first.recovery_authority(),
            )
            .unwrap(),
        AdoptionDecision::Ambiguous
    );
    assert!(matches!(
        session_recovery.restart_supervisor(&admin, foreign_session.name()),
        Err(d2b_provider_shell_terminal::ShellTerminalError::SupervisorAmbiguous)
    ));
}

#[test]
fn controller_and_open_result_debug_are_redacted() {
    let mut controller = ShellTerminalController::default();
    controller.insert_pool(pool()).unwrap();
    let admin = Subject::new("dev", CallerOrigin::Local, [Role::ZoneAdmin]);
    let opened = controller
        .open_session(
            &admin,
            OpenSessionRequest::new("guest-alice", "main", None).unwrap(),
        )
        .unwrap();

    let rendered = format!("{opened:?}");
    assert!(!rendered.contains("guest-alice"));
    assert!(!rendered.contains("alice"));
    assert!(!rendered.contains("artifact://"));
    let controller_rendered = format!("{controller:?}");
    assert!(!controller_rendered.contains("guest-alice"));
    assert!(!controller_rendered.contains("alice"));
}
