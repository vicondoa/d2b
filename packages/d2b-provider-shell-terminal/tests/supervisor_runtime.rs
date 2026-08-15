use d2b_provider_shell_terminal::{
    AttachRequest, CallerOrigin, ExecutionTarget, OpenSessionRequest, PoolSpec, Role, ShellPool,
    ShellTerminalController, Subject, SupervisorIdentity,
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
            .attach_with_capability(&admin, capability)
            .is_ok()
    );
    assert!(
        supervisor
            .attach_with_capability(&admin, capability)
            .is_err()
    );
}
