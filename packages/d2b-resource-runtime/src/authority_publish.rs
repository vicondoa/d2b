//! The broker half of the freeze / commit / publish / acknowledge order
//! (KTD6-KTD7).
//!
//! # Why the manager does not talk to a broker directly
//!
//! The store owns the durable transaction; the broker owns the admitted
//! projection; neither may stand in for the other. This module is the seam
//! between them: the manager drives the four durable steps and calls an
//! [`AuthorityPublisher`] for the two that cross the privileged boundary.
//! The daemon binds the publisher to its own authenticated origination leg;
//! nothing in this crate depends on the broker's wire types, so the ordering
//! stays testable without a broker and the store stays below the transport.
//!
//! # What each call is allowed to mean
//!
//! - [`AuthorityPublisher::prepare`] is the fence. It returns only after the
//!   broker has durably frozen this Zone's new-effect admission for the exact
//!   candidate it was given, and it names the prepared transaction identity
//!   the store then records. A manager that cannot name that identity has no
//!   fence, so it must not commit.
//! - [`AuthorityPublisher::commit`] advances the broker's projection to one
//!   exact committed state and answers with the revision it accepted. It is
//!   not revocation: a reducing change still owes release evidence, reported
//!   separately.
//!
//! There is no "abandon" call. A candidate the broker has fenced is never
//! dropped: recovery either replays the exact bytes the fence validated or
//! leaves the Zone fenced for explicit resynchronization, which is what makes
//! a held fence evidence rather than a lock this half forgot to unlock.
//!
//! Every call takes owned durable facts rather than a store handle, so no
//! SQLite transaction, store lock, or reservation guard can be alive across
//! the round trip.

use async_trait::async_trait;
use d2b_contracts_resource::v3::{DesiredDigest, StoreIncarnation, ZoneDesiredSequence};

use crate::authority_journal::{AcceptedCursor, CommittedPublication, Projection, PublishedRow};


use crate::identity::TransactionId;

/// The module declared name.
pub const MODULE_NAME: &str = "authority_publish";

/// What one staged candidate does to the rows it names.
///
/// The broker re-evaluates the candidate against its own accepted graph, so
/// this is a classification of the change, never permission to make it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationKind {
    /// The mutation creates the row it names.
    Create,
    /// The mutation rewrites a row that already exists, including an
    /// ownership, metadata, or provenance change that leaves the spec bytes
    /// identical.
    Update,
    /// The mutation retires the row, either by marking it deleting or by
    /// removing it.
    Delete,
}

/// One staged candidate, as the broker must validate it.
///
/// Every field is a durable fact the store already decided when it staged the
/// candidate, so nothing here is a handle into the store and nothing here
/// needs a lock to stay valid across the round trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationCandidate {
    /// The exact staged transaction identity.
    pub transaction: TransactionId,
    /// The Zone the candidate belongs to.
    pub zone: String,
    /// The store generation the candidate was staged in.
    pub incarnation: StoreIncarnation,
    /// The last revision the broker is known to have accepted for the Zone.
    pub expected: ZoneDesiredSequence,
    /// The sequence this candidate commits at.
    pub committed: ZoneDesiredSequence,
    /// What the candidate does to the rows it names.
    pub kind: MutationKind,
    /// The exact committed state this candidate installs.
    pub projection: Projection,
}

/// The prepared rows and retired keys one candidate installs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationRows {
    pub rows: Vec<PublishedRow>,
    pub removed: Vec<crate::spec_store::ResourceKey>,
}

impl PublicationRows {
    /// The rows and retirements a store projection publishes.
    pub fn of(projection: &Projection) -> Self {
        Self {
            rows: projection.rows.iter().map(PublishedRow::of).collect(),
            removed: projection.removed.iter().map(|row| row.key.clone()).collect(),
        }
    }
}

/// One Zone's durable projection, as the broker must be able to rebuild it.
///
/// A resynchronization is the one message that restates authority the broker
/// already holds, so this projection has to be COMPLETE rather than
/// illustrative: the cursor the Zone's last acknowledgement reached, every
/// committed desired row it publishes with the revision and digest it
/// committed at, the relationship identity each binding row folds in, and the
/// transaction this store still owes an outcome for. A broker proves the
/// document it receives against its own accepted rows, so a projection that
/// restates a row with different bytes, or carries an authority row the broker
/// never accepted, is refused by name rather than believed.
///
/// It is built from the store's committed rows and never from a manager's
/// in-memory state, so a restarted daemon presents the same projection its
/// previous boot published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneProjection {
    /// The Zone this projection describes.
    pub zone: String,
    /// The store generation this projection was taken in.
    pub incarnation: StoreIncarnation,
    /// The cursor the Zone's last acknowledged publication reached. A Zone that
    /// has published nothing is at the initial cursor, whose digest is the
    /// digest of the empty canonical byte string.
    pub accepted: AcceptedCursor,
    /// Every committed desired row this Zone publishes, with the relationship
    /// identity a binding row's key folds in.
    pub rows: Vec<PublishedRow>,
    /// The transaction this store still owes an outcome for, when one does.
    /// It travels with the reconciliation so a broker that durably froze it
    /// carries the fence forward instead of clearing it.
    pub outstanding: Option<TransactionId>,
    /// The identity this reconciliation is carried under.
    ///
    /// It is derived from the accepted cursor rather than minted per attempt,
    /// so a retry after a failed reconciliation re-presents the same identity
    /// and the broker recognises the replay instead of stacking a second one.
    pub transaction: TransactionId,
}

impl ZoneProjection {
    /// The cursor a Zone that has acknowledged no publication is at.
    ///
    /// Sequence zero has no committed bytes behind it, so its digest is the
    /// digest of the empty canonical byte string - the same value the broker
    /// derives for its own initial cursor.
    pub fn initial_cursor(zone: &str, incarnation: StoreIncarnation) -> AcceptedCursor {
        AcceptedCursor {
            zone: zone.to_owned(),
            incarnation,
            sequence: ZoneDesiredSequence::INITIAL,
            digest: DesiredDigest::of(&[]),
            accepted_at: 0,
        }
    }
}

/// What the broker durably froze for one candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FencedTransaction {
    /// The exact transaction identity both sides now hold.
    pub transaction: TransactionId,
    /// The broker's prepared transaction identity for this candidate, which
    /// the store records before the desired rows commit.
    pub prepared: String,
    /// The sequence the candidate will commit at.
    pub committed: ZoneDesiredSequence,
}

/// The revision the broker accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedRevision {
    pub transaction: TransactionId,
    pub sequence: ZoneDesiredSequence,
    pub candidate: DesiredDigest,
}

/// Why one publication round trip did not produce a fence or an acceptance.
///
/// Nothing here is retried silently: a refusal leaves the Zone fenced, and the
/// manager reports it rather than committing against a fence that is not
/// there.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PublicationRefusal {
    /// The broker, or the transport to it, refused or could not answer.
    #[error("authority publication refused: {0}")]
    Refused(String),
    /// A transaction the broker is not holding.
    ///
    /// This is the answer for an unknown or superseded transaction: the
    /// manager may not publish visibility for facts the broker does not hold.
    #[error("authority publication names transaction {transaction}, which the broker is not holding")]
    UnknownTransaction { transaction: TransactionId },
    /// The broker holds this exact fence already, so the candidate replays
    /// rather than stacking a second transaction under one fence.
    #[error("authority publication replayed transaction {transaction} under its held fence")]
    Replayed { transaction: TransactionId },
}

/// The broker half of one Zone's authority publication.
///
/// Implementations are `Send + Sync` and shared: the manager owns one per
/// Zone, and the round trips happen on the caller's task with no store or
/// manager guard held across them.
#[async_trait]
pub trait AuthorityPublisher: Send + Sync + std::fmt::Debug {
    /// Durably freeze this Zone's new-effect admission for one candidate.
    async fn prepare(
        &self,
        candidate: &PublicationCandidate,
    ) -> Result<FencedTransaction, PublicationRefusal>;

    /// Advance the broker's admitted projection to one exact committed state.
    async fn commit(
        &self,
        publication: &CommittedPublication,
    ) -> Result<AcceptedRevision, PublicationRefusal>;

    /// The Zone cursor the broker currently holds.
    ///
    /// A restarted manager reads this before it publishes anything, so it
    /// never claims a predecessor the broker does not hold.
    async fn accepted(&self) -> Result<ZoneDesiredSequence, PublicationRefusal>;

    /// Reconcile the broker's projection for one Zone with its accepted state.
    ///
    /// This admits nothing: the broker has already durably accepted a
    /// projection for this Zone and will not serve one it cannot prove against
    /// that. The implementation declares the intent together with the accepted
    /// lower bound the broker must not move below, then transfers `projection`
    /// as one bounded document so the broker can check every row against the
    /// projection it holds. It returns only once the Zone is serving again; a
    /// refusal leaves the Zone exactly as fenced as it was.
    ///
    /// Both crash directions are safe without any extra protocol. A daemon that
    /// dies mid-transfer leaves the broker in a transfer-in-progress posture
    /// that admits no ordinary message, and its next start declares the same
    /// reconciliation again from the same durable store. A broker that dies
    /// mid-transfer drops the reassembly buffer and moves every Zone back to
    /// reconciling on its next start - the state this call begins from anyway
    /// - so the whole document is simply transferred again.
    async fn resynchronize(
        &self,
        projection: &ZoneProjection,
    ) -> Result<(), PublicationRefusal>;
}
/// One durable authority mutation's outcome, as the caller reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishOutcome {
    /// The Zone's desired state advanced and the broker accepted the exact
    /// revision this transaction committed.
    Committed {
        publication: crate::authority_journal::CommittedPublication,
        /// Whether this mutation created the row it wrote. Staging decides it,
        /// so a rewrite that lands on the same generation is never mistaken
        /// for the creation that produced it.
        created: bool,
    },
    /// The candidate was byte-identical to the committed desired state, so
    /// nothing advanced: no revision was reserved for publication and the
    /// Zone was never fenced.
    Unchanged(crate::authority_journal::DesiredRow),
}

impl PublishOutcome {
    /// The committed row, or `None` when the mutation retired one.
    pub fn row(&self) -> Option<&crate::spec_store::StoredDesiredResource> {
        match self {
            Self::Committed { publication, .. } => {
                publication.publication.rows.first().map(|row| &row.row)
            }
            Self::Unchanged(row) => Some(&row.row),
        }
    }

    /// The manager's read of a committed ensure.
    ///
    /// `None` when the mutation retired a row instead of writing one: an
    /// ensure always projects exactly one row, so this is the caller asking
    /// whether it holds the answer it came for rather than a second
    /// rendering of the same commit.
    pub fn ensure(self) -> Option<crate::spec_store::EnsureOutcome> {
        let row = self.row()?.clone();
        Some(match &self {
            Self::Unchanged(_) => crate::spec_store::EnsureOutcome::Unchanged(row),
            Self::Committed { created: true, .. } => crate::spec_store::EnsureOutcome::Created(row),
            Self::Committed { .. } => crate::spec_store::EnsureOutcome::Updated(row),
        })
    }
}

/// Why one durable authority mutation did not commit.
#[derive(Debug, thiserror::Error)]
pub enum PublishError {
    /// The store refused the mutation itself, or its own durable record.
    #[error(transparent)]
    Store(#[from] crate::spec_store::SpecStoreError),
    /// The broker half refused, or could not answer. The Zone is left fenced
    /// exactly as the refusal says.
    #[error(transparent)]
    Refused(#[from] PublicationRefusal),
    /// The store committed something this publisher cannot name, which is a
    /// broken protocol rather than a refusal of the mutation.
    #[error("{0}")]
    Inconsistent(String),
}

/// One durable authority mutation, in the only order the protocol permits
/// (KTD6).
///
/// This is the store's only write path: the per-Zone manager and the
/// foundation seed both drive this exact order, so neither can commit a
/// desired row the broker never fenced, and neither can commit one the broker
/// has not accepted.
///
/// 0. recover - whatever an earlier publication for this Zone left outstanding
///    is resolved by the recovery table, so this candidate never has to queue
///    behind a transaction nothing else will release;
/// 1. stage - the candidate and its reserved Zone sequence become durable;
/// 2. prepare - the broker durably freezes this Zone's new-effect admission
///    for the exact bytes staging decided, and names its prepared identity;
/// 3. record the prepared identity, then commit - the desired rows, their
///    revisions, the audit record, and the outbox entry, in one transaction;
/// 4. publish - the broker advances its projection to that exact committed
///    state and answers with the revision it accepted;
/// 5. acknowledge - the store settles the transaction, drops its outbox
///    entry, and moves the accepted cursor.
///
/// Each store call is its own complete transaction and no caller guard is held
/// across the broker round trip, so a failure at any step leaves a state the
/// recovery table can name rather than a half-published authority change.
/// Step 0 is what makes that naming reachable while the daemon runs and not
/// only at its next start: a candidate that reached no fence is released, one
/// that did is replayed exactly as it committed, and one recovery cannot
/// resolve keeps the Zone fenced and refuses this mutation with that refusal.
pub async fn publish(
    store: &crate::spec_store::SpecStore,
    mutation: crate::authority_journal::DesiredMutation,
    authority: &dyn AuthorityPublisher,
) -> Result<PublishOutcome, PublishError> {
    use crate::authority_journal::AcceptedPublication;
    use crate::authority_journal::CommitOutcome;

    let zone = mutation.zone().to_owned();
    // One Zone has at most one outstanding transaction, so a publication that
    // failed at any step after staging holds the Zone's single slot until
    // something resolves it. Only a restart did, which turned one failed
    // publication into a Zone whose every later mutation is refused by the
    // leaked transaction's identity - the store's one-outstanding rule working
    // exactly as declared, with no path back. The recovery table already
    // answers what such a transaction owes, and running it here is the same
    // resolution a restart runs, over the same durable facts, before this
    // candidate stages anything. A transaction recovery cannot resolve still
    // fences the Zone: this propagates that refusal and stages nothing.
    adopt_outstanding(store, &zone, authority).await?;
    let expected = store
        .accepted_cursor(&zone)
        .await?
        .map_or(ZoneDesiredSequence::INITIAL, |cursor| cursor.sequence);
    let staged = store.stage_mutation(mutation).await?;
    let Some(projection) = staged.projection.clone() else {
        // The candidate changes nothing: there is no revision to fence and
        // none to publish, so the transaction settles without ever freezing
        // the Zone.
        return match store.commit_mutation(staged.transaction).await? {
            CommitOutcome::Unchanged { row, .. } => Ok(PublishOutcome::Unchanged(row)),
            other => Err(PublishError::Inconsistent(format!(
                "a staged candidate that projected no change committed {other:?}"
            ))),
        };
    };
    let created =
        projection.rows.first().is_some_and(|row| row.generation_before.is_none());
    let fenced = authority
        .prepare(&PublicationCandidate {
            transaction: staged.transaction,
            zone: staged.zone.clone(),
            incarnation: staged.incarnation.clone(),
            expected,
            committed: staged.sequence,
            kind: mutation_kind(&projection),
            projection,
        })
        .await?;
    if fenced.transaction != staged.transaction {
        // A fence the manager cannot name is not a fence for this candidate.
        return Err(PublishError::Refused(PublicationRefusal::UnknownTransaction {
            transaction: fenced.transaction,
        }));
    }
    store.record_prepared(staged.transaction, &fenced.prepared).await?;
    let committed = match store.commit_mutation(staged.transaction).await? {
        CommitOutcome::Committed(committed) | CommitOutcome::AlreadyCommitted(committed) => {
            committed
        }
        CommitOutcome::Unchanged { .. } => {
            return Err(PublishError::Inconsistent(
                "a fenced candidate committed nothing it had projected".to_owned(),
            ));
        }
    };
    let accepted = authority.commit(&committed).await?;
    store
        .acknowledge(AcceptedPublication {
            transaction: committed.transaction,
            zone: committed.zone.clone(),
            incarnation: committed.incarnation.clone(),
            sequence: committed.sequence,
            candidate: committed.candidate.clone(),
        })
        .await?;
    if accepted.transaction != committed.transaction || accepted.sequence != committed.sequence {
        return Err(PublishError::Refused(PublicationRefusal::UnknownTransaction {
            transaction: accepted.transaction,
        }));
    }
    Ok(PublishOutcome::Committed { publication: committed, created })
}

/// Adopt, or explicitly refuse, every transaction `zone` still owes an
/// outcome for.
///
/// This is the one recovery path for a Zone, and it is a function of the
/// Zone rather than of a writer: the manager calls it for the Zone it
/// manages, and a plane calls it for the Zone it is about to write before
/// that write stages anything. A restart therefore never stages a candidate
/// while the previous boot's transaction is still outstanding, which is what
/// the one-outstanding-transaction-per-Zone rule would otherwise turn into a
/// permanent refusal.
///
/// Nothing here cleans up against authority the broker has not accepted: a
/// staged candidate that committed nothing is released, and a committed
/// candidate whose acknowledgment was lost is republished exactly as it
/// committed. An outstanding transaction that cannot be resolved keeps the
/// Zone fenced and refuses the start.
pub async fn adopt_outstanding(
    store: &crate::spec_store::SpecStore,
    zone: &str,
    authority: &dyn AuthorityPublisher,
) -> Result<(), PublishError> {
    use crate::authority_journal::{CommitOutcome, TransactionRecovery};

    let recovery = store.zone_recovery(zone).await?;
    for (_, decision) in recovery.transactions {
        match decision {
            TransactionRecovery::ResumeOrDiscard { transaction } => {
                store.cancel_transaction(transaction.transaction).await?;
            }
            TransactionRecovery::ReplayOrCancel { transaction } => {
                // The broker holds a fence for a candidate that committed
                // nothing. The store still holds the exact projection that
                // fence was validated against, so recovery replays it rather
                // than releasing the fence: an abandoned fence would be a
                // lock this half could forget to unlock.
                let replayed = store.commit_mutation(transaction.transaction).await?;
                let committed = match replayed {
                    CommitOutcome::Committed(committed)
                    | CommitOutcome::AlreadyCommitted(committed) => committed,
                    CommitOutcome::Unchanged { .. } => {
                        return Err(PublishError::Inconsistent(format!(
                            "publication transaction {} replayed a candidate that commits \
                             nothing",
                            transaction.transaction
                        )));
                    }
                };
                let accepted = authority.commit(&committed).await?;
                store
                    .acknowledge(crate::authority_journal::AcceptedPublication {
                        transaction: committed.transaction,
                        zone: committed.zone.clone(),
                        incarnation: committed.incarnation.clone(),
                        sequence: committed.sequence,
                        candidate: committed.candidate.clone(),
                    })
                    .await?;
                if accepted.sequence != committed.sequence {
                    return Err(PublishError::Refused(PublicationRefusal::UnknownTransaction {
                        transaction: accepted.transaction,
                    }));
                }
            }
            TransactionRecovery::ReplayCommit { transaction, publication } => {
                let committed = CommittedPublication {
                    transaction: transaction.transaction,
                    zone: transaction.zone.clone(),
                    incarnation: transaction.incarnation.clone(),
                    sequence: transaction.sequence,
                    candidate: transaction.candidate.clone(),
                    publication,
                };
                let accepted = authority.commit(&committed).await?;
                store
                    .acknowledge(crate::authority_journal::AcceptedPublication {
                        transaction: committed.transaction,
                        zone: committed.zone.clone(),
                        incarnation: committed.incarnation.clone(),
                        sequence: committed.sequence,
                        candidate: committed.candidate.clone(),
                    })
                    .await?;
                if accepted.sequence != committed.sequence {
                    return Err(PublishError::Refused(PublicationRefusal::UnknownTransaction {
                        transaction: accepted.transaction,
                    }));
                }
            }
        }
    }
    Ok(())
}

/// What a projected mutation does to the rows it names.
fn mutation_kind(projection: &Projection) -> MutationKind {
    match (projection.rows.first(), projection.removed.first()) {
        (_, Some(_)) => MutationKind::Delete,
        (Some(row), None) if row.generation_before.is_none() => MutationKind::Create,
        _ => MutationKind::Update,
    }
}

/// Reconcile the broker's projection for `zone` with the state this store
/// durably holds.
///
/// This runs after [`adopt_outstanding`] and before the Zone reads a row. The
/// order is the point of the sequence: adoption FIRST, because a previous
/// boot's transaction has to settle before the broker is told what the Zone
/// holds - a reconciliation carries the Zone's accepted cursor, and a
/// transaction that still owed a publication would be acknowledging past the
/// cursor the broker is being asked to accept. Reconciliation SECOND, and
/// before any row is loaded or any actor spawned, because a manager that acted
/// on a row before the broker's projection was reconciled would be acting on
/// authority the broker has not confirmed it still holds.
///
/// A refusal here refuses the boot rather than degrading it: the manager cannot
/// know what the broker serves until the broker has been shown, so it declines
/// to start on unconfirmed authority.
pub async fn resynchronize(
    store: &crate::spec_store::SpecStore,
    zone: &str,
    authority: &dyn AuthorityPublisher,
) -> Result<(), PublishError> {
    let projection = store.zone_projection(zone).await?;
    authority
        .resynchronize(&projection)
        .await
        .map_err(PublishError::from)
}
