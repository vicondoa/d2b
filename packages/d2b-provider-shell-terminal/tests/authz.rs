use d2b_provider_shell_terminal::{
    Authorizer, CallerOrigin, ExecutionTarget, PoolSpec, Role, ShellPool, ShellTerminalError,
    Subject,
};

fn host_pool() -> ShellPool {
    ShellPool::new(
        "host-alice",
        "dev",
        PoolSpec::new(
            ExecutionTarget::host("control"),
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
fn authorization_binds_admin_and_zone_to_the_current_request() {
    let admin = Subject::new("dev", CallerOrigin::Local, [Role::ShellAdmin]);
    assert!(Authorizer::authorize(&admin, &host_pool()).is_ok());

    let wrong_zone = Subject::new("other", CallerOrigin::Local, [Role::ZoneAdmin]);
    assert_eq!(
        Authorizer::authorize(&wrong_zone, &host_pool()),
        Err(ShellTerminalError::WrongZone)
    );
    let non_admin = Subject::new("dev", CallerOrigin::Local, [Role::Viewer]);
    assert_eq!(
        Authorizer::authorize(&non_admin, &host_pool()),
        Err(ShellTerminalError::NotAuthorized)
    );
    let relay = Subject::new("dev", CallerOrigin::Relay, [Role::ShellAdmin]);
    assert_eq!(
        Authorizer::authorize(&relay, &host_pool()),
        Err(ShellTerminalError::RelayHostUserDomainDenied)
    );
}
