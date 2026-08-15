use d2b_provider_shell_terminal::{
    AttachRequest, CallerOrigin, ExecutionTarget, OpenSessionRequest, PoolSpec, Role, ShellPool,
    ShellTerminalController, ShellTerminalError, Subject, SupervisorIdentity,
};

#[test]
fn supervisor_rejects_stale_generation_and_reused_capability() {
    let mut controller = ShellTerminalController::default();
    controller
        .insert_pool(
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
            .unwrap(),
        )
        .unwrap();
    let admin = Subject::new("dev", CallerOrigin::Local, [Role::ShellAdmin]);
    let opened = controller
        .open_session(
            &admin,
            OpenSessionRequest::new("guest-alice", "main", None).unwrap(),
        )
        .unwrap();
    assert!(
        opened
            .start_supervisor(SupervisorIdentity::new([1; 32], [2; 32], 2).unwrap())
            .is_err()
    );
    let mut supervisor = opened
        .start_supervisor(
            SupervisorIdentity::new([1; 32], [2; 32], opened.supervisor_generation()).unwrap(),
        )
        .unwrap();

    assert!(
        supervisor
            .attach(
                &admin,
                AttachRequest::new(opened.supervisor_generation() - 1, 0).unwrap(),
            )
            .is_err()
    );
    let capability = opened.capability();
    assert!(
        supervisor
            .attach_with_capability(&admin, capability.clone())
            .is_ok()
    );
    assert!(
        supervisor
            .attach_with_capability(&admin, capability)
            .is_err()
    );
}

#[test]
fn detach_releases_the_bounded_attachment_slot() {
    let mut controller = ShellTerminalController::default();
    controller
        .insert_pool(
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
            .unwrap(),
        )
        .unwrap();
    let admin = Subject::new("dev", CallerOrigin::Local, [Role::ShellAdmin]);
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

    let first = supervisor
        .attach(
            &admin,
            AttachRequest::new(opened.supervisor_generation(), 0).unwrap(),
        )
        .unwrap();
    assert!(
        supervisor
            .attach(
                &admin,
                AttachRequest::new(opened.supervisor_generation(), 0).unwrap(),
            )
            .is_err()
    );
    supervisor.detach(&admin, first.attachment()).unwrap();
    assert!(
        supervisor
            .attach(
                &admin,
                AttachRequest::new(opened.supervisor_generation(), 0).unwrap(),
            )
            .is_ok()
    );
}

#[test]
fn capability_cannot_attach_a_different_session() {
    let mut controller = ShellTerminalController::default();
    for (name, session) in [("guest-alice", "main"), ("guest-bob", "other")] {
        controller
            .insert_pool(
                ShellPool::new(
                    name,
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
                .unwrap(),
            )
            .unwrap();
        assert!(
            OpenSessionRequest::new(name, session, None).is_ok(),
            "fixture request must be valid"
        );
    }
    let admin = Subject::new("dev", CallerOrigin::Local, [Role::ShellAdmin]);
    let first = controller
        .open_session(
            &admin,
            OpenSessionRequest::new("guest-alice", "main", None).unwrap(),
        )
        .unwrap();
    let second = controller
        .open_session(
            &admin,
            OpenSessionRequest::new("guest-bob", "other", None).unwrap(),
        )
        .unwrap();
    let mut second_supervisor = second
        .start_supervisor(
            SupervisorIdentity::new([3; 32], [4; 32], second.supervisor_generation()).unwrap(),
        )
        .unwrap();

    assert!(matches!(
        second_supervisor.attach_with_capability(&admin, first.capability()),
        Err(ShellTerminalError::CapabilitySessionMismatch)
    ));
}

#[test]
fn attachments_share_the_pool_limit_across_sessions() {
    let mut controller = ShellTerminalController::default();
    controller
        .insert_pool(
            ShellPool::new(
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
            .unwrap(),
        )
        .unwrap();
    let admin = Subject::new("dev", CallerOrigin::Local, [Role::ShellAdmin]);
    let first = controller
        .open_session(
            &admin,
            OpenSessionRequest::new("guest-alice", "main", None).unwrap(),
        )
        .unwrap();
    let second = controller
        .open_session(
            &admin,
            OpenSessionRequest::new("guest-alice", "other", None).unwrap(),
        )
        .unwrap();
    let mut first_supervisor = first
        .start_supervisor(
            SupervisorIdentity::new([1; 32], [2; 32], first.supervisor_generation()).unwrap(),
        )
        .unwrap();
    let mut second_supervisor = second
        .start_supervisor(
            SupervisorIdentity::new([3; 32], [4; 32], second.supervisor_generation()).unwrap(),
        )
        .unwrap();

    assert!(
        first_supervisor
            .attach(
                &admin,
                AttachRequest::new(first.supervisor_generation(), 0).unwrap(),
            )
            .is_ok()
    );
    assert!(matches!(
        second_supervisor.attach(
            &admin,
            AttachRequest::new(second.supervisor_generation(), 0).unwrap(),
        ),
        Err(ShellTerminalError::CapacityExceeded)
    ));
}

#[test]
fn supervisor_replays_output_recorded_before_reconnect() {
    let mut controller = ShellTerminalController::default();
    controller
        .insert_pool(
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
            .unwrap(),
        )
        .unwrap();
    let admin = Subject::new("dev", CallerOrigin::Local, [Role::ShellAdmin]);
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

    supervisor.record_pty_output(b"hello");
    let replay = supervisor
        .attach(
            &admin,
            AttachRequest::new(opened.supervisor_generation(), 5).unwrap(),
        )
        .unwrap()
        .replay()
        .bytes()
        .to_vec();
    assert_eq!(replay, b"hello");
}
