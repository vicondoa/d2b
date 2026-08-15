use d2b_provider_shell_terminal::{
    CallerOrigin, ExecutionTarget, OpenSessionRequest, PoolSpec, Role, ShellPool,
    ShellTerminalController, Subject,
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
