//! Restart adoption measured against the admitted `Process` and terminal
//! `Endpoint`.
//!
//! The scenario these cover is reconnect: a controller restart retains a
//! supervisor only when the observation still speaks for the relationship
//! the graph committed. An observation taken against another `Process`, on
//! another stream endpoint, or in an older reconnect generation is refused by
//! its own decision rather than repaired by assumption.

use d2b_contracts_resource::v3::ResourceRef;
use d2b_contracts_resource::v3::identity::ReconnectGeneration;
use d2b_provider_shell_terminal::{
    AdoptionDecision, ExecutionTarget, PoolSpec, ShellPool, ShellSession, SupervisorCandidate,
    SupervisorIdentity, SupervisorObservation, TerminalStreamBinding, adopt_supervisor,
};

const SESSION_NAME: &str = "work-shell-main";
const SESSION_POOL: &str = "work-shell";

fn pool() -> ShellPool {
    ShellPool::new(
        SESSION_POOL,
        "work",
        PoolSpec::new(
            ExecutionTarget::guest("work-vm"),
            "alice",
            "artifact://shells/bash-login",
            1,
            1,
            4096,
        )
        .expect("pool spec"),
    )
    .expect("pool")
}

fn session() -> ShellSession {
    ShellSession::from_pool(&pool(), SESSION_NAME, "main", None).expect("session")
}

fn identity(seed: u8, generation: u64) -> SupervisorIdentity {
    SupervisorIdentity::new([seed; 32], [seed.wrapping_add(1); 32], generation).expect("identity")
}

fn reconnect(value: u64) -> ReconnectGeneration {
    ReconnectGeneration::new(value).expect("reconnect generation")
}

fn binding(minimum_reconnect: u64) -> TerminalStreamBinding {
    TerminalStreamBinding::admitted(
        session().supervisor_process_ref().clone(),
        ResourceRef::parse("Endpoint/work-shell-main-terminal").expect("endpoint"),
        reconnect(minimum_reconnect),
    )
}

fn observation(
    process: &str,
    endpoint: &str,
    reconnect_generation: u64,
) -> SupervisorObservation {
    SupervisorObservation::observed(
        ResourceRef::parse(process).expect("process"),
        ResourceRef::parse(endpoint).expect("endpoint"),
        reconnect(reconnect_generation),
    )
}

fn candidate(
    owner: &str,
    process: &str,
    endpoint: &str,
    reconnect_generation: u64,
    identity: SupervisorIdentity,
) -> SupervisorCandidate {
    SupervisorCandidate::observed(
        owner,
        observation(process, endpoint, reconnect_generation),
        identity,
    )
}

#[test]
fn restart_adoption_requires_one_owned_matching_supervisor() {
    let session = session();
    let stream = binding(2);
    let expected = identity(4, 7);
    let owned = candidate(
        SESSION_NAME,
        "Process/work-shell-main",
        "Endpoint/work-shell-main-terminal",
        3,
        expected.clone(),
    );
    assert_eq!(
        adopt_supervisor(&session, &stream, &expected, std::slice::from_ref(&owned)),
        AdoptionDecision::Adopted
    );

    let stale = candidate(
        SESSION_NAME,
        "Process/work-shell-main",
        "Endpoint/work-shell-main-terminal",
        3,
        identity(4, 6),
    );
    assert_eq!(
        adopt_supervisor(&session, &stream, &expected, &[stale]),
        AdoptionDecision::StaleGeneration
    );
    assert_eq!(
        adopt_supervisor(
            &session,
            &stream,
            &expected,
            &[owned.clone(), owned.clone()],
        ),
        AdoptionDecision::Ambiguous
    );
    assert_eq!(
        adopt_supervisor(&session, &stream, &expected, &[]),
        AdoptionDecision::Missing
    );
    let same_different_digest = candidate(
        SESSION_NAME,
        "Process/work-shell-main",
        "Endpoint/work-shell-main-terminal",
        3,
        identity(9, 7),
    );
    assert_eq!(
        adopt_supervisor(&session, &stream, &expected, &[same_different_digest]),
        AdoptionDecision::Ambiguous
    );
    let another_session = candidate(
        "work-shell-other",
        "Process/work-shell-other",
        "Endpoint/work-shell-main-terminal",
        3,
        expected.clone(),
    );
    assert_eq!(
        adopt_supervisor(&session, &stream, &expected, &[another_session]),
        AdoptionDecision::Missing
    );
}

/// Scenario 3: a reconnect retains only a supervisor measured against the
/// admitted Process and the admitted endpoint.
#[test]
fn reconnect_retains_only_matching_admitted_process_and_endpoint() {
    let session = session();
    let stream = binding(4);
    let expected = identity(5, 11);

    // A supervisor that is running as a different Process row, even under the
    // exact right identity, is not this session's supervisor.
    let foreign_process = candidate(
        SESSION_NAME,
        "Process/work-shell-shadow",
        "Endpoint/work-shell-main-terminal",
        4,
        expected.clone(),
    );
    assert_eq!(
        adopt_supervisor(&session, &stream, &expected, &[foreign_process]),
        AdoptionDecision::ForeignProcess
    );

    // A stream on any other endpoint is not the admitted relationship, even
    // when the Process row is right.
    let foreign_endpoint = candidate(
        SESSION_NAME,
        "Process/work-shell-main",
        "Endpoint/work-shell-other-terminal",
        4,
        expected.clone(),
    );
    assert_eq!(
        adopt_supervisor(&session, &stream, &expected, &[foreign_endpoint]),
        AdoptionDecision::ForeignEndpoint
    );

    // Evidence from before the relationship's reconnect fence cannot revive
    // the incumbent after a reconnect.
    let stale_reconnect = candidate(
        SESSION_NAME,
        "Process/work-shell-main",
        "Endpoint/work-shell-main-terminal",
        3,
        expected.clone(),
    );
    assert_eq!(
        adopt_supervisor(&session, &stream, &expected, &[stale_reconnect]),
        AdoptionDecision::StaleReconnect
    );

    let current = candidate(
        SESSION_NAME,
        "Process/work-shell-main",
        "Endpoint/work-shell-main-terminal",
        4,
        expected.clone(),
    );
    assert_eq!(
        adopt_supervisor(&session, &stream, &expected, std::slice::from_ref(&current)),
        AdoptionDecision::Adopted
    );
    // A newer reconnect keeps the same supervisor admissible.
    let stream = binding(2);
    assert_eq!(
        adopt_supervisor(&session, &stream, &expected, &[current]),
        AdoptionDecision::Adopted
    );
}

/// Scenario 1 at the adoption boundary: a stale or foreign supervisor session
/// cannot be adopted even when it claims this session's name.
#[test]
fn a_foreign_supervisor_cannot_claim_the_session_by_name() {
    let session = session();
    let stream = binding(1);
    let expected = identity(6, 2);

    // A foreign row that only borrows the session name and identity is caught
    // by the admitted Process reference, not by the name.
    let impostor = candidate(
        SESSION_NAME,
        "Process/attacker",
        "Endpoint/work-shell-main-terminal",
        1,
        expected.clone(),
    );
    assert_eq!(
        adopt_supervisor(&session, &stream, &expected, &[impostor]),
        AdoptionDecision::ForeignProcess
    );

    // Two live candidates are ambiguous even when both match the admitted
    // relationship: neither may be retained.
    let first = candidate(
        SESSION_NAME,
        "Process/work-shell-main",
        "Endpoint/work-shell-main-terminal",
        1,
        expected.clone(),
    );
    let second = candidate(
        SESSION_NAME,
        "Process/work-shell-main",
        "Endpoint/work-shell-main-terminal",
        1,
        expected.clone(),
    );
    assert_eq!(
        adopt_supervisor(&session, &stream, &expected, &[first, second]),
        AdoptionDecision::Ambiguous
    );
}