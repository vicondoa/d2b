use d2b_contracts_resource::v3::ResourceRef;
use d2b_contracts_resource::v3::identity::ReconnectGeneration;
use d2b_provider_shell_terminal::{
    AttachRequest, Authorizer, CallerOrigin, ExecutionTarget, InMemoryShellAuthority,
    OpenSessionRequest, PoolSpec, Role, ShellPool, ShellSession, ShellTerminalController,
    ShellTerminalError, Subject, SupervisorIdentity, TerminalAttachEvidence, TerminalStreamBinding,
    WorkloadIdentity,
};
use std::sync::Arc;

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

/// Authorization is decided before any route or capacity lookup, and the
/// requester that opened the session stays the one every later stream verb is
/// measured against.
#[test]
fn the_requester_identity_survives_every_later_stream_verb() {
    let authority = Arc::new(InMemoryShellAuthority::new());
    let mut controller = ShellTerminalController::new(authority);
    controller.insert_pool(host_pool()).unwrap();
    let admin = Subject::new("dev", CallerOrigin::Local, [Role::ZoneAdmin]);
    let opened = controller
        .open_session(
            &admin,
            OpenSessionRequest::new("host-alice", "main", None).unwrap(),
        )
        .unwrap();
    let session = opened.session().clone();
    let mut supervisor = opened
        .start_supervisor_for(
            SupervisorIdentity::new([1; 32], [2; 32], opened.supervisor_generation()).unwrap(),
            &WorkloadIdentity::proven(ResourceRef::parse("User/alice").unwrap()),
        )
        .unwrap();

    let stream = TerminalStreamBinding::admitted(
        session.supervisor_process_ref().clone(),
        ResourceRef::parse("Endpoint/host-alice-main-terminal").unwrap(),
        ReconnectGeneration::new(1).unwrap(),
    );
    let admitted = TerminalAttachEvidence::presented(
        session.supervisor_process_ref().clone(),
        ResourceRef::parse("Endpoint/host-alice-main-terminal").unwrap(),
        ReconnectGeneration::new(1).unwrap(),
    );
    let request = AttachRequest::new(opened.supervisor_generation(), 0).unwrap();

    // A Zone administrator is the authorized requester for this session.
    let attachment = supervisor
        .attach_admitted(&admin, request, &stream, &admitted)
        .unwrap()
        .attachment();

    // A caller from another Zone, a viewer, and a relay-origin caller are all
    // refused at the authorization stage, before the attachment census moves:
    // the reserved slot is still the one the authorized requester holds.
    for (refused, expected) in [
        (
            Subject::new("other", CallerOrigin::Local, [Role::ZoneAdmin]),
            ShellTerminalError::WrongZone,
        ),
        (
            Subject::new("dev", CallerOrigin::Local, [Role::Viewer]),
            ShellTerminalError::NotAuthorized,
        ),
        (
            Subject::new("dev", CallerOrigin::Relay, [Role::ZoneAdmin]),
            ShellTerminalError::RelayHostUserDomainDenied,
        ),
    ] {
        assert_eq!(
            supervisor
                .attach_admitted(&refused, request, &stream, &admitted)
                .expect_err("a refused caller cannot attach"),
            expected
        );
        assert_eq!(
            supervisor
                .detach(&refused, attachment.clone())
                .expect_err("a refused caller cannot release"),
            expected
        );
    }

    // The authorized requester's slot survived every refused caller.
    supervisor.detach(&admin, attachment).unwrap();
    assert!(
        supervisor
            .attach_admitted(&admin, request, &stream, &admitted)
            .is_ok(),
        "the attachment the refused callers could not release is still theirs"
    );

    // The same refusals hold on the controller's session verbs.
    let viewer = Subject::new("dev", CallerOrigin::Local, [Role::Viewer]);
    assert_eq!(
        controller.finalize_session(&viewer, session.name(), None),
        Err(ShellTerminalError::NotAuthorized)
    );
    assert_eq!(
        controller
            .restart_supervisor(&viewer, session.name(), None)
            .expect_err("a viewer cannot restart"),
        ShellTerminalError::NotAuthorized
    );
    // A different Zone's administrator cannot reach this session either.
    let other_zone = Subject::new("other", CallerOrigin::Local, [Role::ZoneAdmin]);
    assert_eq!(
        controller.finalize_session(&other_zone, session.name(), None),
        Err(ShellTerminalError::WrongZone)
    );
}

/// A session's supervisor identity is frozen from its pool, so a session row
/// cannot point its supervisor at another user.
#[test]
fn a_session_freezes_the_pools_workload_identity() {
    let pool = ShellPool::new(
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
    .unwrap();
    let session = ShellSession::from_pool(&pool, "host-alice-main", "main", None).unwrap();
    assert_eq!(
        session.supervisor_user_ref().to_canonical_string(),
        "User/alice".to_owned()
    );
    assert_eq!(
        session.supervisor_execution_ref().to_canonical_string(),
        "Host/control".to_owned()
    );

    let bob_pool = ShellPool::new(
        "host-bob",
        "dev",
        PoolSpec::new(
            ExecutionTarget::host("control"),
            "bob",
            "artifact://shells/bash-login",
            1,
            1,
            4096,
        )
        .unwrap(),
    )
    .unwrap();
    let bob_session = ShellSession::from_pool(&bob_pool, "host-bob-main", "main", None).unwrap();
    assert!(
        WorkloadIdentity::proven(ResourceRef::parse("User/alice").unwrap()).user_ref()
            != bob_session.supervisor_user_ref(),
        "another user's identity does not satisfy this session's admission"
    );
}