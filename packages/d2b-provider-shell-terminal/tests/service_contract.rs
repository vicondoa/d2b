//! The shell Provider's declared service contracts and the admission
//! boundaries they sit on.
//!
//! The scenarios here are the interactive ones a conversion could quietly
//! widen: a stream that is stale or foreign must not attach, a user-domain
//! supervisor must not be selectable onto another user's identity, and
//! removing a session must drain its own stream and child without touching
//! the rest of that user's processes.

use d2b_contracts_resource::v3::ResourceRef;
use d2b_contracts_resource::v3::identity::ReconnectGeneration;
use d2b_provider_shell_terminal::{
    AttachRequest, CONTROLLER_SERVICE, CallerOrigin, ExecutionTarget, InMemoryShellAuthority,
    OpenSessionRequest, PoolSpec, Role, ShellPool, ShellSession, ShellTerminalController,
    ShellTerminalError, Subject, SUPERVISOR_PROCESS_PROVIDER_REF, SUPERVISOR_PROCESS_TEMPLATE,
    SUPERVISOR_SERVICE, SupervisorIdentity, TERMINAL_STREAM, TerminalAttachEvidence,
    TerminalStreamBinding, UserDomainProcess, WorkloadIdentity, supervisor_execution_spec,
};
use std::sync::Arc;

fn pool(user: &str, max_sessions: u32, max_attached: u32) -> ShellPool {
    ShellPool::new(
        "guest-alice",
        "dev",
        PoolSpec::new(
            ExecutionTarget::guest("work"),
            user,
            "artifact://shells/bash-login",
            max_sessions,
            max_attached,
            4096,
        )
        .unwrap(),
    )
    .unwrap()
}

fn controller() -> ShellTerminalController {
    ShellTerminalController::new(Arc::new(InMemoryShellAuthority::new()))
}

fn admin() -> Subject {
    Subject::new("dev", CallerOrigin::Local, [Role::ZoneAdmin])
}

fn reconnect(value: u64) -> ReconnectGeneration {
    ReconnectGeneration::new(value).expect("reconnect generation")
}

fn admitted_stream(session: &ShellSession, minimum_reconnect: u64) -> TerminalStreamBinding {
    TerminalStreamBinding::admitted(
        session.supervisor_process_ref().clone(),
        ResourceRef::parse("Endpoint/guest-alice-main-terminal").expect("endpoint"),
        reconnect(minimum_reconnect),
    )
}

fn evidence(
    session: &ShellSession,
    endpoint: &str,
    reconnect_generation: u64,
) -> TerminalAttachEvidence {
    TerminalAttachEvidence::presented(
        session.supervisor_process_ref().clone(),
        ResourceRef::parse(endpoint).expect("endpoint"),
        reconnect(reconnect_generation),
    )
}

#[test]
fn service_names_and_terminal_stream_are_closed_contracts() {
    assert_eq!(CONTROLLER_SERVICE, "shell-terminal.v3");
    assert_eq!(SUPERVISOR_SERVICE, "shell-session-supervisor.v1");
    assert_eq!(TERMINAL_STREAM, "terminal");
    assert!(AttachRequest::new(1, 4096).is_ok());
    assert!(AttachRequest::new(1, 1_048_577).is_err());
}

/// The supervisor launch is the Provider's declared contract: one Process
/// provider and one trusted executable template, named once.
#[test]
fn the_supervisor_launch_is_one_declared_process_contract() {
    assert_eq!(SUPERVISOR_PROCESS_PROVIDER_REF, "Provider/system-systemd");
    assert_eq!(SUPERVISOR_PROCESS_TEMPLATE, "shell-supervisor-main");

    let spec = supervisor_execution_spec(
        ResourceRef::parse("Guest/work").expect("execution"),
        ResourceRef::parse("User/alice").expect("user"),
    )
    .expect("declared execution spec");
    assert_eq!(
        spec.execution_ref().to_canonical_string(),
        "Guest/work".to_owned()
    );
    assert_eq!(
        spec.user_ref().expect("user domain").to_canonical_string(),
        "User/alice".to_owned()
    );
    assert_eq!(spec.template().as_str(), SUPERVISOR_PROCESS_TEMPLATE);
    assert!(
        supervisor_execution_spec(
            ResourceRef::parse("Process/shadow").expect("execution"),
            ResourceRef::parse("User/alice").expect("user"),
        )
        .is_err(),
        "a Process row is not an execution target"
    );
}

/// Scenario 1: a stale or foreign terminal stream cannot attach.
///
/// The stream's own observation is measured against the admitted
/// `EndpointBinding` before it can reserve an attachment slot, so a stream
/// from an older reconnect, on another endpoint, or claiming another
/// session's `Process` is refused with the reason it failed.
#[test]
fn a_stale_or_foreign_terminal_stream_cannot_attach() {
    let mut controller = controller();
    controller.insert_pool(pool("alice", 2, 2)).unwrap();
    let opened = controller
        .open_session(
            &admin(),
            OpenSessionRequest::new("guest-alice", "main", None).unwrap(),
        )
        .unwrap();
    let session = opened.session().clone();
    let mut supervisor = opened
        .start_supervisor(
            SupervisorIdentity::new([1; 32], [2; 32], opened.supervisor_generation()).unwrap(),
        )
        .unwrap();
    let stream = admitted_stream(&session, 2);
    let request = AttachRequest::new(opened.supervisor_generation(), 0).unwrap();

    // Evidence from before the relationship's reconnect fence.
    assert_eq!(
        supervisor
            .attach_admitted(
                &admin(),
                request,
                &stream,
                &evidence(&session, "Endpoint/guest-alice-main-terminal", 1),
            )
            .expect_err("stale reconnect evidence"),
        ShellTerminalError::StaleReconnect
    );

    // A stream on any other endpoint is not the admitted relationship.
    assert_eq!(
        supervisor
            .attach_admitted(
                &admin(),
                request,
                &stream,
                &evidence(&session, "Endpoint/some-other-terminal", 2),
            )
            .expect_err("foreign endpoint"),
        ShellTerminalError::EndpointBindingMismatch
    );

    // A stream claiming another session's supervisor Process is refused, and
    // the refusal names the mismatch rather than the capacity.
    let foreign_consumer = TerminalAttachEvidence::presented(
        ResourceRef::parse("Process/guest-alice-other").expect("process"),
        ResourceRef::parse("Endpoint/guest-alice-main-terminal").expect("endpoint"),
        reconnect(2),
    );
    assert_eq!(
        supervisor
            .attach_admitted(&admin(), request, &stream, &foreign_consumer)
            .expect_err("foreign consumer"),
        ShellTerminalError::EndpointBindingMismatch
    );

    // None of the refusals consumed the pool's attachment capacity: the
    // admitted stream still attaches afterwards.
    let receipt = supervisor
        .attach_admitted(
            &admin(),
            request,
            &stream,
            &evidence(&session, "Endpoint/guest-alice-main-terminal", 2),
        )
        .expect("the admitted stream attaches");
    assert_eq!(receipt.stream_name(), TERMINAL_STREAM);
    assert_eq!(receipt.generation(), opened.supervisor_generation());

    // A binding whose consumer is not this supervisor's own Process is
    // refused even when the endpoint and generation match.
    let foreign_binding = TerminalStreamBinding::admitted(
        ResourceRef::parse("Process/guest-alice-other").expect("process"),
        ResourceRef::parse("Endpoint/guest-alice-main-terminal").expect("endpoint"),
        reconnect(2),
    );
    supervisor.detach(&admin(), receipt.attachment()).unwrap();
    assert_eq!(
        supervisor
            .attach_admitted(
                &admin(),
                request,
                &foreign_binding,
                &evidence(&session, "Endpoint/guest-alice-main-terminal", 2),
            )
            .expect_err("binding for another supervisor"),
        ShellTerminalError::EndpointBindingMismatch
    );
}

/// Scenario 2: user-domain execution cannot select another user's identity.
///
/// The identity is proven from the launched process rather than carried by the
/// request, so a supervisor started under a different `User` is refused before
/// it can be claimed and before any stream attaches to it.
#[test]
fn user_domain_execution_cannot_select_another_users_identity() {
    let mut controller = controller();
    controller.insert_pool(pool("alice", 1, 1)).unwrap();
    let opened = controller
        .open_session(
            &admin(),
            OpenSessionRequest::new("guest-alice", "main", None).unwrap(),
        )
        .unwrap();
    let session = opened.session().clone();
    assert_eq!(
        session.supervisor_user_ref().to_canonical_string(),
        "User/alice".to_owned()
    );
    let identity =
        SupervisorIdentity::new([1; 32], [2; 32], opened.supervisor_generation()).unwrap();

    let wrong_user = WorkloadIdentity::proven(ResourceRef::parse("User/bob").expect("user"));
    assert_eq!(
        opened
            .start_supervisor_for(identity.clone(), &wrong_user)
            .expect_err("another user's identity"),
        ShellTerminalError::WorkloadIdentityMismatch
    );

    let not_a_user = WorkloadIdentity::proven(ResourceRef::parse("Host/work").expect("target"));
    assert_eq!(
        opened
            .start_supervisor_for(identity.clone(), &not_a_user)
            .expect_err("a non-User identity"),
        ShellTerminalError::WorkloadIdentityMismatch
    );

    // The refused attempts claimed nothing: the session's own identity still
    // starts exactly one supervisor, and a second claim under that identity
    // is refused as ambiguous.
    let mut supervisor = opened
        .start_supervisor_for(
            identity.clone(),
            &WorkloadIdentity::proven(ResourceRef::parse("User/alice").expect("user")),
        )
        .expect("the admitted identity starts the supervisor");
    assert_eq!(
        supervisor
            .attach(&admin(), AttachRequest::new(opened.supervisor_generation(), 0).unwrap())
            .expect("the admitted supervisor attaches")
            .generation(),
        opened.supervisor_generation()
    );
    assert_eq!(
        opened
            .start_supervisor_for(
                identity,
                &WorkloadIdentity::proven(ResourceRef::parse("User/alice").expect("user")),
            )
            .expect_err("a second claim under the same identity"),
        ShellTerminalError::SupervisorAmbiguous
    );
}

/// Scenario 4: removal drains this session's stream and child without
/// deleting unrelated user processes.
///
/// The removal is refused while a stream is still attached, and once the
/// stream drains only this session's own `Process` row disappears. Another
/// session's supervisor, an unrelated process of the same `User`, and a
/// process of a different user all survive.
#[test]
fn removal_drains_the_session_without_deleting_unrelated_user_processes() {
    let authority = Arc::new(InMemoryShellAuthority::new());
    let mut controller = ShellTerminalController::new(authority.clone());
    controller.insert_pool(pool("alice", 2, 2)).unwrap();

    let owned = controller
        .open_session(
            &admin(),
            OpenSessionRequest::new("guest-alice", "main", None).unwrap(),
        )
        .unwrap();
    let sibling = controller
        .open_session(
            &admin(),
            OpenSessionRequest::new("guest-alice", "other", None).unwrap(),
        )
        .unwrap();
    let owned_process = owned
        .session()
        .supervisor_process_ref()
        .to_canonical_string();
    let sibling_process = sibling
        .session()
        .supervisor_process_ref()
        .to_canonical_string();

    // Processes in the same user domain that this Provider does not own.
    authority.declare_user_process(UserDomainProcess::unowned(
        ResourceRef::parse("Process/alice-editor").expect("process"),
        ResourceRef::parse("User/alice").expect("user"),
    ));
    authority.declare_user_process(UserDomainProcess::unowned(
        ResourceRef::parse("Process/mallory-shell").expect("process"),
        ResourceRef::parse("User/mallory").expect("user"),
    ));

    let owned_identity =
        SupervisorIdentity::new([1; 32], [2; 32], owned.supervisor_generation()).unwrap();
    let mut supervisor = owned.start_supervisor(owned_identity.clone()).unwrap();
    let attachment = supervisor
        .attach(
            &admin(),
            AttachRequest::new(owned.supervisor_generation(), 0).unwrap(),
        )
        .unwrap()
        .attachment();

    // The stream is still in use, so the child is not removed yet and nothing
    // else is touched either.
    assert_eq!(
        controller
            .finalize_session(&admin(), owned.session().name(), None)
            .expect_err("an attached stream blocks removal"),
        ShellTerminalError::CapacityExceeded
    );
    assert!(
        authority.user_process(&owned_process).is_some(),
        "the session's own child stays while its stream is attached"
    );

    supervisor.detach(&admin(), attachment).unwrap();
    controller
        .finalize_session(&admin(), owned.session().name(), Some(&owned_identity))
        .expect("the drained session retires");

    assert!(
        authority.user_process(&owned_process).is_none(),
        "the removed session's own Process row is gone"
    );
    assert!(
        authority
            .supervisor_process_resource(owned.session().name())
            .is_none()
    );
    assert!(
        authority.user_process(&sibling_process).is_some(),
        "another session's supervisor of the same user survives"
    );
    assert!(
        authority
            .supervisor_process_resource(sibling.session().name())
            .is_some()
    );
    assert!(
        authority.user_process("Process/alice-editor").is_some(),
        "an unrelated process of the same user survives"
    );
    assert!(
        authority.user_process("Process/mallory-shell").is_some(),
        "another user's process survives"
    );
    assert_eq!(
        authority.user_process_names_for(&ResourceRef::parse("User/alice").expect("user")),
        vec![
            "Process/alice-editor".to_owned(),
            sibling_process.clone(),
        ]
    );
    assert!(
        owned.session().name() != sibling.session().name(),
        "the two sessions are distinct owners"
    );
}