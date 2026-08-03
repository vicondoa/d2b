//! Crash-safe logical restore and physical-schema migration.
//!
//! The storage owner passes this module an already-open parent directory and
//! identity marker.  It never receives a caller-controlled path.  A restore
//! or upgrade is built in a new sibling file, validated as a complete store,
//! synced, and published only by fd-relative renames.  The old file is kept
//! until the new file has been validated and the parent directory has been
//! synced.

use std::fs::File;

use d2b_contracts::v3::RetryClass;
use d2b_resource_store::{StoreError, StoreErrorKind};
use redb::backends::FileBackend;
use redb::{Database, Durability, ReadableDatabase, ReadableTable};
use rustix::fs::{AtFlags, FileType, Mode, OFlags, fsync, openat, renameat, statat, unlinkat};
use rustix::io::{FdFlags, fcntl_getfd};

use crate::backup::{self, LogicalBackup, PublicationState};
use crate::transaction::{
    ALL_TABLES, PHYSICAL_SCHEMA_VERSION, StoreMeta, decode, encode, integrity, meta_key,
    validate_consistency,
};
use crate::{REDB_CACHE_SIZE, StoreIdentity, ValueKind};

/// The currently published physical schema version.
pub const CURRENT_PHYSICAL_SCHEMA_VERSION: u32 = PHYSICAL_SCHEMA_VERSION;

/// The fixed active database name owned by the storage migration owner.
pub const DEFAULT_ACTIVE_FILE_NAME: &str = "store.redb";
/// The fixed staged database name used during restore and upgrade.
pub const DEFAULT_STAGED_FILE_NAME: &str = "store.redb.staged";
/// The fixed rollback database name retained across publication recovery.
pub const DEFAULT_PRIOR_FILE_NAME: &str = "store.redb.prior";

/// One approved edge in the physical-schema migration graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationStep {
    pub from: u32,
    pub to: u32,
    pub name: &'static str,
}

/// The only registered physical-schema migration chain.
///
/// Version zero is the pre-chain layout with the same table and value
/// assignments but an unversioned metadata record.  The migration makes that
/// version explicit before the normal current-schema validators run.
pub const REGISTERED_MIGRATIONS: &[MigrationStep] = &[MigrationStep {
    from: 0,
    to: CURRENT_PHYSICAL_SCHEMA_VERSION,
    name: "physical-schema-v0-to-v1",
}];

/// Result of a restore or upgrade request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationOutcome {
    /// The active file was already current and no publication was needed.
    AlreadyCurrent,
    /// A validated logical backup was published.
    Restored,
    /// An approved physical-schema chain was published.
    Upgraded { from: u32, to: u32 },
    /// A publication left by an interrupted owner was resumed safely.
    Recovered,
}

/// Result of publication recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryOutcome {
    /// There was no incomplete publication to resume.
    Clean,
    /// A staged publication was completed and made active.
    Resumed,
    /// A completed publication had its retained rollback file cleaned up.
    Finalized,
}

/// Return the approved chain from `from` to the current physical version.
pub fn migration_chain(from: u32) -> Result<Vec<MigrationStep>, StoreError> {
    if from == CURRENT_PHYSICAL_SCHEMA_VERSION {
        return Ok(Vec::new());
    }
    let mut chain = Vec::new();
    let mut version = from;
    while version != CURRENT_PHYSICAL_SCHEMA_VERSION {
        let step = REGISTERED_MIGRATIONS
            .iter()
            .find(|step| step.from == version)
            .copied()
            .ok_or_else(|| unsupported_version(from))?;
        if step.to <= step.from
            || chain
                .iter()
                .any(|prior: &MigrationStep| prior.from == step.from || prior.to == step.to)
        {
            return Err(unsupported_version(from));
        }
        chain.push(step);
        version = step.to;
        if chain.len() > REGISTERED_MIGRATIONS.len() {
            return Err(unsupported_version(from));
        }
    }
    Ok(chain)
}

/// Restore a validated logical backup through the staged publication owner.
///
/// The active database is never opened for writing.  If a prior publication
/// is found, it is resumed or finalized before accepting a new request.
pub fn restore_owned(
    parent: &File,
    marker: &mut File,
    backup: &LogicalBackup,
    identity: &StoreIdentity,
) -> Result<MigrationOutcome, StoreError> {
    validate_parent(parent)?;
    validate_marker(marker, identity)?;
    if recover_owned_inner(parent, identity)? != RecoveryOutcome::Clean {
        return Ok(MigrationOutcome::Recovered);
    }
    ensure_active_is_safe_or_absent(parent, identity)?;
    backup
        .validate_for_identity(identity)
        .map_err(|_| quarantine(identity, "migration-backup-identity-mismatch"))?;

    prepare_restore_stage(parent, backup, identity)?;
    publish_with_rollback(parent, None, identity)?;
    validate_named_current(parent, DEFAULT_ACTIVE_FILE_NAME, identity)?;
    finalize_prior(parent, identity)?;
    Ok(MigrationOutcome::Restored)
}

/// Upgrade the active database through the registered physical-schema chain.
///
/// Unknown and unregistered versions are rejected before a staged file is
/// created.  Version zero is copied into a new file and its metadata version
/// is advanced in the staged transaction; the active file remains untouched
/// until all current-schema checks pass.
pub fn upgrade_owned(
    parent: &File,
    marker: &mut File,
    identity: &StoreIdentity,
) -> Result<MigrationOutcome, StoreError> {
    validate_parent(parent)?;
    validate_marker(marker, identity)?;
    if recover_owned_inner(parent, identity)? != RecoveryOutcome::Clean {
        return Ok(MigrationOutcome::Recovered);
    }
    let state = backup::publication_state(
        parent,
        DEFAULT_STAGED_FILE_NAME,
        DEFAULT_ACTIVE_FILE_NAME,
        DEFAULT_PRIOR_FILE_NAME,
    )?;
    if !matches!(state, PublicationState::ActiveOnly) {
        return Err(quarantine(identity, "migration-active-store-missing"));
    }

    let source = open_named_database(parent, DEFAULT_ACTIVE_FILE_NAME, identity)?;
    let source_meta = read_meta(&source, identity)?;
    let chain = migration_chain(source_meta.schema_version)?;
    if chain.is_empty() {
        validate_database(&source, identity, Some(CURRENT_PHYSICAL_SCHEMA_VERSION))?;
        drop(source);
        return Ok(MigrationOutcome::AlreadyCurrent);
    }
    validate_database(&source, identity, Some(source_meta.schema_version))?;
    prepare_upgrade_stage(&source, parent, identity, source_meta.schema_version)?;
    drop(source);

    publish_with_rollback(parent, None, identity)?;
    validate_named_current(parent, DEFAULT_ACTIVE_FILE_NAME, identity)?;
    finalize_prior(parent, identity)?;
    let last = chain.last().expect("nonempty migration chain");
    Ok(MigrationOutcome::Upgraded {
        from: last.from,
        to: last.to,
    })
}

/// Resume or finalize a publication left by a crash or interrupted owner.
pub fn recover_owned(
    parent: &File,
    marker: &mut File,
    identity: &StoreIdentity,
) -> Result<RecoveryOutcome, StoreError> {
    validate_parent(parent)?;
    validate_marker(marker, identity)?;
    recover_owned_inner(parent, identity)
}

fn recover_owned_inner(
    parent: &File,
    identity: &StoreIdentity,
) -> Result<RecoveryOutcome, StoreError> {
    let state = backup::publication_state(
        parent,
        DEFAULT_STAGED_FILE_NAME,
        DEFAULT_ACTIVE_FILE_NAME,
        DEFAULT_PRIOR_FILE_NAME,
    )?;
    match state {
        PublicationState::Empty => Ok(RecoveryOutcome::Clean),
        PublicationState::ActiveOnly => {
            validate_named_current(parent, DEFAULT_ACTIVE_FILE_NAME, identity)?;
            Ok(RecoveryOutcome::Clean)
        }
        PublicationState::ActiveAndStaged => {
            validate_named_current(parent, DEFAULT_ACTIVE_FILE_NAME, identity)?;
            validate_named_current(parent, DEFAULT_STAGED_FILE_NAME, identity)?;
            publish_with_rollback(parent, None, identity)?;
            validate_named_current(parent, DEFAULT_ACTIVE_FILE_NAME, identity)?;
            finalize_prior(parent, identity)?;
            Ok(RecoveryOutcome::Resumed)
        }
        PublicationState::StagedOnly => {
            validate_named_current(parent, DEFAULT_STAGED_FILE_NAME, identity)?;
            publish_with_rollback(parent, None, identity)?;
            validate_named_current(parent, DEFAULT_ACTIVE_FILE_NAME, identity)?;
            Ok(RecoveryOutcome::Resumed)
        }
        PublicationState::StagedAndPrior => {
            validate_named_current(parent, DEFAULT_STAGED_FILE_NAME, identity)?;
            validate_named_supported(parent, DEFAULT_PRIOR_FILE_NAME, identity)?;
            resume_staged_with_prior(parent, identity)?;
            validate_named_current(parent, DEFAULT_ACTIVE_FILE_NAME, identity)?;
            finalize_prior(parent, identity)?;
            Ok(RecoveryOutcome::Resumed)
        }
        PublicationState::ActiveAndPrior => {
            validate_named_current(parent, DEFAULT_ACTIVE_FILE_NAME, identity)?;
            validate_named_supported(parent, DEFAULT_PRIOR_FILE_NAME, identity)?;
            finalize_prior(parent, identity)?;
            Ok(RecoveryOutcome::Finalized)
        }
        PublicationState::PriorOnly | PublicationState::AllPresent => Err(quarantine(
            identity,
            "migration-publication-state-ambiguous",
        )),
    }
}

fn prepare_restore_stage(
    parent: &File,
    backup: &LogicalBackup,
    identity: &StoreIdentity,
) -> Result<(), StoreError> {
    let staged = create_stage(parent, identity)?;
    let database = backup
        .restore_file(staged, identity)
        .map_err(|_| quarantine(identity, "migration-staged-restore-invalid"))?;
    drop(database);
    validate_named_current(parent, DEFAULT_STAGED_FILE_NAME, identity)?;
    sync_named_stage(parent, identity)
}

fn prepare_upgrade_stage(
    source: &Database,
    parent: &File,
    identity: &StoreIdentity,
    from: u32,
) -> Result<(), StoreError> {
    let staged = create_stage(parent, identity)?;
    let backend = FileBackend::new(staged)
        .map_err(|_| quarantine(identity, "migration-staged-open-failed"))?;
    let target = Database::builder()
        .set_cache_size(REDB_CACHE_SIZE)
        .create_with_backend(backend)
        .map_err(|_| quarantine(identity, "migration-staged-open-failed"))?;
    copy_registered_step(source, &target, from, identity)?;
    drop(target);
    validate_named_current(parent, DEFAULT_STAGED_FILE_NAME, identity)?;
    sync_named_stage(parent, identity)
}

fn copy_registered_step(
    source: &Database,
    target: &Database,
    from: u32,
    identity: &StoreIdentity,
) -> Result<(), StoreError> {
    let step = REGISTERED_MIGRATIONS
        .iter()
        .find(|step| step.from == from && step.to == CURRENT_PHYSICAL_SCHEMA_VERSION)
        .copied()
        .ok_or_else(|| unsupported_version(from))?;
    if step.name != "physical-schema-v0-to-v1" {
        return Err(unsupported_version(from));
    }

    let source_read = source
        .begin_read()
        .map_err(|_| quarantine(identity, "migration-source-read-failed"))?;
    if source_read
        .list_tables()
        .map_err(|_| quarantine(identity, "migration-source-table-set-invalid"))?
        .count()
        != ALL_TABLES.len()
    {
        return Err(quarantine(identity, "migration-source-table-set-invalid"));
    }
    let mut write = target
        .begin_write()
        .map_err(|_| quarantine(identity, "migration-staged-write-failed"))?;
    write
        .set_durability(Durability::Immediate)
        .map_err(|_| quarantine(identity, "migration-staged-durability-failed"))?;
    for (table_index, definition) in ALL_TABLES.iter().enumerate() {
        let source_table = source_read
            .open_table(*definition)
            .map_err(|_| quarantine(identity, "migration-source-table-invalid"))?;
        let mut target_table = write
            .open_table(*definition)
            .map_err(|_| quarantine(identity, "migration-staged-table-invalid"))?;
        for row in source_table
            .iter()
            .map_err(|_| quarantine(identity, "migration-source-row-invalid"))?
        {
            let (key, value) =
                row.map_err(|_| quarantine(identity, "migration-source-row-invalid"))?;
            let mut value_bytes = value.value().to_vec();
            if table_index == 0 && key.value() == meta_key().as_slice() {
                let mut meta: StoreMeta = decode(ValueKind::StoreMetaScalar, &value_bytes)
                    .map_err(|_| quarantine(identity, "migration-source-meta-invalid"))?;
                if meta.schema_version != from {
                    return Err(quarantine(identity, "migration-source-version-changed"));
                }
                meta.schema_version = CURRENT_PHYSICAL_SCHEMA_VERSION;
                value_bytes = encode(ValueKind::StoreMetaScalar, &meta)
                    .map_err(|_| quarantine(identity, "migration-staged-meta-invalid"))?;
            }
            if target_table
                .insert(key.value(), value_bytes.as_slice())
                .map_err(|_| quarantine(identity, "migration-staged-row-invalid"))?
                .is_some()
            {
                return Err(quarantine(identity, "migration-staged-duplicate-key"));
            }
        }
    }
    write
        .commit()
        .map_err(|_| quarantine(identity, "migration-staged-commit-failed"))
}

fn publish_with_rollback(
    parent: &File,
    fault: Option<PublicationBoundary>,
    identity: &StoreIdentity,
) -> Result<(), StoreError> {
    let result = publish_once(parent, fault, identity);
    if let Err(error) = result {
        if rollback_publication(parent, identity).is_err() {
            return Err(quarantine(identity, "migration-rollback-failed"));
        }
        return Err(error);
    }
    Ok(())
}

fn publish_once(
    parent: &File,
    fault: Option<PublicationBoundary>,
    identity: &StoreIdentity,
) -> Result<(), StoreError> {
    let state = backup::publication_state(
        parent,
        DEFAULT_STAGED_FILE_NAME,
        DEFAULT_ACTIVE_FILE_NAME,
        DEFAULT_PRIOR_FILE_NAME,
    )?;
    if !matches!(
        state,
        PublicationState::StagedOnly | PublicationState::ActiveAndStaged
    ) {
        return Err(quarantine(identity, "migration-publication-state-invalid"));
    }
    if fault == Some(PublicationBoundary::AfterStageSync) {
        return Err(injected_fault("migration-fault-after-stage-sync", identity));
    }
    if matches!(state, PublicationState::ActiveAndStaged) {
        renameat(
            parent,
            DEFAULT_ACTIVE_FILE_NAME,
            parent,
            DEFAULT_PRIOR_FILE_NAME,
        )
        .map_err(|_| quarantine(identity, "migration-prior-rename-failed"))?;
        sync_parent(parent, identity)?;
        if fault == Some(PublicationBoundary::AfterPriorRename) {
            return Err(injected_fault(
                "migration-fault-after-prior-rename",
                identity,
            ));
        }
    }
    renameat(
        parent,
        DEFAULT_STAGED_FILE_NAME,
        parent,
        DEFAULT_ACTIVE_FILE_NAME,
    )
    .map_err(|_| quarantine(identity, "migration-active-rename-failed"))?;
    if fault == Some(PublicationBoundary::AfterActiveRename) {
        return Err(injected_fault(
            "migration-fault-after-active-rename",
            identity,
        ));
    }
    sync_parent(parent, identity)?;
    if fault == Some(PublicationBoundary::AfterFinalSync) {
        return Err(injected_fault("migration-fault-after-final-sync", identity));
    }
    Ok(())
}

fn resume_staged_with_prior(parent: &File, identity: &StoreIdentity) -> Result<(), StoreError> {
    renameat(
        parent,
        DEFAULT_STAGED_FILE_NAME,
        parent,
        DEFAULT_ACTIVE_FILE_NAME,
    )
    .map_err(|_| quarantine(identity, "migration-active-rename-failed"))?;
    sync_parent(parent, identity)
}

fn rollback_publication(parent: &File, identity: &StoreIdentity) -> Result<(), StoreError> {
    let state = backup::publication_state(
        parent,
        DEFAULT_STAGED_FILE_NAME,
        DEFAULT_ACTIVE_FILE_NAME,
        DEFAULT_PRIOR_FILE_NAME,
    )?;
    match state {
        PublicationState::StagedOnly | PublicationState::ActiveAndStaged => {
            remove_regular(parent, DEFAULT_STAGED_FILE_NAME, identity)?;
            sync_parent(parent, identity)
        }
        PublicationState::StagedAndPrior => {
            remove_regular(parent, DEFAULT_STAGED_FILE_NAME, identity)?;
            renameat(
                parent,
                DEFAULT_PRIOR_FILE_NAME,
                parent,
                DEFAULT_ACTIVE_FILE_NAME,
            )
            .map_err(|_| quarantine(identity, "migration-rollback-rename-failed"))?;
            sync_parent(parent, identity)
        }
        PublicationState::ActiveAndPrior => {
            remove_regular(parent, DEFAULT_ACTIVE_FILE_NAME, identity)?;
            renameat(
                parent,
                DEFAULT_PRIOR_FILE_NAME,
                parent,
                DEFAULT_ACTIVE_FILE_NAME,
            )
            .map_err(|_| quarantine(identity, "migration-rollback-rename-failed"))?;
            sync_parent(parent, identity)
        }
        PublicationState::ActiveOnly | PublicationState::Empty => Ok(()),
        PublicationState::PriorOnly | PublicationState::AllPresent => {
            Err(quarantine(identity, "migration-rollback-state-ambiguous"))
        }
    }
}

fn finalize_prior(parent: &File, identity: &StoreIdentity) -> Result<(), StoreError> {
    if matches!(
        entry_type(parent, DEFAULT_PRIOR_FILE_NAME)?,
        Some(FileType::RegularFile)
    ) {
        remove_regular(parent, DEFAULT_PRIOR_FILE_NAME, identity)?;
        sync_parent(parent, identity)?;
    } else if entry_type(parent, DEFAULT_PRIOR_FILE_NAME)?.is_some() {
        return Err(quarantine(identity, "migration-prior-not-regular"));
    }
    Ok(())
}

fn ensure_active_is_safe_or_absent(
    parent: &File,
    identity: &StoreIdentity,
) -> Result<(), StoreError> {
    if entry_type(parent, DEFAULT_ACTIVE_FILE_NAME)?.is_some() {
        validate_named_current(parent, DEFAULT_ACTIVE_FILE_NAME, identity)?;
    }
    Ok(())
}

fn validate_named_current(
    parent: &File,
    name: &str,
    identity: &StoreIdentity,
) -> Result<(), StoreError> {
    validate_named_database(
        parent,
        name,
        identity,
        Some(CURRENT_PHYSICAL_SCHEMA_VERSION),
    )
}

fn validate_named_supported(
    parent: &File,
    name: &str,
    identity: &StoreIdentity,
) -> Result<(), StoreError> {
    validate_named_database(parent, name, identity, None)
}

fn validate_named_database(
    parent: &File,
    name: &str,
    identity: &StoreIdentity,
    expected_version: Option<u32>,
) -> Result<(), StoreError> {
    let database = open_named_database(parent, name, identity)?;
    let meta = read_meta(&database, identity)?;
    if expected_version.is_some_and(|expected| meta.schema_version != expected)
        || expected_version.is_none()
            && meta.schema_version != CURRENT_PHYSICAL_SCHEMA_VERSION
            && !REGISTERED_MIGRATIONS
                .iter()
                .any(|step| step.from == meta.schema_version)
    {
        return Err(unsupported_version(meta.schema_version).with_store_slot(identity.slot()));
    }
    validate_identity_fields(&meta, identity)?;
    validate_consistency(&database)
        .map_err(|_| quarantine(identity, "migration-database-corrupt"))?;
    Ok(())
}

fn validate_database(
    database: &Database,
    identity: &StoreIdentity,
    expected_version: Option<u32>,
) -> Result<(), StoreError> {
    let meta = read_meta(database, identity)?;
    if expected_version.is_some_and(|expected| meta.schema_version != expected)
        || expected_version.is_none()
            && meta.schema_version != CURRENT_PHYSICAL_SCHEMA_VERSION
            && !REGISTERED_MIGRATIONS
                .iter()
                .any(|step| step.from == meta.schema_version)
    {
        return Err(unsupported_version(meta.schema_version).with_store_slot(identity.slot()));
    }
    validate_identity_fields(&meta, identity)?;
    validate_consistency(database).map_err(|_| quarantine(identity, "migration-database-corrupt"))
}

fn validate_identity_fields(meta: &StoreMeta, identity: &StoreIdentity) -> Result<(), StoreError> {
    if meta.store_uuid != identity.store_uuid.as_str()
        || meta.zone_name != identity.zone.as_str()
        || meta.zone_uid != identity.zone_uid.as_str()
        || meta.created_at != identity.created_at
        || meta.compaction_floor > meta.current_revision
        || meta.active_configuration_revision
            != identity.revisions.active_configuration_revision.get()
        || meta.policy_revision != identity.revisions.policy_revision
        || meta.api_catalog_revision != identity.revisions.api_catalog_revision
        || meta.controller_generation
            != identity
                .revisions
                .controller_generation
                .map(|generation| generation.get())
    {
        return Err(quarantine(identity, "migration-store-identity-mismatch"));
    }
    Ok(())
}

fn read_meta(database: &Database, identity: &StoreIdentity) -> Result<StoreMeta, StoreError> {
    crate::transaction::read_meta(
        &database
            .begin_read()
            .map_err(|_| quarantine(identity, "migration-meta-read-failed"))?,
    )
    .map_err(|_| quarantine(identity, "migration-meta-invalid"))
}

fn open_named_database(
    parent: &File,
    name: &str,
    identity: &StoreIdentity,
) -> Result<Database, StoreError> {
    let file = open_named_file(parent, name, identity)?;
    if file
        .metadata()
        .map_err(|_| quarantine(identity, "migration-database-stat-failed"))?
        .len()
        == 0
    {
        return Err(quarantine(identity, "migration-database-empty"));
    }
    let backend = FileBackend::new(file)
        .map_err(|_| quarantine(identity, "migration-database-open-failed"))?;
    Database::builder()
        .set_cache_size(REDB_CACHE_SIZE)
        .create_with_backend(backend)
        .map_err(|_| quarantine(identity, "migration-database-open-failed"))
}

fn open_named_file(
    parent: &File,
    name: &str,
    identity: &StoreIdentity,
) -> Result<File, StoreError> {
    let fd = openat(
        parent,
        name,
        OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(|_| quarantine(identity, "migration-file-open-failed"))?;
    let file = File::from(fd);
    crate::validate_owned_file(&file)
        .map_err(|_| quarantine(identity, "migration-file-posture-invalid"))?;
    Ok(file)
}

fn create_stage(parent: &File, identity: &StoreIdentity) -> Result<File, StoreError> {
    match entry_type(parent, DEFAULT_STAGED_FILE_NAME)? {
        None => {}
        Some(FileType::RegularFile) => {
            return Err(quarantine(identity, "migration-staged-already-present"));
        }
        Some(_) => return Err(quarantine(identity, "migration-staged-not-regular")),
    }
    let fd = openat(
        parent,
        DEFAULT_STAGED_FILE_NAME,
        OFlags::CREATE | OFlags::EXCL | OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|_| quarantine(identity, "migration-staged-create-failed"))?;
    let file = File::from(fd);
    crate::validate_owned_file(&file)
        .map_err(|_| quarantine(identity, "migration-staged-posture-invalid"))?;
    Ok(file)
}

fn sync_named_stage(parent: &File, identity: &StoreIdentity) -> Result<(), StoreError> {
    let staged = open_named_file(parent, DEFAULT_STAGED_FILE_NAME, identity)?;
    backup::sync_staged_file(&staged, parent)
        .map_err(|_| quarantine(identity, "migration-staged-sync-failed"))
}

fn entry_type(parent: &File, name: &str) -> Result<Option<FileType>, StoreError> {
    match statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => Ok(Some(FileType::from_raw_mode(stat.st_mode))),
        Err(error) if error == rustix::io::Errno::NOENT => Ok(None),
        Err(_) => Err(integrity("migration-publication-stat-failed")),
    }
}

fn remove_regular(parent: &File, name: &str, identity: &StoreIdentity) -> Result<(), StoreError> {
    if !matches!(entry_type(parent, name)?, Some(FileType::RegularFile)) {
        return Err(quarantine(identity, "migration-remove-nonregular"));
    }
    unlinkat(parent, name, AtFlags::empty())
        .map_err(|_| quarantine(identity, "migration-remove-failed"))
}

fn sync_parent(parent: &File, identity: &StoreIdentity) -> Result<(), StoreError> {
    fsync(parent).map_err(|_| quarantine(identity, "migration-parent-fsync-failed"))
}

fn validate_parent(parent: &File) -> Result<(), StoreError> {
    if !parent
        .metadata()
        .map_err(|_| integrity("migration-parent-stat-failed"))?
        .file_type()
        .is_dir()
    {
        return Err(integrity("migration-parent-not-directory"));
    }
    if !fcntl_getfd(parent)
        .map_err(|_| integrity("migration-parent-fd-invalid"))?
        .contains(FdFlags::CLOEXEC)
    {
        return Err(integrity("migration-parent-fd-missing-cloexec"));
    }
    Ok(())
}

fn validate_marker(marker: &mut File, identity: &StoreIdentity) -> Result<(), StoreError> {
    crate::validate_provisioning_marker(marker, identity)
        .map_err(|_| quarantine(identity, "migration-marker-identity-mismatch"))
}

fn unsupported_version(version: u32) -> StoreError {
    let _ = version;
    StoreError::new(
        StoreErrorKind::UpgradeRequired,
        None,
        None,
        RetryClass::Never,
        "physical-schema-version-unsupported",
    )
}

fn quarantine(identity: &StoreIdentity, reason: &'static str) -> StoreError {
    crate::transaction::quarantined_reason(reason).with_store_slot(identity.slot())
}

fn injected_fault(reason: &'static str, identity: &StoreIdentity) -> StoreError {
    StoreError::new(
        StoreErrorKind::StoreIntegrityFailure,
        None,
        None,
        RetryClass::Never,
        reason,
    )
    .with_store_slot(identity.slot())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicationBoundary {
    AfterStageSync,
    AfterPriorRename,
    AfterActiveRename,
    AfterFinalSync,
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::{ConfigurationGeneration, ResourceUid, Timestamp, ZoneId};
    use d2b_resource_store::PolicySnapshot;
    use std::fs::OpenOptions;

    fn identity() -> StoreIdentity {
        StoreIdentity::new(
            d2b_resource_store::StoreSlot::new(0).unwrap(),
            ResourceUid::parse("11111111-1111-4111-8111-111111111111").unwrap(),
            ZoneId::parse("work").unwrap(),
            ResourceUid::parse("22222222-2222-4222-8222-222222222222").unwrap(),
            Timestamp::parse("2026-07-31T00:00:00.000Z").unwrap(),
            PolicySnapshot {
                policy_revision: 7,
                api_catalog_revision: 8,
                active_configuration_revision: ConfigurationGeneration::new(9).unwrap(),
                controller_generation: None,
            },
        )
    }

    fn parent() -> (tempfile::TempDir, File, File) {
        let directory = tempfile::tempdir().unwrap();
        let parent = File::open(directory.path()).unwrap();
        let mut marker = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(directory.path().join("store.marker"))
            .unwrap();
        crate::write_provisioning_marker(&mut marker, &identity()).unwrap();
        (directory, parent, marker)
    }

    fn empty_backup() -> LogicalBackup {
        let directory = tempfile::tempdir().unwrap();
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(directory.path().join("source.redb"))
            .unwrap();
        let database = Database::builder()
            .set_cache_size(REDB_CACHE_SIZE)
            .create_with_backend(FileBackend::new(file).unwrap())
            .unwrap();
        let identity = identity();
        crate::transaction::initialize(&database, &identity).unwrap();
        LogicalBackup::from_database(&database, &identity).unwrap()
    }

    fn create_current_file(directory: &tempfile::TempDir, name: &str) {
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(directory.path().join(name))
            .unwrap();
        let database = Database::builder()
            .set_cache_size(REDB_CACHE_SIZE)
            .create_with_backend(FileBackend::new(file).unwrap())
            .unwrap();
        crate::transaction::initialize(&database, &identity()).unwrap();
        drop(database);
    }

    fn set_schema_version(directory: &tempfile::TempDir, version: u32) {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(directory.path().join(DEFAULT_ACTIVE_FILE_NAME))
            .unwrap();
        let database = Database::builder()
            .set_cache_size(REDB_CACHE_SIZE)
            .create_with_backend(FileBackend::new(file).unwrap())
            .unwrap();
        let mut write = database.begin_write().unwrap();
        write.set_durability(Durability::Immediate).unwrap();
        let mut meta: StoreMeta = crate::transaction::read_meta_in_write(&write).unwrap();
        meta.schema_version = version;
        let value = encode(ValueKind::StoreMetaScalar, &meta).unwrap();
        write
            .open_table(STORE_META)
            .unwrap()
            .insert(meta_key().as_slice(), value.as_slice())
            .unwrap();
        write.commit().unwrap();
    }

    fn create_staged_current(directory: &tempfile::TempDir) {
        create_current_file(directory, DEFAULT_STAGED_FILE_NAME);
        let parent = File::open(directory.path()).unwrap();
        let staged = OpenOptions::new()
            .read(true)
            .write(true)
            .open(directory.path().join(DEFAULT_STAGED_FILE_NAME))
            .unwrap();
        backup::sync_staged_file(&staged, &parent).unwrap();
    }

    #[test]
    fn registered_chain_is_explicit_and_unknown_versions_refuse() {
        assert_eq!(REGISTERED_MIGRATIONS.len(), 1);
        assert_eq!(
            migration_chain(0).unwrap(),
            vec![MigrationStep {
                from: 0,
                to: CURRENT_PHYSICAL_SCHEMA_VERSION,
                name: "physical-schema-v0-to-v1",
            }]
        );
        assert!(migration_chain(2).unwrap_err().kind() == StoreErrorKind::UpgradeRequired);
    }

    #[test]
    fn restore_publishes_only_after_staged_validation() {
        let (directory, parent, mut marker) = parent();
        let backup = empty_backup();
        let outcome = restore_owned(&parent, &mut marker, &backup, &identity()).unwrap();
        assert_eq!(outcome, MigrationOutcome::Restored);
        assert_eq!(
            backup::publication_state(
                &parent,
                DEFAULT_STAGED_FILE_NAME,
                DEFAULT_ACTIVE_FILE_NAME,
                DEFAULT_PRIOR_FILE_NAME
            )
            .unwrap(),
            PublicationState::ActiveOnly
        );
        let file = open_named_database(&parent, DEFAULT_ACTIVE_FILE_NAME, &identity()).unwrap();
        validate_database(&file, &identity(), Some(CURRENT_PHYSICAL_SCHEMA_VERSION)).unwrap();
        drop(file);
        drop(directory);
    }

    #[test]
    fn version_zero_upgrade_uses_a_new_file_and_is_idempotent() {
        let (directory, parent, mut marker) = parent();
        create_current_file(&directory, DEFAULT_ACTIVE_FILE_NAME);
        set_schema_version(&directory, 0);
        let outcome = upgrade_owned(&parent, &mut marker, &identity()).unwrap();
        assert_eq!(
            outcome,
            MigrationOutcome::Upgraded {
                from: 0,
                to: CURRENT_PHYSICAL_SCHEMA_VERSION
            }
        );
        assert_eq!(
            upgrade_owned(&parent, &mut marker, &identity()).unwrap(),
            MigrationOutcome::AlreadyCurrent
        );
        assert_eq!(
            backup::publication_state(
                &parent,
                DEFAULT_STAGED_FILE_NAME,
                DEFAULT_ACTIVE_FILE_NAME,
                DEFAULT_PRIOR_FILE_NAME
            )
            .unwrap(),
            PublicationState::ActiveOnly
        );
    }

    #[test]
    fn every_publication_boundary_rolls_back_without_changing_active() {
        for boundary in [
            PublicationBoundary::AfterStageSync,
            PublicationBoundary::AfterPriorRename,
            PublicationBoundary::AfterActiveRename,
            PublicationBoundary::AfterFinalSync,
        ] {
            let (directory, parent, _marker) = parent();
            create_current_file(&directory, DEFAULT_ACTIVE_FILE_NAME);
            create_staged_current(&directory);
            let error = publish_with_rollback(&parent, Some(boundary), &identity()).unwrap_err();
            assert!(error.reason_code().contains("migration-fault"));
            assert_eq!(
                backup::publication_state(
                    &parent,
                    DEFAULT_STAGED_FILE_NAME,
                    DEFAULT_ACTIVE_FILE_NAME,
                    DEFAULT_PRIOR_FILE_NAME
                )
                .unwrap(),
                PublicationState::ActiveOnly
            );
            validate_named_current(&parent, DEFAULT_ACTIVE_FILE_NAME, &identity()).unwrap();
        }
    }

    #[test]
    fn crash_states_resume_idempotently_and_corruption_quarantines() {
        let (directory, parent, mut marker) = parent();
        create_current_file(&directory, DEFAULT_ACTIVE_FILE_NAME);
        create_staged_current(&directory);
        renameat(
            &parent,
            DEFAULT_ACTIVE_FILE_NAME,
            &parent,
            DEFAULT_PRIOR_FILE_NAME,
        )
        .unwrap();
        sync_parent(&parent, &identity()).unwrap();
        assert_eq!(
            recover_owned(&parent, &mut marker, &identity()).unwrap(),
            RecoveryOutcome::Resumed
        );
        assert_eq!(
            recover_owned(&parent, &mut marker, &identity()).unwrap(),
            RecoveryOutcome::Clean
        );

        let (corrupt_directory, corrupt_parent, mut corrupt_marker) = parent();
        let corrupt = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(corrupt_directory.path().join(DEFAULT_STAGED_FILE_NAME))
            .unwrap();
        corrupt.sync_all().unwrap();
        let error = recover_owned(&corrupt_parent, &mut corrupt_marker, &identity()).unwrap_err();
        assert_eq!(error.kind(), StoreErrorKind::StoreQuarantined);
    }

    #[test]
    fn marker_and_identity_mismatch_quarantine_before_stage_creation() {
        let (directory, parent, mut marker) = parent();
        let other = StoreIdentity::new(
            d2b_resource_store::StoreSlot::new(0).unwrap(),
            ResourceUid::parse("33333333-3333-4333-8333-333333333333").unwrap(),
            ZoneId::parse("work").unwrap(),
            ResourceUid::parse("22222222-2222-4222-8222-222222222222").unwrap(),
            Timestamp::parse("2026-07-31T00:00:00.000Z").unwrap(),
            identity().revisions.clone(),
        );
        let error = restore_owned(&parent, &mut marker, &empty_backup(), &other).unwrap_err();
        assert_eq!(error.kind(), StoreErrorKind::StoreQuarantined);
        assert_eq!(
            backup::publication_state(
                &parent,
                DEFAULT_STAGED_FILE_NAME,
                DEFAULT_ACTIVE_FILE_NAME,
                DEFAULT_PRIOR_FILE_NAME
            )
            .unwrap(),
            PublicationState::Empty
        );
        drop(directory);
    }
}
