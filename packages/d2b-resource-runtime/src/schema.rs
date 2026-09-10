//! Embedded SQLite schema migrations for the spec store.
//!
//! The migration list is the single source of schema truth. Migrations apply
//! on [`crate::spec_store::SpecStore::open`] via `rusqlite_migration` and are
//! idempotent by construction (`user_version` bookkeeping), so reopen after a
//! restart never rewrites an existing schema.

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