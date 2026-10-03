//! The manager's freeze / commit / publish / acknowledge coordinator (U7, KTD6-KTD7).
//!
//! # The order this module exists to enforce
//!
//! One authority mutation travels through four durable steps, and the plan's
//! ordering is what makes manager commit and broker admission agree across
//! failure and restart:
//!
//! ```text
//! stage   the manager persists the staged candidate and its reserved sequence
//! prepare PrepareChange: the broker validates the candidate against its prior
//!         accepted graph and durably freezes the Zone's new-effect admission
//! commit  the manager's SQLite transaction commits the desired rows, the
//!         revisions, the audit record, and the outbox entry together
//! publish CommitChange: the broker advances its projection and answers with
//!         the accepted revision
//! visible the manager publishes accepted visibility and the Zone unfreezes
//! ```
//!
//! `AuthorityAccepted` is the last broker answer and it is *not* revocation:
//! the release evidence a reducing change still owes is a separate
//! [`RevocationOutcome`](d2b_contracts_broker::broker_wire::RevocationConvergence).
//!
//! # Nothing crosses the transport wait
//!
//! [`AuthorityPublicationCoordinator::prepare`] and
//! [`AuthorityPublicationCoordinator::commit`] are callback-free and take owned
//! durable facts, not a store handle: the caller has already closed its SQLite
//! transaction and dropped its manager and reservation guards before either is
//! called, and neither keeps any borrow alive across the round trip. The
//! coordinator returns to its caller's mailbox as soon as each answer is in, and
//! the broker I/O itself runs on one owned bounded worker rather than on an
//! executor thread.
//!
//! # A pending transaction blocks, it does not vanish
//!
//! At most one transaction is outstanding per Zone. While one is, other
//! authority mutations for that Zone queue behind it - they are refused by name
//! and their identity is durable, so the queue is a fact the store already
//! holds - while observation and safe drain stay serviceable through the
//! bounded control lane.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use d2b_contracts_broker::broker_wire::{
    AuthorityCursor, AuthorityProjectionRow, AuthorityPublicationEnvelope,
    AuthorityPublicationOpen, AuthorityPublicationRequest, AuthorityPublicationResponse,
    AuthoritySnapshot, BeginSnapshotRequest, CancelTransactionRequest, CommitChangeRequest,
    ZoneAuthorityState,
    ControlActionRequest, EffectExitRequest, EndSnapshotRequest,
    MAX_PUBLICATION_CHUNK_BYTES, PrepareChangeRequest,
    PreparedTransaction, PublicationControlKind, PublicationMutationKind, PublicationSession,
    PublicationTransactionId, ReleaseEffectRequest, ResynchronizeRequest, RevocationConvergence,
    SnapshotChunkRequest, publication_candidate_digest, publication_snapshot_bytes,
    publication_snapshot_digest,
};
use d2b_contracts_resource::v3::{
    AdmissionStage, AuthoritySubject, AuthoritySubjectKind, CanonicalJsonObject, DesiredDigest,
    RefusalReason, ResourceRef, StoreIncarnation, ZoneDesiredSequence,
};
use tokio::sync::{Mutex, mpsc, oneshot};

/// The most authority mutations that may queue behind one pending transaction.
///
/// The queue holds identities, not desired state, so it is a bounded list of
/// names rather than a second store of candidates.
const MAX_QUEUED_MUTATIONS: usize = 64;

/// What one publication round trip refused, and what the Zone is left as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicationError {
    /// The transport itself failed: the broker was unreachable or answered
    /// something that is not a publication response.
    Transport {
        /// What failed.
        detail: String,
    },
    /// The broker refused, and the Zone is left as the refusal says.
    Refused {
        /// The closed refusal code.
        code: String,
        /// The stage that refused.
        stage: AdmissionStage,
        /// The typed reason.
        reason: RefusalReason,
        /// Whether the Zone is fenced after this refusal.
        fenced: bool,
        /// The Zone's own state the broker answered with.
        ///
        /// The broker decides every refusal against the Zone it durably
        /// holds, so this is the only statement of that Zone's posture the
        /// manager ever sees. Dropping it left the manager unable to tell a
        /// Zone that owes an outstanding transaction from one that simply
        /// holds a different store generation, and both read as the same
        /// closed code with nothing to reconcile against.
        state: ZoneAuthorityState,
    },
    /// A control action gave up on its budget.
    ///
    /// A control timeout keeps the fence and the conservative ownership state.
    /// It never thaws the Zone: a caller that went away is not evidence that
    /// anything was resolved.
    ControlTimedOut {
        /// The transaction the abandoned control action was bound to.
        transaction: PublicationTransactionId,
    },
    /// The completion did not match the transaction or desired sequence this
    /// coordinator still had pending.
    UnmatchedCompletion {
        /// The transaction the completion named.
        completion: PublicationTransactionId,
        /// The transaction the coordinator still holds.
        pending: PublicationTransactionId,
    },
    /// The Zone already has a pending transaction and this call would have
    /// started a second one.
    AlreadyPending {
        /// The transaction still outstanding.
        pending: PublicationTransactionId,
    },
    /// The broker answered an open with no session in it.
    NoSession,
}

impl std::fmt::Display for PublicationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport { detail } => write!(f, "authority publication transport: {detail}"),
            Self::Refused {
                code,
                stage,
                reason,
                fenced,
                state,
            } => write!(
                f,
                "authority publication refused: {code} at {stage:?} ({reason:?}), \
                 fenced={fenced}, zone-state={state:?}"
            ),
            Self::ControlTimedOut { transaction } => {
                write!(f, "control action timed out for {transaction}; the fence is kept")
            }
            Self::UnmatchedCompletion { completion, pending } => write!(
                f,
                "completion for {completion} does not match the pending {pending}"
            ),
            Self::AlreadyPending { pending } => {
                write!(f, "{pending} is still pending; other mutations queue behind it")
            }
            Self::NoSession => f.write_str("the broker's open carried no session"),
        }
    }
}

impl std::error::Error for PublicationError {}

impl PublicationError {
    /// Whether the Zone stays fenced after this error.
    pub fn is_fenced(&self) -> bool {
        match self {
            Self::Refused { fenced, .. } => *fenced,
            // A transport that could not answer proves nothing, so the Zone is
            // treated as fenced rather than thawed.
            Self::Transport { .. } | Self::ControlTimedOut { .. } => true,
            Self::UnmatchedCompletion { .. } | Self::AlreadyPending { .. } | Self::NoSession => true,
        }
    }

    fn transport(detail: impl Into<String>) -> Self {
        Self::Transport {
            detail: detail.into(),
        }
    }

    fn from_response(response: &AuthorityPublicationResponse) -> Result<(), Self> {
        match response {
            AuthorityPublicationResponse::Refused(refusal) => Err(Self::Refused {
                code: refusal.code.clone(),
                stage: refusal.stage,
                reason: refusal.reason,
                fenced: refusal.fenced,
                state: refusal.state.clone(),
            }),
            _ => Ok(()),
        }
    }
}

/// The transport seam: one authenticated publication round trip over the
/// established daemon/broker origination connection.
///
/// The seam is a trait so the coordinator's ordering can be driven by a fixture
/// that injects an interruption at each boundary, and so the real link and the
/// fixture cannot disagree about which call is a round trip. A provider handler
/// cannot obtain an implementation: the composition owns the link, and the
/// session it carries is the only authority the transport itself confers.
#[async_trait]
pub trait AuthorityPublicationLink: Send + Sync + std::fmt::Debug {
    /// Open one Zone publication session.
    ///
    /// The answer is the publication family's own response type, so a broker
    /// that refuses the open is as readable here as a broker that minted one:
    /// the two are the same message with the same answer vocabulary, and a
    /// refusal this leg could not express would reach the manager as an
    /// opaque transport failure instead of the fence it actually is.
    async fn open_session(
        &self,
        open: AuthorityPublicationOpen,
    ) -> Result<AuthorityPublicationResponse, String>;
    /// Serve one publication message.
    async fn serve(
        &self,
        envelope: AuthorityPublicationEnvelope,
    ) -> Result<AuthorityPublicationResponse, String>;
}

/// The link over the daemon's own broker socket.
///
/// One length-prefixed JSON frame each way on the same seqpacket connection the
/// daemon's other broker operations already use, so the publication family
/// inherits that connection's peer authentication rather than opening a second
/// privileged surface.
#[derive(Debug, Clone)]
pub struct OriginationPublicationLink {
    socket_path: PathBuf,
    budget: Duration,
}

impl OriginationPublicationLink {
    /// Bind the link to one broker socket and one round-trip budget.
    ///
    /// The budget bounds the connect AND the exchange: it is applied as the
    /// socket's read and write deadline for the duration of the round trip, so
    /// a broker that accepts and then stalls cannot park the single worker
    /// thread. A budget that runs out keeps the fence rather than thawing the
    /// Zone.
    pub fn new(socket_path: impl Into<PathBuf>, budget: Duration) -> Self {
        Self {
            socket_path: socket_path.into(),
            budget,
        }
    }

    /// The socket this link dials.
    pub fn socket_path(&self) -> &Path {
        self.socket_path.as_path()
    }
}

#[async_trait]
impl AuthorityPublicationLink for OriginationPublicationLink {
    async fn open_session(
        &self,
        open: AuthorityPublicationOpen,
    ) -> Result<AuthorityPublicationResponse, String> {
        // The blocking socket round trip runs on the coordinator's own bounded
        // worker, not on an executor thread: this is the one place the daemon
        // performs blocking publication I/O and it never happens inline.
        let budget = self.budget;
        round_trip_on_worker(
            self.socket_path.clone(),
            Some(budget),
            move |socket| {
            d2bd_runtime::unix_transport::write_json_frame(socket, &open)
                .map_err(|error| error.to_string())?;
            let body = d2bd_runtime::unix_transport::read_frame(socket)
                .map_err(|error| error.to_string())?;
            serde_json::from_slice::<AuthorityPublicationResponse>(&body)
                .map_err(|error| error.to_string())
            },
        )
        .await
    }

    async fn serve(
        &self,
        envelope: AuthorityPublicationEnvelope,
    ) -> Result<AuthorityPublicationResponse, String> {
        let budget = self.budget;
        round_trip_on_worker(
            self.socket_path.clone(),
            Some(budget),
            move |socket| {
            d2bd_runtime::unix_transport::write_json_frame(socket, &envelope)
                .map_err(|error| error.to_string())?;
            let body = d2bd_runtime::unix_transport::read_frame(socket)
                .map_err(|error| error.to_string())?;
            serde_json::from_slice::<AuthorityPublicationResponse>(&body)
                .map_err(|error| error.to_string())
            },
        )
        .await
    }
}

/// Run one blocking socket exchange on the coordinator's own worker.
///
/// The plan bans `spawn_blocking` because it reaches the runtime's shared
/// blocking pool, and an async-to-sync bridge is banned outright; the shape
/// this uses is the sanctioned one instead - a caller-supplied blocking closure
/// on the coordinator's own single bounded worker, reached by a oneshot reply.
/// The caller of this function is the async coordinator, which awaits the reply;
/// only the worker thread itself blocks.
///
/// The budget bounds the whole exchange and not only the connect: it rides on
/// the connected socket as that socket's own read and write deadline, so a
/// broker that accepts and then stops answering returns a typed error to this
/// worker instead of parking it.
async fn round_trip_on_worker<T, F>(
    socket_path: PathBuf,
    budget: Option<Duration>,
    exchange: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&std::os::fd::OwnedFd) -> Result<T, String> + Send + 'static,
{
    let (reply_tx, reply_rx) = oneshot::channel();
    blocking_worker()
        .send(Box::new(move || {
            let result =
                d2bd_runtime::unix_transport::connect_seqpacket_with_timeout(&socket_path, budget)
                    .map_err(|error| error.to_string())
                    .and_then(|socket| {
                        let socket = socket2::Socket::from(socket);
                        let bounded = install_exchange_budget(&socket, budget);
                        let fd: std::os::fd::OwnedFd = socket.into();
                        bounded.and(exchange(&fd))
                    });
            let _ = reply_tx.send(result);
        }))
        .map_err(|_| "the publication worker is unavailable".to_owned())?;
    // The reply is awaited rather than blocked on: the coordinator runs on a
    // runtime worker, and only the worker thread itself blocks. Blocking here
    // would panic on every publication the daemon ever attempted.
    reply_rx
        .await
        .map_err(|_| "the publication worker dropped the exchange".to_owned())?
}

/// Install `budget` as the connected broker socket's own read and write
/// deadline, so the blocking exchange inside the round trip is bounded by the
/// same budget as the connect.
///
/// A failure here refuses the round trip instead of running it unbounded. The
/// accept loop's own frame deadline is deliberately best-effort because it has
/// a handler slot per peer to lose, but there is exactly one publication
/// worker here: a socket that entered the exchange without a deadline is the
/// one condition that wedges every publication the daemon ever makes.
fn install_exchange_budget(
    socket: &socket2::Socket,
    budget: Option<Duration>,
) -> Result<(), String> {
    let Some(budget) = budget else {
        return Ok(());
    };
    socket
        .set_read_timeout(Some(budget))
        .and_then(|()| socket.set_write_timeout(Some(budget)))
        .map_err(|error| {
            format!("the publication budget could not be installed on the broker socket: {error}")
        })
}

/// One owned bounded worker for the coordinator's blocking publication I/O.
struct BlockingWorker {
    commands: mpsc::Sender<Box<dyn FnOnce() + Send>>,
}

impl std::fmt::Debug for BlockingWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BlockingWorker(<publication>)")
    }
}

/// The process's one publication worker.
///
/// A single thread with a bounded queue, so a burst of publications cannot grow
/// a thread per call and a slow broker delays the next exchange rather than
/// spawning another worker.
fn blocking_worker() -> &'static BlockingWorker {
    static WORKER: std::sync::LazyLock<BlockingWorker> = std::sync::LazyLock::new(|| {
        let (commands, mut receiver) = mpsc::channel::<Box<dyn FnOnce() + Send>>(64);
        std::thread::Builder::new()
            .name("d2bd-authority-publication".to_owned())
            .spawn(move || {
                // The worker's own dedicated thread: a blocking receive here is
                // the sanctioned bounded-worker boundary, not a runtime worker
                // being parked.
                #[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
                while let Some(command) = receiver.blocking_recv() {
                    command();
                }
            })
            .expect("the publication worker thread spawns");
        BlockingWorker { commands }
    });
    &WORKER
}

impl BlockingWorker {
    fn send(&self, command: Box<dyn FnOnce() + Send>) -> Result<(), ()> {
        self.commands.try_send(command).map_err(|_| ())
    }
}

/// One staged candidate, as the coordinator presents it to the broker.
///
/// Every field is a durable fact the manager already committed or reserved, so
/// nothing here is a handle into the store and nothing here needs a lock to
/// stay valid across the round trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareCandidate {
    /// The exact staged transaction identity.
    pub transaction: PublicationTransactionId,
    /// The store generation the candidate was staged in.
    pub store_incarnation: StoreIncarnation,
    /// The predecessor the manager saw accepted.
    pub expected: AuthorityCursor,
    /// The cursor this change will commit at.
    pub committed: AuthorityCursor,
    /// The authenticated initiating subject of the mutation.
    pub subject: AuthoritySubject,
    /// What the change does to the rows it names.
    pub kind: PublicationMutationKind,
    /// The rows the change introduces or rewrites.
    pub candidate: Vec<AuthorityProjectionRow>,
    /// The rows the change retires.
    pub removed: Vec<d2b_contracts_resource::v3::ResourceRef>,
}

impl PrepareCandidate {
    /// The digest of the exact committed bytes this candidate installs.
    ///
    /// It is derived through the shared contract helper, so the manager and the
    /// broker cannot disagree about which bytes an identity names.
    pub fn digest(&self) -> DesiredDigest {
        publication_candidate_digest(&self.candidate, &self.removed)
    }

    fn to_request(&self) -> PrepareChangeRequest {
        PrepareChangeRequest {
            transaction: self.transaction.clone(),
            store_incarnation: self.store_incarnation.clone(),
            expected: self.expected.clone(),
            committed: self.committed.clone(),
            digest: self.digest(),
            subject: self.subject.clone(),
            kind: self.kind,
            candidate: self.candidate.clone(),
            removed: self.removed.clone(),
        }
    }
}

/// The exact committed state a commit installs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitCandidate {
    /// The exact staged transaction identity the fence was prepared for.
    pub transaction: PublicationTransactionId,
    /// The store generation the rows were committed in.
    pub store_incarnation: StoreIncarnation,
    /// The predecessor the prepare validated against.
    pub expected: AuthorityCursor,
    /// The cursor this commit installs.
    pub committed: AuthorityCursor,
    /// The rows the commit installed.
    pub rows: Vec<AuthorityProjectionRow>,
    /// The rows the commit retired.
    pub removed: Vec<d2b_contracts_resource::v3::ResourceRef>,
}

impl CommitCandidate {
    /// The digest of the exact committed bytes this commit installs.
    pub fn digest(&self) -> DesiredDigest {
        publication_candidate_digest(&self.rows, &self.removed)
    }

    fn to_request(&self) -> CommitChangeRequest {
        CommitChangeRequest {
            transaction: self.transaction.clone(),
            store_incarnation: self.store_incarnation.clone(),
            expected: self.expected.clone(),
            committed: self.committed.clone(),
            digest: self.digest(),
            rows: self.rows.clone(),
            removed: self.removed.clone(),
        }
    }
}

/// What the broker durably froze for one candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedPublication {
    /// The exact transaction identity both sides now hold.
    pub transaction: PublicationTransactionId,
    /// The cursor the candidate will commit at.
    pub committed: AuthorityCursor,
    /// The digest of the exact candidate bytes.
    pub digest: DesiredDigest,
    /// Whether accepting this transaction would reduce the Zone's authority.
    pub reducing: bool,
    /// The broker's own view of the transaction, for a caller that wants to
    /// assert both sides agree.
    pub broker_view: PreparedTransaction,
}

/// The one outcome the manager may publish accepted visibility for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedPublication {
    /// The accepted sequence.
    pub sequence: d2b_contracts_resource::v3::ZoneDesiredSequence,
    /// The accepted digest.
    pub digest: DesiredDigest,
    /// Whether this acceptance reduced the Zone's authority, in which case
    /// release evidence is still owed and is reported separately.
    pub reducing: bool,
}

/// One outstanding use that reached its declared safe state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevocationOutcome {
    /// The transaction whose reducing change drove this use to its end.
    pub transaction: PublicationTransactionId,
    /// The effect that ended.
    pub effect: d2b_contracts_broker::broker_wire::PublicationEffectId,
    /// Whether the use was proved exited or accounted as a pre-fence release.
    pub proven: bool,
}

/// One durable publication transaction this coordinator still owes an outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingTransaction {
    transaction: PublicationTransactionId,
    expected: AuthorityCursor,
    committed: AuthorityCursor,
    digest: DesiredDigest,
    reducing: bool,
}

/// The manager's publication coordinator for one Zone.
///
/// It is owned, cloneable, and `Send + Sync`, so the manager can hand the
/// publication work to a coordinator task and return to its mailbox: no manager
/// lock, store transaction, or source-reservation guard is alive while the
/// broker round trip runs.
#[derive(Debug, Clone)]
pub struct AuthorityPublicationCoordinator {
    zone: String,
    incarnation: StoreIncarnation,
    subject: AuthoritySubject,
    link: Arc<dyn AuthorityPublicationLink>,
    session: Arc<Mutex<Option<PublicationSession>>>,
    accepted: Arc<Mutex<AuthorityCursor>>,
    pending: Arc<Mutex<Option<PendingTransaction>>>,
    queue: Arc<Mutex<VecDeque<PublicationTransactionId>>>,
}

impl AuthorityPublicationCoordinator {
    /// Bind a coordinator to one Zone, store generation, and initiating subject.
    pub fn new(
        zone: impl Into<String>,
        incarnation: StoreIncarnation,
        subject: AuthoritySubject,
        link: Arc<dyn AuthorityPublicationLink>,
    ) -> Self {
        Self {
            zone: zone.into(),
            incarnation,
            subject,
            link,
            session: Arc::new(Mutex::new(None)),
            accepted: Arc::new(Mutex::new(AuthorityCursor::initial())),
            pending: Arc::new(Mutex::new(None)),
            queue: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// The Zone this coordinator publishes for.
    pub fn zone(&self) -> &str {
        &self.zone
    }

    /// The cursor this coordinator believes the broker has accepted.
    pub async fn accepted(&self) -> AuthorityCursor {
        self.accepted.lock().await.clone()
    }

    /// The transaction still outstanding, if any.
    pub async fn pending(&self) -> Option<PublicationTransactionId> {
        self.pending.lock().await.as_ref().map(|pending| pending.transaction.clone())
    }

    /// The transactions queued behind the pending one.
    pub async fn queued(&self) -> Vec<PublicationTransactionId> {
        self.queue.lock().await.iter().cloned().collect()
    }

    /// Establish the Zone publication session this coordinator publishes under.
    ///
    /// The session is bound to the Zone, the store generation, the broker epoch,
    /// the authenticated initiating subject the trusted daemon admission
    /// coordinator vouched for, and the accepted cursor. Re-opening is cheap and
    /// idempotent from the manager's side: the broker answers with the cursor it
    /// holds, which is what a manager whose own view has moved on needs.
    pub async fn open_session(&self) -> Result<PublicationSession, PublicationError> {
        let accepted = self.accepted.lock().await.clone();
        let open = AuthorityPublicationOpen {
            request: d2b_contracts_broker::broker_wire::OpenPublicationSessionRequest {
                zone: self.zone.clone(),
                store_incarnation: self.incarnation.clone(),
                broker_epoch: 0,
                initiating_subject: self.subject.clone(),
                accepted,
            },
        };
        let reply = self
            .link
            .open_session(open)
            .await
            .map_err(PublicationError::transport)?;
        // A refused open is a refusal like any other: it keeps the Zone fenced
        // and it is the difference between a retry and a dead projection, so it
        // is propagated with its own code rather than reported as a transport
        // that never arrived.
        if let Err(error) = PublicationError::from_response(&reply) {
            // The broker's own view of the Zone and the generation this manager
            // asked for are the two halves of a store-generation refusal, and
            // only one of them was ever readable. Both are named here so the
            // refusal is diagnosable from the daemon's own log.
            tracing::error!(
                zone = %self.zone,
                requested_store_incarnation = %self.incarnation.as_str(),
                %error,
                "the broker refused a Zone publication session"
            );
            return Err(error);
        }
        let AuthorityPublicationResponse::Opened(opened) = &reply else {
            // Anything else is an answer to a different message: the broker
            // claims a step this one never took, so the manager holds no
            // session rather than a session it cannot justify.
            return Err(PublicationError::NoSession);
        };
        if opened.limits.max_chunk_bytes == 0 || opened.limits.max_chunks == 0 {
            // A broker that declares no bounds is refusing to be bounded, which
            // the plan does not permit: the manager will not stream into it.
            return Err(PublicationError::NoSession);
        }
        *self.accepted.lock().await = opened.binding.accepted.clone();
        *self.session.lock().await = Some(opened.session.clone());
        Ok(opened.session.clone())
    }

    /// Durably freeze the Zone's new-effect admission for one candidate.
    ///
    /// Callback-free: it records the staged transaction locally and performs one
    /// broker round trip, holding no guard of any kind across the wait. The
    /// caller commits its desired rows only after this returns, so the broker's
    /// fence is durable before the manager's transaction is.
    pub async fn prepare(
        &self,
        candidate: &PrepareCandidate,
    ) -> Result<PreparedPublication, PublicationError> {
        if let Some(pending) = self.pending().await
            && pending != candidate.transaction
        {
            return Err(PublicationError::AlreadyPending { pending });
        }
        let envelope = self
            .envelope(AuthorityPublicationRequest::PrepareChange(candidate.to_request()))
            .await?;
        let response = self
            .link
            .serve(envelope)
            .await
            .map_err(PublicationError::transport)?;
        // A refusal is an answer, not a missing one: it carries the closed code
        // and the Zone's state, and a manager that reported it as an unmatched
        // completion would lose exactly the evidence it needs to reconcile.
        PublicationError::from_response(&response)?;
        let AuthorityPublicationResponse::Prepared(prepared) = response else {
            return Err(PublicationError::UnmatchedCompletion {
                completion: candidate.transaction.clone(),
                pending: self
                    .pending()
                    .await
                    .unwrap_or_else(|| candidate.transaction.clone()),
            });
        };
        if prepared.transaction != candidate.transaction {
            // The broker prepared a different identity than the manager staged;
            // nothing may be committed against a fence this coordinator cannot
            // name.
            return Err(PublicationError::UnmatchedCompletion {
                completion: prepared.transaction,
                pending: candidate.transaction.clone(),
            });
        }
        *self.pending.lock().await = Some(PendingTransaction {
            transaction: prepared.transaction.clone(),
            expected: candidate.expected.clone(),
            committed: prepared.committed.clone(),
            digest: prepared.digest.clone(),
            reducing: prepared.reducing,
        });
        Ok(PreparedPublication {
            transaction: prepared.transaction.clone(),
            committed: prepared.committed.clone(),
            digest: prepared.digest.clone(),
            reducing: prepared.reducing,
            broker_view: prepared,
        })
    }

    /// Advance the broker's projection to one exact committed state.
    ///
    /// The completion applies only if the expected transaction and desired
    /// sequence still match what this coordinator has pending. A commit naming
    /// anything else is refused by name, so a stale actor's publication can
    /// never move the manager's accepted cursor.
    pub async fn commit(
        &self,
        commit: &CommitCandidate,
    ) -> Result<AcceptedPublication, PublicationError> {
        let Some(pending) = self.pending.lock().await.clone() else {
            return Err(PublicationError::UnmatchedCompletion {
                completion: commit.transaction.clone(),
                pending: commit.transaction.clone(),
            });
        };
        if pending.transaction != commit.transaction
            || pending.expected != commit.expected
            || pending.committed != commit.committed
            || pending.digest != commit.digest()
        {
            return Err(PublicationError::UnmatchedCompletion {
                completion: commit.transaction.clone(),
                pending: pending.transaction,
            });
        }
        self.publish_commit(commit).await
    }

    /// Advance the broker's projection for a fence this process did not take.
    ///
    /// This is the recovery entry point, and it exists because the record
    /// [`Self::commit`] matches against is per-process: the fence it would
    /// match was written by a previous process, and that process's record of
    /// it is gone. What survives is the store's own committed transaction and
    /// the broker's own durable fence, and the broker is the authority on
    /// whether that fence still exists - it re-checks the exact prepared
    /// identity, the exact predecessor, and the exact committed bytes before it
    /// installs anything. Every answer check below is unchanged, and a broker
    /// that holds no such fence refuses by name, so nothing is given up except
    /// a local echo that no longer exists.
    pub async fn adopt_commit(
        &self,
        commit: &CommitCandidate,
    ) -> Result<AcceptedPublication, PublicationError> {
        self.publish_commit(commit).await
    }

    /// The commit round trip and its answer, shared by both entry points.
    async fn publish_commit(
        &self,
        commit: &CommitCandidate,
    ) -> Result<AcceptedPublication, PublicationError> {
        let envelope = self
            .envelope(AuthorityPublicationRequest::CommitChange(commit.to_request()))
            .await?;
        let response = self
            .link
            .serve(envelope)
            .await
            .map_err(PublicationError::transport)?;
        PublicationError::from_response(&response)?;
        let AuthorityPublicationResponse::Accepted(accepted) = response else {
            return Err(PublicationError::UnmatchedCompletion {
                completion: commit.transaction.clone(),
                pending: commit.transaction.clone(),
            });
        };
        if accepted.transaction != commit.transaction
            || accepted.sequence != commit.committed.sequence
            || accepted.digest != commit.digest()
        {
            return Err(PublicationError::UnmatchedCompletion {
                completion: accepted.transaction,
                pending: commit.transaction.clone(),
            });
        }
        *self.accepted.lock().await = commit.committed.clone();
        *self.pending.lock().await = None;
        let mut queue = self.queue.lock().await;
        queue.retain(|queued| queued != &commit.transaction);
        Ok(AcceptedPublication {
            sequence: accepted.sequence,
            digest: accepted.digest,
            reducing: accepted.reducing,
        })
    }

    /// Release one prepared identity that committed nothing.
    ///
    /// The manager proves it holds no committed desired row for this
    /// transaction before calling; the broker checks its own side of the same
    /// fact by requiring the exact prepared identity and its digest.
    pub async fn cancel(
        &self,
        transaction: &PublicationTransactionId,
        digest: &DesiredDigest,
    ) -> Result<(), PublicationError> {
        let request = CancelTransactionRequest {
            transaction: transaction.clone(),
            store_incarnation: self.incarnation.clone(),
            digest: digest.clone(),
        };
        let envelope = self
            .envelope(AuthorityPublicationRequest::CancelTransaction(request))
            .await?;
        let response = self
            .link
            .serve(envelope)
            .await
            .map_err(PublicationError::transport)?;
        PublicationError::from_response(&response)?;
        let mut pending = self.pending.lock().await;
        if pending.as_ref().is_some_and(|held| &held.transaction == transaction) {
            *pending = None;
        }
        Ok(())
    }

    /// Transfer one full snapshot in bounded chunks.
    ///
    /// The chunks carry the transaction id, the ordinal, the total count, and -
    /// on the final message - the digest of the whole document. Nothing becomes
    /// active until that final digest matches, so a truncated transfer leaves
    /// the Zone fenced rather than half-installed.
    pub async fn publish_snapshot(
        &self,
        transaction: &PublicationTransactionId,
        snapshot: &AuthoritySnapshot,
    ) -> Result<(), PublicationError> {
        let bytes = publication_snapshot_bytes(snapshot);
        let digest = publication_snapshot_digest(snapshot);
        let chunk_count = bytes.len().div_ceil(MAX_PUBLICATION_CHUNK_BYTES).max(1) as u32;
        let begin = AuthorityPublicationRequest::BeginSnapshot(BeginSnapshotRequest {
            transaction: transaction.clone(),
            store_incarnation: self.incarnation.clone(),
            cursor: snapshot.cursor.clone(),
            total_chunks: chunk_count,
            total_bytes: bytes.len() as u64,
        });
        self.exchange(begin).await?;
        for ordinal in 0..chunk_count {
            let start = ordinal as usize * MAX_PUBLICATION_CHUNK_BYTES;
            let end = bytes.len().min(start + MAX_PUBLICATION_CHUNK_BYTES);
            let chunk = AuthorityPublicationRequest::SnapshotChunk(SnapshotChunkRequest {
                transaction: transaction.clone(),
                ordinal,
                total_chunks: chunk_count,
                payload: bytes[start..end].to_vec(),
            });
            self.exchange(chunk).await?;
        }
        self.exchange(AuthorityPublicationRequest::EndSnapshot(EndSnapshotRequest {
            transaction: transaction.clone(),
            total_chunks: chunk_count,
            digest,
        }))
        .await?;
        *self.accepted.lock().await = snapshot.cursor.clone();
        Ok(())
    }

    /// Declare the intent to reconcile after a broker restart.
    ///
    /// The floor names the cursor the broker must not move below, so a
    /// resynchronization cannot install a projection behind the one it already
    /// accepted.
    pub async fn resynchronize(
        &self,
        transaction: &PublicationTransactionId,
        accepted_floor: AuthorityCursor,
        cursor: AuthorityCursor,
    ) -> Result<(), PublicationError> {
        let request = ResynchronizeRequest {
            transaction: transaction.clone(),
            store_incarnation: self.incarnation.clone(),
            accepted_floor,
            cursor,
        };
        self.exchange(AuthorityPublicationRequest::Resynchronize(request))
            .await
            .map(|_| ())
    }

    /// Serve one bounded control action under a budget.
    ///
    /// This is the lane a fence keeps open. A budget that runs out keeps the
    /// fence and the conservative ownership state: the coordinator reports the
    /// timeout and does not clear anything.
    pub async fn control(
        &self,
        action: ControlActionRequest,
        budget: Duration,
    ) -> Result<AuthorityPublicationResponse, PublicationError> {
        if action.kind.stage().admits_new_use() {
            // A control kind that would admit new use is not a control action;
            // the coordinator refuses it before it reaches the transport.
            return Err(PublicationError::Refused {
                code: d2b_contracts_broker::broker_wire::PUBLICATION_CONTROL_NOT_BOUND.to_owned(),
                stage: action.kind.stage(),
                reason: RefusalReason::UntrustedImplementation,
                fenced: true,
                // This half refused before it reached the transport, so it has
                // no broker answer to carry: the Zone is reported as the
                // unprovisioned state this half can still name, which is the
                // fail-closed reading rather than a claim about the broker.
                state: ZoneAuthorityState::Unprovisioned,
            });
        }
        let request = AuthorityPublicationRequest::ControlAction(action.clone());
        match tokio::time::timeout(budget, self.exchange(request)).await {
            Ok(result) => result,
            Err(_) => Err(PublicationError::ControlTimedOut {
                transaction: action.transaction,
            }),
        }
    }

    /// Observe already-owned state. Never queued behind a pending transaction
    /// and never subject to a control budget, because it changes nothing.
    pub async fn observe(
        &self,
        transaction: &PublicationTransactionId,
        target: d2b_contracts_resource::v3::ResourceRef,
    ) -> Result<AuthorityPublicationResponse, PublicationError> {
        self.exchange(AuthorityPublicationRequest::ControlAction(
            ControlActionRequest {
                transaction: transaction.clone(),
                effect: None,
                target,
                kind: PublicationControlKind::Observe,
            },
        ))
        .await
    }

    /// Admit one effect under the Zone's currently accepted projection.
    pub async fn begin_effect(
        &self,
        request: d2b_contracts_broker::broker_wire::BeginEffectRequest,
    ) -> Result<AuthorityPublicationResponse, PublicationError> {
        self.exchange(AuthorityPublicationRequest::BeginEffect(request))
            .await
    }

    /// The child's exec-release gate.
    ///
    /// A launch that passed `BeginEffect` but has not completed its exec and
    /// registration handshake is pending new use. The broker's answer decides
    /// which of the two it is, and a refusal is the fence winning: setup is
    /// cancelled and reaped and the child cannot exec under its earlier
    /// `BeginEffect`.
    pub async fn release_effect(
        &self,
        request: ReleaseEffectRequest,
    ) -> Result<AuthorityPublicationResponse, PublicationError> {
        self.exchange(AuthorityPublicationRequest::ReleaseEffect(request))
            .await
    }

    /// Report one child's completion from outside the authority worker's
    /// mailbox, and read the separate `RevocationConverged` outcome when the
    /// child had already reached exec.
    pub async fn effect_exit(
        &self,
        request: EffectExitRequest,
    ) -> Result<RevocationOutcome, PublicationError> {
        let response = self
            .exchange(AuthorityPublicationRequest::EffectExit(request.clone()))
            .await?;
        match response {
            AuthorityPublicationResponse::RevocationConverged(RevocationConvergence {
                transaction,
                effect,
                proven,
                ..
            }) => Ok(RevocationOutcome {
                transaction,
                effect,
                proven,
            }),
            other => {
                PublicationError::from_response(&other)?;
                Err(PublicationError::UnmatchedCompletion {
                    completion: request.transaction.clone(),
                    pending: request.transaction,
                })
            }
        }
    }

    /// Exchange one ordinary publication message, propagating a refusal.
    async fn exchange(
        &self,
        request: AuthorityPublicationRequest,
    ) -> Result<AuthorityPublicationResponse, PublicationError> {
        let envelope = self.envelope(request).await?;
        let response = self
            .link
            .serve(envelope)
            .await
            .map_err(PublicationError::transport)?;
        PublicationError::from_response(&response)?;
        Ok(response)
    }

    /// Wrap one request in the Zone's session, opening one if none is held.
    async fn envelope(
        &self,
        request: AuthorityPublicationRequest,
    ) -> Result<AuthorityPublicationEnvelope, PublicationError> {
        // The held session is read into a local before the match: a temporary
        // in a `match` scrutinee lives until the end of the `match`, so the
        // guard would still be held across the open below, and the open takes
        // the same lock. Every message that had to open its own session
        // deadlocked.
        let held = self.session.lock().await.clone();
        let session = match held {
            Some(session) => session,
            None => self.open_session().await?,
        };
        Ok(AuthorityPublicationEnvelope {
            zone: self.zone.clone(),
            request,
            session,
        })
    }

    /// Queue one mutation behind the pending transaction.
    ///
    /// The identity is durable in the manager's own store, so this list is a
    /// bounded view of what the store already owes rather than a second copy of
    /// the candidates.
    pub async fn queue_behind(&self, transaction: &PublicationTransactionId) -> Result<(), PublicationError> {
        let mut queue = self.queue.lock().await;
        if queue.len() >= MAX_QUEUED_MUTATIONS {
            return Err(PublicationError::Refused {
                code: d2b_contracts_broker::broker_wire::PUBLICATION_SNAPSHOT_TOO_LARGE.to_owned(),
                stage: AdmissionStage::Authorize,
                reason: RefusalReason::LimitExceedsCeiling,
                fenced: true,
                // The queue is this manager's own bound and it never reached
                // the transport, so the broker has not been asked and has
                // nothing to report.
                state: ZoneAuthorityState::Unprovisioned,
            });
        }
        queue.push_back(transaction.clone());
        Ok(())
    }
}

#[cfg(test)]
mod stall_regression_tests {
    use super::*;

    /// The budget the stalled round trip carries.
    const BUDGET: Duration = Duration::from_millis(200);
    /// How long the stalled peer holds the accepted connection before it gives
    /// up and closes it.
    ///
    /// A peer that closed immediately would end the unbounded exchange too,
    /// and a budget that outran this hold could not tell the two apart.
    const STALL_HOLD: Duration = Duration::from_secs(2);
    /// The most a budget-bounded exchange may take.
    ///
    /// Five budgets of headroom for a loaded machine, and half the peer's
    /// hold, so an unbounded exchange is a failed assertion rather than a
    /// timeout.
    const BOUNDED_BY: Duration = Duration::from_secs(1);
    /// The most the fake broker may wait for a connection before it gives up.
    ///
    /// A hang is a worse failure signal than a failed assertion, so nothing in
    /// this harness may block without a bound of its own.
    const ACCEPT_WITHIN: Duration = Duration::from_secs(20);
    /// What the answering peer sends back.
    const ANSWER: &[u8] = b"accepted";

    /// One fake broker socket: it stalls the first publication and answers the
    /// next.
    ///
    /// The listener is `SOCK_SEQPACKET` because that is the family the daemon's
    /// broker link speaks. An `AF_UNIX` connect from a seqpacket socket to a
    /// stream listener is refused with `EPROTOTYPE`, so a stream listener would
    /// refuse the exchange before the budget was ever reached and the test
    /// would pass for the wrong reason.
    struct FakeBroker {
        socket_path: PathBuf,
        peer: std::thread::JoinHandle<()>,
    }

    impl FakeBroker {
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn start(test_name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("d2b-publication-stall-{}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("create the stall harness scratch");
            let socket_path = dir.join(format!("{test_name}.sock"));
            let listener =
                socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::SEQPACKET, None)
                    .expect("create the fake broker listener");
            listener
                .set_read_timeout(Some(ACCEPT_WITHIN))
                .expect("bound the fake broker accept");
            listener
                .bind(&socket2::SockAddr::unix(&socket_path).expect("the broker socket address"))
                .expect("bind the fake broker socket");
            listener.listen(16).expect("listen on the fake broker socket");
            let peer = std::thread::spawn(move || {
                // Both publications are accepted before the stall is
                // released: the budget is what ends the first exchange, not
                // this peer, so the second one arrives while the first is
                // still unanswered.
                let stalled = accept(&listener);
                let answered = accept(&listener);
                d2bd_runtime::unix_transport::read_frame(&answered)
                    .expect("read the answering peer's request");
                d2bd_runtime::unix_transport::write_frame(&answered, ANSWER)
                    .expect("answer the publication exchange");
                // The stall outlives the exchange it stalls, so an unbounded
                // exchange still ends - as a failed assertion rather than a
                // worker parked for good.
                std::thread::sleep(STALL_HOLD);
                drop(answered);
                drop(stalled);
            });
            Self {
                socket_path,
                peer,
            }
        }

        /// Join the peer thread and take its socket away.
        fn finish(self) {
            self.peer.join().expect("the fake broker completes");
            let _ = std::fs::remove_file(&self.socket_path);
            if let Some(dir) = self.socket_path.parent() {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }

    /// Accept one peer, bounded by the listener's own accept deadline.
    fn accept(listener: &socket2::Socket) -> socket2::Socket {
        let (peer, _) = listener.accept().expect("accept a fake broker peer");
        peer
    }

    /// A broker that accepts the connection and then stops answering must not
    /// park the publication worker, and the exchange behind it must still run.
    ///
    /// There is exactly one worker thread, so one stalled peer would otherwise
    /// wedge every later publication: nothing behind it would ever drain, and
    /// once the queue filled the link would refuse by name. The round trip
    /// carries its budget as the socket's own deadline for exactly this reason.
    #[tokio::test(flavor = "multi_thread")]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn a_stalled_broker_does_not_wedge_the_round_trip() {
        let broker = FakeBroker::start("stalled-publication");

        let started = std::time::Instant::now();
        let stalled = round_trip_on_worker::<(), _>(
            broker.socket_path.clone(),
            Some(BUDGET),
            |socket| {
                d2bd_runtime::unix_transport::write_frame(socket, b"open")
                    .map_err(|error| error.to_string())?;
                d2bd_runtime::unix_transport::read_frame(socket)
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            },
        )
        .await;
        let stalled_for = started.elapsed();

        assert!(
            stalled_for < BOUNDED_BY,
            "the stalled exchange waited for the peer's {STALL_HOLD:?} hold instead of giving up \
             on its {BUDGET:?} budget (took {stalled_for:?})"
        );
        assert!(
            stalled.is_err(),
            "a broker that accepts and never answers must not produce a successful round trip"
        );

        let started = std::time::Instant::now();
        let answered = round_trip_on_worker::<Vec<u8>, _>(
            broker.socket_path.clone(),
            Some(BUDGET),
            |socket| {
                d2bd_runtime::unix_transport::write_frame(socket, b"serve")
                    .map_err(|error| error.to_string())?;
                d2bd_runtime::unix_transport::read_frame(socket)
                    .map_err(|error| error.to_string())
            },
        )
        .await;
        let answered_for = started.elapsed();

        assert_eq!(
            answered.as_deref().ok(),
            Some(ANSWER),
            "the exchange behind a stalled peer still reaches the broker"
        );
        assert!(
            answered_for < BOUNDED_BY,
            "the exchange behind a stalled peer was held behind it (took {answered_for:?})"
        );

        broker.finish();
    }
}

// ---------------------------------------------------------------------------
// The production publisher (KTD6-KTD7)
// ---------------------------------------------------------------------------

/// The manager's [`AuthorityPublisher`], bound to the daemon's own
/// publication coordinator over the origination leg.
///
/// This is the whole production answer to "where does the fence come from":
/// every durable Zone transaction the manager starts or recovers goes through
/// this type, and every answer it returns is the broker's. It holds no
/// desired state and decides nothing - it renders a staged projection into the
/// wire shape the broker validates and reports what the broker answered.
#[derive(Debug, Clone)]
pub struct CoordinatorPublisher {
    /// One coordinator per Zone this plane publishes for, keyed by the Zone.
    ///
    /// The coordinator carries the Zone its session is bound to and the
    /// accepted cursor that Zone's candidates must name, so it cannot serve
    /// two Zones: a candidate routed to the wrong coordinator would publish a
    /// Zone's rows under another Zone's fence. A plane publishes for its own
    /// Zone and, when it carries the foundation seed, for the system Zone
    /// whose rows that seed homes, so the table is keyed rather than singular.
    coordinators: std::collections::BTreeMap<String, Arc<AuthorityPublicationCoordinator>>,
    incarnation: StoreIncarnation,
    subject: AuthoritySubject,
}

impl CoordinatorPublisher {
    /// Bind a publisher to one Zone's coordinator, store generation, and the
    /// identity the publication session runs under.
    ///
    /// The subject is the Zone's authenticated publication identity, not the
    /// row being mutated: a publisher that presented a candidate's own
    /// identity would be letting a row authorize its own introduction.
    pub fn new(
        coordinator: Arc<AuthorityPublicationCoordinator>,
        incarnation: StoreIncarnation,
        subject: AuthoritySubject,
    ) -> Arc<Self> {
        let coordinators = std::iter::once((coordinator.zone().to_owned(), coordinator))
            .collect();
        Arc::new(Self { coordinators, incarnation, subject })
    }

    /// Bind a publisher to a second Zone's coordinator.
    ///
    /// The foundation plane publishes for its own Zone and for the system Zone
    /// the seed homes its rows in, and each Zone has its own fence and its own
    /// accepted cursor, so each needs its own coordinator.
    pub fn with_zone(
        self: &Arc<Self>,
        coordinator: Arc<AuthorityPublicationCoordinator>,
    ) -> Arc<Self> {
        let mut coordinators = self.coordinators.clone();
        coordinators.insert(coordinator.zone().to_owned(), coordinator);
        Arc::new(Self {
            coordinators,
            incarnation: self.incarnation.clone(),
            subject: self.subject.clone(),
        })
    }

    /// The coordinator that speaks for `zone`.
    ///
    /// A candidate whose Zone this publisher was not bound for is refused by
    /// name rather than served under another Zone's fence.
    fn coordinator(
        &self,
        zone: &str,
    ) -> Result<&Arc<AuthorityPublicationCoordinator>, d2b_resource_runtime::PublicationRefusal> {
        self.coordinators.get(zone).ok_or_else(|| {
            d2b_resource_runtime::PublicationRefusal::Refused(format!(
                "this plane publishes for Zones {:?}, not for {zone}",
                self.coordinators.keys().collect::<Vec<_>>()
            ))
        })
    }

    /// The cursor the broker holds for `zone`, which every candidate for that
    /// Zone must name as its exact predecessor.
    async fn expected(
        &self,
        zone: &str,
    ) -> Result<AuthorityCursor, d2b_resource_runtime::PublicationRefusal> {
        Ok(self.coordinator(zone)?.accepted().await)
    }
}

/// One publication failure as the manager sees it.
///
/// The broker's own answer is preserved where it has one: a refusal carries
/// its closed code, the stage that refused, and the typed reason, and none of
/// them is flattened into a message that would read the same as a transport
/// that never arrived.
fn transport(error: PublicationError) -> d2b_resource_runtime::PublicationRefusal {
    match error {
        PublicationError::Refused {
            code,
            stage,
            reason,
            fenced,
            state,
        } => d2b_resource_runtime::PublicationRefusal::Refused(format!(
            "{code} at {stage:?} ({reason:?}, fenced: {fenced}, zone-state: {state:?})"
        )),
        other => d2b_resource_runtime::PublicationRefusal::Refused(other.to_string()),
    }
}

/// The wire spelling of one durable transaction identity.
fn transaction_token(transaction: d2b_resource_runtime::TransactionId) -> Result<PublicationTransactionId, d2b_resource_runtime::PublicationRefusal> {
    PublicationTransactionId::parse(transaction.to_string()).map_err(|error| {
        d2b_resource_runtime::PublicationRefusal::Refused(format!(
            "a transaction identity is not a wire token: {error}"
        ))
    })
}

/// The rows one publication carries to the broker, and the keys it retires.
///
/// Both the staged projection and the committed publication go through this
/// one mapping, so the bytes a fence is validated against and the bytes a
/// commit installs are the same bytes by construction rather than by two
/// renderings that were meant to agree.
///
/// A row whose committed bytes are not a canonical object is refused rather
/// than summarized: the broker stores the bytes it accepted and re-evaluates
/// policy against them, so a projection this side shaped would be a second
/// authority.
fn publication_rows(
    rows: &[d2b_resource_runtime::PublishedRow],
    removed: &[d2b_resource_runtime::spec_store::ResourceKey],
) -> Result<(Vec<AuthorityProjectionRow>, Vec<ResourceRef>), d2b_resource_runtime::PublicationRefusal>
{
    let mut published = Vec::with_capacity(rows.len());
    for desired in rows {
        let key = &desired.row.key;
        let reference = ResourceRef::parse(format!("{}/{}", key.type_name, key.name).as_str())
            .map_err(|error| {
                d2b_resource_runtime::PublicationRefusal::Refused(format!(
                    "{key} has no canonical reference: {error}"
                ))
            })?;
        let admitted = CanonicalJsonObject::parse(&desired.row.spec).map_err(|error| {
            d2b_resource_runtime::PublicationRefusal::Refused(format!(
                "{key} commits bytes that are not a canonical desired object: {error}"
            ))
        })?;
        published.push(AuthorityProjectionRow {
            resource_ref: reference,
            desired_revision: desired.revision,
            desired_digest: desired.digest.clone(),
            admitted,
            // A binding relationship's key is over committed identity rather
            // than over references, and the store resolved that identity once
            // against the Zone's committed rows. Publishing it is what lets the
            // broker fold the same key the manager did, and what lets a
            // resynchronization restate the relationship the broker already
            // accepted instead of presenting an unresolved one it must refuse.
            source_uid: desired
                .source_uid
                .map(|uid| crate::resource_plane_v3::resource_uid(&uid))
                .transpose()
                .map_err(|()| {
                    d2b_resource_runtime::PublicationRefusal::Refused(format!(
                        "{key} resolved a source identity that is not a canonical uid"
                    ))
                })?,
            consumer_uid: desired
                .consumer_uid
                .map(|uid| crate::resource_plane_v3::resource_uid(&uid))
                .transpose()
                .map_err(|()| {
                    d2b_resource_runtime::PublicationRefusal::Refused(format!(
                        "{key} resolved a consumer identity that is not a canonical uid"
                    ))
                })?,
        });
    }
    let mut retired = Vec::with_capacity(removed.len());
    for key in removed {
        retired.push(
            ResourceRef::parse(format!("{}/{}", key.type_name, key.name).as_str()).map_err(
                |error| {
                    d2b_resource_runtime::PublicationRefusal::Refused(format!(
                        "{key} has no canonical reference: {error}"
                    ))
                },
            )?,
        );
    }
    Ok((published, retired))
}

/// The rows one staged projection will commit, rendered for publication.
fn staged_rows(
    projection: &d2b_resource_runtime::Projection,
) -> Result<(Vec<AuthorityProjectionRow>, Vec<ResourceRef>), d2b_resource_runtime::PublicationRefusal>
{
    let rows: Vec<d2b_resource_runtime::PublishedRow> = d2b_resource_runtime::PublicationRows::of(projection).rows;
    let removed: Vec<d2b_resource_runtime::spec_store::ResourceKey> =
        projection.removed.iter().map(|row| row.key.clone()).collect();
    publication_rows(&rows, &removed)
}

/// The bounded document one Zone's resynchronization transfers.
///
/// It carries the Zone's own committed rows and the relationship identity the
/// store resolved for each binding row, under the same deployment root the
/// Zone was bootstrapped with, at the cursor the BROKER reports it holds. That
/// cursor is a fact the broker has and the store does not: the digest at a
/// sequence is the broker's own publication digest, which is a different value
/// from the store's desired-row digest over the same mutation, so the document
/// restates what the broker accepted and the daemon separately checks that the
/// store reached that sequence.
///
/// A broker proves this document against the projection it already accepted,
/// so it is the store's committed state and nothing shaped here: it names no
/// retirement, because a retirement is a commit and a commit goes through the
/// fence.
fn resynchronization_document(
    projection: &d2b_resource_runtime::ZoneProjection,
    held: AuthorityCursor,
) -> Result<AuthoritySnapshot, d2b_resource_runtime::PublicationRefusal> {
    let (rows, _) = publication_rows(&projection.rows, &[])?;
    Ok(AuthoritySnapshot {
        zone: projection.zone.clone(),
        store_incarnation: projection.incarnation.clone(),
        cursor: held,
        root_subject: AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap),
        rows,
        outstanding: projection
            .outstanding
            .map(transaction_token)
            .transpose()?,
    })
}

/// The mutation class the broker re-evaluates.
fn mutation_kind(kind: d2b_resource_runtime::MutationKind) -> PublicationMutationKind {
    match kind {
        d2b_resource_runtime::MutationKind::Create => PublicationMutationKind::Create,
        d2b_resource_runtime::MutationKind::Update => PublicationMutationKind::UpdateSpec,
        d2b_resource_runtime::MutationKind::Delete => PublicationMutationKind::Delete,
    }
}

/// Render one committed publication into the wire candidate this publisher's
/// coordinator sends, under the cursor the broker holds now.
///
/// Both commit entry points build it this way, so a recovery replay and the
/// live publication it replays are the same bytes on the wire.
async fn commit_candidate(
    publisher: &CoordinatorPublisher,
    publication: &d2b_resource_runtime::CommittedPublication,
) -> Result<CommitCandidate, d2b_resource_runtime::PublicationRefusal> {
    // The committed publication's own Zone selects the coordinator, for the
    // same reason the fence did.
    let coordinator = publisher.coordinator(publication.zone.as_str())?;
    coordinator.open_session().await.map_err(transport)?;
    let expected = publisher.expected(publication.zone.as_str()).await?;
    let (rows, removed) =
        publication_rows(&publication.publication.rows, &publication.publication.removed)?;
    let digest = publication_candidate_digest(&rows, &removed);
    Ok(CommitCandidate {
        transaction: transaction_token(publication.transaction)?,
        store_incarnation: publisher.incarnation.clone(),
        expected,
        committed: AuthorityCursor { sequence: publication.sequence, digest },
        rows,
        removed,
    })
}

#[async_trait]
impl d2b_resource_runtime::AuthorityPublisher for CoordinatorPublisher {
    async fn prepare(
        &self,
        candidate: &d2b_resource_runtime::PublicationCandidate,
    ) -> Result<d2b_resource_runtime::FencedTransaction, d2b_resource_runtime::PublicationRefusal> {
        // The candidate's own Zone selects the coordinator, so a row staged
        // under the system Zone is fenced by the system Zone's coordinator
        // and never under the plane's own.
        let coordinator = self.coordinator(candidate.zone.as_str())?;
        coordinator.open_session().await.map_err(transport)?;
        let expected = self.expected(candidate.zone.as_str()).await?;
        if expected.sequence != candidate.expected {
            // The store reserved its candidate against a predecessor the
            // broker does not hold. Committing it would publish visibility
            // for a revision built on an authority the broker has moved past.
            return Err(d2b_resource_runtime::PublicationRefusal::Refused(format!(
                "zone {} publishes against sequence {} while the broker holds {}",
                candidate.zone,
                candidate.expected.get(),
                expected.sequence.get()
            )));
        }
        let (rows, removed) = staged_rows(&candidate.projection)?;
        let digest = publication_candidate_digest(&rows, &removed);
        let request = PrepareCandidate {
            transaction: transaction_token(candidate.transaction)?,
            store_incarnation: self.incarnation.clone(),
            expected,
            committed: AuthorityCursor {
                sequence: candidate.committed,
                digest: digest.clone(),
            },
            subject: self.subject.clone(),
            kind: mutation_kind(candidate.kind),
            candidate: rows,
            removed,
        };
        let prepared = coordinator.prepare(&request).await.map_err(transport)?;
        Ok(d2b_resource_runtime::FencedTransaction {
            transaction: candidate.transaction,
            prepared: prepared.transaction.to_string(),
            // The broker's own committed cursor is the fence's sequence; the
            // caller already reserved that sequence in the store, and a broker
            // that froze a different one is refusing by name rather than
            // answering a question this half did not ask.
            committed: prepared.committed.sequence,
        })
    }

    async fn commit(
        &self,
        publication: &d2b_resource_runtime::CommittedPublication,
    ) -> Result<d2b_resource_runtime::AcceptedRevision, d2b_resource_runtime::PublicationRefusal> {
        let commit = commit_candidate(self, publication).await?;
        let accepted = self
            .coordinator(publication.zone.as_str())?
            .commit(&commit)
            .await
            .map_err(transport)?;
        Ok(d2b_resource_runtime::AcceptedRevision {
            transaction: publication.transaction,
            sequence: accepted.sequence,
            candidate: accepted.digest,
        })
    }

    async fn adopt_committed(
        &self,
        publication: &d2b_resource_runtime::CommittedPublication,
    ) -> Result<d2b_resource_runtime::AcceptedRevision, d2b_resource_runtime::PublicationRefusal> {
        // The recovery entry point, for a fence a PREVIOUS process took. The
        // broker decides whether that fence is still there, and it re-checks
        // every fact the fence was written for before it installs anything.
        let commit = commit_candidate(self, publication).await?;
        let accepted = self
            .coordinator(publication.zone.as_str())?
            .adopt_commit(&commit)
            .await
            .map_err(transport)?;
        Ok(d2b_resource_runtime::AcceptedRevision {
            transaction: publication.transaction,
            sequence: accepted.sequence,
            candidate: accepted.digest,
        })
    }

    async fn accepted(&self) -> Result<ZoneDesiredSequence, d2b_resource_runtime::PublicationRefusal> {
        // The Zone a restarted manager reads before publishing is the Zone it
        // was bound to; with more than one Zone on this publisher it is
        // whichever coordinator holds the furthest-advanced accepted cursor,
        // because a manager resuming its own Zone must name that Zone's
        // predecessor and no other Zone's cursor is a substitute.
        let mut held: Option<AuthorityCursor> = None;
        for coordinator in self.coordinators.values() {
            let cursor = coordinator.accepted().await;
            if held.as_ref().is_none_or(|best| cursor.sequence > best.sequence) {
                held = Some(cursor);
            }
        }
        held.map(|cursor| cursor.sequence).ok_or_else(|| {
            d2b_resource_runtime::PublicationRefusal::Refused(
                "this publisher is bound to no Zone".to_owned(),
            )
        })
    }

    async fn resynchronize(
        &self,
        projection: &d2b_resource_runtime::ZoneProjection,
    ) -> Result<(), d2b_resource_runtime::PublicationRefusal> {
        // A resynchronization runs under this plane's own authenticated
        // origination leg, exactly as a publication does: the document restates
        // authority, and authority is never restated under an identity the
        // broker cannot attribute to the plane that owns it.
        let coordinator = self.coordinator(projection.zone.as_str())?;
        coordinator.open_session().await.map_err(transport)?;
        let transaction = transaction_token(projection.transaction)?;
        // The cursor the broker reports it holds is the fact the whole
        // reconciliation is proved against, and the store's own acknowledged
        // sequence is the claim about reaching it. The two disagreeing means
        // this store accepted a publication the broker never did, which no
        // document can reconcile: it is refused here, by name, rather than sent
        // as a projection the broker would have to prove.
        let held = coordinator.accepted().await;
        if held.sequence != projection.accepted.sequence {
            return Err(d2b_resource_runtime::PublicationRefusal::Refused(format!(
                "zone {} acknowledged sequence {} while the broker holds {}",
                projection.zone,
                projection.accepted.sequence.get(),
                held.sequence.get()
            )));
        }
        // The floor is the cursor the broker already accepted, which it must
        // not move below. The document then restates that same cursor with the
        // store's committed rows, and the broker checks every row of it
        // against the projection it already holds rather than installing the
        // claim.
        let document = resynchronization_document(projection, held.clone())?;
        coordinator
            .resynchronize(&transaction, held.clone(), held)
            .await
            .map_err(transport)?;
        coordinator
            .publish_snapshot(&transaction, &document)
            .await
            .map_err(transport)?;
        Ok(())
    }
}
