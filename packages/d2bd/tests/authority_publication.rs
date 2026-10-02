//! The manager's freeze / commit / publish / acknowledge coordinator (U7,
//! KTD6-KTD7; R8, R33-R36, R41).
//!
//! The coordinator is the half of the protocol the daemon owns, and what has
//! to hold is an ordering and a memory: the broker's fence is durable before
//! the manager's desired rows are, the accepted cursor moves only on the
//! broker's own `Accepted` answer, a completion that does not match the
//! transaction and sequence still pending changes nothing, and every mutation
//! recovery boundary the transport can interrupt leaves an explicit outcome
//! rather than a fabricated success.
//!
//! [`AuthorityPublicationLink`] is the seam the coordinator declares for
//! exactly this: it is where an interruption is injected, so the link below
//! records the ordered exchanges and can drop, stall, or refuse one of them.
//! The link asserts nothing about the broker - the broker's own fences, effect
//! journal, and refusals are proven against the real projection in
//! `//packages/d2b-broker:authority_publication`. What these cases pin is that
//! the manager never publishes a visibility, moves its accepted cursor, or
//! clears a pending identity the broker has not answered for.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use d2b_contracts_broker::broker_wire::{
    AcceptedAuthority, AuthorityCursor, AuthorityProjectionRow, AuthorityPublicationEnvelope,
    AuthorityPublicationOpen, AuthorityPublicationRequest, AuthorityPublicationResponse,
    AuthoritySnapshot, BeginEffectRequest, ControlActionRequest, EffectExitRequest,
    OpenedPublicationSessionResponse, PreparedTransaction,
    PublicationControlKind, PublicationEffectId, PublicationLimits, PublicationMutationKind,
    PublicationRefusal, PublicationSession, PublicationSessionBinding, PublicationTransactionId,
    ReleaseEffectRequest, RevocationConvergence, ZoneAuthorityState,
    publication_snapshot_digest, PUBLICATION_CONTROL_NOT_BOUND, PUBLICATION_EFFECT_UNPROVEN,
    PUBLICATION_RECONCILIATION_REQUIRED, PUBLICATION_UNKNOWN_TRANSACTION,
};
use d2b_contracts_resource::v3::{
    AdmissionStage, AuthoritySubject, AuthoritySubjectKind, CanonicalJsonObject, DesiredDigest,
    DesiredRevision, RefusalReason, ResourceRef, StoreIncarnation, ZoneDesiredSequence,
};
use d2bd::authority_publication::{
    AcceptedPublication, AuthorityPublicationCoordinator, AuthorityPublicationLink, CommitCandidate,
    PrepareCandidate, PublicationError, RevocationOutcome,
};

const ZONE: &str = "pubzone";
const STORE: &str = "store-generation-1";

// Publication identities are bounded lower-hex tokens.
const TX_ONE: &str = "b1";
const TX_TWO: &str = "b2";
const TX_THREE: &str = "b3";
const TX_FOREIGN: &str = "bf";
const EFFECT_ONE: &str = "e1";

fn reference(value: &str) -> ResourceRef {
    ResourceRef::parse(value).expect("the fixture references are canonical")
}

fn tx_id(value: &str) -> PublicationTransactionId {
    PublicationTransactionId::parse(value).expect("the fixture transaction ids are canonical")
}

fn incarnation() -> StoreIncarnation {
    StoreIncarnation::parse(STORE).expect("the fixture incarnation is a bounded token")
}

fn shell() -> AuthoritySubject {
    AuthoritySubject::named(AuthoritySubjectKind::Process, reference("Process/shell"))
}

fn bootstrap() -> AuthoritySubject {
    AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap)
}

fn sequence(value: u64) -> ZoneDesiredSequence {
    let mut out = ZoneDesiredSequence::INITIAL;
    for _ in 0..value {
        out = out.try_next().expect("the desired sequence has room");
    }
    out
}

fn cursor(value: u64) -> AuthorityCursor {
    AuthorityCursor {
        sequence: sequence(value),
        digest: DesiredDigest::of(format!("cursor-{value}").as_bytes()),
    }
}

fn canonical(value: &impl serde::Serialize) -> CanonicalJsonObject {
    CanonicalJsonObject::parse(&serde_json::to_vec(value).expect("the fixture row serializes"))
        .expect("the fixture row is a canonical JSON object")
}

fn row(name: &str) -> AuthorityProjectionRow {
    let admitted = canonical(&serde_json::json!({ "declared": name }));
    AuthorityProjectionRow {
        resource_ref: reference(&format!("Process/{name}")),
        desired_revision: DesiredRevision::INITIAL.try_next().expect("room"),
        desired_digest: DesiredDigest::of(&admitted.to_canonical_bytes()),
        admitted,
        // A non-binding row resolves no relationship identity, so it publishes
        // none and contributes no accepted source.
        source_uid: None,
        consumer_uid: None,
    }
}

// ---------------------------------------------------------------------------
// The link
// ---------------------------------------------------------------------------

/// How far one admitted effect has got, as the broker's journal records it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// Admitted for launch, exec not yet released.
    Admitted,
    /// Exec released before any fence.
    Released,
    /// The child's completion was reported.
    Exited,
}

impl Phase {
    /// Whether a reducing change can be accepted with this effect outstanding.
    const fn is_settled(self) -> bool {
        matches!(self, Self::Released | Self::Exited)
    }
}

/// What one exchange through the link produced.
enum Step {
    /// The broker answered.
    Answer(Box<AuthorityPublicationResponse>),
    /// The answer was dropped on the floor at this boundary.
    Dropped,
    /// The answer never arrives.
    Stalled,
}

/// The recording link: the ordered exchanges plus one injectable interruption.
#[derive(Debug)]
struct Broker {
    ops: Mutex<Vec<&'static str>>,
    /// How many more answers of this op name to drop on the floor.
    drop_ops: Mutex<BTreeMap<&'static str, usize>>,
    /// Whether a control exchange never answers at all.
    stall_control: Mutex<bool>,
    /// The next exchange answers with this refusal instead.
    refuse_next: Mutex<Option<PublicationRefusal>>,
    /// The accepted cursor the broker holds.
    accepted: Mutex<Option<AuthorityCursor>>,
    /// The fence the broker durably holds.
    fence: Mutex<Option<(PublicationTransactionId, DesiredDigest)>>,
    /// The transaction identities the broker durably saw.
    seen: Mutex<BTreeSet<String>>,
    /// The effect journal.
    effects: Mutex<BTreeMap<String, (PublicationTransactionId, Phase)>>,
}

impl Default for Broker {
    fn default() -> Self {
        Self {
            ops: Mutex::new(Vec::new()),
            drop_ops: Mutex::new(BTreeMap::new()),
            stall_control: Mutex::new(false),
            refuse_next: Mutex::new(None),
            accepted: Mutex::new(Some(AuthorityCursor::initial())),
            fence: Mutex::new(None),
            seen: Mutex::new(BTreeSet::new()),
            effects: Mutex::new(BTreeMap::new()),
        }
    }
}

impl Broker {
    fn accepted(&self) -> AuthorityCursor {
        self.accepted
            .lock()
            .expect("the record lock is not poisoned")
            .clone()
            .unwrap_or_else(AuthorityCursor::initial)
    }

    fn ops(&self) -> Vec<&'static str> {
        self.ops.lock().expect("the record lock is not poisoned").clone()
    }

    fn count_of(&self, op: &str) -> usize {
        self.ops().iter().filter(|seen| **seen == op).count()
    }

    fn drop_next(&self, op: &'static str, times: usize) {
        self.drop_ops
            .lock()
            .expect("the record lock is not poisoned")
            .insert(op, times);
    }

    fn refuse_next(&self, refusal: PublicationRefusal) {
        *self.refuse_next.lock().expect("the record lock is not poisoned") = Some(refusal);
    }

    fn stage_control(&self, stalled: bool) {
        *self.stall_control.lock().expect("the record lock is not poisoned") = stalled;
    }

    fn snapshot(&self) -> ZoneAuthorityState {
        let accepted = self.accepted();
        let fence = self.fence.lock().expect("the record lock is not poisoned").clone();
        match fence {
            Some((transaction, _)) => ZoneAuthorityState::Fenced {
                store_incarnation: incarnation(),
                accepted: accepted.clone(),
                transaction,
                committed: AuthorityCursor {
                    sequence: accepted
                        .sequence
                        .try_next()
                        .expect("the desired sequence has room"),
                    digest: DesiredDigest::of(b"fenced-candidate"),
                },
            },
            None => ZoneAuthorityState::Unfenced {
                store_incarnation: incarnation(),
                accepted,
            },
        }
    }

    /// One refusal this link issues, carrying the state the Zone is left in.
    fn refusal(
        &self,
        code: &'static str,
        reason: RefusalReason,
        stage: AdmissionStage,
    ) -> PublicationRefusal {
        PublicationRefusal {
            code: code.to_owned(),
            stage,
            reason,
            fenced: true,
            state: self.snapshot(),
        }
    }

    fn saw(&self, transaction: &str) -> bool {
        self.seen
            .lock()
            .expect("the record lock is not poisoned")
            .contains(transaction)
    }

    fn effect_phase(&self, effect: &str) -> Option<Phase> {
        self.effects
            .lock()
            .expect("the record lock is not poisoned")
            .get(effect)
            .map(|(_, phase)| *phase)
    }

    /// Record one exchange and produce its answer, or the interruption the
    /// fixture injects. Synchronous on purpose: the async seam above awaits
    /// nothing while a lock is held.
    fn step(&self, envelope: &AuthorityPublicationEnvelope) -> Step {
        self.ops
            .lock()
            .expect("the record lock is not poisoned")
            .push(envelope.request.op_name());
        if *self.stall_control.lock().expect("the record lock is not poisoned")
            && envelope.request.op_name() == "AuthorityControlAction"
        {
            return Step::Stalled;
        }
        match self.answer(&envelope.request) {
            Some(response) => Step::Answer(Box::new(response)),
            None => Step::Dropped,
        }
    }

    /// The session one open mints, with the accepted cursor the link holds.
    fn mint(&self, open: AuthorityPublicationOpen) -> AuthorityPublicationResponse {
        self.ops
            .lock()
            .expect("the record lock is not poisoned")
            .push("OpenSession");
        AuthorityPublicationResponse::Opened(OpenedPublicationSessionResponse {
            session: PublicationSession::parse("pub-link-session")
                .expect("the link's token is canonical"),
            binding: PublicationSessionBinding {
                zone: open.request.zone.clone(),
                store_incarnation: open.request.store_incarnation.clone(),
                broker_epoch: 1,
                initiating_subject: open.request.initiating_subject.clone(),
                accepted: self.accepted(),
            },
            limits: PublicationLimits::default(),
        })
    }

    fn answer(&self, request: &AuthorityPublicationRequest) -> Option<AuthorityPublicationResponse> {
        if let Some(stale) = self
            .drop_ops
            .lock()
            .expect("the record lock is not poisoned")
            .get_mut(request.op_name())
            .filter(|left| **left > 0)
        {
            *stale -= 1;
            return None;
        }
        if let Some(refusal) = self
            .refuse_next
            .lock()
            .expect("the record lock is not poisoned")
            .take()
        {
            return Some(AuthorityPublicationResponse::Refused(refusal));
        }
        Some(match request {
            AuthorityPublicationRequest::PrepareChange(request) => {
                self.seen
                    .lock()
                    .expect("the record lock is not poisoned")
                    .insert(request.transaction.to_string());
                *self.fence.lock().expect("the record lock is not poisoned") =
                    Some((request.transaction.clone(), request.digest.clone()));
                AuthorityPublicationResponse::Prepared(PreparedTransaction {
                    transaction: request.transaction.clone(),
                    expected: request.expected.clone(),
                    committed: request.committed.clone(),
                    digest: request.digest.clone(),
                    reducing: request.kind.is_reducing(),
                    state: self.snapshot(),
                })
            }
            AuthorityPublicationRequest::CommitChange(request) => {
                let unproven = self
                    .effects
                    .lock()
                    .expect("the record lock is not poisoned")
                    .values()
                    .any(|(transaction, phase)| {
                        !phase.is_settled() && transaction != &request.transaction
                    });
                if unproven {
                    return Some(AuthorityPublicationResponse::Refused(self.refusal(
                        PUBLICATION_EFFECT_UNPROVEN,
                        RefusalReason::UnprovenEffect,
                        AdmissionStage::Drain,
                    )));
                }
                *self.fence.lock().expect("the record lock is not poisoned") = None;
                *self.accepted.lock().expect("the record lock is not poisoned") =
                    Some(request.committed.clone());
                AuthorityPublicationResponse::Accepted(AcceptedAuthority {
                    transaction: request.transaction.clone(),
                    sequence: request.committed.sequence,
                    digest: request.digest.clone(),
                    reducing: false,
                    unfrozen: true,
                    state: self.snapshot(),
                })
            }
            AuthorityPublicationRequest::BeginEffect(request) => {
                self.effects
                    .lock()
                    .expect("the record lock is not poisoned")
                    .insert(
                        request.effect.to_string(),
                        (request.transaction.clone(), Phase::Admitted),
                    );
                AuthorityPublicationResponse::Progressed(self.snapshot())
            }
            AuthorityPublicationRequest::ReleaseEffect(request) => {
                let mut effects = self.effects.lock().expect("the record lock is not poisoned");
                if let Some((_, phase)) = effects.get_mut(request.effect.as_str()) {
                    *phase = Phase::Released;
                }
                drop(effects);
                AuthorityPublicationResponse::Progressed(self.snapshot())
            }
            AuthorityPublicationRequest::EffectExit(request) => {
                let known = {
                    let mut effects =
                        self.effects.lock().expect("the record lock is not poisoned");
                    match effects.get_mut(request.effect.as_str()) {
                        Some((transaction, phase)) if transaction == &request.transaction => {
                            *phase = Phase::Exited;
                            true
                        }
                        _ => false,
                    }
                };
                if !known {
                    return Some(AuthorityPublicationResponse::Refused(self.refusal(
                        PUBLICATION_CONTROL_NOT_BOUND,
                        RefusalReason::UnprovenEffect,
                        AdmissionStage::Recover,
                    )));
                }
                if request.reached_exec {
                    AuthorityPublicationResponse::RevocationConverged(RevocationConvergence {
                        transaction: request.transaction.clone(),
                        effect: request.effect.clone(),
                        target: reference("Process/worker"),
                        proven: true,
                        state: self.snapshot(),
                    })
                } else {
                    AuthorityPublicationResponse::Progressed(self.snapshot())
                }
            }
            AuthorityPublicationRequest::ControlAction(_) => {
                AuthorityPublicationResponse::Progressed(self.snapshot())
            }
            AuthorityPublicationRequest::CancelTransaction(request) => {
                let released = {
                    let mut fence =
                        self.fence.lock().expect("the record lock is not poisoned");
                    match fence.as_ref() {
                        Some((transaction, digest))
                            if transaction == &request.transaction && digest == &request.digest =>
                        {
                            *fence = None;
                            true
                        }
                        _ => false,
                    }
                };
                if released {
                    AuthorityPublicationResponse::Progressed(self.snapshot())
                } else {
                    AuthorityPublicationResponse::Refused(self.refusal(
                        PUBLICATION_UNKNOWN_TRANSACTION,
                        RefusalReason::UnprovenEffect,
                        AdmissionStage::Recover,
                    ))
                }
            }
            AuthorityPublicationRequest::BeginSnapshot(_)
            | AuthorityPublicationRequest::SnapshotChunk(_)
            | AuthorityPublicationRequest::EndSnapshot(_)
            | AuthorityPublicationRequest::Resynchronize(_) => {
                AuthorityPublicationResponse::Progressed(self.snapshot())
            }
        })
    }
}

#[async_trait]
impl AuthorityPublicationLink for Broker {
    async fn open_session(
        &self,
        open: AuthorityPublicationOpen,
    ) -> Result<AuthorityPublicationResponse, String> {
        Ok(self.mint(open))
    }

    async fn serve(
        &self,
        envelope: AuthorityPublicationEnvelope,
    ) -> Result<AuthorityPublicationResponse, String> {
        match self.step(&envelope) {
            Step::Answer(response) => Ok(*response),
            Step::Dropped => Err(format!(
                "interrupted before {} was answered",
                envelope.request.op_name()
            )),
            Step::Stalled => {
                // The caller went away mid-round-trip and the answer never
                // arrives. Nothing is held across this await.
                std::future::pending::<Result<AuthorityPublicationResponse, String>>().await
            }
        }
    }
}

fn bind(broker: Arc<Broker>) -> AuthorityPublicationCoordinator {
    AuthorityPublicationCoordinator::new(ZONE, incarnation(), bootstrap(), broker)
}

fn prepare_candidate(
    id: &str,
    expected: AuthorityCursor,
    committed: AuthorityCursor,
    rows: Vec<AuthorityProjectionRow>,
) -> PrepareCandidate {
    PrepareCandidate {
        transaction: tx_id(id),
        store_incarnation: incarnation(),
        expected,
        committed,
        subject: shell(),
        kind: PublicationMutationKind::Create,
        candidate: rows,
        removed: Vec::new(),
    }
}

fn commit_candidate(
    id: &str,
    expected: AuthorityCursor,
    committed: AuthorityCursor,
    rows: Vec<AuthorityProjectionRow>,
) -> CommitCandidate {
    CommitCandidate {
        transaction: tx_id(id),
        store_incarnation: incarnation(),
        expected,
        committed,
        rows,
        removed: Vec::new(),
    }
}

fn control(
    id: &str,
    effect: Option<&str>,
    kind: PublicationControlKind,
) -> ControlActionRequest {
    ControlActionRequest {
        transaction: tx_id(id),
        effect: effect.map(|name| {
            PublicationEffectId::parse(name).expect("the fixture effect ids are canonical")
        }),
        target: reference("Process/worker"),
        kind,
    }
}

fn snapshot(cursor: AuthorityCursor, outstanding: Option<&str>) -> AuthoritySnapshot {
    AuthoritySnapshot {
        zone: ZONE.to_owned(),
        store_incarnation: incarnation(),
        cursor,
        root_subject: bootstrap(),
        rows: vec![row("worker")],
        outstanding: outstanding.map(tx_id),
    }
}

// ---------------------------------------------------------------------------
// The order
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_broker_fence_precedes_the_publish_and_the_acknowledgment_closes_it() {
    let broker = Arc::new(Broker::default());
    let coordinator = bind(broker.clone());
    coordinator
        .open_session()
        .await
        .expect("the link mints a session");
    assert_eq!(coordinator.accepted().await, AuthorityCursor::initial());

    let prepared = coordinator
        .prepare(&prepare_candidate(TX_ONE, cursor(0), cursor(1), vec![row("worker")]))
        .await
        .expect("the candidate prepares");
    assert_eq!(prepared.transaction, tx_id(TX_ONE));
    assert_eq!(coordinator.pending().await, Some(tx_id(TX_ONE)));
    // The freeze is durable before anything is published, and the manager's
    // accepted cursor has not moved: only the broker's answer moves it.
    assert_eq!(
        coordinator.accepted().await,
        AuthorityCursor::initial(),
        "a prepare publishes no visibility"
    );

    let accepted = coordinator
        .commit(&commit_candidate(
            TX_ONE,
            cursor(0),
            cursor(1),
            vec![row("worker")],
        ))
        .await
        .expect("the commit is accepted");
    assert_eq!(accepted.sequence, sequence(1));
    assert_eq!(coordinator.accepted().await, cursor(1));
    assert_eq!(coordinator.pending().await, None);

    assert_eq!(
        broker.ops(),
        vec!["OpenSession", "AuthorityPrepareChange", "AuthorityCommitChange"],
        "the session opens, then the freeze, then the publish that answers it"
    );
}

#[tokio::test]
async fn prepare_and_commit_are_callback_free_and_hold_no_guard() {
    // The structural form of "no manager lock, store transaction, or
    // reservation guard is alive across the round trip": both messages take
    // owned durable facts and an immutable borrow, and every value that crosses
    // is `'static`, `Send`, and `Sync`, so a caller can hand the work to its own
    // bounded worker and return to its mailbox.
    fn assert_owned<T: Send + Sync + 'static>() {}
    assert_owned::<AuthorityPublicationCoordinator>();
    assert_owned::<PrepareCandidate>();
    assert_owned::<CommitCandidate>();
    assert_owned::<AcceptedPublication>();
    assert_owned::<RevocationOutcome>();

    // The whole exchange is one `Send` future over owned values, so a manager
    // hands it to its own bounded worker and returns to its mailbox: there is
    // no store handle, no manager lock, no reservation guard, and no callback
    // for the broker to re-enter through.
    async fn drive(
        coordinator: AuthorityPublicationCoordinator,
        candidate: PrepareCandidate,
        commit: CommitCandidate,
    ) -> Result<AcceptedPublication, PublicationError> {
        coordinator.prepare(&candidate).await?;
        coordinator.commit(&commit).await
    }
    fn assert_send<T: Send>(value: T) -> T {
        value
    }

    let broker = Arc::new(Broker::default());
    let coordinator: AuthorityPublicationCoordinator = bind(broker);
    let candidate = prepare_candidate(TX_ONE, cursor(0), cursor(1), vec![row("worker")]);
    let commit = commit_candidate(TX_ONE, cursor(0), cursor(1), vec![row("worker")]);
    let accepted = assert_send(drive(coordinator.clone(), candidate, commit));
    let accepted = accepted.await.expect("the exchange is served");
    assert_eq!(accepted.sequence, sequence(1));
}

// ---------------------------------------------------------------------------
// A completion applies only to what this coordinator still has pending
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_completion_applies_only_to_the_matching_transaction_and_sequence() {
    let broker = Arc::new(Broker::default());
    let coordinator = bind(broker.clone());
    coordinator
        .prepare(&prepare_candidate(TX_ONE, cursor(0), cursor(1), vec![row("worker")]))
        .await
        .expect("the candidate prepares");

    // A different transaction is not this coordinator's pending one.
    let error = coordinator
        .commit(&commit_candidate(
            TX_FOREIGN,
            cursor(0),
            cursor(1),
            vec![row("worker")],
        ))
        .await
        .expect_err("a commit for another identity is refused");
    assert!(matches!(error, PublicationError::UnmatchedCompletion { .. }));
    assert!(error.is_fenced(), "a refused completion leaves the Zone fenced");
    assert_eq!(
        broker.count_of("AuthorityCommitChange"),
        0,
        "the mismatched completion never reached the broker"
    );
    assert_eq!(coordinator.pending().await, Some(tx_id(TX_ONE)));
    assert_eq!(coordinator.accepted().await, AuthorityCursor::initial());

    // Neither is a commit that moves a different sequence or installs different
    // bytes than the fence was prepared for.
    for commit in [
        commit_candidate(TX_ONE, cursor(0), cursor(2), vec![row("worker")]),
        commit_candidate(TX_ONE, cursor(0), cursor(1), vec![row("other")]),
    ] {
        let error = coordinator
            .commit(&commit)
            .await
            .expect_err("a commit that does not match the pending fence is refused");
        assert!(matches!(error, PublicationError::UnmatchedCompletion { .. }));
    }
    assert_eq!(broker.count_of("AuthorityCommitChange"), 0);
    assert_eq!(coordinator.accepted().await, AuthorityCursor::initial());

    // And with nothing pending at all, a commit publishes nothing.
    coordinator
        .commit(&commit_candidate(
            TX_ONE,
            cursor(0),
            cursor(1),
            vec![row("worker")],
        ))
        .await
        .expect("the matching commit is accepted");
    let error = coordinator
        .commit(&commit_candidate(
            TX_ONE,
            cursor(0),
            cursor(1),
            vec![row("worker")],
        ))
        .await
        .expect_err("a second commit publishes nothing");
    assert!(matches!(error, PublicationError::UnmatchedCompletion { .. }));
    assert_eq!(broker.count_of("AuthorityCommitChange"), 1, "applied once");
    assert_eq!(coordinator.accepted().await, cursor(1));
}

// ---------------------------------------------------------------------------
// One pending transaction per Zone
// ---------------------------------------------------------------------------

#[tokio::test]
async fn other_mutations_queue_behind_the_pending_one_while_observation_and_drain_stay_serviceable() {
    let broker = Arc::new(Broker::default());
    let coordinator = bind(broker.clone());
    coordinator
        .prepare(&prepare_candidate(TX_ONE, cursor(0), cursor(1), vec![row("worker")]))
        .await
        .expect("the candidate prepares");

    // A second mutation cannot start a second transaction.
    let error = coordinator
        .prepare(&prepare_candidate(TX_TWO, cursor(0), cursor(1), vec![row("other")]))
        .await
        .expect_err("a second mutation refuses while one is pending");
    match error {
        PublicationError::AlreadyPending { pending } => {
            assert_eq!(pending, tx_id(TX_ONE), "the outstanding identity is named");
        }
        other => panic!("a pending transaction blocks rather than vanishing: {other}"),
    }
    assert_eq!(broker.count_of("AuthorityPrepareChange"), 1);

    // Its identity is durable in the manager's own store, so the queue is a
    // bounded view of what the store already owes.
    coordinator
        .queue_behind(&tx_id(TX_TWO))
        .await
        .expect("the mutation queues");
    coordinator
        .queue_behind(&tx_id(TX_THREE))
        .await
        .expect("the second mutation queues");
    assert_eq!(
        coordinator.queued().await,
        vec![tx_id(TX_TWO), tx_id(TX_THREE)],
        "queued identities are durable, not candidates"
    );

    // Observation is never queued and never spends a budget: it changes
    // nothing, so it stays serviceable while the Zone is fenced.
    let observed = coordinator
        .observe(&tx_id(TX_ONE), reference("Process/worker"))
        .await
        .expect("observation stays serviceable under a fence");
    assert!(matches!(observed, AuthorityPublicationResponse::Progressed(_)));

    // So does a safe drain on the bounded control lane.
    let drained = coordinator
        .control(
            control(TX_ONE, None, PublicationControlKind::Revoke),
            Duration::from_secs(1),
        )
        .await
        .expect("a control action stays serviceable under a fence");
    assert!(matches!(drained, AuthorityPublicationResponse::Progressed(_)));

    // The accepted cursor never moved while the mutation waited.
    assert_eq!(coordinator.accepted().await, AuthorityCursor::initial());

    // Committing the pending one clears it and releases its own queue entry.
    coordinator
        .commit(&commit_candidate(
            TX_ONE,
            cursor(0),
            cursor(1),
            vec![row("worker")],
        ))
        .await
        .expect("the commit is accepted");
    assert_eq!(coordinator.pending().await, None);
    assert_eq!(
        coordinator.queued().await,
        vec![tx_id(TX_TWO), tx_id(TX_THREE)],
        "settling the pending identity frees the next one, and both waiting identities are still owed"
    );
    coordinator
        .prepare(&prepare_candidate(TX_TWO, cursor(1), cursor(2), vec![row("other")]))
        .await
        .expect("the head of the queue starts the next transaction");
    assert_eq!(coordinator.pending().await, Some(tx_id(TX_TWO)));
}

// ---------------------------------------------------------------------------
// A control timeout keeps the fence
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_control_timeout_keeps_the_fence_rather_than_thawing_it() {
    let broker = Arc::new(Broker::default());
    let coordinator = bind(broker.clone());
    coordinator
        .prepare(&prepare_candidate(TX_ONE, cursor(0), cursor(1), vec![row("worker")]))
        .await
        .expect("the candidate prepares");
    broker.stage_control(true);

    let error = coordinator
        .control(
            control(TX_ONE, None, PublicationControlKind::Revoke),
            Duration::from_millis(30),
        )
        .await
        .expect_err("a control action that never answers times out");
    let named = match &error {
        PublicationError::ControlTimedOut { transaction } => transaction.clone(),
        other => panic!("a stalled control action is a timeout: {other}"),
    };
    assert_eq!(
        named,
        tx_id(TX_ONE),
        "the abandoned action names the transaction it was bound to"
    );
    assert!(
        error.is_fenced(),
        "a caller that went away is not evidence that anything resolved"
    );
    assert_eq!(
        broker.count_of("AuthorityControlAction"),
        1,
        "the action was attempted, not silently dropped"
    );
    assert_eq!(
        coordinator.pending().await,
        Some(tx_id(TX_ONE)),
        "the coordinator cleared nothing"
    );
    assert_eq!(
        coordinator.accepted().await,
        AuthorityCursor::initial(),
        "no visibility was published"
    );

    // The fence is still there for the next attempt.
    broker.stage_control(false);
    coordinator
        .control(
            control(TX_ONE, None, PublicationControlKind::Revoke),
            Duration::from_secs(1),
        )
        .await
        .expect("the control lane is still serviceable");
    assert_eq!(coordinator.pending().await, Some(tx_id(TX_ONE)));
}

#[tokio::test]
async fn a_control_action_that_would_admit_new_use_never_reaches_the_transport() {
    let broker = Arc::new(Broker::default());
    let coordinator = bind(broker.clone());
    for kind in PublicationControlKind::ALL {
        assert!(
            !kind.stage().admits_new_use(),
            "{kind:?} must not admit new use"
        );
        let response = coordinator
            .control(control(TX_ONE, None, kind), Duration::from_secs(1))
            .await
            .expect("the control lane is serviceable");
        assert!(matches!(response, AuthorityPublicationResponse::Progressed(_)));
    }
    assert_eq!(broker.count_of("AuthorityControlAction"), PublicationControlKind::ALL.len());
}

// ---------------------------------------------------------------------------
// Acceptance is not revocation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn authority_accepted_is_not_revocation_converged() {
    let broker = Arc::new(Broker::default());
    let coordinator = bind(broker.clone());
    coordinator
        .prepare(&prepare_candidate(TX_ONE, cursor(0), cursor(1), vec![row("worker")]))
        .await
        .expect("the candidate prepares");
    let accepted = coordinator
        .commit(&commit_candidate(
            TX_ONE,
            cursor(0),
            cursor(1),
            vec![row("worker")],
        ))
        .await
        .expect("the commit is accepted");
    // The accepted outcome names a revision and says nothing about release
    // evidence, which is a different result with a different payload.
    assert_eq!(accepted.sequence, sequence(1));
    assert_eq!(
        accepted.digest,
        commit_candidate(TX_ONE, cursor(0), cursor(1), vec![row("worker")]).digest(),
        "the accepted digest is the digest of the exact committed bytes"
    );
    assert!(!accepted.reducing, "adding a row reduced nothing");
    assert_eq!(coordinator.queued().await, Vec::new());

    // A release outcome is reachable only from a completion, and it names the
    // effect and the proof rather than a sequence.
    coordinator
        .begin_effect(BeginEffectRequest {
            effect: PublicationEffectId::parse(EFFECT_ONE).expect("canonical token"),
            transaction: tx_id(TX_ONE),
            accepted: cursor(1),
            subject: shell(),
            target: reference("Process/worker"),
        })
        .await
        .expect("an unfenced Zone admits the launch");
    coordinator
        .release_effect(ReleaseEffectRequest {
            effect: PublicationEffectId::parse(EFFECT_ONE).expect("canonical token"),
            accepted: cursor(1),
        })
        .await
        .expect("the launch is released for exec");
    let outcome = coordinator
        .effect_exit(EffectExitRequest {
            effect: PublicationEffectId::parse(EFFECT_ONE).expect("canonical token"),
            transaction: tx_id(TX_ONE),
            reached_exec: true,
        })
        .await
        .expect("a child that had reached exec converges as release evidence");
    assert_eq!(outcome.transaction, tx_id(TX_ONE));
    assert_eq!(
        outcome.effect,
        PublicationEffectId::parse(EFFECT_ONE).expect("canonical token"),
        "the release evidence names the effect it is about"
    );
    assert!(outcome.proven, "an exited child is proved");
    // The two outcomes are different results about different things: the
    // accepted revision is still exactly the one the commit installed.
    assert_eq!(coordinator.accepted().await, cursor(1));
}

#[tokio::test]
async fn the_reducing_policy_race_leaves_no_unaccounted_child_in_either_ordering() {
    // Ordering one: the release authorization wins before the fence, so the
    // launch is existing use and the reducing change accounts for it.
    let broker = Arc::new(Broker::default());
    let coordinator = bind(broker.clone());
    coordinator.open_session().await.expect("a session");
    coordinator
        .publish_snapshot(&tx_id("a1"), &snapshot(cursor(1), None))
        .await
        .expect("the bootstrap snapshot installs");
    coordinator
        .begin_effect(BeginEffectRequest {
            effect: PublicationEffectId::parse(EFFECT_ONE).expect("canonical token"),
            transaction: tx_id(TX_ONE),
            accepted: cursor(1),
            subject: shell(),
            target: reference("Process/worker"),
        })
        .await
        .expect("an unfenced Zone admits the launch");
    coordinator
        .release_effect(ReleaseEffectRequest {
            effect: PublicationEffectId::parse(EFFECT_ONE).expect("canonical token"),
            accepted: cursor(1),
        })
        .await
        .expect("the launch is released for exec");
    assert_eq!(broker.effect_phase(EFFECT_ONE), Some(Phase::Released));

    coordinator
        .prepare(&reduce_candidate(TX_TWO, cursor(1), cursor(2)))
        .await
        .expect("the reducing change prepares");
    let accepted = coordinator
        .commit(&reduce_commit(TX_TWO, cursor(1), cursor(2)))
        .await
        .expect("a pre-fence release is accounted, so the change is accepted");
    assert_eq!(accepted.sequence, sequence(2));
    assert_eq!(coordinator.accepted().await, cursor(2));
    assert_eq!(coordinator.pending().await, None);

    // Ordering two: the fence wins before the release, so the reducing change
    // is refused while that child is unaccounted, and it stays refused until
    // the completion settles the record.
    let broker = Arc::new(Broker::default());
    let coordinator = bind(broker.clone());
    coordinator.open_session().await.expect("a session");
    coordinator
        .publish_snapshot(&tx_id("a1"), &snapshot(cursor(1), None))
        .await
        .expect("the bootstrap snapshot installs");
    coordinator
        .begin_effect(BeginEffectRequest {
            effect: PublicationEffectId::parse(EFFECT_ONE).expect("canonical token"),
            transaction: tx_id(TX_ONE),
            accepted: cursor(1),
            subject: shell(),
            target: reference("Process/worker"),
        })
        .await
        .expect("an unfenced Zone admits the launch");
    coordinator
        .prepare(&reduce_candidate(TX_TWO, cursor(1), cursor(2)))
        .await
        .expect("the reducing change prepares");
    assert_eq!(broker.effect_phase(EFFECT_ONE), Some(Phase::Admitted));

    let error = coordinator
        .commit(&reduce_commit(TX_TWO, cursor(1), cursor(2)))
        .await
        .expect_err("a reducing change waits for the unaccounted child");
    assert!(error.is_fenced(), "the Zone stays fenced");
    assert_eq!(
        coordinator.accepted().await,
        cursor(1),
        "no visibility was published for the unaccepted revision"
    );
    assert_eq!(
        coordinator.pending().await,
        Some(tx_id(TX_TWO)),
        "the transaction is still owed an outcome"
    );

    coordinator
        .effect_exit(EffectExitRequest {
            effect: PublicationEffectId::parse(EFFECT_ONE).expect("canonical token"),
            transaction: tx_id(TX_ONE),
            reached_exec: true,
        })
        .await
        .expect("the completion settles the record");
    let accepted = coordinator
        .commit(&reduce_commit(TX_TWO, cursor(1), cursor(2)))
        .await
        .expect("the settled record lets the change through");
    assert_eq!(accepted.sequence, sequence(2));
    assert_eq!(coordinator.accepted().await, cursor(2));
    assert_eq!(coordinator.pending().await, None);
}

fn reduce_candidate(id: &str, expected: AuthorityCursor, committed: AuthorityCursor) -> PrepareCandidate {
    let mut candidate = prepare_candidate(id, expected, committed, vec![row("worker")]);
    candidate.kind = PublicationMutationKind::Delete;
    candidate.removed = vec![reference("Process/worker")];
    candidate
}

fn reduce_commit(id: &str, expected: AuthorityCursor, committed: AuthorityCursor) -> CommitCandidate {
    let mut commit = commit_candidate(id, expected, committed, vec![row("worker")]);
    commit.removed = vec![reference("Process/worker")];
    commit
}

// ---------------------------------------------------------------------------
// Mutation recovery
// ---------------------------------------------------------------------------

/// Before the broker's `PrepareChange` acknowledgment: the staged candidate
/// may exist, the desired rows are unchanged, and the transaction is resumed.
#[tokio::test]
async fn recovery_before_the_prepare_acknowledgment_resumes_the_staged_candidate() {
    let broker = Arc::new(Broker::default());
    let coordinator = bind(broker.clone());
    broker.drop_next("AuthorityPrepareChange", 1);

    let error = coordinator
        .prepare(&prepare_candidate(TX_ONE, cursor(0), cursor(1), vec![row("worker")]))
        .await
        .expect_err("the prepare never reached the broker");
    assert!(matches!(error, PublicationError::Transport { .. }));
    assert!(error.is_fenced(), "a transport that could not answer is not a thaw");
    assert_eq!(coordinator.pending().await, None, "no outcome is owed");
    assert_eq!(coordinator.accepted().await, AuthorityCursor::initial());
    assert_eq!(broker.ops(), vec!["OpenSession", "AuthorityPrepareChange"]);

    // The same identity replays: the broker answers for it on the second try
    // and the fence is the one the first attempt would have written.
    let prepared = coordinator
        .prepare(&prepare_candidate(TX_ONE, cursor(0), cursor(1), vec![row("worker")]))
        .await
        .expect("the staged candidate resumes");
    assert_eq!(prepared.transaction, tx_id(TX_ONE));
    assert_eq!(coordinator.pending().await, Some(tx_id(TX_ONE)));
    assert!(broker.saw(TX_ONE), "the broker durably saw the identity");
}

/// After the fence and before the desired commit: the manager holds a pending
/// identity, so another mutation is refused by name and the prepared identity
/// is either replayed exactly or cancelled.
#[tokio::test]
async fn recovery_after_the_fence_replays_the_exact_candidate_or_cancels_it() {
    let broker = Arc::new(Broker::default());
    let coordinator = bind(broker.clone());
    coordinator
        .prepare(&prepare_candidate(TX_ONE, cursor(0), cursor(1), vec![row("worker")]))
        .await
        .expect("the candidate prepares");

    // A different identity cannot be committed against this fence: the manager
    // will not publish a view for a transaction it does not have pending.
    let error = coordinator
        .commit(&commit_candidate(
            TX_TWO,
            cursor(0),
            cursor(1),
            vec![row("worker")],
        ))
        .await
        .expect_err("a commit for another identity is refused");
    assert!(matches!(error, PublicationError::UnmatchedCompletion { .. }));

    // The exact commit is what the manager retries.
    let accepted = coordinator
        .commit(&commit_candidate(
            TX_ONE,
            cursor(0),
            cursor(1),
            vec![row("worker")],
        ))
        .await
        .expect("the exact commit replays");
    assert_eq!(accepted.sequence, sequence(1));
    assert_eq!(coordinator.pending().await, None);

    // The cancel path: a manager that proves it committed nothing releases the
    // identity, and a second mutation can then start.
    let broker = Arc::new(Broker::default());
    let coordinator = bind(broker.clone());
    let prepared = coordinator
        .prepare(&prepare_candidate(TX_ONE, cursor(0), cursor(1), vec![row("worker")]))
        .await
        .expect("the candidate prepares");
    coordinator
        .cancel(&prepared.transaction, &prepared.digest)
        .await
        .expect("the exact identity and digest are released");
    assert_eq!(coordinator.pending().await, None);
    assert_eq!(coordinator.accepted().await, AuthorityCursor::initial());
    coordinator
        .prepare(&prepare_candidate(TX_TWO, cursor(0), cursor(1), vec![row("other")]))
        .await
        .expect("a second mutation may start after the cancel");
    assert_eq!(coordinator.pending().await, Some(tx_id(TX_TWO)));

    // A cancel for bytes the fence was not prepared for is refused and clears
    // nothing.
    let error = coordinator
        .cancel(&prepared.transaction, &DesiredDigest::of(b"other-bytes"))
        .await
        .expect_err("a cancel cannot release a fence for other bytes");
    assert!(matches!(error, PublicationError::Refused { .. }));
    assert_eq!(coordinator.pending().await, Some(tx_id(TX_TWO)));
}

/// After the desired commit and before the broker acknowledgment: the replay is
/// exact, and nothing uses the unaccepted revision in the meantime.
#[tokio::test]
async fn recovery_after_the_desired_commit_replays_the_exact_commit() {
    let broker = Arc::new(Broker::default());
    let coordinator = bind(broker.clone());
    coordinator
        .prepare(&prepare_candidate(TX_ONE, cursor(0), cursor(1), vec![row("worker")]))
        .await
        .expect("the candidate prepares");
    broker.drop_next("AuthorityCommitChange", 1);

    let error = coordinator
        .commit(&commit_candidate(
            TX_ONE,
            cursor(0),
            cursor(1),
            vec![row("worker")],
        ))
        .await
        .expect_err("the commit never reached the broker");
    assert!(matches!(error, PublicationError::Transport { .. }));
    assert!(error.is_fenced(), "an unanswered commit is not a success");
    assert_eq!(
        coordinator.accepted().await,
        AuthorityCursor::initial(),
        "no success acknowledgment may use the unaccepted revision"
    );
    assert_eq!(
        coordinator.pending().await,
        Some(tx_id(TX_ONE)),
        "the staged identity is still owed its outcome"
    );

    let accepted = coordinator
        .commit(&commit_candidate(
            TX_ONE,
            cursor(0),
            cursor(1),
            vec![row("worker")],
        ))
        .await
        .expect("the exact commit replays");
    assert_eq!(accepted.sequence, sequence(1));
    assert_eq!(coordinator.accepted().await, cursor(1));
    assert_eq!(
        broker.count_of("AuthorityCommitChange"),
        2,
        "the change was published, and the replay was not a second mutation"
    );
}

/// After the broker acknowledgment and before the API response: the second
/// commit applies nothing, and the accepted revision is the one that landed.
#[tokio::test]
async fn recovery_after_the_acknowledgment_does_not_apply_the_mutation_twice() {
    let broker = Arc::new(Broker::default());
    let coordinator = bind(broker.clone());
    coordinator
        .prepare(&prepare_candidate(TX_ONE, cursor(0), cursor(1), vec![row("worker")]))
        .await
        .expect("the candidate prepares");
    let first = coordinator
        .commit(&commit_candidate(
            TX_ONE,
            cursor(0),
            cursor(1),
            vec![row("worker")],
        ))
        .await
        .expect("the commit is accepted");

    // The API response was never sent, so the caller retries. The coordinator
    // owes no outcome any more, and a second commit reaches nobody.
    let error = coordinator
        .commit(&commit_candidate(
            TX_ONE,
            cursor(0),
            cursor(1),
            vec![row("worker")],
        ))
        .await
        .expect_err("a second commit publishes nothing");
    assert!(matches!(error, PublicationError::UnmatchedCompletion { .. }));
    assert!(error.is_fenced(), "an unanswered caller is not a thaw");
    assert_eq!(broker.count_of("AuthorityCommitChange"), 1, "applied once");
    assert_eq!(coordinator.accepted().await, cursor(1), "the revision is the one that landed");
    assert_eq!(coordinator.pending().await, None);
    assert_eq!(first.sequence, sequence(1));
}

/// An unknown or mismatched acknowledgment publishes no visibility, and the
/// Zone is reconciled before anything is served again.
#[tokio::test]
async fn an_unmatched_acknowledgment_publishes_no_visibility() {
    let broker = Arc::new(Broker::default());
    let coordinator = bind(broker.clone());
    let prepared = coordinator
        .prepare(&prepare_candidate(TX_ONE, cursor(0), cursor(1), vec![row("worker")]))
        .await
        .expect("the candidate prepares");
    // That change committed nothing, so the manager releases the identity
    // before starting the reducing one it is actually testing.
    coordinator
        .cancel(&prepared.transaction, &prepared.digest)
        .await
        .expect("the settled identity is released");
    coordinator
        .begin_effect(BeginEffectRequest {
            effect: PublicationEffectId::parse(EFFECT_ONE).expect("canonical token"),
            transaction: tx_id(TX_ONE),
            accepted: cursor(0),
            subject: shell(),
            target: reference("Process/worker"),
        })
        .await
        .expect("an unfenced Zone admits the launch");

    // A completion naming a transaction the effect is not under settles nothing.
    let error = coordinator
        .effect_exit(EffectExitRequest {
            effect: PublicationEffectId::parse(EFFECT_ONE).expect("canonical token"),
            transaction: tx_id(TX_FOREIGN),
            reached_exec: true,
        })
        .await
        .expect_err("a mismatched acknowledgment is refused");
    assert!(matches!(error, PublicationError::Refused { .. }));
    assert!(error.is_fenced(), "the Zone stays fenced");
    assert_eq!(broker.effect_phase(EFFECT_ONE), Some(Phase::Admitted));

    // The reducing change is still owed the child's completion, and the manager
    // publishes nothing until it has it.
    coordinator
        .prepare(&reduce_candidate(TX_TWO, cursor(0), cursor(1)))
        .await
        .expect("the reducing change prepares");
    let error = coordinator
        .commit(&reduce_commit(TX_TWO, cursor(0), cursor(1)))
        .await
        .expect_err("the reducing change waits for the child");
    assert!(matches!(error, PublicationError::Refused { .. }));
    assert_eq!(coordinator.accepted().await, AuthorityCursor::initial());
    assert_eq!(coordinator.pending().await, Some(tx_id(TX_TWO)));

    // Reconciling the exact transaction state is the documented way out.
    coordinator
        .resynchronize(&tx_id(TX_THREE), cursor(0), cursor(0))
        .await
        .expect("the reconciliation is served");
    coordinator
        .publish_snapshot(&tx_id(TX_THREE), &snapshot(cursor(0), Some(TX_TWO)))
        .await
        .expect("the resynchronization carries the outstanding identity");
    coordinator
        .effect_exit(EffectExitRequest {
            effect: PublicationEffectId::parse(EFFECT_ONE).expect("canonical token"),
            transaction: tx_id(TX_ONE),
            reached_exec: true,
        })
        .await
        .expect("the completion settles the record");
    coordinator
        .commit(&reduce_commit(TX_TWO, cursor(0), cursor(1)))
        .await
        .expect("the reconciled transaction finishes");
    assert_eq!(coordinator.accepted().await, cursor(1));
}

/// A broker restart: the manager learns the Zone is unreconciled from the
/// broker's own refusal, keeps its durable facts, and resynchronizes.
#[tokio::test]
async fn a_broker_restart_is_served_as_a_refusal_not_as_a_missing_answer() {
    let broker = Arc::new(Broker::default());
    let coordinator = bind(broker.clone());
    coordinator
        .prepare(&prepare_candidate(TX_ONE, cursor(0), cursor(1), vec![row("worker")]))
        .await
        .expect("the candidate prepares");
    broker.refuse_next(broker.refusal(
        PUBLICATION_RECONCILIATION_REQUIRED,
        RefusalReason::UnprovenEffect,
        AdmissionStage::Recover,
    ));

    // A broker that answered "reconcile me" is a refusal, and the manager
    // reports it as one: an unmatched completion would say the answer never
    // arrived, which is not what happened and is not what a manager reconciles.
    let error = coordinator
        .prepare(&prepare_candidate(TX_ONE, cursor(0), cursor(1), vec![row("worker")]))
        .await
        .expect_err("a broker that requires reconciliation refuses the candidate");
    match error {
        PublicationError::Refused { fenced, .. } => assert!(fenced, "the Zone stays fenced"),
        other => panic!("a refusal is a refusal: {other}"),
    }
    assert_eq!(
        coordinator.pending().await,
        Some(tx_id(TX_ONE)),
        "the outstanding identity is durable across the refusal"
    );
    assert_eq!(coordinator.accepted().await, AuthorityCursor::initial());

    // The documented recovery, and the only one that serves again.
    coordinator
        .resynchronize(&tx_id(TX_THREE), cursor(0), cursor(0))
        .await
        .expect("the reconciliation is served");
    coordinator
        .publish_snapshot(&tx_id(TX_THREE), &snapshot(cursor(0), Some(TX_ONE)))
        .await
        .expect("the outstanding identity travels with the resynchronization");
    let accepted = coordinator
        .commit(&commit_candidate(
            TX_ONE,
            cursor(0),
            cursor(1),
            vec![row("worker")],
        ))
        .await
        .expect("the reconciled transaction finishes");
    assert_eq!(accepted.sequence, sequence(1));
}

/// A daemon restart: a fresh coordinator owes no outcome, so it reconciles the
/// outstanding transaction before publishing any view.
#[tokio::test]
async fn a_daemon_restart_reconciles_before_publishing_any_view() {
    let broker = Arc::new(Broker::default());
    let first = bind(broker.clone());
    first.open_session().await.expect("a session");
    first.publish_snapshot(&tx_id("a1"), &snapshot(cursor(1), None))
        .await
        .expect("the bootstrap snapshot installs");
    first.prepare(&reduce_candidate(TX_TWO, cursor(1), cursor(2)))
        .await
        .expect("the reducing change prepares");

    // A fresh coordinator has no memory of the pending transaction, so it may
    // not publish the accepted view the old one owed.
    let second = bind(broker.clone());
    assert_eq!(second.pending().await, None);
    let error = second
        .commit(&reduce_commit(TX_TWO, cursor(1), cursor(2)))
        .await
        .expect_err("a fresh coordinator publishes no view");
    assert!(matches!(error, PublicationError::UnmatchedCompletion { .. }));
    assert_eq!(broker.count_of("AuthorityCommitChange"), 0);

    // Reconciling the exact transaction identity is what lets it finish.
    second
        .open_session()
        .await
        .expect("the re-established session");
    second
        .resynchronize(&tx_id(TX_THREE), cursor(1), cursor(1))
        .await
        .expect("the reconciliation is served");
    second
        .publish_snapshot(&tx_id(TX_THREE), &snapshot(cursor(1), Some(TX_TWO)))
        .await
        .expect("the outstanding identity travels with the resynchronization");
    second
        .prepare(&reduce_candidate(TX_TWO, cursor(1), cursor(2)))
        .await
        .expect("the exact candidate replays against the reconciled fence");
    let accepted = second
        .commit(&reduce_commit(TX_TWO, cursor(1), cursor(2)))
        .await
        .expect("the reconciled transaction finishes");
    assert_eq!(accepted.sequence, sequence(2));
    assert_eq!(second.accepted().await, cursor(2));
}

#[tokio::test]
async fn an_unbounded_queue_is_refused_rather_than_grown() {
    let broker = Arc::new(Broker::default());
    let coordinator = bind(broker.clone());
    for index in 0..64u32 {
        coordinator
            .queue_behind(&tx_id(&format!("{index:x}")))
            .await
            .expect("the bounded queue accepts up to its ceiling");
    }
    let error = coordinator
        .queue_behind(&tx_id("ff"))
        .await
        .expect_err("the queue is bounded");
    assert!(matches!(error, PublicationError::Refused { .. }));
    assert_eq!(coordinator.queued().await.len(), 64);
    assert!(broker.ops().is_empty(), "the queue holds identities, not traffic");
}

// ---------------------------------------------------------------------------
// The bounds the broker declares
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_broker_that_declares_no_bounds_is_not_streamed_to() {
    #[derive(Debug)]
    struct Boundless(Arc<Broker>);

    #[async_trait]
    impl AuthorityPublicationLink for Boundless {
        async fn open_session(
            &self,
            open: AuthorityPublicationOpen,
        ) -> Result<AuthorityPublicationResponse, String> {
            self.0.open_session(open).await.map(|reply| {
                let AuthorityPublicationResponse::Opened(mut opened) = reply else {
                    return reply;
                };
                opened.limits.max_chunk_bytes = 0;
                opened.limits.max_chunks = 0;
                AuthorityPublicationResponse::Opened(opened)
            })
        }

        async fn serve(
            &self,
            envelope: AuthorityPublicationEnvelope,
        ) -> Result<AuthorityPublicationResponse, String> {
            self.0.serve(envelope).await
        }
    }

    let broker = Arc::new(Broker::default());
    let coordinator: AuthorityPublicationCoordinator = AuthorityPublicationCoordinator::new(
        ZONE,
        incarnation(),
        bootstrap(),
        Arc::new(Boundless(Arc::clone(&broker))),
    );
    let error = coordinator
        .open_session()
        .await
        .expect_err("a broker that declares no ceiling is refused");
    assert!(matches!(error, PublicationError::NoSession));
    assert_eq!(broker.ops(), vec!["OpenSession"], "nothing was streamed");

    // A broker that does declare bounds is streamed to, and the document a
    // bounded manager transfers is digested through the shared contract helper,
    // so both legs cover the same bytes.
    let broker = Arc::new(Broker::default());
    let coordinator = bind(broker.clone());
    coordinator.open_session().await.expect("a session");
    coordinator
        .publish_snapshot(&tx_id("a1"), &snapshot(cursor(1), None))
        .await
        .expect("the bounded transfer is served");
    assert_eq!(
        broker.ops(),
        vec![
            "OpenSession",
            "AuthorityBeginSnapshot",
            "AuthoritySnapshotChunk",
            "AuthorityEndSnapshot",
        ],
        "a snapshot is transferred in bounded chunks and closed with its digest"
    );
    let document = snapshot(cursor(1), None);
    let mut other = document.clone();
    other.cursor = cursor(2);
    assert_ne!(
        publication_snapshot_digest(&document).as_str(),
        publication_snapshot_digest(&other).as_str(),
        "a different cursor is a different document"
    );
}
