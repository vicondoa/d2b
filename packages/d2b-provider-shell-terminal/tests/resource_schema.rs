use d2b_provider_shell_terminal::{
    DEFAULT_OUTPUT_RING_CAPACITY, ExecutionTarget, PoolSpec, SessionPhase, ShellPool, ShellSession,
};

#[test]
fn pool_and_session_keep_qualified_schema_and_inherited_fields() {
    let pool = ShellPool::new(
        "dev-alice-shell",
        "dev",
        PoolSpec::new(
            ExecutionTarget::guest("work"),
            "alice",
            "artifact://shells/bash-login",
            8,
            1,
            DEFAULT_OUTPUT_RING_CAPACITY,
        )
        .expect("valid pool specification"),
    )
    .expect("valid qualified pool");

    assert_eq!(pool.resource_type(), "shell-terminal.d2bus.org.ShellPool");
    assert_eq!(pool.active_session_capacity(), 8);

    let session = ShellSession::from_pool(&pool, "dev-alice-shell-main", "main", None)
        .expect("pool-derived session");
    assert_eq!(
        session.resource_type(),
        "shell-terminal.d2bus.org.ShellSession"
    );
    assert_eq!(session.phase(), SessionPhase::Pending);
    assert_eq!(session.execution_target(), pool.execution_target());
    assert_eq!(session.workload_user(), pool.workload_user());
    assert_eq!(session.output_ring_capacity(), DEFAULT_OUTPUT_RING_CAPACITY);
}
