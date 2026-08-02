//! Bounded logical backup and staged publication primitives.
//!
//! Backups contain validated logical rows, never a copied open database file.
//! Publication accepts only directory descriptors plus single path
//! components.  The broker remains responsible for resolving those
//! descriptors from the opaque storage contract.

use std::collections::BTreeSet;
use std::fmt;
use std::fs::File;

use d2b_contracts::v3::{RetryClass, canonical_digest, canonical_json_bytes};
use redb::{Database, Durability, ReadableDatabase, ReadableTable, TableDefinition};
use rustix::fs::{AtFlags, FileType, fsync, renameat, statat};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::schema::TABLE_SCHEMAS;
use crate::transaction::{
    ALL_TABLES, PHYSICAL_SCHEMA_VERSION, StoreMeta, decode, integrity, read_meta,
    validate_consistency, validate_identity,
};
use crate::{DecodedKey, DecodedValue, KeySpace, StoreIdentity, ValueKind};

/// Logical backup format version.
pub const LOGICAL_BACKUP_FORMAT_VERSION: u32 = 1;
/// Maximum serialized logical backup size accepted by this backend.
pub const MAX_LOGICAL_BACKUP_BYTES: usize = 256 * 1024 * 1024;
/// Maximum number of rows in one logical backup.
pub const MAX_LOGICAL_BACKUP_ROWS: usize = 1_000_000;
/// Maximum bytes in one publication name.
pub const MAX_PUBLICATION_NAME_BYTES: usize = 128;

const BACKUP_DIGEST_DOMAIN: &str = "d2b:v3:resource-store-backup";

/// One raw logical row from a named physical table.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackupRow {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

impl fmt::Debug for BackupRow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackupRow")
            .field("key_bytes", &self.key.len())
            .field("value_bytes", &self.value.len())
            .finish()
    }
}

/// One complete table image and its deterministic checksum.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackupTable {
    pub name: String,
    pub key_space: u8,
    pub value_kind: u16,
    pub checksum: String,
    pub rows: Vec<BackupRow>,
}

impl fmt::Debug for BackupTable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackupTable")
            .field("name", &self.name)
            .field("key_space", &self.key_space)
            .field("value_kind", &self.value_kind)
            .field("row_count", &self.rows.len())
            .field("checksum", &"<redacted>")
            .finish()
    }
}

/// A consistent logical snapshot of one Zone store.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LogicalBackup {
    pub format_version: u32,
    pub store_uuid: String,
    pub zone_name: String,
    pub zone_uid: String,
    pub created_at: String,
    pub schema_version: u32,
    pub current_revision: u64,
    pub compaction_floor: u64,
    pub active_configuration_revision: u64,
    pub policy_revision: u64,
    pub api_catalog_revision: u64,
    pub controller_generation: Option<u64>,
    pub clean_shutdown: bool,
    pub backup_generation: u64,
    pub tables: Vec<BackupTable>,
}

impl fmt::Debug for LogicalBackup {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LogicalBackup")
            .field("format_version", &self.format_version)
            .field("schema_version", &self.schema_version)
            .field("current_revision", &self.current_revision)
            .field("compaction_floor", &self.compaction_floor)
            .field("table_count", &self.tables.len())
            .field("store_uuid", &"<redacted>")
            .field("zone_name", &"<redacted>")
            .field("zone_uid", &"<redacted>")
            .finish()
    }
}

impl LogicalBackup {
    /// Capture one validated MVCC snapshot from an open database.
    pub fn from_database(
        database: &Database,
        identity: &StoreIdentity,
    ) -> Result<Self, crate::StoreError> {
        validate_consistency(database).map_err(|error| error.with_store_slot(identity.slot()))?;
        let read = database
            .begin_read()
            .map_err(integrity)
            .map_err(|error| error.with_store_slot(identity.slot()))?;
        let meta = read_meta(&read).map_err(|error| error.with_store_slot(identity.slot()))?;
        validate_meta_identity(&meta, identity)
            .map_err(|error| error.with_store_slot(identity.slot()))?;

        let mut tables = Vec::with_capacity(TABLE_SCHEMAS.len());
        for schema in TABLE_SCHEMAS {
            let definition = table_definition(schema.key_space)
                .ok_or_else(|| integrity("backup-table-definition-missing"))?;
            let table = read
                .open_table(definition)
                .map_err(integrity)
                .map_err(|error| error.with_store_slot(identity.slot()))?;
            let mut rows = Vec::new();
            for row in table
                .iter()
                .map_err(integrity)
                .map_err(|error| error.with_store_slot(identity.slot()))?
            {
                let (key, value) = row
                    .map_err(integrity)
                    .map_err(|error| error.with_store_slot(identity.slot()))?;
                rows.push(BackupRow {
                    key: key.value().to_vec(),
                    value: value.value().to_vec(),
                });
            }
            tables.push(BackupTable {
                name: schema.name.to_owned(),
                key_space: schema.key_space.discriminant(),
                value_kind: schema.value_kind.discriminant(),
                checksum: checksum_rows(&rows),
                rows,
            });
        }

        let backup = Self {
            format_version: LOGICAL_BACKUP_FORMAT_VERSION,
            store_uuid: meta.store_uuid,
            zone_name: meta.zone_name,
            zone_uid: meta.zone_uid,
            created_at: meta.created_at,
            schema_version: meta.schema_version,
            current_revision: meta.current_revision,
            compaction_floor: meta.compaction_floor,
            active_configuration_revision: meta.active_configuration_revision,
            policy_revision: meta.policy_revision,
            api_catalog_revision: meta.api_catalog_revision,
            controller_generation: meta.controller_generation,
            clean_shutdown: meta.clean_shutdown,
            backup_generation: meta.backup_generation,
            tables,
        };
        backup
            .validate()
            .map_err(|error| error.with_store_slot(identity.slot()))?;
        Ok(backup)
    }

    /// Validate the backup's closed schema and all row framing.
    pub fn validate(&self) -> Result<(), crate::StoreError> {
        if self.format_version != LOGICAL_BACKUP_FORMAT_VERSION {
            return Err(integrity("backup-format-version-unknown"));
        }
        if self.schema_version != PHYSICAL_SCHEMA_VERSION {
            return Err(crate::StoreError::new(
                crate::StoreErrorKind::UpgradeRequired,
                None,
                None,
                RetryClass::Never,
                "backup-schema-version-unsupported",
            ));
        }
        if self.compaction_floor > self.current_revision {
            return Err(integrity("backup-revision-range-invalid"));
        }
        if self.tables.len() != ALL_TABLES.len() {
            return Err(integrity("backup-table-set-invalid"));
        }

        let mut names = BTreeSet::new();
        let mut total_rows = 0_usize;
        for table in &self.tables {
            if !names.insert(table.name.as_str()) {
                return Err(integrity("backup-table-duplicate"));
            }
            let schema = TABLE_SCHEMAS
                .iter()
                .find(|schema| schema.name == table.name)
                .ok_or_else(|| integrity("backup-table-unknown"))?;
            if table.key_space != schema.key_space.discriminant()
                || table.value_kind != schema.value_kind.discriminant()
            {
                return Err(integrity("backup-table-schema-mismatch"));
            }
            total_rows = total_rows
                .checked_add(table.rows.len())
                .ok_or_else(|| integrity("backup-row-count-overflow"))?;
            if total_rows > MAX_LOGICAL_BACKUP_ROWS {
                return Err(integrity("backup-row-count-over-limit"));
            }
            if checksum_rows(&table.rows) != table.checksum {
                return Err(integrity("backup-table-checksum-mismatch"));
            }
            let mut keys = BTreeSet::new();
            for row in &table.rows {
                if !keys.insert(row.key.as_slice()) {
                    return Err(integrity("backup-table-key-duplicate"));
                }
                validate_row(schema.key_space, schema.value_kind, row)?;
            }
        }
        if names.len() != ALL_TABLES.len() {
            return Err(integrity("backup-table-set-invalid"));
        }

        let meta_table = self
            .tables
            .iter()
            .find(|table| table.name == "store_meta")
            .ok_or_else(|| integrity("backup-store-meta-missing"))?;
        if meta_table.rows.len() != 1 {
            return Err(integrity("backup-store-meta-cardinality-invalid"));
        }
        let meta: StoreMeta = decode(
            ValueKind::StoreMetaScalar,
            meta_table
                .rows
                .first()
                .ok_or_else(|| integrity("backup-store-meta-missing"))?
                .value
                .as_slice(),
        )?;
        if meta.store_uuid != self.store_uuid
            || meta.zone_name != self.zone_name
            || meta.zone_uid != self.zone_uid
            || meta.created_at != self.created_at
            || meta.schema_version != self.schema_version
            || meta.current_revision != self.current_revision
            || meta.compaction_floor != self.compaction_floor
            || meta.active_configuration_revision != self.active_configuration_revision
            || meta.policy_revision != self.policy_revision
            || meta.api_catalog_revision != self.api_catalog_revision
            || meta.controller_generation != self.controller_generation
            || meta.clean_shutdown != self.clean_shutdown
            || meta.backup_generation != self.backup_generation
        {
            return Err(integrity("backup-store-meta-mismatch"));
        }
        Ok(())
    }

    /// Validate the immutable store identity carried by a backup.
    pub fn validate_for_identity(
        &self,
        identity: &StoreIdentity,
    ) -> Result<(), crate::StoreError> {
        self.validate()?;
        if self.store_uuid != identity.store_uuid.as_str()
            || self.zone_name != identity.zone.as_str()
            || self.zone_uid != identity.zone_uid.as_str()
            || self.created_at != identity.created_at
            || self.schema_version != PHYSICAL_SCHEMA_VERSION
            || self.active_configuration_revision
                != identity.revisions.active_configuration_revision.get()
            || self.policy_revision != identity.revisions.policy_revision
            || self.api_catalog_revision != identity.revisions.api_catalog_revision
            || self.controller_generation
                != identity.revisions.controller_generation.map(|value| value.get())
        {
            return Err(integrity("backup-store-identity-mismatch"));
        }
        Ok(())
    }

    /// Serialize a deterministic canonical logical backup.
    pub fn to_bytes(&self) -> Result<Vec<u8>, crate::StoreError> {
        let bytes = canonical_json_bytes(self).map_err(integrity)?;
        if bytes.len() > MAX_LOGICAL_BACKUP_BYTES {
            return Err(integrity("backup-size-over-limit"));
        }
        Ok(bytes)
    }

    /// Decode a canonical logical backup and reject all framing drift.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, crate::StoreError> {
        if bytes.is_empty() || bytes.len() > MAX_LOGICAL_BACKUP_BYTES {
            return Err(integrity("backup-size-invalid"));
        }
        let backup: Self = serde_json::from_slice(bytes).map_err(integrity)?;
        if canonical_json_bytes(&backup).map_err(integrity)? != bytes {
            return Err(integrity("backup-not-canonical"));
        }
        backup.validate()?;
        Ok(backup)
    }

    /// Return the digest of the canonical logical representation.
    pub fn digest(&self) -> Result<String, crate::StoreError> {
        Ok(canonical_digest(BACKUP_DIGEST_DOMAIN, &self.to_bytes()?))
    }

    /// Restore this logical image into an empty staged database.
    pub fn restore_into(
        &self,
        database: &Database,
        identity: &StoreIdentity,
    ) -> Result<(), crate::StoreError> {
        self.validate_for_identity(identity)
            .map_err(|error| error.with_store_slot(identity.slot()))?;
        let read = database
            .begin_read()
            .map_err(integrity)
            .map_err(|error| error.with_store_slot(identity.slot()))?;
        if read
            .list_tables()
            .map_err(integrity)
            .map_err(|error| error.with_store_slot(identity.slot()))?
            .next()
            .is_some()
        {
            return Err(crate::transaction::quarantined_reason(
                "restore-target-not-empty",
            )
            .with_store_slot(identity.slot()));
        }
        drop(read);

        let mut write = database
            .begin_write()
            .map_err(integrity)
            .map_err(|error| error.with_store_slot(identity.slot()))?;
        write
            .set_durability(Durability::Immediate)
            .map_err(integrity)
            .map_err(|error| error.with_store_slot(identity.slot()))?;
        for table_snapshot in &self.tables {
            let schema = TABLE_SCHEMAS
                .iter()
                .find(|schema| schema.name == table_snapshot.name)
                .ok_or_else(|| integrity("backup-table-unknown"))?;
            let definition = table_definition(schema.key_space)
                .ok_or_else(|| integrity("backup-table-definition-missing"))?;
            let mut table = write
                .open_table(definition)
                .map_err(integrity)
                .map_err(|error| error.with_store_slot(identity.slot()))?;
            for row in &table_snapshot.rows {
                if table
                    .insert(row.key.as_slice(), row.value.as_slice())
                    .map_err(integrity)
                    .map_err(|error| error.with_store_slot(identity.slot()))?
                    .is_some()
                {
                    return Err(crate::transaction::quarantined_reason(
                        "restore-duplicate-key",
                    )
                    .with_store_slot(identity.slot()));
                }
            }
        }
        write
            .commit()
            .map_err(integrity)
            .map_err(|error| error.with_store_slot(identity.slot()))?;

        validate_identity(database, identity)
            .and_then(|_| validate_consistency(database))
            .map_err(|_| {
                crate::transaction::quarantined_reason("restore-validation-failed")
                    .with_store_slot(identity.slot())
            })
    }

    /// Restore into a descriptor-backed staged database.
    pub fn restore_file(
        &self,
        file: File,
        identity: &StoreIdentity,
    ) -> Result<Database, crate::StoreError> {
        crate::validate_owned_file(&file)
            .map_err(|error| error.with_store_slot(identity.slot()))?;
        if file
            .metadata()
            .map_err(integrity)
            .map_err(|error| error.with_store_slot(identity.slot()))?
            .len()
            != 0
        {
            return Err(crate::transaction::quarantined_reason(
                "restore-target-not-empty",
            )
            .with_store_slot(identity.slot()));
        }
        let backend = redb::backends::FileBackend::new(file)
            .map_err(integrity)
            .map_err(|error| error.with_store_slot(identity.slot()))?;
        let database = Database::builder()
            .create_with_backend(backend)
            .map_err(integrity)
            .map_err(|error| error.with_store_slot(identity.slot()))?;
        self.restore_into(&database, identity)?;
        Ok(database)
    }
}

fn validate_meta_identity(
    meta: &StoreMeta,
    identity: &StoreIdentity,
) -> Result<(), crate::StoreError> {
    if meta.schema_version != PHYSICAL_SCHEMA_VERSION
        || meta.store_uuid != identity.store_uuid.as_str()
        || meta.zone_name != identity.zone.as_str()
        || meta.zone_uid != identity.zone_uid.as_str()
        || meta.created_at != identity.created_at
        || meta.active_configuration_revision
            != identity.revisions.active_configuration_revision.get()
        || meta.policy_revision != identity.revisions.policy_revision
        || meta.api_catalog_revision != identity.revisions.api_catalog_revision
        || meta.controller_generation
            != identity.revisions.controller_generation.map(|value| value.get())
    {
        return Err(integrity("backup-store-identity-mismatch"));
    }
    Ok(())
}

fn validate_row(
    key_space: KeySpace,
    value_kind: ValueKind,
    row: &BackupRow,
) -> Result<(), crate::StoreError> {
    let key = DecodedKey::decode(&row.key).map_err(integrity)?;
    if key.key_space() != key_space {
        return Err(integrity("backup-row-key-space-mismatch"));
    }
    let value = DecodedValue::decode(&row.value).map_err(integrity)?;
    if value.kind() != value_kind {
        return Err(integrity("backup-row-value-kind-mismatch"));
    }
    Ok(())
}

fn checksum_rows(rows: &[BackupRow]) -> String {
    let mut digest = Sha256::new();
    for row in rows {
        digest.update((row.key.len() as u64).to_be_bytes());
        digest.update(&row.key);
        digest.update((row.value.len() as u64).to_be_bytes());
        digest.update(&row.value);
    }
    format!("sha256:{:x}", digest.finalize())
}

fn table_definition(
    key_space: KeySpace,
) -> Option<TableDefinition<'static, &'static [u8], &'static [u8]>> {
    match key_space {
        KeySpace::StoreMeta => Some(crate::transaction::STORE_META),
        KeySpace::ApiSchemas => Some(crate::transaction::API_SCHEMAS),
        KeySpace::Resources => Some(crate::transaction::RESOURCES),
        KeySpace::TypeIndex => Some(crate::transaction::TYPE_INDEX),
        KeySpace::OwnerIndex => Some(crate::transaction::OWNER_INDEX),
        KeySpace::ProducerIndex => Some(crate::transaction::PRODUCER_INDEX),
        KeySpace::ControllerIndex => Some(crate::transaction::CONTROLLER_INDEX),
        KeySpace::RevisionLog => Some(crate::transaction::REVISION_LOG),
        KeySpace::Operations => Some(crate::transaction::OPERATIONS),
        KeySpace::ZoneLinkCursors => Some(crate::transaction::ZONE_LINK_CURSORS),
    }
}

/// The state of one fd-relative staged publication set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationState {
    Empty,
    ActiveOnly,
    StagedOnly,
    PriorOnly,
    ActiveAndStaged,
    ActiveAndPrior,
    StagedAndPrior,
    AllPresent,
}

/// Inspect publication names without following symlinks.
pub fn publication_state(
    parent: &File,
    staged_name: &str,
    active_name: &str,
    prior_name: &str,
) -> Result<PublicationState, crate::StoreError> {
    validate_publication_name(staged_name)?;
    validate_publication_name(active_name)?;
    validate_publication_name(prior_name)?;
    let staged = publication_entry(parent, staged_name)?;
    let active = publication_entry(parent, active_name)?;
    let prior = publication_entry(parent, prior_name)?;
    let mask = u8::from(staged.is_some()) | (u8::from(active.is_some()) << 1)
        | (u8::from(prior.is_some()) << 2);
    Ok(match mask {
        0 => PublicationState::Empty,
        1 => PublicationState::StagedOnly,
        2 => PublicationState::ActiveOnly,
        3 => PublicationState::ActiveAndStaged,
        4 => PublicationState::PriorOnly,
        5 => PublicationState::StagedAndPrior,
        6 => PublicationState::ActiveAndPrior,
        7 => PublicationState::AllPresent,
        _ => unreachable!("publication bit mask is three bits"),
    })
}

/// Fsync a staged database and its anchored parent directory.
pub fn sync_staged_file(file: &File, parent: &File) -> Result<(), crate::StoreError> {
    if !parent
        .metadata()
        .map_err(integrity)?
        .file_type()
        .is_dir()
    {
        return Err(integrity("publication-parent-not-directory"));
    }
    file.sync_all().map_err(crate::transaction::durability_failure)?;
    fsync(parent)
        .map_err(crate::transaction::durability_failure)
        .map_err(|_| crate::transaction::quarantined_reason("publication-parent-fsync-failed"))
}

/// Publish a staged file using only an already-open parent directory.
///
/// The prior active file is retained.  Any pre-existing prior file or
/// ambiguous name set fails closed rather than overwriting state.
pub fn publish_staged(
    parent: &File,
    staged_name: &str,
    active_name: &str,
    prior_name: &str,
) -> Result<(), crate::StoreError> {
    validate_publication_name(staged_name)?;
    validate_publication_name(active_name)?;
    validate_publication_name(prior_name)?;
    if staged_name == active_name
        || staged_name == prior_name
        || active_name == prior_name
    {
        return Err(crate::transaction::quarantined_reason(
            "publication-name-collision",
        ));
    }
    if !parent
        .metadata()
        .map_err(integrity)?
        .file_type()
        .is_dir()
    {
        return Err(integrity("publication-parent-not-directory"));
    }
    let state = publication_state(parent, staged_name, active_name, prior_name)?;
    if !matches!(
        state,
        PublicationState::StagedOnly | PublicationState::ActiveAndStaged
    ) {
        return Err(crate::transaction::quarantined_reason(
            "publication-state-ambiguous",
        ));
    }
    if matches!(state, PublicationState::ActiveOnly) {
        return Err(crate::transaction::quarantined_reason(
            "publication-staged-missing",
        ));
    }
    if !matches!(
        publication_entry(parent, staged_name)?,
        Some(FileType::RegularFile)
    ) {
        return Err(crate::transaction::quarantined_reason(
            "publication-staged-not-regular",
        ));
    }
    if publication_entry(parent, active_name)?.is_some() {
        if !matches!(
            publication_entry(parent, active_name)?,
            Some(FileType::RegularFile)
        ) {
            return Err(crate::transaction::quarantined_reason(
                "publication-active-not-regular",
            ));
        }
        renameat(parent, active_name, parent, prior_name)
            .map_err(crate::transaction::durability_failure)
            .map_err(|_| crate::transaction::quarantined_reason("publication-prior-rename-failed"))?;
        fsync(parent)
            .map_err(crate::transaction::durability_failure)
            .map_err(|_| crate::transaction::quarantined_reason("publication-parent-fsync-failed"))?;
    }
    renameat(parent, staged_name, parent, active_name)
        .map_err(crate::transaction::durability_failure)
        .map_err(|_| crate::transaction::quarantined_reason("publication-active-rename-failed"))?;
    fsync(parent)
        .map_err(crate::transaction::durability_failure)
        .map_err(|_| crate::transaction::quarantined_reason("publication-parent-fsync-failed"))
}

fn validate_publication_name(name: &str) -> Result<(), crate::StoreError> {
    if name.is_empty()
        || name.len() > MAX_PUBLICATION_NAME_BYTES
        || name == "."
        || name == ".."
        || name.bytes().any(|byte| {
            byte == 0
                || byte == b'/'
                || !matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'-' | b'_')
        })
    {
        return Err(integrity("publication-name-invalid"));
    }
    Ok(())
}

fn publication_entry(
    parent: &File,
    name: &str,
) -> Result<Option<FileType>, crate::StoreError> {
    match statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => Ok(Some(FileType::from_raw_mode(stat.st_mode))),
        Err(error) if error == rustix::io::Errno::NOENT => Ok(None),
        Err(_) => Err(integrity("publication-entry-stat-failed")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;

    #[test]
    fn publication_names_reject_path_injection() {
        let parent = tempfile::tempdir().unwrap();
        assert_eq!(
            publication_state(
                &File::open(parent.path()).unwrap(),
                "../store",
                "store",
                "prior",
            )
            .unwrap_err()
            .reason_code(),
            "publication-name-invalid"
        );
    }

    #[test]
    fn staged_publication_retains_prior_and_is_fd_relative() {
        let parent = tempfile::tempdir().unwrap();
        let directory = File::open(parent.path()).unwrap();
        let active_path = parent.path().join("active");
        let staged_path = parent.path().join("staged");
        File::create(&active_path).unwrap();
        File::create(&staged_path).unwrap();
        let active = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&active_path)
            .unwrap();
        active.sync_all().unwrap();
        let staged = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&staged_path)
            .unwrap();
        sync_staged_file(&staged, &directory).unwrap();
        publish_staged(&directory, "staged", "active", "prior").unwrap();
        assert_eq!(
            publication_state(&directory, "staged", "active", "prior").unwrap(),
            PublicationState::ActiveAndPrior
        );
    }
}
