//! SQLite spec store: the durable desired-state store for the v3 resource
//! runtime (plan unit U2, KTD2/KTD12).
//!
//! ## Store shape
//!
//! One dedicated **blocking writer thread** owns the sole
//! [`rusqlite::Connection`] (Send, never Sync, never shared across actors).
//! The async [`SpecStore`] surface is a thin handle: every call sends a
//! request over a bounded std `mpsc` channel to the writer thread and awaits
//! the reply through a `tokio::sync::oneshot`. This is the dedicated-thread
//! variant of the message-passing surface allowed by KTD2: SQLite calls never
//! run inside an async context or an actor mailbox (KTD12), writers serialize
//! structurally on the single connection, and `busy_timeout` covers the
//! remaining cross-connection case (two stores open on one file, e.g. during
//! handover).
//!
//! Admission is refuse-don't-queue (the loader_worker doctrine): a full
//! 256-slot queue refuses with [`SpecStoreError::Busy`] - backpressure, the
//! writer is alive and a retry succeeds - while a closed channel refuses
//! with [`SpecStoreError::WriterGone`] - terminal, the store must be
//! reopened. The reply await is unbounded once admitted (a deadline cannot
//! preempt the serial writer); a writer panic drops the reply sender and the
//! await ends with WriterGone.
//!
//! ## Durability posture
//!
//! WAL mode, `busy_timeout` 5s, `synchronous=NORMAL`, and **IMMEDIATE
//! transactions**: every durable mutation (ensure / mark-deleting / remove)
//! and its audit-log record commit inside one `BEGIN IMMEDIATE` transaction,
//! and the async call returns only after that commit (commit-before-return,
//! R7/R10, AE1). File posture: the store creates its file with mode 0600
//! (directory 0700 when it creates the directory); the database's
//! `<name>-wal` and `<name>-shm` side files are tightened to 0600 as well.
//!
//! ## Store formats
//!
//! [`StoreFormat`] names the two formats this module opens.
//! [`Self::open`] keeps the production desired-row schema and does not move;
//! [`Self::open_authority_journal`] opens the authority-journal format
//! (U5, KTD5-KTD6), which persists a per-row desired revision, a per-Zone
//! desired sequence, durable publication transactions, their outbox, and the
//! accepted-publication cursor.
//!
//! The two formats never mix writes. A production-format store refuses every
//! journal operation and an authority-journal store refuses the direct
//! desired-row mutations, because a durable authority change that could be
//! written outside the journal protocol is exactly the change KTD6 requires to
//! be staged, fenced, published, and acknowledged. There is no migration
//! between them: opening one format's database as the other is refused by
//! [`crate::schema::apply_authority_journal`].

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::mpsc::sync_channel;
use std::time::Duration;

use d2b_contracts_resource::v3::authority::{StoreIncarnation, ZoneDesiredSequence};
use rusqlite::{Connection, OptionalExtension, params};
use tokio::sync::oneshot;

use crate::authority_journal::{
    AcceptedCursor, AcceptedPublication, CommitOutcome, DesiredMutation, DesiredRow,
    PublicationTransaction, StagedMutation, ZoneRecovery,
};
use crate::identity::TransactionId;
use crate::schema::StoreFormat;

pub const MODULE_NAME: &str = "spec_store";

/// The production format's projection. It stores no desired revision, so the
/// trailing column is an explicit `NULL`: a production row can never be
/// mistaken for a row whose revision was read back.
pub(crate) const LEGACY_ROW_COLUMNS: &str = "zone, type, name, uid, generation, owner_uid, \
     provenance, deleting, spec, metadata, created_at, NULL AS desired_revision";

/// The authority-journal format's projection, which carries the durable
/// desired revision the format exists to persist.
pub(crate) const AUTHORITY_ROW_COLUMNS: &str = "zone, type, name, uid, generation, owner_uid, \
     provenance, deleting, spec, metadata, created_at, desired_revision";

/// The projection the store's own format reads rows through.
fn row_columns(format: StoreFormat) -> &'static str {
    match format {
        StoreFormat::DesiredRows => LEGACY_ROW_COLUMNS,
        StoreFormat::AuthorityJournal => AUTHORITY_ROW_COLUMNS,
    }
}

/// Deadline one writer request waits on the busy connection before failing.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Durable identity of one resource: unique within `(zone, type, name)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResourceKey {
    pub zone: String,
    pub type_name: String,
    pub name: String,
}

impl ResourceKey {
    /// Build a key from its three owned components.
    pub fn new(zone: impl Into<String>, type_name: impl Into<String>, name: impl Into<String>) -> Self {
        Self { zone: zone.into(), type_name: type_name.into(), name: name.into() }
    }
}

/// Where a desired resource came from. Persisted as the `provenance` column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceProvenance {
    Nix,
    Api,
    Resource,
}

impl ResourceProvenance {
    /// The database spelling this provenance persists as.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Nix =>"nix",
            Self::Api =>"api",
            Self::Resource =>"resource",
        }
    }
}

impl std::str::FromStr for ResourceProvenance {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "nix" => Ok(Self::Nix),
            "api" => Ok(Self::Api),
            "resource" => Ok(Self::Resource),
            _ => Err(()),
        }
    }
}

/// One persisted desired-resource row: the full resource envelope minus
/// status (R6). `spec` and `metadata` are opaque encoded envelopes owned by
/// the callers (metadata carries finalizers, annotations, owner reference,
/// creation timestamp); the store never interprets them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredDesiredResource {
    pub key: ResourceKey,
    /// 16-byte stable identity; survives generation changes.
    pub uid: [u8; 16],
    pub generation: u64,
    /// Owner resource uid, when this resource is an owned child (R8).
    pub owner_uid: Option<[u8; 16]>,
    pub provenance: ResourceProvenance,
    /// Terminal desired state: set by [`SpecStore::mark_deleting`], cleared
    /// only by removal (R10).
    pub deleting: bool,
    pub spec: Vec<u8>,
    pub metadata: Vec<u8>,
    pub created_at: i64,
}

/// Filter for [`SpecStore::list`]. Absent fields are wildcards.
#[derive(Debug, Clone, Default)]
pub struct SpecSelector {
    pub zone: Option<String>,
    pub type_name: Option<String>,
    pub owner_uid: Option<[u8; 16]>,
}

/// Outcome of [`SpecStore::ensure`] (R7 idempotence).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnsureOutcome {
    /// Row absent: committed at generation 1 before this value returned.
    Created(StoredDesiredResource),
    /// Same resource, every committed column identical: no-op, current
    /// handle returned. A row whose spec bytes match but whose metadata or
    /// owner binding differ is **not** unchanged - it is [`Self::Updated`],
    /// or the incoming columns would be silently discarded.
    Unchanged(StoredDesiredResource),
    /// The committed row changed before this value returned: either a new
    /// spec (which advances the generation) or differing
    /// metadata/owner_uid/provenance written without advancing it.
    Updated(StoredDesiredResource),
}

impl EnsureOutcome {
    pub fn row(&self) -> &StoredDesiredResource {
        match self {
            Self::Created(row) | Self::Unchanged(row) | Self::Updated(row) => row,
        }
    }
}

/// One committed-mutation audit record (durable-mutation audit; inserted in
/// the same transaction as the mutation it describes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRecord {
    pub id: i64,
    pub ts: i64,
    pub subject: String,
    pub provenance: String,
    pub resource_zone: Option<String>,
    pub resource_type: Option<String>,
    pub resource_name: Option<String>,
    pub operation: String,
    pub generation_before: Option<i64>,
    pub generation_after: Option<i64>,
    pub detail: Option<Vec<u8>>,
}

#[derive(Debug, thiserror::Error)]
pub enum SpecStoreError {
    /// `ensure` against a row whose `deleting` mark is set. Deleting is
    /// terminal desired state until cleanup removes the row (R10).
    #[error("resource {zone}/{type_name}/{name} is marked deleting; ensure rejected")]
    ResourceDeleting { zone: String, type_name: String, name: String },
    #[error("resource {zone}/{type_name}/{name} not found")]
    NotFound { zone: String, type_name: String, name: String },
    /// A stored row whose uid column is not the 16-byte identity this store
    /// writes. The row is corrupt: admitting it with a zero identity would
    /// collide with every other zero-uid row, so the call fails instead.
    #[error("corrupt stored row {zone}/{type_name}/{name}: uid column is not 16 bytes")]
    CorruptRow { zone: String, type_name: String, name: String },
    #[error("spec store io: {0}")]
    Io(#[from] std::io::Error),
    #[error("spec store sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("spec store migration: {0}")]
    Migration(String),
    /// The writer's bounded queue (256 slots) is full: the call was not
    /// admitted. This is backpressure, never writer death - the writer is
    /// alive and the call can be retried. Per-caller handling: the manager
    /// surfaces it as [`crate::error::ResourceError::Store`] and the daemon
    /// plane surfaces it as its own store error; both treat it as retryable
    /// (a later admission succeeds once the queue drains), unlike
    /// [`Self::WriterGone`], which is terminal and requires reopening the
    /// store.
    #[error("spec store writer busy (queue full)")]
    Busy,
    /// Writer thread gone (store handle raced past shutdown, or the writer
    /// panicked). Terminal: the store must be reopened. Never produced by a
    /// full queue - a full queue is [`Self::Busy`].
    #[error("spec store writer unavailable")]
    WriterGone,
    /// The opened store's format cannot serve the requested operation: the
    /// journal protocol is absent from the production format, and the direct
    /// desired-row mutations are absent from the journal format, so no
    /// durable authority change can be written outside the protocol.
    #[error("spec store format {actual:?} cannot serve this operation, which requires {required:?}")]
    WrongStoreFormat { actual: StoreFormat, required: StoreFormat },
    /// The authority-journal schema could not be applied. The refusal case
    /// carries the database's own `user_version`: this release starts from a
    /// fresh store and never converts existing data.
    #[error(transparent)]
    Schema(#[from] crate::schema::SchemaError),
    /// A publication transaction is not recorded in this store.
    #[error("publication transaction {transaction} is not recorded in this store")]
    TransactionNotFound { transaction: TransactionId },
    /// A publication transaction exists but not in a state that permits the
    /// requested transition.
    #[error("publication transaction {transaction} is {state}; {transition} is not permitted")]
    TransactionStateConflict {
        transaction: TransactionId,
        state: &'static str,
        transition: &'static str,
    },
    /// The Zone already has an outstanding transaction, so another authority
    /// mutation must queue behind it rather than overtake its fence.
    #[error("zone {zone} already has publication transaction {transaction} outstanding")]
    ZoneTransactionOutstanding { zone: String, transaction: TransactionId },
    /// An acknowledgment names facts this store never committed. Publishing
    /// visibility for them is refused rather than accepted.
    #[error("publication transaction {transaction} was acknowledged for facts this store did not commit")]
    PublicationMismatch { transaction: TransactionId },
    /// The Zone's durable desired sequence cannot advance. The counter fails
    /// closed instead of wrapping into a sequence that looks older than what
    /// it replaced.
    #[error("zone {zone} desired sequence is exhausted; no further desired mutation can be ordered")]
    ZoneSequenceExhausted { zone: String },
    /// One row's spec generation cannot advance.
    #[error("spec generation for {zone}/{type_name}/{name} is exhausted")]
    GenerationExhausted { zone: String, type_name: String, name: String },
    /// One row's durable desired revision cannot advance.
    #[error("desired revision for {zone}/{type_name}/{name} is exhausted")]
    RowRevisionExhausted { zone: String, type_name: String, name: String },
    /// The accepted cursor cannot move to the sequence being acknowledged.
    #[error("publication transaction {transaction} commits sequence {committed}, which the accepted cursor at {acknowledged} does not accept")]
    AcceptedSequenceConflict {
        transaction: TransactionId,
        committed: u64,
        acknowledged: String,
    },
    /// A stored counter column is not a usable counter, or a journal record
    /// cannot be decoded. Authority ordered against a counter the store
    /// cannot account for has no meaning, so the read fails instead of
    /// folding it into an unrelated revision.
    #[error("spec store durable counter for {zone} is not a usable counter")]
    CorruptCounter { zone: String },
    #[error("spec store journal record is not readable: {detail}")]
    JournalCorrupt { transaction: TransactionId, detail: &'static str },
    #[error("spec store journal payload is not a canonical encoding: {detail}")]
    CorruptJournalPayload { detail: &'static str },
}

// ---------------------------------------------------------------------------
// Request plumbing
// ---------------------------------------------------------------------------

enum Request {
    Ensure {
        row: StoredDesiredResource,
        reply: oneshot::Sender<Result<EnsureOutcome, SpecStoreError>>,
    },
    Get {
        key: ResourceKey,
        reply: oneshot::Sender<Result<Option<StoredDesiredResource>, SpecStoreError>>,
    },
    List {
        selector: SpecSelector,
        reply: oneshot::Sender<Result<Vec<StoredDesiredResource>, SpecStoreError>>,
    },
    MarkDeleting {
        key: ResourceKey,
        reply: oneshot::Sender<Result<StoredDesiredResource, SpecStoreError>>,
    },
    RemoveAfterCleanup {
        key: ResourceKey,
        reply: oneshot::Sender<Result<(), SpecStoreError>>,
    },
    History {
        limit: usize,
        reply: oneshot::Sender<Result<Vec<AuditRecord>, SpecStoreError>>,
    },
    Migrate {
        reply: oneshot::Sender<Result<(), SpecStoreError>>,
    },
    StageMutation {
        mutation: DesiredMutation,
        reply: oneshot::Sender<Result<StagedMutation, SpecStoreError>>,
    },
    RecordPrepared {
        transaction: TransactionId,
        prepared: String,
        reply: oneshot::Sender<Result<PublicationTransaction, SpecStoreError>>,
    },
    CommitMutation {
        transaction: TransactionId,
        reply: oneshot::Sender<Result<CommitOutcome, SpecStoreError>>,
    },
    Acknowledge {
        accepted: AcceptedPublication,
        reply: oneshot::Sender<Result<AcceptedCursor, SpecStoreError>>,
    },
    CancelTransaction {
        transaction: TransactionId,
        reply: oneshot::Sender<Result<PublicationTransaction, SpecStoreError>>,
    },
    DesiredRow {
        key: ResourceKey,
        reply: oneshot::Sender<Result<DesiredRow, SpecStoreError>>,
    },
    DesiredRows {
        selector: SpecSelector,
        reply: oneshot::Sender<Result<Vec<DesiredRow>, SpecStoreError>>,
    },
    ZoneSequence {
        zone: String,
        reply: oneshot::Sender<Result<ZoneDesiredSequence, SpecStoreError>>,
    },
    AcceptedCursor {
        zone: String,
        reply: oneshot::Sender<Result<Option<AcceptedCursor>, SpecStoreError>>,
    },
    ZoneRecovery {
        zone: String,
        reply: oneshot::Sender<Result<ZoneRecovery, SpecStoreError>>,
    },
    StoreIncarnation {
        reply: oneshot::Sender<Result<StoreIncarnation, SpecStoreError>>,
    },
}

/// Run an operation only the authority-journal format provides.
///
/// The refusal is the enforcement point for KTD6: a production-format store
/// has no publication journal, so it cannot stage, fence, publish, or
/// acknowledge an authority change.
fn journal_only<T>(
    format: StoreFormat,
    conn: &mut Connection,
    run: impl FnOnce(&mut Connection) -> Result<T, SpecStoreError>,
) -> Result<T, SpecStoreError> {
    match format {
        StoreFormat::AuthorityJournal => run(conn),
        StoreFormat::DesiredRows => Err(SpecStoreError::WrongStoreFormat {
            actual: format,
            required: StoreFormat::AuthorityJournal,
        }),
    }
}

/// Run an operation only the production format provides.
///
/// The mirror image of [`journal_only`]: an authority-journal store has no
/// direct desired-row mutation, because a durable authority change that could
/// commit without an outbox entry and an accepted-publication acknowledgment
/// would leave the broker's projection permanently behind the store.
fn desired_rows_only<T>(
    format: StoreFormat,
    conn: &mut Connection,
    run: impl FnOnce(&mut Connection) -> Result<T, SpecStoreError>,
) -> Result<T, SpecStoreError> {
    match format {
        StoreFormat::DesiredRows => run(conn),
        StoreFormat::AuthorityJournal => Err(SpecStoreError::WrongStoreFormat {
            actual: format,
            required: StoreFormat::DesiredRows,
        }),
    }
}

/// The writer thread's exclusive connection owner. All SQLite happens here.
///
/// The blocking `recv` is the sanctioned bounded-worker channel boundary
/// (plan R4): the writer is a dedicated thread, never an executor worker, so
/// the blocking recv parks only the writer's own thread. The reply travels
/// back over a `tokio::sync::oneshot`, exactly the loader_worker shape.
#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
fn writer_loop(mut conn: Connection, format: StoreFormat, requests: Receiver<Request>) {
    while let Ok(request) = requests.recv() {
        match request {
            Request::Ensure { row, reply } => {
                let result = desired_rows_only(format, &mut conn, |conn| {
                    ensure_transactional(conn, row, format)
                });
                let _ = reply.send(result);
            }
            Request::Get { key, reply } => {
                let _ = reply.send(get(&conn, &key, format).map(Some));
            }
            Request::List { selector, reply } => {
                let _ = reply.send(list(&conn, &selector, format));
            }
            Request::MarkDeleting { key, reply } => {
                let result = desired_rows_only(format, &mut conn, |conn| {
                    mark_deleting_transactional(conn, &key, format)
                });
                let _ = reply.send(result);
            }
            Request::RemoveAfterCleanup { key, reply } => {
                let result = desired_rows_only(format, &mut conn, |conn| {
                    remove_after_cleanup(conn, &key, format)
                });
                let _ = reply.send(result);
            }
            Request::History { limit, reply } => {
                let _ = reply.send(history(&conn, limit));
            }
            Request::Migrate { reply } => {
                let result = desired_rows_only(format, &mut conn, |conn| {
                    crate::schema::migrate(conn)
                        .map_err(|err| SpecStoreError::Migration(err.to_string()))
                });
                let _ = reply.send(result);
            }
            Request::StageMutation { mutation, reply } => {
                let result = journal_only(format, &mut conn, |conn| {
                    crate::authority_journal::stage_mutation(conn, mutation)
                });
                let _ = reply.send(result);
            }
            Request::RecordPrepared { transaction, prepared, reply } => {
                let result = journal_only(format, &mut conn, |conn| {
                    crate::authority_journal::record_prepared(conn, transaction, &prepared)
                });
                let _ = reply.send(result);
            }
            Request::CommitMutation { transaction, reply } => {
                let result = journal_only(format, &mut conn, |conn| {
                    crate::authority_journal::commit_mutation(conn, transaction)
                });
                let _ = reply.send(result);
            }
            Request::Acknowledge { accepted, reply } => {
                let result = journal_only(format, &mut conn, |conn| {
                    crate::authority_journal::acknowledge(conn, accepted)
                });
                let _ = reply.send(result);
            }
            Request::CancelTransaction { transaction, reply } => {
                let result = journal_only(format, &mut conn, |conn| {
                    crate::authority_journal::cancel_transaction(conn, transaction)
                });
                let _ = reply.send(result);
            }
            Request::DesiredRow { key, reply } => {
                let result = journal_only(format, &mut conn, |conn| {
                    crate::authority_journal::desired_row(conn, &key)
                });
                let _ = reply.send(result);
            }
            Request::DesiredRows { selector, reply } => {
                let result = journal_only(format, &mut conn, |conn| {
                    crate::authority_journal::desired_rows(conn, &selector)
                });
                let _ = reply.send(result);
            }
            Request::ZoneSequence { zone, reply } => {
                let result = journal_only(format, &mut conn, |conn| {
                    crate::authority_journal::zone_sequence(conn, &zone)
                });
                let _ = reply.send(result);
            }
            Request::AcceptedCursor { zone, reply } => {
                let result = journal_only(format, &mut conn, |conn| {
                    crate::authority_journal::accepted_cursor(conn, &zone)
                });
                let _ = reply.send(result);
            }
            Request::ZoneRecovery { zone, reply } => {
                let result = journal_only(format, &mut conn, |conn| {
                    crate::authority_journal::zone_recovery(conn, &zone)
                });
                let _ = reply.send(result);
            }
            Request::StoreIncarnation { reply } => {
                let result = journal_only(format, &mut conn, |conn| {
                    crate::authority_journal::store_incarnation(conn)
                });
                let _ = reply.send(result);
            }
        }
    }
    // Channel closed: the last handle dropped. Checkpoint and close cleanly.
    let _ = conn.pragma_update(None, "wal_checkpoint(TRUNCATE)", 0);
}

// ---------------------------------------------------------------------------
fn ensure_transactional(
    conn: &mut Connection,
    row: StoredDesiredResource,
    format: StoreFormat,
) -> Result<EnsureOutcome, SpecStoreError> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let outcome = ensure_transactional_inner(&tx, row, format)?;
    tx.commit()?;
    Ok(outcome)
}

/// One `resources` row as the ensure comparison reads it (spec section 9):
/// generation, deleting mark, spec and metadata bytes, owner uid, provenance.
/// An alias because the column tuple would otherwise exceed clippy's type
/// complexity ceiling at its only two type sites.
type ExistingRow = (u64, bool, Vec<u8>, Vec<u8>, Option<Vec<u8>>, String);

fn ensure_transactional_inner(
    tx: &rusqlite::Transaction<'_>,
    row: StoredDesiredResource,
    format: StoreFormat,
) -> Result<EnsureOutcome, SpecStoreError> {
    // Every column the row's readers observe is compared, not only the spec:
    // a `metadata`-only ensure (the authored envelope the display status and
    // the owned-child annotations read) must never be a silent no-op.
    let existing: Option<ExistingRow> = tx
        .query_row(
            "SELECT generation, deleting, spec, metadata, owner_uid, provenance FROM resources \
             WHERE zone = ?1 AND type = ?2 AND name = ?3",
            params![row.key.zone, row.key.type_name, row.key.name],
            |r| {
                Ok((
                    r.get::<_, i64>(0)? as u64,
                    r.get::<_, i64>(1)? != 0,
                    r.get::<_, Vec<u8>>(2)?,
                    r.get::<_, Vec<u8>>(3)?,
                    r.get::<_, Option<Vec<u8>>>(4)?,
                    r.get::<_, String>(5)?,
                ))
            },
        )
        .optional()?;
    let Some((
        generation,
        deleting,
        existing_spec,
        existing_metadata,
        existing_owner,
        existing_provenance,
    )) = existing
    else {
        let stored = insert_new(tx, row)?;
        insert_audit(
            tx,
            AuditWrite {
                ts: now(),
                subject: "resource.ensure",
                provenance: stored.provenance.as_str(),
                key: Some(&stored.key),
                operation: "ensure.create",
                generation_before: None,
                generation_after: Some(stored.generation as i64),
            },
        )?;
        return Ok(EnsureOutcome::Created(stored));
    };
    if deleting {
        return Err(SpecStoreError::ResourceDeleting {
            zone: row.key.zone.clone(),
            type_name: row.key.type_name.clone(),
            name: row.key.name.clone(),
        });
    }
    let incoming_owner = row.owner_uid.map(|uid| uid.to_vec());
    let spec_changed = existing_spec != row.spec;
    if !spec_changed
        && existing_metadata == row.metadata
        && existing_owner == incoming_owner
        && existing_provenance == row.provenance.as_str()
    {
        let stored =
            load_row(tx, &row.key, format)?.expect("row present within its own transaction");
        return Ok(EnsureOutcome::Unchanged(stored));
    }
    if !spec_changed {
        // Spec bytes identical: the row's authored metadata, owner binding,
        // and provenance still move to the incoming values, at the committed
        // generation (identity and generation are the spec's contract; the
        // annotation columns are not).
        tx.execute(
            "UPDATE resources SET owner_uid = ?4, provenance = ?5, metadata = ?6 \
             WHERE zone = ?1 AND type = ?2 AND name = ?3",
            params![
                row.key.zone,
                row.key.type_name,
                row.key.name,
                incoming_owner,
                row.provenance.as_str(),
                row.metadata,
            ],
        )?;
        insert_audit(
            tx,
            AuditWrite {
                ts: now(),
                subject: "resource.ensure",
                provenance: row.provenance.as_str(),
                key: Some(&row.key),
                operation: "ensure.metadata",
                generation_before: Some(generation as i64),
                generation_after: Some(generation as i64),
            },
        )?;
        let stored =
            load_row(tx, &row.key, format)?.expect("row present within its own transaction");
        return Ok(EnsureOutcome::Updated(stored));
    }
    let next = generation + 1;
    tx.execute(
        "UPDATE resources SET generation = ?4, uid = ?5, owner_uid = ?6, provenance = ?7, \
         spec = ?8, metadata = ?9 WHERE zone = ?1 AND type = ?2 AND name = ?3",
        params![
            row.key.zone,
            row.key.type_name,
            row.key.name,
            next as i64,
            row.uid.as_slice(),
            incoming_owner,
            row.provenance.as_str(),
            row.spec,
            row.metadata,
        ],
    )?;
    insert_audit(
        tx,
        AuditWrite {
            ts: now(),
            subject: "resource.ensure",
            provenance: row.provenance.as_str(),
            key: Some(&row.key),
            operation: "ensure.update",
            generation_before: Some(generation as i64),
            generation_after: Some(next as i64),
        },
    )?;
    let stored = load_row(tx, &row.key, format)?.expect("row present within its own transaction");
    Ok(EnsureOutcome::Updated(stored))
}

// ---------------------------------------------------------------------------
// Connection setup
// ---------------------------------------------------------------------------

/// The store's open half is synchronous by construction (rusqlite has no
/// async form) and runs either on the dedicated worker's setup path or, in
/// production, on d2bd's bounded loader seat (plan KTD2); the posture
/// chmods are best-effort and bounded.
#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
fn tighten_file_modes(path: &Path) {
    // Best-effort posture enforcement: the store file and its WAL/SHM side
    // files carry the daemon's private-data mode (0600). SQLite names the
    // side files by appending `-wal`/`-shm` to the *database file name*, so
    // the suffix is appended here too - `with_extension` would rewrite the
    // real suffix (`spec-store.sqlite3` -> `spec-store.db-wal`) and leave
    // the files SQLite actually created at their creation mode. Side files
    // only exist while a connection holds the database open in WAL mode.
    let tighten = |p: &Path| {
        if let Ok(file) = std::fs::File::open(p) {
            let _ = file.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600));
        }
    };
    let side_file = |suffix: &str| {
        path.file_name().map(|name| {
            let mut name = name.to_os_string();
            name.push(suffix);
            path.with_file_name(name)
        })
    };
    tighten(path);
    if let Some(wal) = side_file("-wal") {
        tighten(&wal);
    }
    if let Some(shm) = side_file("-shm") {
        tighten(&shm);
    }
}

/// Same synchronous-open rationale as [`tighten_file_modes`]: the directory
/// create/chmod is the dedicated worker's setup half (the production caller
/// pre-creates the parent with `tokio::fs` and runs this on the bounded
/// loader seat, so the create is a no-op there).
#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
fn open_connection(path: &Path) -> Result<Connection, SpecStoreError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
        // Only when we create it: private directory for private data.
        let _ = std::fs::set_permissions(parent, std::os::unix::fs::PermissionsExt::from_mode(0o700));
    }
    let conn = Connection::open(path)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(conn)
}

// ---------------------------------------------------------------------------
// Transactional operations (run on the writer thread)
// ---------------------------------------------------------------------------

fn begin_immediate(conn: &mut Connection) -> Result<(), SpecStoreError> {
    Ok(conn.execute_batch("BEGIN IMMEDIATE")?)
}

/// One audit-log write committed with the transaction (spec section 9): a
/// subject, the row provenance, the optional resource key, the operation, and
/// the generation transition the operation recorded. Grouped so the SQL write
/// below stays under clippy's argument ceiling.
pub(crate) struct AuditWrite<'a> {
    pub(crate) ts: i64,
    pub(crate) subject: &'a str,
    pub(crate) provenance: &'a str,
    pub(crate) key: Option<&'a ResourceKey>,
    pub(crate) operation: &'a str,
    pub(crate) generation_before: Option<i64>,
    pub(crate) generation_after: Option<i64>,
}

pub(crate) fn insert_audit(
    conn: &Connection,
    entry: AuditWrite<'_>,
) -> Result<(), SpecStoreError> {
    conn.execute(
        "INSERT INTO audit_log (ts, subject, provenance, resource_zone, resource_type, \
         resource_name, operation, generation_before, generation_after, detail) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL)",
        params![
            entry.ts,
            entry.subject,
            entry.provenance,
            entry.key.map(|k| k.zone.as_str()),
            entry.key.map(|k| k.type_name.as_str()),
            entry.key.map(|k| k.name.as_str()),
            entry.operation,
            entry.generation_before,
            entry.generation_after,
        ],
    )?;
    Ok(())
}

pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn insert_new(
    tx: &rusqlite::Transaction<'_>,
    row: StoredDesiredResource,
) -> Result<StoredDesiredResource, SpecStoreError> {
    let StoredDesiredResource {
        key,
        uid,
        owner_uid,
        provenance,
        spec,
        metadata,
        ..
    } = row;
    let created_at = now();
    tx.execute(
        "INSERT INTO resources (zone, type, name, uid, generation, owner_uid, provenance, \
         deleting, spec, metadata, created_at) \
         VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, 0, ?7, ?8, ?9)",
        params![
            key.zone,
            key.type_name,
            key.name,
            uid.as_slice(),
            owner_uid.map(|u| u.to_vec()),
            provenance.as_str(),
            spec,
            metadata,
            created_at,
        ],
    )?;
    Ok(StoredDesiredResource {
        generation: 1,
        deleting: false,
        created_at,
        key,
        uid,
        owner_uid,
        provenance,
        spec,
        metadata,
    })
}

/// One desired row read as plain SQLite types.
///
/// Reading the columns and decoding them are separate steps so a decode
/// failure keeps the store's own typed error instead of being flattened into
/// the one error slot `rusqlite` offers a row closure.
pub(crate) type DesiredRowTuple = (
    String,
    String,
    String,
    Vec<u8>,
    i64,
    Option<Vec<u8>>,
    String,
    i64,
    Vec<u8>,
    Vec<u8>,
    i64,
    Option<i64>,
);

/// Read one desired row in [`ROW_COLUMNS`] order plus the trailing revision
/// column the projection names.
pub(crate) fn desired_row_tuple(r: &rusqlite::Row<'_>) -> rusqlite::Result<DesiredRowTuple> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
        r.get(9)?,
        r.get(10)?,
        r.get(11)?,
    ))
}

/// Decode one [`DesiredRowTuple`], leaving the revision column raw.
///
/// The revision stays raw here: decoding it is the authority-journal
/// format's business, and it refuses a column that is not a usable counter
/// rather than coercing it.
pub(crate) fn decode_desired_row(
    values: DesiredRowTuple,
) -> Result<(StoredDesiredResource, Option<i64>), SpecStoreError> {
    let (
        zone,
        type_name,
        name,
        uid,
        generation,
        owner_uid,
        provenance,
        deleting,
        spec,
        metadata,
        created_at,
        revision,
    ): DesiredRowTuple = values;
    let corrupt = || SpecStoreError::CorruptRow {
        zone: zone.clone(),
        type_name: type_name.clone(),
        name: name.clone(),
    };
    let uid: [u8; 16] = uid.try_into().map_err(|_| corrupt())?;
    let owner_uid: Option<[u8; 16]> = owner_uid
        .map(|value| value.try_into().map_err(|_| corrupt()))
        .transpose()?;
    let row = StoredDesiredResource {
        key: ResourceKey {
            zone,
            type_name,
            name,
        },
        uid,
        generation: generation as u64,
        owner_uid,
        provenance: provenance.parse().unwrap_or(ResourceProvenance::Api),
        deleting: deleting != 0,
        spec,
        metadata,
        created_at,
    };
    Ok((row, revision))
}

/// The production format's decode: the same row, without a revision.
fn row_from(r: &rusqlite::Row<'_>) -> Result<StoredDesiredResource, SpecStoreError> {
    decode_desired_row(desired_row_tuple(r)?).map(|(row, _)| row)
}

fn load_row(
    conn: &Connection,
    key: &ResourceKey,
    format: StoreFormat,
) -> Result<Option<StoredDesiredResource>, SpecStoreError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM resources WHERE zone = ?1 AND type = ?2 AND name = ?3",
        row_columns(format)
    ))?;
    let mut rows = stmt.query(params![key.zone, key.type_name, key.name])?;
    match rows.next()? {
        Some(row) => Ok(Some(row_from(row)?)),
        None => Ok(None),
    }
}

fn get(
    conn: &Connection,
    key: &ResourceKey,
    format: StoreFormat,
) -> Result<StoredDesiredResource, SpecStoreError> {
    load_row(conn, key, format)?
        .ok_or_else(|| SpecStoreError::NotFound {
            zone: key.zone.clone(),
            type_name: key.type_name.clone(),
            name: key.name.clone(),
        })
}

fn list(
    conn: &Connection,
    selector: &SpecSelector,
    format: StoreFormat,
) -> Result<Vec<StoredDesiredResource>, SpecStoreError> {
    let sql = format!(
        "SELECT {} FROM resources \
         WHERE (?1 IS NULL OR zone = ?1) \
           AND (?2 IS NULL OR type = ?2) \
           AND (?3 IS NULL OR owner_uid = ?3) \
         ORDER BY zone, type, name",
        row_columns(format)
    );
    let zone = selector.zone.as_deref();
    let type_name = selector.type_name.as_deref();
    let owner = selector.owner_uid.map(|u| u.to_vec());
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(params![zone, type_name, owner])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(row_from(row)?);
    }
    Ok(out)
}

/// Set the terminal deleting mark (R10). Fails with
/// [`SpecStoreError::NotFound`] when absent. Mutation + audit commit in one
/// IMMEDIATE transaction before the caller sees the result.
fn mark_deleting_transactional(
    conn: &mut Connection,
    key: &ResourceKey,
    format: StoreFormat,
) -> Result<StoredDesiredResource, SpecStoreError> {
    begin_immediate(conn)?;
    let result = (|| {
        let Some(before) = load_row(conn, key, format)? else {
            return Err(SpecStoreError::NotFound {
                zone: key.zone.clone(),
                type_name: key.type_name.clone(),
                name: key.name.clone(),
            });
        };
        if !before.deleting {
            conn.execute(
                "UPDATE resources SET deleting = 1 WHERE zone = ?1 AND type = ?2 AND name = ?3",
                params![key.zone, key.type_name, key.name],
            )?;
            insert_audit(
                conn,
                AuditWrite {
                    ts: now(),
                    subject: "resource.deletion",
                    provenance: before.provenance.as_str(),
                    key: Some(key),
                    operation: "deletion.mark",
                    generation_before: Some(before.generation as i64),
                    generation_after: Some(before.generation as i64),
                },
            )?;
        }
        Ok(load_row(conn, key, format)?.expect("row present within its own transaction"))
    })();
    match result {
        Ok(row) => {
            conn.execute_batch("COMMIT")?;
            Ok(row)
        }
        Err(err) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(err)
        }
    }
}

/// Remove a row after cleanup completed (R10: the deleting mark, not this
/// call, gates cleanup). Mutation + audit commit in one IMMEDIATE transaction.
fn remove_after_cleanup(
    conn: &mut Connection,
    key: &ResourceKey,
    format: StoreFormat,
) -> Result<(), SpecStoreError> {
    begin_immediate(conn)?;
    let result = (|| {
        let Some(existing) = load_row(conn, key, format)? else {
            return Err(SpecStoreError::NotFound {
                zone: key.zone.clone(),
                type_name: key.type_name.clone(),
                name: key.name.clone(),
            });
        };
        conn.execute(
            "DELETE FROM resources WHERE zone = ?1 AND type = ?2 AND name = ?3",
            params![key.zone, key.type_name, key.name],
        )?;
        insert_audit(
            conn,
            AuditWrite {
                ts: now(),
                subject: "resource.deletion",
                provenance: existing.provenance.as_str(),
                key: Some(key),
                operation: "deletion.removed",
                generation_before: Some(existing.generation as i64),
                generation_after: None,
            },
        )?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            conn.execute_batch("COMMIT")?;
            Ok(())
        }
        Err(err) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(err)
        }
    }
}

fn history(conn: &Connection, limit: usize) -> Result<Vec<AuditRecord>, SpecStoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, ts, subject, provenance, resource_zone, resource_type, resource_name, \
         operation, generation_before, generation_after, detail FROM audit_log \
         ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = stmt
        .query_map(
            params![limit as i64],
            |r| {
                Ok(AuditRecord {
                    id: r.get(0)?,
                    ts: r.get(1)?,
                    subject: r.get(2)?,
                    provenance: r.get(3)?,
                    resource_zone: r.get(4)?,
                    resource_type: r.get(5)?,
                    resource_name: r.get(6)?,
                    operation: r.get(7)?,
                    generation_before: r.get(8)?,
                    generation_after: r.get(9)?,
                    detail: r.get(10)?,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Async handle
// ---------------------------------------------------------------------------

/// Handle over the single-writer spec store. Cheap to clone? No: one handle
/// owns the writer channel; drop closes the writer thread after draining
/// pending requests. Callers needing multi-task access wrap this in `Arc`.
pub struct SpecStore {
    path: PathBuf,
    format: StoreFormat,
    sender: Option<SyncSender<Request>>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl SpecStore {
    /// Open (creating when absent) the store at `path`, apply pending
    /// migrations, and start the writer thread. Returns after migrations are
    /// committed.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, SpecStoreError> {
        Self::open_with_bound(path, 256)
    }

    /// [`Self::open`] with an explicit writer-queue bound. The production
    /// bound is 256; tests shrink it to inject queue-full backpressure.
    fn open_with_bound(path: impl Into<PathBuf>, bound: usize) -> Result<Self, SpecStoreError> {
        Self::open_formatted(path, bound, StoreFormat::DesiredRows)
    }

    /// Open (creating when absent) an authority-journal store: the format
    /// that persists a per-row desired revision, a per-Zone desired sequence,
    /// durable publication transactions, their outbox, and the
    /// accepted-publication cursor (KTD5-KTD6).
    ///
    /// Creating it applies [`crate::schema::apply_authority_journal`], which
    /// refuses an existing production-format database instead of converting
    /// it: the clean break starts from a fresh store.
    pub fn open_authority_journal(path: impl Into<PathBuf>) -> Result<Self, SpecStoreError> {
        Self::open_formatted(path, 256, StoreFormat::AuthorityJournal)
    }

    fn open_formatted(
        path: impl Into<PathBuf>,
        bound: usize,
        format: StoreFormat,
    ) -> Result<Self, SpecStoreError> {
        let path = path.into();
        let mut conn = open_connection(&path)?;
        match format {
            StoreFormat::DesiredRows => {
                crate::schema::migrate(&mut conn)
                    .map_err(|err| SpecStoreError::Migration(err.to_string()))?;
            }
            StoreFormat::AuthorityJournal => {
                crate::schema::apply_authority_journal(&mut conn)?;
            }
        }
        tighten_file_modes(&path);
        let (sender, receiver) = sync_channel::<Request>(bound);
        let join = std::thread::Builder::new()
            .name("spec-store-writer".into())
            .spawn(move || writer_loop(conn, format, receiver))
            .map_err(|err| SpecStoreError::Io(std::io::Error::other(err.to_string())))?;
        Ok(Self { path, format, sender: Some(sender), join: Some(join) })
    }

    /// The format this store was opened with.
    pub fn format(&self) -> StoreFormat {
        self.format
    }

    /// The database file path this store opened.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Idempotent desired-state write (R7). Rejects ensure against a
    /// deleting row. Returns only after the commit.
    pub async fn ensure(&self, row: StoredDesiredResource) -> Result<EnsureOutcome, SpecStoreError> {
        self.call(|reply| Request::Ensure { row, reply }).await
    }

    pub async fn get(&self, key: ResourceKey) -> Result<StoredDesiredResource, SpecStoreError> {
        match self.call(|reply| Request::Get { key: key.clone(), reply }).await? {
            Some(row) => Ok(row),
            None => Err(SpecStoreError::NotFound {
                zone: key.zone,
                type_name: key.type_name,
                name: key.name,
            }),
        }
    }

    pub async fn list(&self, selector: SpecSelector) -> Result<Vec<StoredDesiredResource>, SpecStoreError> {
        self.call(|reply| Request::List { selector, reply }).await
    }

    /// Commit the terminal deleting mark before cleanup starts (R10).
    pub async fn mark_deleting(&self, key: ResourceKey) -> Result<StoredDesiredResource, SpecStoreError> {
        self.call(|reply| Request::MarkDeleting { key, reply }).await
    }

    /// Remove the row once cleanup completed. There is no API surface for
    /// writing status: the store only persists the envelope minus status (R6,
    /// AE6 at unit scale).
    pub async fn remove_after_cleanup(&self, key: ResourceKey) -> Result<(), SpecStoreError> {
        self.call(|reply| Request::RemoveAfterCleanup { key, reply }).await
    }

    /// Newest-first tail of committed-mutation audit records.
    pub async fn history(&self, limit: usize) -> Result<Vec<AuditRecord>, SpecStoreError> {
        self.call(move |reply| Request::History { limit, reply }).await
    }

    /// Apply pending schema migrations explicitly (normally already applied
    /// by [`Self::open`]); idempotent.
    pub async fn migrate(&self) -> Result<(), SpecStoreError> {
        self.call(|reply| Request::Migrate { reply }).await.map(|_| ())
    }

    // -----------------------------------------------------------------
    // Authority-journal protocol (KTD5-KTD6)
    //
    // Every call below commits before it returns, and every one of them is a
    // complete transaction: the caller's broker I/O happens strictly between
    // calls, so no open SQLite transaction, store lock, or source-reservation
    // lock can cross the transport wait.
    // -----------------------------------------------------------------

    /// The store generation this database is. A commit or acknowledgment
    /// naming a different incarnation names a different store, so ordinary
    /// acceptance can never install one.
    pub async fn store_incarnation(&self) -> Result<StoreIncarnation, SpecStoreError> {
        self.call(|reply| Request::StoreIncarnation { reply }).await
    }

    /// The Zone's last committed desired sequence, or
    /// [`ZoneDesiredSequence::INITIAL`] when the Zone has committed none.
    pub async fn zone_sequence(
        &self,
        zone: &str,
    ) -> Result<ZoneDesiredSequence, SpecStoreError> {
        let zone = zone.to_owned();
        self.call(|reply| Request::ZoneSequence { zone, reply }).await
    }

    /// One committed desired row with the revision it committed at.
    ///
    /// Runtime status has no surface here and cannot appear in the answer:
    /// the store persists the desired envelope only.
    pub async fn desired_row(&self, key: ResourceKey) -> Result<DesiredRow, SpecStoreError> {
        self.call(|reply| Request::DesiredRow { key, reply }).await
    }

    /// Every committed desired row the selector matches, with revisions.
    pub async fn desired_rows(
        &self,
        selector: SpecSelector,
    ) -> Result<Vec<DesiredRow>, SpecStoreError> {
        self.call(|reply| Request::DesiredRows { selector, reply }).await
    }

    /// Stage one desired mutation and reserve its Zone sequence.
    ///
    /// The candidate is persisted here, before any broker I/O, and the
    /// mutation is refused while another transaction for the Zone is
    /// outstanding: authority mutations queue behind the pending transaction
    /// rather than overtaking its fence.
    pub async fn stage_mutation(
        &self,
        mutation: DesiredMutation,
    ) -> Result<StagedMutation, SpecStoreError> {
        self.call(|reply| Request::StageMutation { mutation, reply }).await
    }

    /// Record the broker's prepared transaction identity for a staged
    /// candidate, which is the point at which the Zone's new-effect
    /// admission is durably frozen.
    pub async fn record_prepared(
        &self,
        transaction: TransactionId,
        prepared: &str,
    ) -> Result<PublicationTransaction, SpecStoreError> {
        let prepared = prepared.to_owned();
        self.call(|reply| Request::RecordPrepared { transaction, prepared, reply }).await
    }

    /// Commit the staged candidate: desired rows, per-row revisions, the
    /// audit record, the outbox entry, and the Zone sequence in one
    /// transaction.
    ///
    /// Replaying a committed transaction returns the recorded publication
    /// instead of applying the mutation a second time.
    pub async fn commit_mutation(
        &self,
        transaction: TransactionId,
    ) -> Result<CommitOutcome, SpecStoreError> {
        self.call(|reply| Request::CommitMutation { transaction, reply }).await
    }

    /// Record the broker's accepted revision: settle the transaction, drop
    /// its outbox entry, and move the accepted cursor. A repeated
    /// acknowledgment of the same facts returns the recorded cursor.
    pub async fn acknowledge(
        &self,
        accepted: AcceptedPublication,
    ) -> Result<AcceptedCursor, SpecStoreError> {
        self.call(|reply| Request::Acknowledge { accepted, reply }).await
    }

    /// Abandon a staged or prepared transaction that committed no desired
    /// row. Cancelling after the desired commit is refused.
    pub async fn cancel_transaction(
        &self,
        transaction: TransactionId,
    ) -> Result<PublicationTransaction, SpecStoreError> {
        self.call(|reply| Request::CancelTransaction { transaction, reply }).await
    }

    /// The Zone's last accepted revision, if the broker has accepted one.
    pub async fn accepted_cursor(
        &self,
        zone: &str,
    ) -> Result<Option<AcceptedCursor>, SpecStoreError> {
        let zone = zone.to_owned();
        self.call(|reply| Request::AcceptedCursor { zone, reply }).await
    }

    /// Everything one Zone owes after a restart, with the explicit recovery
    /// decision for each outstanding transaction.
    pub async fn zone_recovery(&self, zone: &str) -> Result<ZoneRecovery, SpecStoreError> {
        let zone = zone.to_owned();
        self.call(|reply| Request::ZoneRecovery { zone, reply }).await
    }

    async fn call<R, F>(&self, make: F) -> Result<R, SpecStoreError>
    where
        F: FnOnce(oneshot::Sender<Result<R, SpecStoreError>>) -> Request,
    {
        let (tx, rx) = oneshot::channel();
        // The sender is `None` only while the store is being dropped (the
        // writer already drained and exited): a call racing the teardown is
        // the terminal writer-gone case, not a panic.
        let Some(sender) = self.sender.as_ref() else {
            return Err(SpecStoreError::WriterGone);
        };
        // Admission is refuse-don't-queue (the loader_worker doctrine): a
        // full 256-slot queue refuses with `Busy` (backpressure, retryable)
        // and a closed channel refuses with `WriterGone` (terminal). The
        // two were previously collapsed into WriterGone, so saturation
        // presented as phantom writer death.
        sender.try_send(make(tx)).map_err(|error| match error {
            std::sync::mpsc::TrySendError::Full(_) => SpecStoreError::Busy,
            std::sync::mpsc::TrySendError::Disconnected(_) => SpecStoreError::WriterGone,
        })?;
        // The reply await is deliberately unbounded, matching loader_worker:
        // the writer is a serial worker, so a deadline here cannot preempt a
        // request already running on it - it would only turn a slow-but-
        // progressing store into a spurious failure. The wait is bounded in
        // practice: the writer drains FIFO and every op is capped by
        // `busy_timeout` (5s). A writer panic drops the reply sender, so the
        // await ends with WriterGone instead of hanging on a dead worker.
        rx.await.map_err(|_| SpecStoreError::WriterGone)?
    }
}

/// Drop is synchronous by construction and has no async form: the writer
/// teardown (drain, checkpoint, exit) is the dedicated worker's own bounded
/// work, so the join parks only the caller until the worker finishes it.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
impl Drop for SpecStore {
    fn drop(&mut self) {
        // Drop the sender first so the writer drains pending requests,
        // checkpoints, and exits; then join.
        drop(self.sender.take());
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn row(key: &str, spec: &[u8]) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("host", "Volume", key),
            uid: uid_for(key),
            generation: 0,
            owner_uid: None,
            provenance: ResourceProvenance::Api,
            deleting: false,
            spec: spec.to_vec(),
            metadata: b"meta".to_vec(),
            created_at: 0,
        }
    }

    fn uid_for(name: &str) -> [u8; 16] {
        let mut uid = [0u8; 16];
        uid[..name.len().min(16)].copy_from_slice(&name.as_bytes()[..name.len().min(16)]);
        uid
    }

    fn open_in(dir: &TempDir) -> SpecStore {
        SpecStore::open(dir.path().join("specs.db")).expect("open")
    }

    /// Persist-then-reopen: rows, audit, and schema survive close/reopen and
    /// migrations apply idempotently.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn persist_then_reopen() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("specs.db");
        {
            let store = SpecStore::open(&path).unwrap();
            let outcome = store.ensure(row("data", b"spec-v1")).await.unwrap();
            assert!(matches!(outcome, EnsureOutcome::Created(_)));
        }
        // Reopen on the same file; run migrate() explicitly (idempotent).
        let store = SpecStore::open(&path).unwrap();
        store.migrate().await.unwrap();
        store.migrate().await.unwrap();
        let got = store.get(ResourceKey::new("host", "Volume", "data")).await.unwrap();
        assert_eq!(got.spec, b"spec-v1");
        assert_eq!(got.generation, 1);
        assert!(!got.deleting);
        let history = store.history(10).await.unwrap();
        assert!(history.iter().any(|rec| rec.operation == "ensure.create"));
    }

    /// A stored row whose uid column is not the 16-byte identity the store
    /// writes is corrupt: reads fail with the typed error instead of
    /// admitting a zero identity that collides with every other zero-uid
    /// row.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn a_corrupt_uid_column_fails_reads_with_a_typed_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("specs.db");
        {
            let store = SpecStore::open(&path).unwrap();
            store.ensure(row("data", b"spec-v1")).await.unwrap();
        }
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute(
            "UPDATE resources SET uid = ?1 WHERE name = 'data'",
            [vec![1u8, 2, 3]],
        )
        .unwrap();
        drop(conn);

        let store = SpecStore::open(&path).unwrap();
        let error = store
            .get(ResourceKey::new("host", "Volume", "data"))
            .await
            .expect_err("a corrupt uid column is not a zero identity");
        assert!(matches!(
            error,
            SpecStoreError::CorruptRow {
                zone,
                type_name,
                name,
            } if zone == "host" && type_name == "Volume" && name == "data"
        ));
        let error = store
            .list(SpecSelector::default())
            .await
            .expect_err("list surfaces the same corrupt row");
        assert!(matches!(error, SpecStoreError::CorruptRow { .. }));
    }

    /// Durability boundary (AE1): ensure returns only after the commit. The
    /// second store call observes the committed generation, proving the
    /// first call's write was durable before its Ok.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn ensure_returns_after_commit() {
        let dir = TempDir::new().unwrap();
        let store = open_in(&dir);
        let first = store.ensure(row("data", b"spec-v1")).await.unwrap().row().clone();
        assert_eq!(first.generation, 1);
        // A subsequent write in a *separate* transaction observes the commit.
        let second = store
            .ensure(row("data", b"spec-v2"))
            .await
            .unwrap()
            .row()
            .clone();
        assert_eq!(second.generation, 2, "changed spec advances exactly once per ensure");
        let read = store.get(first.key.clone()).await.unwrap();
        assert_eq!(read.generation, 2);
        assert_eq!(read.spec, b"spec-v2");
    }

    /// Idempotent Ensure (R7): equal spec keeps generation; changed spec
    /// advances exactly one generation per call.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn idempotent_ensure_generation() {
        let dir = TempDir::new().unwrap();
        let store = open_in(&dir);
        let created = store.ensure(row("data", b"spec-v1")).await.unwrap();
        let unchanged = store.ensure(row("data", b"spec-v1")).await.unwrap();
        assert!(matches!(unchanged, EnsureOutcome::Unchanged(_)));
        assert_eq!(unchanged.row().generation, created.row().generation);
        let updated = store.ensure(row("data", b"spec-v2")).await.unwrap();
        assert!(matches!(updated, EnsureOutcome::Updated(_)));
        assert_eq!(updated.row().generation, created.row().generation + 1);
    }

    /// Deleting mark survives a simulated crash: the connection is dropped
    /// without cleanup and the store reopens with `deleting` still set.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn deleting_survives_crash() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("specs.db");
        {
            let store = SpecStore::open(&path).unwrap();
            store.ensure(row("data", b"spec-v1")).await.unwrap();
            store.mark_deleting(ResourceKey::new("host", "Volume", "data")).await.unwrap();
            // Simulated crash: writer thread and connection torn down with no
            // graceful checkpoint (abandon the handle mid-flight).
            std::mem::forget(store);
        }
        let store = SpecStore::open(&path).unwrap();
        let got = store.get(ResourceKey::new("host", "Volume", "data")).await.unwrap();
        assert!(got.deleting, "deleting mark must survive crash");
    }

    /// Ensure against a deleting row is rejected with a typed error; the
    /// deleting mark stays terminal until removal (R10).
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn ensure_against_deleting_rejected() {
        let dir = TempDir::new().unwrap();
        let store = open_in(&dir);
        let key = ResourceKey::new("host", "Volume", "data");
        store.ensure(row("data", b"spec-v1")).await.unwrap();
        store.mark_deleting(key.clone()).await.unwrap();
        let err = store.ensure(row("data", b"spec-v2")).await.unwrap_err();
        assert!(matches!(err, SpecStoreError::ResourceDeleting { .. }), "got {err:?}");
        // After cleanup removes the row, ensure recreates it fresh.
        store.remove_after_cleanup(key.clone()).await.unwrap();
        assert!(matches!(
            store.ensure(row("data", b"spec-v2")).await.unwrap(),
            EnsureOutcome::Created(_)
        ));
    }

    /// Status-shaped writes have no API surface: nothing in this module's
    /// public API accepts or persists a status payload (R6, AE6). Compile-level
    /// assertion by exhaustiveness of the surface itself; repeated store
    /// calls produce no audit records beyond the mutations themselves.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn no_status_write_surface() {
        let dir = TempDir::new().unwrap();
        let store = open_in(&dir);
        store.ensure(row("data", b"spec-v1")).await.unwrap();
        // The only mutations are ensure/mark_deleting/remove_after_cleanup.
        // If status churn had a path, it would appear as audit records; it
        // cannot, because no request variant carries a status payload.
        let history = store.history(100).await.unwrap();
        assert!(history.iter().all(|rec| {
            matches!(
                rec.operation.as_str(),
                "ensure.create" | "ensure.update" | "ensure.metadata" | "deletion.mark"
                    | "deletion.removed"
            )
        }));
    }

    /// Concurrent writers serialize without SQLITE_BUSY surfacing: two
    /// independent store handles on the same file hammer ensure on different
    /// keys; `busy_timeout` + IMMEDIATE transactions absorb contention.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn concurrent_writers_serialize() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("specs.db");
        let a = SpecStore::open(&path).unwrap();
        let b = SpecStore::open(&path).unwrap();
        let a = std::sync::Arc::new(a);
        let b = std::sync::Arc::new(b);
        let stores = [a.clone(), b];
        let mut handles = Vec::new();
        for (store, name) in stores.into_iter().zip(["a", "b"]) {
            for i in 0..25u32 {
                let store = store.clone();
                let spec = format!("spec-{i}").into_bytes();
                let mut r = row(&format!("{name}-{i}"), &spec);
                r.provenance = ResourceProvenance::Nix;
                handles.push(tokio::spawn(async move {
                    store.ensure(r).await.expect("ensure must not surface SQLITE_BUSY");
                }));
            }
        }
        for handle in handles {
            handle.await.unwrap();
        }
        let rows = store_list_all(&a).await;
        assert_eq!(rows.len(), 50);
        // Every row committed exactly once at generation 1.
        assert!(rows.iter().all(|r| r.generation == 1));
        // Audit recorded one create per mutation, from either writer.
        let history = a.history(1000).await.unwrap();
        assert_eq!(history.iter().filter(|rec| rec.operation == "ensure.create").count(), 50);
    }

    async fn store_list_all(store: &SpecStore) -> Vec<StoredDesiredResource> {
        store.list(SpecSelector::default()).await.unwrap()
    }

    /// A metadata-only ensure is not a silent no-op (issue: the display
    /// status and owned-child annotations read the authored metadata): the
    /// incoming metadata/owner binding are written without advancing the
    /// committed generation, and a byte-identical ensure still returns
    /// `Unchanged`.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn metadata_only_ensure_writes_columns_without_a_generation() {
        let dir = TempDir::new().unwrap();
        let store = open_in(&dir);
        let key = ResourceKey::new("host", "Volume", "data");
        let created = store.ensure(row("data", b"spec-v1")).await.unwrap().row().clone();
        assert_eq!(created.generation, 1);

        // Same spec, new authored metadata: written at the same generation.
        let mut with_metadata = row("data", b"spec-v1");
        with_metadata.metadata = b"authored-2".to_vec();
        let updated = store.ensure(with_metadata.clone()).await.unwrap();
        assert!(
            matches!(updated, EnsureOutcome::Updated(_)),
            "a metadata-only ensure is a change, not Unchanged"
        );
        assert_eq!(
            updated.row().generation,
            created.generation,
            "the metadata-only change does not advance the generation"
        );
        assert_eq!(updated.row().metadata, b"authored-2");
        let read = store.get(key.clone()).await.unwrap();
        assert_eq!(read.metadata, b"authored-2", "the incoming metadata is durable");
        assert_eq!(read.spec, b"spec-v1", "the spec bytes are untouched");

        // Byte-identical ensure: unchanged (no spurious write).
        let unchanged = store.ensure(with_metadata).await.unwrap();
        assert!(matches!(unchanged, EnsureOutcome::Unchanged(_)));

        // Same spec, new owner binding: also a change, same generation.
        let mut with_owner = row("data", b"spec-v1");
        with_owner.metadata = b"authored-2".to_vec();
        with_owner.owner_uid = Some(uid_for("owner"));
        let owned = store.ensure(with_owner).await.unwrap();
        assert!(matches!(owned, EnsureOutcome::Updated(_)));
        assert_eq!(owned.row().generation, created.generation);
        assert_eq!(owned.row().owner_uid, Some(uid_for("owner")));
        assert_eq!(store.get(key).await.unwrap().owner_uid, Some(uid_for("owner")));
    }

    /// File posture (0600 store / WAL / SHM, 0700 dir) asserted after writes.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn file_mode_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("private").join("specs.db");
        let store = SpecStore::open(&path).unwrap();
        store.ensure(row("data", b"spec-v1")).await.unwrap();
        let mode = |p: PathBuf| async move {
            tokio::fs::metadata(&p)
                .await
                .map(|m| m.permissions().mode() & 0o777)
                .unwrap_or(u32::MAX)
        };
        assert_eq!(mode(path.clone()).await, 0o600, "store file mode");
        assert_eq!(mode(path.with_extension("db-wal")).await, 0o600, "wal mode");
        assert_eq!(mode(path.with_extension("db-shm")).await, 0o600, "shm mode");
        assert_eq!(mode(path.parent().unwrap().to_path_buf()).await, 0o700, "store dir mode");
    }

    /// The side files of the *actual* database path are the ones tightened
    /// (issue: `with_extension` rewrote the suffix for
    /// `*.<suffix-db>` names, so a `.sqlite3` store kept SQLite's creation
    /// mode on its real WAL/SHM while the module claimed 0600).
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn side_files_of_a_suffixed_database_are_tightened() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("zones").join("dev").join("spec-store.sqlite3");
        let store = SpecStore::open(&path).expect("open");
        store.ensure(row("data", b"spec-v1")).await.unwrap();
        let side = |suffix: &str| path.with_file_name(format!("spec-store.sqlite3{suffix}"));
        let chmod = |p: PathBuf, mode: u32| async move {
            let file = tokio::fs::File::open(&p).await.unwrap_or_else(|error| panic!("{p:?}: {error}"));
            file.set_permissions(PermissionsExt::from_mode(mode)).await.unwrap();
        };
        // The connection created these side files; force a world-readable
        // mode so the assertions prove the store tightened them, not that
        // the process umask happened to.
        chmod(side("-wal"), 0o644).await;
        chmod(side("-shm"), 0o644).await;
        // A second open on the same file re-runs `tighten_file_modes` while
        // the first connection keeps the side files alive.
        let reopened = SpecStore::open(&path).expect("reopen");
        let mode = |p: PathBuf| async move {
            tokio::fs::metadata(&p).await.unwrap().permissions().mode() & 0o777
        };
        assert_eq!(mode(side("-wal")).await, 0o600, "the real wal is tightened");
        assert_eq!(mode(side("-shm")).await, 0o600, "the real shm is tightened");
        drop(reopened);
        drop(store);
    }

    /// List honors selector filters (zone / type / owner).
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn list_by_selector() {
        let dir = TempDir::new().unwrap();
        let store = open_in(&dir);
        let mut child = row("child", b"s");
        child.owner_uid = Some(uid_for("data"));
        store.ensure(row("data", b"s")).await.unwrap();
        store.ensure(child).await.unwrap();
        let all = store.list(SpecSelector::default()).await.unwrap();
        assert_eq!(all.len(), 2);
        let by_owner = store
            .list(SpecSelector { owner_uid: Some(uid_for("data")), ..Default::default() })
            .await
            .unwrap();
        assert_eq!(by_owner.len(), 1);
        assert_eq!(by_owner[0].key.name, "child");
        let by_zone = store
            .list(SpecSelector { zone: Some("guest".into()), ..Default::default() })
            .await
            .unwrap();
        assert!(by_zone.is_empty());
    }

    /// Failure taxonomy (U4): a full writer queue is backpressure - the
    /// writer is alive and the call is retryable - never phantom writer
    /// death. The writer is stalled on a second connection's exclusive lock
    /// (its IMMEDIATE transaction waits on `busy_timeout`), so the bounded
    /// queue cannot drain while the test floods it; the first refused call
    /// must be `Busy`, and the store must serve again once the lock is
    /// released.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn a_full_queue_returns_busy_backpressure_not_writer_gone() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("specs.db");
        // Bound 1: one in-flight request saturates the queue.
        let store = std::sync::Arc::new(SpecStore::open_with_bound(&path, 1).unwrap());
        // Hold the file's write lock on a second connection: the writer's
        // next IMMEDIATE transaction blocks, so the queue cannot drain.
        let blocker = rusqlite::Connection::open(&path).unwrap();
        blocker.execute_batch("BEGIN EXCLUSIVE").unwrap();
        // Flood the queue while the writer is stalled.
        let handles: Vec<_> = (0..3)
            .map(|i| {
                let store = std::sync::Arc::clone(&store);
                tokio::spawn(async move {
                    store.ensure(row(&format!("data-{i}"), b"spec")).await
                })
            })
            .collect();
        // Release the writer before collecting: the admitted request then
        // completes instead of timing out on busy_timeout.
        drop(blocker);
        let mut outcomes = Vec::with_capacity(handles.len());
        for handle in handles {
            outcomes.push(handle.await.expect("ensure task"));
        }
        assert!(
            outcomes.iter().any(|outcome| matches!(outcome, Err(SpecStoreError::Busy))),
            "a saturated queue must refuse with Busy, got {outcomes:?}"
        );
        assert!(
            outcomes
                .iter()
                .all(|outcome| !matches!(outcome, Err(SpecStoreError::WriterGone))),
            "a full queue is not writer death, got {outcomes:?}"
        );
        // The writer survived: round-trips still work after the backlog
        // drains.
        let outcome = store.ensure(row("data", b"spec-v1")).await.unwrap();
        assert!(matches!(outcome, EnsureOutcome::Created(_)));
    }

    /// Failure taxonomy (U4): a killed writer is terminal. Dropping the
    /// sender closes the channel; the writer drains and exits, and every
    /// later call refuses with `WriterGone` - the store must be reopened.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn a_killed_writer_returns_writer_gone_terminal() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("specs.db");
        let mut store = SpecStore::open_with_bound(&path, 1).unwrap();
        // Kill the writer: dropping the sender closes the channel (the
        // writer drains pending requests, checkpoints, and exits).
        drop(store.sender.take());
        let err = store.ensure(row("data", b"spec-v1")).await.unwrap_err();
        assert!(
            matches!(err, SpecStoreError::WriterGone),
            "a dead writer is terminal WriterGone, got {err:?}"
        );
    }
}