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
    ControlActionRequest, EffectExitRequest, EndSnapshotRequest,
    MAX_PUBLICATION_CHUNK_BYTES, OpenPublicationSessionResponse, PrepareChangeRequest,
    PreparedTransaction, PublicationControlKind, PublicationMutationKind, PublicationSession,
    PublicationTransactionId, ReleaseEffectRequest, ResynchronizeRequest, RevocationConvergence,
    SnapshotChunkRequest, publication_candidate_digest, publication_snapshot_bytes,
    publication_snapshot_digest,
};
use d2b_contracts_resource::v3::{
    AdmissionStage, AuthoritySubject, DesiredDigest, RefusalReason, StoreIncarnation,
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
            } => write!(
                f,
                "authority publication refused: {code} at {stage:?} ({reason:?}), fenced={fenced}"
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
    async fn open_session(
        &self,
        open: AuthorityPublicationOpen,
    ) -> Result<OpenPublicationSessionResponse, String>;

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
    ) -> Result<OpenPublicationSessionResponse, String> {
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
            serde_json::from_slice::<OpenPublicationSessionResponse>(&body)
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
        if reply.limits.max_chunk_bytes == 0 || reply.limits.max_chunks == 0 {
            // A broker that declares no bounds is refusing to be bounded, which
            // the plan does not permit: the manager will not stream into it.
            return Err(PublicationError::NoSession);
        }
        *self.accepted.lock().await = reply.binding.accepted.clone();
        *self.session.lock().await = Some(reply.session.clone());
        Ok(reply.session)
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
                pending: pending.transaction,
            });
        };
        if accepted.transaction != commit.transaction
            || accepted.sequence != commit.committed.sequence
            || accepted.digest != commit.digest()
        {
            return Err(PublicationError::UnmatchedCompletion {
                completion: accepted.transaction,
                pending: pending.transaction,
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
