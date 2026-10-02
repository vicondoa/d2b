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
//! `busy_timeout` 5s, `synchronous=FULL`, a **rollback journal**, and
//! **IMMEDIATE transactions**: every durable mutation (ensure /
//! mark-deleting / remove) and its audit-log record commit inside one
//! `BEGIN IMMEDIATE` transaction, and the async call returns only after that
//! commit (commit-before-return, R7/R10, AE1). File posture: the store
//! creates its file with mode 0600 (directory 0700 when it creates the
//! directory); the database's `<name>-journal` side file is tightened to
//! 0600 as well.
//!
//! # Why a rollback journal and not a write-ahead log
//!
//! The store's `store_incarnation` is an identity: it is what every broker
//! authority projection for this store's Zones is bound to, and a projection
//! that names an incarnation the store no longer carries can never be
//! republished against - the Zone refuses every publication until the
//! ownership-bounded reset clears both halves. An identity that lives only in
//! a write-ahead log is not one.
//!
//! In WAL mode the committed rows sit in `<name>-wal` and the main database
//! file stays a single page until a checkpoint runs, so a store whose
//! process is killed without one - a power cut, a `system_reset`, a snapshot
//! taken and restored while the daemon is running - reopens as an EMPTY
//! database. `apply_authority_journal` cannot tell that from a first boot:
//! both present a page count of one and an empty schema. It then mints a
//! fresh incarnation, and the split is permanent and silent.
//!
//! A rollback journal puts the committed state in the database file itself
//! and leaves the in-flight transaction in the side file, so an abrupt end
//! rolls back to the last commit and the store reopens on exactly the
//! incarnation it committed. `synchronous=FULL` is the matching setting: with
//! it every commit is fsynced through the journal, so the state this mode
//! keeps is state that survived. The store is the durable authority for
//! every Zone it holds and its writes are per desired-row mutation, not per
//! read, so the fsync is the cost of an identity that means something.
//!
//! ## One format, and one write path
//!
//! The authority-journal format is the only format this store has. It
//! persists a per-row desired revision, a per-Zone desired sequence, durable
//! publication transactions, their outbox, and the accepted-publication
//! cursor, so every authority change carries a durable identity of its own
//! instead of borrowing the spec generation (KTD5-KTD6).
//!
//! There is deliberately no second way to write a desired row: the store
//! exposes reads and the journal protocol, and a durable authority change
//! that could commit without a staged candidate, an outbox entry, and an
//! accepted-publication acknowledgment is exactly the change KTD6 requires to
//! be staged, fenced, committed, and acknowledged. Opening a database written
//! by an earlier release is refused by
//! [`crate::schema::apply_authority_journal`] rather than converted.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::mpsc::sync_channel;
use std::time::Duration;

use d2b_contracts_resource::v3::authority::{StoreIncarnation, ZoneDesiredSequence};
use rusqlite::{Connection, params};
use tokio::sync::oneshot;

use crate::authority_journal::{
    AcceptedCursor, AcceptedPublication, CommitOutcome, DesiredMutation, DesiredRow,
    PublicationTransaction, StagedMutation, ZoneRecovery,
};
use crate::identity::TransactionId;

pub const MODULE_NAME: &str = "spec_store";

/// The projection every read of a desired row goes through. It carries the
/// durable desired revision, so a reader that wants the revision gets it and
/// a reader that does not cannot read the row as one without it.
pub(crate) const AUTHORITY_ROW_COLUMNS: &str = "zone, type, name, uid, generation, owner_uid, \
     provenance, deleting, spec, metadata, created_at, desired_revision";

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
    /// Terminal desired state: set by the deleting mutation, cleared only by
    /// the removing one (R10).
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

/// What one committed ensure produced (R7 idempotence).
///
/// This is the manager's read of the journal's [`CommitOutcome`], not a second
/// write path: a row is only ever committed through
/// [`SpecStore::commit_mutation`], so the generation and revision below are
/// the ones that transaction committed.
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
    /// The database's schema could not be applied. The refusal case carries
    /// the database's own `user_version`: this release starts from a fresh
    /// store and never converts existing data.
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
    History {
        limit: usize,
        reply: oneshot::Sender<Result<Vec<AuditRecord>, SpecStoreError>>,
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
    ReadDesiredRows {
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

/// The writer thread's exclusive connection owner. All SQLite happens here.
///
/// The blocking `recv` is the sanctioned bounded-worker channel boundary
/// (plan R4): the writer is a dedicated thread, never an executor worker, so
/// the blocking recv parks only the writer's own thread. The reply travels
/// back over a `tokio::sync::oneshot`, exactly the loader_worker shape.
#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
fn writer_loop(mut conn: Connection, requests: Receiver<Request>) {
    while let Ok(request) = requests.recv() {
        match request {
            Request::History { limit, reply } => {
                let _ = reply.send(history(&conn, limit));
            }
            Request::StageMutation { mutation, reply } => {
                let _ = reply.send(crate::authority_journal::stage_mutation(&mut conn, mutation));
            }
            Request::RecordPrepared { transaction, prepared, reply } => {
                let _ = reply
                    .send(crate::authority_journal::record_prepared(&mut conn, transaction, &prepared));
            }
            Request::CommitMutation { transaction, reply } => {
                let _ = reply.send(crate::authority_journal::commit_mutation(&mut conn, transaction));
            }
            Request::Acknowledge { accepted, reply } => {
                let _ = reply.send(crate::authority_journal::acknowledge(&mut conn, accepted));
            }
            Request::CancelTransaction { transaction, reply } => {
                let _ =
                    reply.send(crate::authority_journal::cancel_transaction(&mut conn, transaction));
            }
            Request::DesiredRow { key, reply } => {
                let _ = reply.send(crate::authority_journal::desired_row(&conn, &key));
            }
            Request::ReadDesiredRows { selector, reply } => {
                let _ = reply.send(crate::authority_journal::desired_rows(&conn, &selector));
            }
            Request::ZoneSequence { zone, reply } => {
                let _ = reply.send(crate::authority_journal::zone_sequence(&conn, &zone));
            }
            Request::AcceptedCursor { zone, reply } => {
                let _ = reply.send(crate::authority_journal::accepted_cursor(&conn, &zone));
            }
            Request::ZoneRecovery { zone, reply } => {
                let _ = reply.send(crate::authority_journal::zone_recovery(&conn, &zone));
            }
            Request::StoreIncarnation { reply } => {
                let _ = reply.send(crate::authority_journal::store_incarnation(&conn));
            }
        }
    }
    // Channel closed: the last handle dropped. A rollback journal leaves
    // nothing to fold in - closing the connection rolls an in-flight
    // transaction back and removes the journal - so the close is the whole
    // of it, and the database file is already the last commit.
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
    // Best-effort posture enforcement: the store file and its side files
    // carry the daemon's private-data mode (0600). SQLite names a side file
    // by appending its suffix to the *database file name*, so the suffix is
    // appended here too - `with_extension` would rewrite the real suffix
    // (`spec-store.sqlite3` -> `spec-store.db-journal`) and leave the file
    // SQLite actually created at its creation mode. `-journal` is this
    // store's journal; `-wal` and `-shm` are tightened as well because a
    // database an earlier release left in write-ahead-log mode still has
    // them, and opening it converts it.
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
    for suffix in ["-journal", "-wal", "-shm"] {
        if let Some(side) = side_file(suffix) {
            tighten(&side);
        }
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
    // The durability posture the module header states, and the reason for it:
    // the store's incarnation has to live in the database file rather than in
    // a write-ahead log, or a store whose writer is killed without a
    // checkpoint reopens as a different, brand-new store and orphans every
    // broker projection bound to the one it used to carry.
    conn.pragma_update(None, "journal_mode", "DELETE")?;
    conn.pragma_update(None, "synchronous", "FULL")?;
    Ok(conn)
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

/// Read one desired row in [`AUTHORITY_ROW_COLUMNS`] order: the committed row
/// plus its durable desired revision.
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
    sender: Option<SyncSender<Request>>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl SpecStore {
    /// Open (creating when absent) the store at `path` and start the writer
    /// thread. Returns after the authority-journal schema is committed, so
    /// the store's incarnation is durable before any caller can stage a
    /// candidate in it.
    ///
    /// A database written by an earlier release is refused rather than
    /// converted: the clean break starts from a fresh store.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, SpecStoreError> {
        Self::open_with_bound(path, 256)
    }

    /// [`Self::open`] with an explicit writer-queue bound. The production
    /// bound is 256; tests shrink it to inject queue-full backpressure.
    fn open_with_bound(path: impl Into<PathBuf>, bound: usize) -> Result<Self, SpecStoreError> {
        let path = path.into();
        let mut conn = open_connection(&path)?;
        crate::schema::apply_authority_journal(&mut conn)?;
        tighten_file_modes(&path);
        let (sender, receiver) = sync_channel::<Request>(bound);
        let join = std::thread::Builder::new()
            .name("spec-store-writer".into())
            .spawn(move || writer_loop(conn, receiver))
            .map_err(|err| SpecStoreError::Io(std::io::Error::other(err.to_string())))?;
        Ok(Self { path, sender: Some(sender), join: Some(join) })
    }

    /// The database file path this store opened.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// One committed desired row, or [`SpecStoreError::NotFound`].
    pub async fn get(&self, key: ResourceKey) -> Result<StoredDesiredResource, SpecStoreError> {
        Ok(self.desired_row(key).await?.row)
    }

    /// Every committed desired row the selector matches.
    pub async fn list(
        &self,
        selector: SpecSelector,
    ) -> Result<Vec<StoredDesiredResource>, SpecStoreError> {
        Ok(self
            .desired_rows(selector)
            .await?
            .into_iter()
            .map(|desired| desired.row)
            .collect())
    }

    /// Newest-first tail of committed-mutation audit records.
    pub async fn history(&self, limit: usize) -> Result<Vec<AuditRecord>, SpecStoreError> {
        self.call(move |reply| Request::History { limit, reply }).await
    }

    // -----------------------------------------------------------------
    // Authority-journal protocol (KTD5-KTD6)
    //
    // This is the only way a desired row changes. Every call below commits
    // before it returns, and every one of them is a complete transaction: the
    // caller's broker I/O happens strictly between calls, so no open SQLite
    // transaction, store lock, or source-reservation lock can cross the
    // transport wait.
    // -----------------------------------------------------------------


    /// One durable authority mutation, in the only order the protocol permits.
    ///
    /// The store is where a desired row changes, so this is its single write
    /// path: see [`crate::authority_publish::publish`] for the ordering and
    /// why each step is a separate transaction.
    pub async fn publish(
        &self,
        mutation: DesiredMutation,
        authority: &dyn crate::authority_publish::AuthorityPublisher,
    ) -> Result<
        crate::authority_publish::PublishOutcome,
        crate::authority_publish::PublishError,
    > {
        crate::authority_publish::publish(self, mutation, authority).await
    }

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
        self.call(|reply| Request::ReadDesiredRows { selector, reply }).await
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
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::authority_journal::{CommitOutcome, DesiredMutation};
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

    /// The store's own half of the protocol, without the broker round trip
    /// between the steps: a manager drives exactly this sequence around the
    /// fence it obtained out of band, and the store's mechanics are what
    /// these tests are about.
    async fn publish(store: &SpecStore, mutation: DesiredMutation) -> CommitOutcome {
        let staged = store.stage_mutation(mutation).await.expect("stage");
        store.record_prepared(staged.transaction, "prepared-1").await.expect("prepare");
        let outcome = store.commit_mutation(staged.transaction).await.expect("commit");
        if let CommitOutcome::Committed(committed) | CommitOutcome::AlreadyCommitted(committed) =
            &outcome
        {
            store
                .acknowledge(AcceptedPublication {
                    transaction: committed.transaction,
                    zone: committed.zone.clone(),
                    incarnation: committed.incarnation.clone(),
                    sequence: committed.sequence,
                    candidate: committed.candidate.clone(),
                })
                .await
                .expect("acknowledge");
        }
        outcome
    }

    /// The committed row a mutation produced, for the `Unchanged` outcome too.
    fn committed_row(outcome: CommitOutcome) -> StoredDesiredResource {
        match outcome {
            CommitOutcome::Committed(committed) => committed.publication.rows[0].row.clone(),
            CommitOutcome::AlreadyCommitted(committed) => committed.publication.rows[0].row.clone(),
            CommitOutcome::Unchanged { row, .. } => row.row,
        }
    }

    /// Persist-then-reopen: rows, audit, and the schema survive close/reopen,
    /// and reopening an existing database rewrites nothing.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn persist_then_reopen() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("specs.db");
        let first = {
            let store = SpecStore::open(&path).unwrap();
            let committed = committed_row(
                publish(&store, DesiredMutation::Ensure(row("data", b"spec-v1"))).await,
            );
            assert_eq!(committed.generation, 1);
            store.store_incarnation().await.unwrap()
        };
        let store = SpecStore::open(&path).unwrap();
        assert_eq!(store.store_incarnation().await.unwrap(), first, "one database, one incarnation");
        let got = store.get(ResourceKey::new("host", "Volume", "data")).await.unwrap();
        assert_eq!(got.spec, b"spec-v1");
        assert_eq!(got.generation, 1);
        assert!(!got.deleting);
        let history = store.history(10).await.unwrap();
        assert!(history.iter().any(|rec| rec.operation == "authority.ensure"));
    }

    /// A database written by an earlier release is refused rather than
    /// converted: its desired rows carry no revision and no journal, and
    /// reading them through the protocol would report authority changes that
    /// never happened.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn a_foreign_schema_version_is_refused_rather_than_converted() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("specs.db");
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE resources (zone TEXT); PRAGMA user_version = 1;")
                .unwrap();
        }
        let Err(error) = SpecStore::open(&path) else {
            panic!("an older store is refused, not reopened");
        };
        assert!(
            matches!(
                error,
                SpecStoreError::Schema(crate::schema::SchemaError::RefusedSchema { user_version: 1 })
            ),
            "got {error:?}"
        );
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
            publish(&store, DesiredMutation::Ensure(row("data", b"spec-v1"))).await;
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

    /// Durability boundary (AE1): the commit returns only after the desired
    /// rows are committed. The second call observes the committed generation,
    /// proving the first call's write was durable before its Ok.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn commit_returns_after_the_desired_rows_are_durable() {
        let dir = TempDir::new().unwrap();
        let store = open_in(&dir);
        let first = committed_row(publish(&store, DesiredMutation::Ensure(row("data", b"spec-v1"))).await);
        assert_eq!(first.generation, 1);
        // A subsequent transaction observes the commit.
        let second =
            committed_row(publish(&store, DesiredMutation::Ensure(row("data", b"spec-v2"))).await);
        assert_eq!(second.generation, 2, "changed spec advances exactly once per mutation");
        let read = store.get(first.key.clone()).await.unwrap();
        assert_eq!(read.generation, 2);
        assert_eq!(read.spec, b"spec-v2");
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
            publish(&store, DesiredMutation::Ensure(row("data", b"spec-v1"))).await;
            publish(
                &store,
                DesiredMutation::MarkDeleting(ResourceKey::new("host", "Volume", "data")),
            )
            .await;
            // Simulated crash: writer thread and connection torn down with no
            // graceful checkpoint (abandon the handle mid-flight).
            std::mem::forget(store);
        }
        let store = SpecStore::open(&path).unwrap();
        let got = store.get(ResourceKey::new("host", "Volume", "data")).await.unwrap();
        assert!(got.deleting, "deleting mark must survive crash");
    }

    /// An ensure against a deleting row is rejected with a typed error; the
    /// deleting mark stays terminal until removal (R10).
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn ensure_against_deleting_rejected() {
        let dir = TempDir::new().unwrap();
        let store = open_in(&dir);
        let key = ResourceKey::new("host", "Volume", "data");
        publish(&store, DesiredMutation::Ensure(row("data", b"spec-v1"))).await;
        publish(&store, DesiredMutation::MarkDeleting(key.clone())).await;
        // The deleting mark is terminal, so the candidate is refused while it
        // is staged: nothing is reserved and no Zone sequence is consumed for
        // a mutation that could never commit.
        let error = store
            .stage_mutation(DesiredMutation::Ensure(row("data", b"spec-v2")))
            .await
            .unwrap_err();
        assert!(matches!(error, SpecStoreError::ResourceDeleting { .. }), "got {error:?}");
        // After cleanup removes the row, an ensure recreates it fresh.
        publish(&store, DesiredMutation::Remove(key)).await;
        let created =
            committed_row(publish(&store, DesiredMutation::Ensure(row("data", b"spec-v2"))).await);
        assert_eq!(created.generation, 1, "a removed row is created fresh, never updated");
    }

    /// One Zone has at most one outstanding publication transaction, so two
    /// writer handles on one database serialize by refusing the second
    /// candidate by name rather than by racing the first one's fence.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn a_second_writer_queues_behind_the_outstanding_transaction() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("specs.db");
        let a = Arc::new(SpecStore::open(&path).unwrap());
        let b = Arc::new(SpecStore::open(&path).unwrap());
        let mut handles = Vec::new();
        for (store, name) in [(Arc::clone(&a), "a"), (Arc::clone(&b), "b")] {
            for i in 0..10u32 {
                let store = Arc::clone(&store);
                let mut r = row(&format!("{name}-{i}"), b"spec");
                r.provenance = ResourceProvenance::Nix;
                handles.push(tokio::spawn(async move {
                    // A refused candidate leaves the Zone fenced, so the
                    // retry after the outstanding transaction settles is the
                    // whole production behavior.
                    for _ in 0..2_000 {
                        match store.stage_mutation(DesiredMutation::Ensure(r.clone())).await {
                            Ok(staged) => {
                                publish_after_stage(&store, staged).await;
                                return;
                            }
                            Err(SpecStoreError::ZoneTransactionOutstanding { .. }) => {
                                tokio::time::sleep(Duration::from_millis(1)).await;
                            }
                            Err(error) => panic!("a staged mutation must not fail: {error:?}"),
                        }
                    }
                    panic!("every mutation commits exactly once");
                }));
            }
        }
        for handle in handles {
            handle.await.unwrap();
        }
        let rows = a.list(SpecSelector::default()).await.unwrap();
        assert_eq!(rows.len(), 20, "two writer handles, no lost or doubled row");
        assert!(rows.iter().all(|row| row.generation == 1));
        let history = a.history(1000).await.unwrap();
        assert_eq!(history.iter().filter(|rec| rec.operation == "authority.ensure").count(), 20);
    }

    async fn publish_after_stage(store: &SpecStore, staged: StagedMutation) {
        store.record_prepared(staged.transaction, "prepared-1").await.expect("prepare");
        let outcome = store.commit_mutation(staged.transaction).await.expect("commit");
        if let CommitOutcome::Committed(committed) | CommitOutcome::AlreadyCommitted(committed) =
            &outcome
        {
            store
                .acknowledge(AcceptedPublication {
                    transaction: committed.transaction,
                    zone: committed.zone.clone(),
                    incarnation: committed.incarnation.clone(),
                    sequence: committed.sequence,
                    candidate: committed.candidate.clone(),
                })
                .await
                .expect("acknowledge");
        }
    }

    /// File posture (0600 store / side files, 0700 dir) asserted after writes.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn file_mode_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("private").join("specs.db");
        let store = SpecStore::open(&path).unwrap();
        publish(&store, DesiredMutation::Ensure(row("data", b"spec-v1"))).await;
        let mode = |p: PathBuf| async move {
            tokio::fs::metadata(&p)
                .await
                .map(|m| m.permissions().mode() & 0o777)
                .unwrap_or(u32::MAX)
        };
        assert_eq!(mode(path.clone()).await, 0o600, "store file mode");
        // A rollback journal only exists while a transaction is open, so the
        // side file's posture is asserted on one this store actually names:
        // it is put there world-readable and the store is reopened, which is
        // what runs the posture enforcement over it.
        tokio::fs::write(path.with_extension("db-journal"), b"").await.expect("side file");
        let reopened = SpecStore::open(&path).expect("reopen runs the posture enforcement");
        drop(reopened);
        assert_eq!(mode(path.with_extension("db-journal")).await, 0o600, "journal mode");
        assert_eq!(mode(path.parent().unwrap().to_path_buf()).await, 0o700, "store dir mode");
    }

    /// The side files of the *actual* database path are the ones tightened
    /// (issue: `with_extension` rewrote the suffix for
    /// `*.<suffix-db>` names, so a `.sqlite3` store kept SQLite's creation
    /// mode on its real side file while the module claimed 0600).
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn side_files_of_a_suffixed_database_are_tightened() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("zones").join("dev").join("spec-store.sqlite3");
        let store = SpecStore::open(&path).expect("open");
        publish(&store, DesiredMutation::Ensure(row("data", b"spec-v1"))).await;
        let side = |suffix: &str| path.with_file_name(format!("spec-store.sqlite3{suffix}"));
        let chmod = |p: PathBuf, mode: u32| async move {
            let file = tokio::fs::File::open(&p).await.unwrap_or_else(|error| panic!("{p:?}: {error}"));
            file.set_permissions(PermissionsExt::from_mode(mode)).await.unwrap();
        };
        // SQLite only holds its journal open while a transaction runs, so
        // each side file this store names is put there directly and forced
        // world-readable: the assertions then prove the store tightened
        // them, not that the process umask happened to.
        for suffix in ["-journal", "-wal", "-shm"] {
            tokio::fs::write(side(suffix), b"").await.expect("side file");
            chmod(side(suffix), 0o644).await;
        }
        // A second open on the same file re-runs `tighten_file_modes`.
        let reopened = SpecStore::open(&path).expect("reopen");
        let mode = |p: PathBuf| async move {
            tokio::fs::metadata(&p).await.unwrap().permissions().mode() & 0o777
        };
        for suffix in ["-journal", "-wal", "-shm"] {
            assert_eq!(
                mode(side(suffix)).await,
                0o600,
                "the real {suffix} side file is tightened"
            );
        }
        drop(reopened);
        drop(store);
    }

    /// The committed state lives in the database file, not only beside it.
    ///
    /// This is the property the store's incarnation depends on. A store whose
    /// committed rows sit only in a write-ahead log reopens as an EMPTY
    /// database when its process ends without closing the connection - a
    /// power cut, a `system_reset`, a snapshot taken and restored while the
    /// daemon is running - because closing is what makes SQLite fold and drop
    /// the log, and an abrupt end never closes.
    /// `apply_authority_journal` cannot tell that from a first boot: both
    /// present one page and an empty schema. It then mints a fresh
    /// incarnation, and every broker projection bound to the previous one is
    /// orphaned with nothing left to recover through.
    ///
    /// The end is modelled by leaking the connection rather than closing it,
    /// which is what an abrupt end leaves behind, and the proof is the
    /// database file alone: with every side file removed, the state must
    /// still be there.
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn the_committed_state_survives_an_abrupt_end_without_any_side_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("spec-store.sqlite3");
        {
            let mut conn = open_connection(&path).expect("open");
            let applied = crate::schema::apply_authority_journal(&mut conn)
                .expect("the store's own journal is applied");
            assert_eq!(applied, crate::schema::SchemaOutcome::Created);
            conn.execute_batch(
                "INSERT INTO zone_desired_sequence (zone, sequence) VALUES ('system', 7);",
            )
            .expect("a committed row");
            // The abrupt end: the connection is never closed, so nothing
            // folds a write-ahead log into the database file.
            std::mem::forget(conn);
        }
        for suffix in ["-journal", "-wal", "-shm"] {
            let _ = std::fs::remove_file(path.with_file_name(format!("spec-store.sqlite3{suffix}")));
        }
        let conn = rusqlite::Connection::open(&path).expect("reopen on the database file alone");
        let sequence: Option<i64> = conn
            .query_row(
                "SELECT sequence FROM zone_desired_sequence WHERE zone = 'system'",
                [],
                |row| row.get(0),
            )
            .ok();
        let incarnation: Option<String> = conn
            .query_row(
                "SELECT value FROM store_meta WHERE key = 'store_incarnation'",
                [],
                |row| row.get(0),
            )
            .ok();
        assert_eq!(
            sequence,
            Some(7),
            "a committed row is carried by the database file itself, so a store whose writer ended \
             abruptly is still the store it committed"
        );
        assert!(
            incarnation.is_some(),
            "the incarnation is carried by the database file itself: an identity that lives only \
             in a write-ahead log is not one, because a host that loses it mints a fresh store and \
             orphans every broker projection bound to the one it used to carry"
        );
    }

    /// List honors selector filters (zone / type / owner).
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn list_by_selector() {
        let dir = TempDir::new().unwrap();
        let store = open_in(&dir);
        let mut child = row("child", b"s");
        child.owner_uid = Some(uid_for("data"));
        publish(&store, DesiredMutation::Ensure(row("data", b"s"))).await;
        publish(&store, DesiredMutation::Ensure(child)).await;
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
                let store = Arc::clone(&store);
                tokio::spawn(async move {
                    store
                        .zone_sequence("host")
                        .await
                        .map(|sequence| i as u64 ^ sequence.get())
                })
            })
            .collect();
        // Release the writer before collecting: the admitted request then
        // completes instead of timing out on busy_timeout.
        drop(blocker);
        let mut outcomes = Vec::with_capacity(handles.len());
        for handle in handles {
            outcomes.push(handle.await.expect("request task"));
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
        assert_eq!(store.zone_sequence("host").await.unwrap().get(), 0);
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
        let err = store.zone_sequence("host").await.unwrap_err();
        assert!(
            matches!(err, SpecStoreError::WriterGone),
            "a dead writer is terminal WriterGone, got {err:?}"
        );
    }
}
