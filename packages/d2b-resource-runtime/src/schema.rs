//! Embedded SQLite schema migrations for the spec store.
//!
//! The migration list is the single source of schema truth. Migrations apply
//! on [`crate::spec_store::SpecStore::open`] via `rusqlite_migration` and are
//! idempotent by construction (`user_version` bookkeeping), so reopen after a
//! restart never rewrites an existing schema.
//!
//! The authority-journal format (plan unit U5, KTD5-KTD6) is the second
//! store format, and it is deliberately not a migration of this one: it is
//! created only on a fresh database, and a database at any other
//! `user_version` is refused rather than converted. See [`StoreFormat`] and
//! [`apply_authority_journal`].

use rusqlite_migration::{M, Migrations};

/// All schema migrations, oldest first. Never edit a shipped migration; append.
pub fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(
            r#"
            CREATE TABLE resources (
                zone         TEXT    NOT NULL,
                type         TEXT    NOT NULL,
                name         TEXT    NOT NULL,
                uid          BLOB    NOT NULL,
                generation   INTEGER NOT NULL,
                owner_uid    BLOB,
                provenance   TEXT    NOT NULL
                    CHECK (provenance IN ('nix', 'api', 'resource')),
                deleting     INTEGER NOT NULL DEFAULT 0,
                spec         BLOB    NOT NULL,
                metadata     BLOB    NOT NULL,
                created_at   INTEGER NOT NULL,
                PRIMARY KEY (zone, type, name)
            );
            CREATE INDEX resources_owner_uid ON resources (owner_uid);
            CREATE INDEX resources_zone_type ON resources (zone, type);
            CREATE TABLE audit_log (
                id                INTEGER PRIMARY KEY AUTOINCREMENT,
                ts                INTEGER NOT NULL,
                subject           TEXT    NOT NULL,
                provenance        TEXT    NOT NULL,
                resource_zone     TEXT,
                resource_type     TEXT,
                resource_name     TEXT,
                operation         TEXT    NOT NULL,
                generation_before INTEGER,
                generation_after  INTEGER,
                detail            BLOB
            );
            CREATE INDEX audit_log_subject_ts ON audit_log (subject, ts);
            "#,
        ),
    ])
}

/// Apply pending migrations to `conn` and report the resulting schema version.
pub fn migrate(conn: &mut rusqlite::Connection) -> Result<(), rusqlite_migration::Error> {
    migrations().to_latest(conn)
}

/// The two store formats this module owns.
///
/// The production composition keeps [`Self::DesiredRows`] and this change
/// does not move it: the authority-journal format is a distinct schema
/// version that the new graph construction opens explicitly. There is no
/// path from one format to the other, because the clean break starts from a
/// fresh store rather than converting existing data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreFormat {
    /// The production desired-row schema: spec, metadata, ownership, and the
    /// audit log, with the spec generation as the only ordering value.
    DesiredRows,
    /// The authority-journal schema: everything [`Self::DesiredRows`]
    /// carries plus a per-row desired revision, a per-Zone desired
    /// sequence, durable publication transactions, their outbox, and the
    /// accepted-publication cursor (KTD5-KTD6).
    AuthorityJournal,
}

impl StoreFormat {
    /// The SQLite `user_version` this format stamps.
    pub const fn user_version(self) -> i64 {
        match self {
            Self::DesiredRows => 1,
            Self::AuthorityJournal => AUTHORITY_JOURNAL_USER_VERSION,
        }
    }
}

/// `user_version` of the authority-journal store format.
///
/// It is deliberately distinct from the production format's `1`: a database
/// at any other version is refused by [`apply_authority_journal`] rather than
/// converted, so an existing store is never read as if it carried desired
/// revisions and a publication journal it never had.
pub const AUTHORITY_JOURNAL_USER_VERSION: i64 = 2;

/// The complete authority-journal schema.
///
/// One `resources` table with a durable `desired_revision` column, a
/// per-Zone sequence, one durable publication transaction per staged
/// candidate, its publication outbox entry, and the accepted-publication
/// cursor. `store_meta` carries the store incarnation, which is minted once
/// when the schema is created and only ever read back afterwards: a different
/// incarnation is a different store, never a newer one.
const AUTHORITY_JOURNAL_SCHEMA: &str = r#"
    CREATE TABLE resources (
        zone             TEXT    NOT NULL,
        type             TEXT    NOT NULL,
        name             TEXT    NOT NULL,
        uid              BLOB    NOT NULL,
        generation       INTEGER NOT NULL,
        desired_revision INTEGER NOT NULL,
        owner_uid        BLOB,
        provenance       TEXT    NOT NULL
            CHECK (provenance IN ('nix', 'api', 'resource')),
        deleting         INTEGER NOT NULL DEFAULT 0,
        spec             BLOB    NOT NULL,
        metadata         BLOB    NOT NULL,
        created_at       INTEGER NOT NULL,
        PRIMARY KEY (zone, type, name)
    );
    CREATE INDEX resources_owner_uid ON resources (owner_uid);
    CREATE INDEX resources_zone_type ON resources (zone, type);
    CREATE TABLE audit_log (
        id                INTEGER PRIMARY KEY AUTOINCREMENT,
        ts                INTEGER NOT NULL,
        subject           TEXT    NOT NULL,
        provenance        TEXT    NOT NULL,
        resource_zone     TEXT,
        resource_type     TEXT,
        resource_name     TEXT,
        operation         TEXT    NOT NULL,
        generation_before INTEGER,
        generation_after  INTEGER,
        detail            BLOB
    );
    CREATE INDEX audit_log_subject_ts ON audit_log (subject, ts);
    CREATE TABLE store_meta (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    CREATE TABLE zone_desired_sequence (
        zone     TEXT    PRIMARY KEY,
        sequence INTEGER NOT NULL
    );
    CREATE TABLE authority_transaction (
        transaction_id   BLOB    PRIMARY KEY,
        zone             TEXT    NOT NULL,
        incarnation      TEXT    NOT NULL,
        sequence         INTEGER NOT NULL,
        candidate_digest TEXT    NOT NULL,
        candidate        BLOB    NOT NULL,
        state            TEXT    NOT NULL
            CHECK (state IN ('staged', 'prepared', 'committed', 'accepted', 'cancelled')),
        prepared_id      TEXT,
        committed_at     INTEGER,
        created_at       INTEGER NOT NULL,
        updated_at       INTEGER NOT NULL
    );
    CREATE INDEX authority_transaction_zone ON authority_transaction (zone, state);
    CREATE TABLE publication_outbox (
        transaction_id   BLOB    PRIMARY KEY
            REFERENCES authority_transaction (transaction_id),
        zone             TEXT    NOT NULL,
        incarnation      TEXT    NOT NULL,
        sequence         INTEGER NOT NULL,
        candidate_digest TEXT    NOT NULL,
        payload          BLOB    NOT NULL,
        created_at       INTEGER NOT NULL
    );
    CREATE INDEX publication_outbox_zone ON publication_outbox (zone, sequence);
    CREATE TABLE accepted_cursor (
        zone              TEXT    PRIMARY KEY,
        incarnation       TEXT    NOT NULL,
        accepted_sequence INTEGER NOT NULL,
        accepted_digest   TEXT    NOT NULL,
        updated_at        INTEGER NOT NULL
    );
"#;

/// What applying [`AUTHORITY_JOURNAL_SCHEMA`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaOutcome {
    /// The database was fresh: the schema and its store incarnation were
    /// written and stamped.
    Created,
    /// The database already carried this exact schema version: nothing was
    /// rewritten.
    AlreadyCurrent,
}

/// Why the authority-journal schema could not be applied.
#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    /// The database carries another format's `user_version`. This is a
    /// refusal, not a pending migration: the authority-journal format has no
    /// migration into it, and the clean break expects a fresh store.
    #[error("spec store schema version {user_version} is not the authority-journal format ({AUTHORITY_JOURNAL_USER_VERSION}); this release starts from a fresh store and never converts existing data")]
    RefusedSchema { user_version: i64 },
    #[error("spec store schema: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// The minted store incarnation is not a canonical bounded token, which
    /// would make every freshness tuple naming this store unrepresentable.
    #[error("minted store incarnation is not canonical")]
    Incarnation,
}

/// Create the authority-journal schema when `conn` is fresh, and refuse a
/// database that already carries another format.
///
/// The refusal is the point: an old store has desired rows with no desired
/// revision and no publication journal, and reading them through the journal
/// protocol would report authority changes that never happened. Reopening the
/// same authority-journal database is idempotent.
pub fn apply_authority_journal(
    conn: &mut rusqlite::Connection,
) -> Result<SchemaOutcome, SchemaError> {
    let user_version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if user_version == AUTHORITY_JOURNAL_USER_VERSION {
        return Ok(SchemaOutcome::AlreadyCurrent);
    }
    if user_version != 0 {
        return Err(SchemaError::RefusedSchema { user_version });
    }
    let incarnation = crate::authority_journal::mint_store_incarnation()?;
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let result: Result<(), rusqlite::Error> = (|| {
        conn.execute_batch(AUTHORITY_JOURNAL_SCHEMA)?;
        conn.execute(
            "INSERT INTO store_meta (key, value) VALUES ('store_incarnation', ?1)",
            [incarnation.as_str()],
        )?;
        // Stamped inside the creating transaction, so a half-created
        // database is a fresh database again rather than one at an unknown
        // version.
        conn.execute_batch(&format!("PRAGMA user_version = {AUTHORITY_JOURNAL_USER_VERSION}"))?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            conn.execute_batch("COMMIT")?;
            Ok(SchemaOutcome::Created)
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error.into())
        }
    }
}