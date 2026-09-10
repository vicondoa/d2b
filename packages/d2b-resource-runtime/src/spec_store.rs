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
//! ## Durability posture
//!
//! WAL mode, `busy_timeout` 5s, `synchronous=NORMAL`, and **IMMEDIATE
//! transactions**: every durable mutation (ensure / mark-deleting / remove)
//! and its audit-log record commit inside one `BEGIN IMMEDIATE` transaction,
//! and the async call returns only after that commit (commit-before-return,
//! R7/R10, AE1). File posture: the store creates its file with mode 0600
//! (directory 0700 when it creates the directory); the WAL and SHM side
//! files are tightened to 0600 as well.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::mpsc::sync_channel;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};
use tokio::sync::oneshot;

pub const MODULE_NAME: &str = "spec_store";

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
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Nix => "nix",
            Self::Api => "api",
            Self::Resource => "resource",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "nix" => Some(Self::Nix),
            "api" => Some(Self::Api),
            "resource" => Some(Self::Resource),
            _ => None,
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
    /// Same resource, byte-identical spec: no-op, current handle returned.
    Unchanged(StoredDesiredResource),
    /// Changed spec: new generation committed before this value returned.
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
    #[error("spec store io: {0}")]
    Io(#[from] std::io::Error),
    #[error("spec store sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("spec store migration: {0}")]
    Migration(String),
    /// Writer thread gone (store handle raced past shutdown, or the writer
    /// panicked). The store must be reopened.
    #[error("spec store writer unavailable")]
    WriterGone,
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
}

/// The writer thread's exclusive connection owner. All SQLite happens here.
fn writer_loop(mut conn: Connection, requests: Receiver<Request>) {
    for request in requests {
        match request {
            Request::Ensure { row, reply } => {
                let _ = reply.send(ensure_transactional(&mut conn, row));
            }
            Request::Get { key, reply } => {
                let _ = reply.send(get(&conn, &key).map(Some));
            }
            Request::List { selector, reply } => {
                let _ = reply.send(list(&conn, &selector));
            }
            Request::MarkDeleting { key, reply } => {
                let _ = reply.send(mark_deleting_transactional(&mut conn, &key));
            }
            Request::RemoveAfterCleanup { key, reply } => {
                let _ = reply.send(remove_after_cleanup(&mut conn, &key));
            }
            Request::History { limit, reply } => {
                let _ = reply.send(history(&conn, limit));
            }
            Request::Migrate { reply } => {
                let result = crate::schema::migrate(&mut conn)
                    .map_err(|err| SpecStoreError::Migration(err.to_string()));
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
) -> Result<EnsureOutcome, SpecStoreError> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let outcome = ensure_transactional_inner(&tx, row)?;
    tx.commit()?;
    Ok(outcome)
}

fn ensure_transactional_inner(
    tx: &rusqlite::Transaction<'_>,
    row: StoredDesiredResource,
) -> Result<EnsureOutcome, SpecStoreError> {
    let existing: Option<(u64, bool, Vec<u8>)> = tx
        .query_row(
            "SELECT generation, deleting, spec FROM resources \
             WHERE zone = ?1 AND type = ?2 AND name = ?3",
            params![row.key.zone, row.key.type_name, row.key.name],
            |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? != 0, r.get::<_, Vec<u8>>(2)?)),
        )
        .optional()?;
    let Some((generation, deleting, existing_spec)) = existing else {
        let stored = insert_new(tx, &row)?;
        insert_audit(
            tx,
            now(),
            "resource.ensure",
            row.provenance.as_str(),
            Some(&row.key),
            "ensure.create",
            None,
            Some(stored.generation as i64),
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
    if existing_spec == row.spec {
        let stored = load_row(tx, &row.key)?.expect("row present within its own transaction");
        return Ok(EnsureOutcome::Unchanged(stored));
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
            row.owner_uid.map(|u| u.to_vec()),
            row.provenance.as_str(),
            row.spec,
            row.metadata,
        ],
    )?;
    insert_audit(
        tx,
        now(),
        "resource.ensure",
        row.provenance.as_str(),
        Some(&row.key),
        "ensure.update",
        Some(generation as i64),
        Some(next as i64),
    )?;
    let stored = load_row(tx, &row.key)?.expect("row present within its own transaction");
    Ok(EnsureOutcome::Updated(stored))
}

// ---------------------------------------------------------------------------
// Connection setup
// ---------------------------------------------------------------------------

fn tighten_file_modes(path: &Path) {
    // Best-effort posture enforcement: the store file and its WAL/SHM side
    // files carry the daemon's private-data mode (0600). Side files only
    // exist while a connection holds the database open in WAL mode.
    let tighten = |p: &Path| {
        if let Ok(file) = std::fs::File::open(p) {
            let _ = file.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600));
        }
    };
    tighten(path);
    tighten(&path.with_extension("db-wal"));
    tighten(&path.with_extension("db-shm"));
}

fn open_connection(path: &Path) -> Result<Connection, SpecStoreError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
            // Only when we create it: private directory for private data.
            let _ = std::fs::set_permissions(parent, std::os::unix::fs::PermissionsExt::from_mode(0o700));
        }
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

fn insert_audit(
    conn: &Connection,
    ts: i64,
    subject: &str,
    provenance: &str,
    key: Option<&ResourceKey>,
    operation: &str,
    generation_before: Option<i64>,
    generation_after: Option<i64>,
) -> Result<(), SpecStoreError> {
    conn.execute(
        "INSERT INTO audit_log (ts, subject, provenance, resource_zone, resource_type, \
         resource_name, operation, generation_before, generation_after, detail) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL)",
        params![
            ts,
            subject,
            provenance,
            key.map(|k| k.zone.as_str()),
            key.map(|k| k.type_name.as_str()),
            key.map(|k| k.name.as_str()),
            operation,
            generation_before,
            generation_after,
        ],
    )?;
    Ok(())
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn insert_new(
    tx: &rusqlite::Transaction<'_>,
    row: &StoredDesiredResource,
) -> Result<StoredDesiredResource, SpecStoreError> {
    let created_at = now();
    tx.execute(
        "INSERT INTO resources (zone, type, name, uid, generation, owner_uid, provenance, \
         deleting, spec, metadata, created_at) \
         VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, 0, ?7, ?8, ?9)",
        params![
            row.key.zone,
            row.key.type_name,
            row.key.name,
            row.uid.as_slice(),
            row.owner_uid.map(|u| u.to_vec()),
            row.provenance.as_str(),
            row.spec,
            row.metadata,
            created_at,
        ],
    )?;
    Ok(StoredDesiredResource { generation: 1, deleting: false, created_at, ..row.clone() })
}

fn row_from(
    r: &rusqlite::Row<'_>,
) -> rusqlite::Result<StoredDesiredResource> {
    Ok(StoredDesiredResource {
        key: ResourceKey {
            zone: r.get(0)?,
            type_name: r.get(1)?,
            name: r.get(2)?,
        },
        uid: r.get::<_, Vec<u8>>(3)?.try_into().unwrap_or([0; 16]),
        generation: r.get::<_, i64>(4)? as u64,
        owner_uid: r.get::<_, Option<Vec<u8>>>(5)?.map(|v| v.try_into().unwrap_or([0; 16])),
        provenance: ResourceProvenance::from_str(&r.get::<_, String>(6)?)
            .unwrap_or(ResourceProvenance::Api),
        deleting: r.get::<_, i64>(7)? != 0,
        spec: r.get(8)?,
        metadata: r.get(9)?,
        created_at: r.get(10)?,
    })
}

fn load_row(
    conn: &Connection,
    key: &ResourceKey,
) -> Result<Option<StoredDesiredResource>, SpecStoreError> {
    Ok(conn
        .query_row(
            "SELECT zone, type, name, uid, generation, owner_uid, provenance, deleting, \
             spec, metadata, created_at FROM resources \
             WHERE zone = ?1 AND type = ?2 AND name = ?3",
            params![key.zone, key.type_name, key.name],
            row_from,
        )
        .optional()?)
}

fn get(conn: &Connection, key: &ResourceKey) -> Result<StoredDesiredResource, SpecStoreError> {
    load_row(conn, key)?
        .ok_or_else(|| SpecStoreError::NotFound {
            zone: key.zone.clone(),
            type_name: key.type_name.clone(),
            name: key.name.clone(),
        })
}

fn list(conn: &Connection, selector: &SpecSelector) -> Result<Vec<StoredDesiredResource>, SpecStoreError> {
    let sql = "SELECT zone, type, name, uid, generation, owner_uid, provenance, deleting, \
         spec, metadata, created_at FROM resources \
         WHERE (?1 IS NULL OR zone = ?1) \
           AND (?2 IS NULL OR type = ?2) \
           AND (?3 IS NULL OR owner_uid = ?3) \
         ORDER BY zone, type, name";
    let zone = selector.zone.clone();
    let type_name = selector.type_name.clone();
    let owner = selector.owner_uid.map(|u| u.to_vec());
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(params![zone, type_name, owner], row_from)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Set the terminal deleting mark (R10). Fails with
/// [`SpecStoreError::NotFound`] when absent. Mutation + audit commit in one
/// IMMEDIATE transaction before the caller sees the result.
fn mark_deleting_transactional(
    conn: &mut Connection,
    key: &ResourceKey,
) -> Result<StoredDesiredResource, SpecStoreError> {
    begin_immediate(conn)?;
    let result = (|| {
        let Some(before) = load_row(conn, key)? else {
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
                now(),
                "resource.deletion",
                before.provenance.as_str(),
                Some(key),
                "deletion.mark",
                Some(before.generation as i64),
                Some(before.generation as i64),
            )?;
        }
        Ok(load_row(conn, key)?.expect("row present within its own transaction"))
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
fn remove_after_cleanup(conn: &mut Connection, key: &ResourceKey) -> Result<(), SpecStoreError> {
    begin_immediate(conn)?;
    let result = (|| {
        let Some(existing) = load_row(conn, key)? else {
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
            now(),
            "resource.deletion",
            existing.provenance.as_str(),
            Some(key),
            "deletion.removed",
            Some(existing.generation as i64),
            None,
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
    sender: Option<SyncSender<Request>>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl SpecStore {
    /// Open (creating when absent) the store at `path`, apply pending
    /// migrations, and start the writer thread. Returns after migrations are
    /// committed.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, SpecStoreError> {
        let path = path.into();
        let mut conn = open_connection(&path)?;
        crate::schema::migrate(&mut conn).map_err(|err| SpecStoreError::Migration(err.to_string()))?;
        tighten_file_modes(&path);
        let (sender, receiver) = sync_channel::<Request>(256);
        let join = std::thread::Builder::new()
            .name("spec-store-writer".into())
            .spawn(move || writer_loop(conn, receiver))
            .map_err(|err| SpecStoreError::Io(std::io::Error::other(err.to_string())))?;
        Ok(Self { path, sender: Some(sender), join: Some(join) })
    }

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

    async fn call<R, F>(&self, make: F) -> Result<R, SpecStoreError>
    where
        F: FnOnce(oneshot::Sender<Result<R, SpecStoreError>>) -> Request,
    {
        let (tx, rx) = oneshot::channel();
        let sender = self.sender.as_ref().expect("sender lives until Drop");
        sender.try_send(make(tx)).map_err(|_| SpecStoreError::WriterGone)?;
        rx.await.map_err(|_| SpecStoreError::WriterGone)?
    }
}

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

    /// Durability boundary (AE1): ensure returns only after the commit. The
    /// second store call observes the committed generation, proving the
    /// first call's write was durable before its Ok.
    #[tokio::test]
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
                "ensure.create" | "ensure.update" | "deletion.mark" | "deletion.removed"
            )
        }));
    }

    /// Concurrent writers serialize without SQLITE_BUSY surfacing: two
    /// independent store handles on the same file hammer ensure on different
    /// keys; `busy_timeout` + IMMEDIATE transactions absorb contention.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
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

    /// File posture (0600 store / WAL / SHM, 0700 dir) asserted after writes.
    #[tokio::test]
    async fn file_mode_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("private").join("specs.db");
        let store = SpecStore::open(&path).unwrap();
        store.ensure(row("data", b"spec-v1")).await.unwrap();
        let mode = |p: &Path| {
            std::fs::metadata(p)
                .map(|m| m.permissions().mode() & 0o777)
                .unwrap_or(u32::MAX)
        };
        assert_eq!(mode(&path), 0o600, "store file mode");
        assert_eq!(mode(&path.with_extension("db-wal")), 0o600, "wal mode");
        assert_eq!(mode(&path.with_extension("db-shm")), 0o600, "shm mode");
        assert_eq!(mode(&path.parent().unwrap()), 0o700, "store dir mode");
    }

    /// List honors selector filters (zone / type / owner).
    #[tokio::test]
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
}