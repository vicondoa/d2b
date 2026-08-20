//! Crash-safe logical restore and physical-schema migration.
//!
//! The storage owner passes this module an already-open parent directory and
//! identity marker.  It never receives a caller-controlled path.  A restore
//! or upgrade is built in a new sibling file, validated as a complete store,
//! synced, and published only by fd-relative renames.  The old file is kept
//! until the new file has been validated and the parent directory has been
//! synced.

use std::fs::File;
use std::io::{Read as _, Write as _};

use d2b_contracts_resource::v3::RetryClass;
use d2b_resource_store::{StoreError, StoreErrorKind};
use redb::backends::FileBackend;
use redb::{Database, Durability, ReadableDatabase, ReadableTable};
use rustix::fs::{
    AtFlags, FileType, Mode, OFlags, RenameFlags, fsync, openat, renameat_with, statat, unlinkat,
};
use rustix::io::{FdFlags, fcntl_getfd};

use crate::backup::{self, LogicalBackup, PublicationState};
use crate::transaction::{
    ALL_TABLES, OperationRecord, PHYSICAL_SCHEMA_VERSION, StoreMeta, decode, encode, integrity,
    meta_key,
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

const STAGED_PREPARED_MARKER_FILE_NAME: &str = "store.redb.staged.prepared";
const STAGED_PREPARED_MARKER_TEMP_FILE_NAME: &str = "store.redb.staged.prepared.tmp";

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
/// assignments but an unversioned metadata record. Version two adds the
/// authority lifecycle row shape and its explicit closing state.
pub const REGISTERED_MIGRATIONS: &[MigrationStep] = &[
    MigrationStep {
        from: 0,
        to: 1,
        name: "physical-schema-v0-to-v1",
    },
    MigrationStep {
        from: 1,
        to: 2,
        name: "physical-schema-v1-to-v2-authority-lifecycle",
    },
];

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
    if let Err(error) = backup.validate_for_identity(identity) {
        if error.kind() == StoreErrorKind::UpgradeRequired {
            return Err(error.with_store_slot(identity.slot()));
        }
        return Err(quarantine(identity, "migration-backup-identity-mismatch"));
    }

    let next_backup_generation = next_backup_generation(parent, backup, identity)?;
    prepare_restore_stage(parent, backup, identity, next_backup_generation)?;
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
    // Legacy outboxes intentionally predate the current U4 fields. Validate
    // the physical envelope and identity here, then normalize the staged
    // copy before applying current-schema consistency checks.
    validate_database(&source, identity, Some(source_meta.schema_version))?;
    prepare_upgrade_stage(&source, parent, identity, source_meta.schema_version)?;
    drop(source);

    publish_with_rollback(parent, None, identity)?;
    validate_named_current(parent, DEFAULT_ACTIVE_FILE_NAME, identity)?;
    finalize_prior(parent, identity)?;
    let last = chain.last().expect("nonempty migration chain");
    Ok(MigrationOutcome::Upgraded {
        from: source_meta.schema_version,
        to: last.to,
    })
}

/// Upgrade only after the caller has captured and identity-validated a
/// logical image of the active store.
///
/// The backup is intentionally supplied by value so a storage owner cannot
/// accidentally advance the physical schema without first crossing the
/// logical recovery boundary. The active file is still validated by
/// [`upgrade_owned`] before staged publication.
pub fn upgrade_owned_after_backup(
    parent: &File,
    marker: &mut File,
    identity: &StoreIdentity,
    backup: LogicalBackup,
) -> Result<MigrationOutcome, StoreError> {
    backup.validate_for_upgrade_identity(identity)?;
    upgrade_owned(parent, marker, identity)
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
    let marker = inspect_stage_prepared_marker(parent, identity)?;
    let marker_temp_present = match entry_type(parent, STAGED_PREPARED_MARKER_TEMP_FILE_NAME)? {
        None => false,
        Some(FileType::RegularFile) => true,
        Some(_) => {
            return Err(quarantine(
                identity,
                "migration-stage-marker-temp-not-regular",
            ));
        }
    };
    match state {
        PublicationState::Empty => match marker {
            StageMarkerState::Absent => {
                if marker_temp_present {
                    remove_stage_prepared_marker_temp(parent, identity)?;
                    sync_parent(parent, identity)?;
                    return Ok(RecoveryOutcome::Finalized);
                }
                Ok(RecoveryOutcome::Clean)
            }
            StageMarkerState::Prepared(_) => {
                discard_marker_residue(parent, identity)?;
                Ok(RecoveryOutcome::Finalized)
            }
            StageMarkerState::Invalid => {
                Err(quarantine(identity, "migration-stage-marker-without-stage"))
            }
            StageMarkerState::Unsafe => Err(quarantine(
                identity,
                "migration-stage-marker-identity-mismatch",
            )),
        },
        PublicationState::ActiveOnly => {
            if matches!(&marker, StageMarkerState::Unsafe) {
                return Err(quarantine(
                    identity,
                    "migration-stage-marker-identity-mismatch",
                ));
            }
            validate_named_supported(parent, DEFAULT_ACTIVE_FILE_NAME, identity)?;
            match marker {
                StageMarkerState::Absent => {
                    if marker_temp_present {
                        remove_stage_prepared_marker_temp(parent, identity)?;
                        sync_parent(parent, identity)?;
                        return Ok(RecoveryOutcome::Finalized);
                    }
                    Ok(RecoveryOutcome::Clean)
                }
                StageMarkerState::Prepared(_) | StageMarkerState::Invalid => {
                    discard_marker_residue(parent, identity)?;
                    Ok(RecoveryOutcome::Finalized)
                }
                StageMarkerState::Unsafe => Err(quarantine(
                    identity,
                    "migration-stage-marker-identity-mismatch",
                )),
            }
        }
        PublicationState::ActiveAndStaged => {
            if matches!(&marker, StageMarkerState::Unsafe) {
                return Err(quarantine(
                    identity,
                    "migration-stage-marker-identity-mismatch",
                ));
            }
            validate_named_supported(parent, DEFAULT_ACTIVE_FILE_NAME, identity)?;
            match marker {
                StageMarkerState::Absent | StageMarkerState::Invalid => {
                    discard_unpublished_stage(parent, identity)?;
                    Ok(RecoveryOutcome::Clean)
                }
                StageMarkerState::Unsafe => unreachable!("unsafe marker returned above"),
                StageMarkerState::Prepared(_) => {
                    validate_named_prepared_current(parent, identity)?;
                    publish_with_rollback(parent, None, identity)?;
                    validate_named_current(parent, DEFAULT_ACTIVE_FILE_NAME, identity)?;
                    finalize_prior(parent, identity)?;
                    Ok(RecoveryOutcome::Resumed)
                }
            }
        }
        PublicationState::StagedOnly => match marker {
            StageMarkerState::Absent => {
                discard_unpublished_stage(parent, identity)?;
                Ok(RecoveryOutcome::Clean)
            }
            StageMarkerState::Invalid => Err(quarantine(
                identity,
                "migration-unprepared-stage-without-active",
            )),
            StageMarkerState::Unsafe => Err(quarantine(
                identity,
                "migration-stage-marker-identity-mismatch",
            )),
            StageMarkerState::Prepared(_) => {
                validate_named_prepared_current(parent, identity)?;
                publish_with_rollback(parent, None, identity)?;
                validate_named_current(parent, DEFAULT_ACTIVE_FILE_NAME, identity)?;
                Ok(RecoveryOutcome::Resumed)
            }
        },
        PublicationState::StagedAndPrior => {
            if !matches!(&marker, StageMarkerState::Prepared(_)) {
                return Err(match marker {
                    StageMarkerState::Unsafe => {
                        quarantine(identity, "migration-stage-marker-identity-mismatch")
                    }
                    _ => quarantine(identity, "migration-unprepared-stage-without-active"),
                });
            }
            validate_named_prepared_current(parent, identity)?;
            validate_named_supported(parent, DEFAULT_PRIOR_FILE_NAME, identity)?;
            resume_staged_with_prior(parent, identity)?;
            validate_named_current(parent, DEFAULT_ACTIVE_FILE_NAME, identity)?;
            remove_stage_prepared_marker(parent, identity)?;
            remove_stage_prepared_marker_temp(parent, identity)?;
            sync_parent(parent, identity)?;
            finalize_prior(parent, identity)?;
            Ok(RecoveryOutcome::Resumed)
        }
        PublicationState::ActiveAndPrior => {
            if matches!(&marker, StageMarkerState::Unsafe) {
                return Err(quarantine(
                    identity,
                    "migration-stage-marker-identity-mismatch",
                ));
            }
            validate_named_current(parent, DEFAULT_ACTIVE_FILE_NAME, identity)?;
            validate_named_supported(parent, DEFAULT_PRIOR_FILE_NAME, identity)?;
            let marker_cleanup = matches!(
                &marker,
                StageMarkerState::Prepared(_) | StageMarkerState::Invalid
            );
            match marker {
                StageMarkerState::Unsafe => unreachable!("unsafe marker returned above"),
                StageMarkerState::Absent => {}
                StageMarkerState::Prepared(_) | StageMarkerState::Invalid => {
                    discard_marker_residue(parent, identity)?;
                }
            }
            if marker_temp_present && !marker_cleanup {
                remove_stage_prepared_marker_temp(parent, identity)?;
                sync_parent(parent, identity)?;
            }
            finalize_prior(parent, identity)?;
            Ok(RecoveryOutcome::Finalized)
        }
        PublicationState::PriorOnly | PublicationState::AllPresent => Err(quarantine(
            identity,
            "migration-publication-state-ambiguous",
        )),
    }
}

fn next_backup_generation(
    parent: &File,
    backup: &LogicalBackup,
    identity: &StoreIdentity,
) -> Result<u64, StoreError> {
    let current_generation = match entry_type(parent, DEFAULT_ACTIVE_FILE_NAME)? {
        None => 0,
        Some(FileType::RegularFile) => {
            let active = open_named_database(parent, DEFAULT_ACTIVE_FILE_NAME, identity)?;
            read_meta(&active, identity)?.backup_generation
        }
        Some(_) => return Err(quarantine(identity, "migration-active-not-regular")),
    };
    current_generation
        .max(backup.backup_generation)
        .checked_add(1)
        .ok_or_else(|| quarantine(identity, "migration-backup-generation-exhausted"))
}

fn prepare_restore_stage(
    parent: &File,
    backup: &LogicalBackup,
    identity: &StoreIdentity,
    next_backup_generation: u64,
) -> Result<(), StoreError> {
    prepare_restore_stage_with_fault(parent, backup, identity, next_backup_generation, None)
}

fn prepare_restore_stage_with_fault(
    parent: &File,
    backup: &LogicalBackup,
    identity: &StoreIdentity,
    next_backup_generation: u64,
    fault: Option<PublicationBoundary>,
) -> Result<(), StoreError> {
    let staged = create_stage(parent, identity)?;
    inject_boundary_fault(fault, PublicationBoundary::StageCreate, identity)?;
    let result = (|| {
        inject_boundary_fault(fault, PublicationBoundary::StageDatabaseCreate, identity)?;
        let database = backup
            .restore_file_with_generation(staged, identity, next_backup_generation)
            .map_err(|error| {
                if error.reason_code() == crate::transaction::UNINTERPRETABLE_REQUEST_DIGEST_REASON
                {
                    error.with_store_slot(identity.slot())
                } else {
                    quarantine(identity, "migration-staged-restore-invalid")
                }
            })?;
        inject_boundary_fault(fault, PublicationBoundary::StageDatabaseCommit, identity)?;
        drop(database);
        finish_stage_preparation_with_fault(parent, identity, fault)
    })();
    cleanup_failed_stage_with_fault(parent, identity, result, fault)
}

fn prepare_upgrade_stage(
    source: &Database,
    parent: &File,
    identity: &StoreIdentity,
    from: u32,
) -> Result<(), StoreError> {
    prepare_upgrade_stage_with_fault(source, parent, identity, from, None)
}

fn prepare_upgrade_stage_with_fault(
    source: &Database,
    parent: &File,
    identity: &StoreIdentity,
    from: u32,
    fault: Option<PublicationBoundary>,
) -> Result<(), StoreError> {
    let staged = create_stage(parent, identity)?;
    inject_boundary_fault(fault, PublicationBoundary::StageCreate, identity)?;
    let result = (|| {
        let backend = FileBackend::new(staged)
            .map_err(|_| quarantine(identity, "migration-staged-open-failed"))?;
        let target = Database::builder()
            .set_cache_size(REDB_CACHE_SIZE)
            .create_with_backend(backend)
            .map_err(|_| quarantine(identity, "migration-staged-open-failed"))?;
        inject_boundary_fault(fault, PublicationBoundary::StageDatabaseCreate, identity)?;
        copy_registered_step(source, &target, from, identity)?;
        inject_boundary_fault(fault, PublicationBoundary::StageDatabaseCommit, identity)?;
        drop(target);
        // Normalize U4 outbox identity, mutation, and timestamp fields in
        // the staged copy before the strict current-schema validation runs.
        let staged_database = open_named_database(parent, DEFAULT_STAGED_FILE_NAME, identity)?;
        crate::transaction::normalize_audit_outboxes(&staged_database).map_err(|error| {
            if error.reason_code() == crate::transaction::UNINTERPRETABLE_REQUEST_DIGEST_REASON {
                error.with_store_slot(identity.slot())
            } else {
                quarantine(identity, "migration-audit-outbox-normalization-failed")
            }
        })?;
        drop(staged_database);
        finish_stage_preparation_with_fault(parent, identity, fault)
    })();
    cleanup_failed_stage_with_fault(parent, identity, result, fault)
}

fn finish_stage_preparation_with_fault(
    parent: &File,
    identity: &StoreIdentity,
    fault: Option<PublicationBoundary>,
) -> Result<(), StoreError> {
    validate_named_current(parent, DEFAULT_STAGED_FILE_NAME, identity)?;
    sync_named_stage_with_fault(parent, identity, fault)?;
    mark_stage_prepared_with_fault(parent, identity, fault)
}

fn cleanup_failed_stage_with_fault<T>(
    parent: &File,
    identity: &StoreIdentity,
    result: Result<T, StoreError>,
    fault: Option<PublicationBoundary>,
) -> Result<T, StoreError> {
    let Err(error) = result else {
        return result;
    };
    remove_stage_prepared_marker(parent, identity)?;
    inject_boundary_fault(fault, PublicationBoundary::MarkerRemove, identity)?;
    sync_parent(parent, identity)?;
    remove_stage_prepared_marker_temp(parent, identity)?;
    inject_boundary_fault(fault, PublicationBoundary::MarkerTempRemove, identity)?;
    sync_parent(parent, identity)?;
    remove_regular_if_present(parent, DEFAULT_STAGED_FILE_NAME, identity)?;
    inject_boundary_fault(fault, PublicationBoundary::StageRemove, identity)?;
    sync_parent(parent, identity)?;
    Err(error)
}

fn copy_registered_step(
    source: &Database,
    target: &Database,
    from: u32,
    identity: &StoreIdentity,
) -> Result<(), StoreError> {
    let chain = migration_chain(from)?;
    if chain
        .last()
        .is_none_or(|step| step.to != CURRENT_PHYSICAL_SCHEMA_VERSION)
    {
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
            if from == 1 && table_index == 8 {
                validate_v2_authority_payload(&value_bytes).map_err(|_| {
                    quarantine(identity, "migration-authority-payload-incompatible")
                })?;
            }
            if target_table
                .insert(key.value(), value_bytes.as_slice())
                .map_err(|_| quarantine(identity, "migration-staged-row-invalid"))?
                .is_some()
            {
                return Err(quarantine(identity, "migration-staged-duplicate-key"));
            }

            fn validate_v2_authority_payload(bytes: &[u8]) -> Result<(), StoreError> {
                let operation: OperationRecord = decode(ValueKind::OperationRecord, bytes)
                    .map_err(|_| integrity("authority-operation-payload-invalid"))?;
                let Some(authority) = operation.authority else {
                    return Ok(());
                };
                let payload: serde_json::Value = serde_json::from_slice(&authority.payload)
                    .map_err(|_| integrity("authority-operation-payload-invalid"))?;
                let object = payload
                    .as_object()
                    .ok_or_else(|| integrity("authority-operation-payload-invalid"))?;
                for key in [
                    "operationId",
                    "claim",
                    "state",
                    "claimDigest",
                    "storeBindingDigest",
                ] {
                    if !object.contains_key(key) {
                        return Err(integrity("authority-operation-payload-incompatible"));
                    }
                }
                if !matches!(
                    object.get("state").and_then(serde_json::Value::as_str),
                    Some(
                        "pending"
                            | "effect-confirmed"
                            | "effect-retryable"
                            | "effect-terminal"
                            | "closing"
                            | "closed"
                            | "released"
                    )
                ) {
                    return Err(integrity("authority-operation-payload-incompatible"));
                }
                for key in ["claimDigest", "storeBindingDigest"] {
                    if !object
                        .get(key)
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(valid_digest_text)
                    {
                        return Err(integrity("authority-operation-payload-incompatible"));
                    }
                }
                Ok(())
            }

            fn valid_digest_text(value: &str) -> bool {
                value.strip_prefix("sha256:").is_some_and(|hex| {
                    hex.len() == 64
                        && hex
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
                })
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
    validate_named_prepared_current(parent, identity)?;
    sync_named_stage_with_fault(parent, identity, fault)?;
    if matches!(state, PublicationState::ActiveAndStaged) {
        renameat_with(
            parent,
            DEFAULT_ACTIVE_FILE_NAME,
            parent,
            DEFAULT_PRIOR_FILE_NAME,
            RenameFlags::NOREPLACE,
        )
        .map_err(|_| quarantine(identity, "migration-prior-rename-failed"))?;
        sync_parent(parent, identity)?;
        if fault == Some(PublicationBoundary::PriorRename) {
            return Err(injected_fault(
                "migration-fault-after-prior-rename",
                identity,
            ));
        }
    }
    renameat_with(
        parent,
        DEFAULT_STAGED_FILE_NAME,
        parent,
        DEFAULT_ACTIVE_FILE_NAME,
        RenameFlags::NOREPLACE,
    )
    .map_err(|_| quarantine(identity, "migration-active-rename-failed"))?;
    if fault == Some(PublicationBoundary::ActiveRename) {
        return Err(injected_fault(
            "migration-fault-after-active-rename",
            identity,
        ));
    }
    sync_parent(parent, identity)?;
    if fault == Some(PublicationBoundary::FinalSync) {
        return Err(injected_fault("migration-fault-after-final-sync", identity));
    }
    remove_stage_prepared_marker(parent, identity)?;
    inject_boundary_fault(fault, PublicationBoundary::MarkerRemove, identity)?;
    sync_parent(parent, identity)?;
    remove_stage_prepared_marker_temp(parent, identity)?;
    sync_parent(parent, identity)?;
    Ok(())
}

fn resume_staged_with_prior(parent: &File, identity: &StoreIdentity) -> Result<(), StoreError> {
    renameat_with(
        parent,
        DEFAULT_STAGED_FILE_NAME,
        parent,
        DEFAULT_ACTIVE_FILE_NAME,
        RenameFlags::NOREPLACE,
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
            remove_stage_prepared_marker(parent, identity)?;
            remove_stage_prepared_marker_temp(parent, identity)?;
            remove_regular_if_present(parent, DEFAULT_STAGED_FILE_NAME, identity)?;
            sync_parent(parent, identity)
        }
        PublicationState::StagedAndPrior => {
            validate_named_supported(parent, DEFAULT_PRIOR_FILE_NAME, identity)?;
            remove_stage_prepared_marker(parent, identity)?;
            remove_stage_prepared_marker_temp(parent, identity)?;
            remove_regular_if_present(parent, DEFAULT_STAGED_FILE_NAME, identity)?;
            renameat_with(
                parent,
                DEFAULT_PRIOR_FILE_NAME,
                parent,
                DEFAULT_ACTIVE_FILE_NAME,
                RenameFlags::NOREPLACE,
            )
            .map_err(|_| quarantine(identity, "migration-rollback-rename-failed"))?;
            sync_parent(parent, identity)
        }
        PublicationState::ActiveAndPrior => {
            remove_stage_prepared_marker(parent, identity)?;
            remove_stage_prepared_marker_temp(parent, identity)?;
            remove_regular(parent, DEFAULT_ACTIVE_FILE_NAME, identity)?;
            validate_named_supported(parent, DEFAULT_PRIOR_FILE_NAME, identity)?;
            renameat_with(
                parent,
                DEFAULT_PRIOR_FILE_NAME,
                parent,
                DEFAULT_ACTIVE_FILE_NAME,
                RenameFlags::NOREPLACE,
            )
            .map_err(|_| quarantine(identity, "migration-rollback-rename-failed"))?;
            sync_parent(parent, identity)
        }
        PublicationState::ActiveOnly => {
            let marker_present = entry_type(parent, STAGED_PREPARED_MARKER_FILE_NAME)?.is_some();
            let temp_present = entry_type(parent, STAGED_PREPARED_MARKER_TEMP_FILE_NAME)?.is_some();
            if marker_present || temp_present {
                remove_stage_prepared_marker(parent, identity)?;
                remove_stage_prepared_marker_temp(parent, identity)?;
                sync_parent(parent, identity)?;
            }
            Ok(())
        }
        PublicationState::Empty => {
            let marker_present = entry_type(parent, STAGED_PREPARED_MARKER_FILE_NAME)?.is_some();
            let temp_present = entry_type(parent, STAGED_PREPARED_MARKER_TEMP_FILE_NAME)?.is_some();
            if marker_present || temp_present {
                remove_stage_prepared_marker(parent, identity)?;
                remove_stage_prepared_marker_temp(parent, identity)?;
                sync_parent(parent, identity)?;
            }
            Ok(())
        }
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
        validate_named_supported(parent, DEFAULT_PRIOR_FILE_NAME, identity)?;
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

fn validate_named_prepared_current(
    parent: &File,
    identity: &StoreIdentity,
) -> Result<(), StoreError> {
    let database = open_named_database(parent, DEFAULT_STAGED_FILE_NAME, identity)?;
    let meta = validate_database(&database, identity, Some(CURRENT_PHYSICAL_SCHEMA_VERSION))?;
    validate_stage_prepared_marker(parent, identity, &meta)
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
    validate_database(&database, identity, expected_version)?;
    Ok(())
}

fn validate_database(
    database: &Database,
    identity: &StoreIdentity,
    expected_version: Option<u32>,
) -> Result<StoreMeta, StoreError> {
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
    crate::transaction::normalize_and_validate(database, identity, meta.schema_version, true)
        .map_err(|error| map_migration_validation_error(error, identity))
}

fn read_meta(database: &Database, identity: &StoreIdentity) -> Result<StoreMeta, StoreError> {
    crate::transaction::read_meta(
        &database
            .begin_read()
            .map_err(|_| quarantine(identity, "migration-meta-read-failed"))?,
    )
    .map_err(|_| quarantine(identity, "migration-meta-invalid"))
}

fn map_migration_validation_error(error: StoreError, identity: &StoreIdentity) -> StoreError {
    if error.reason_code() == crate::transaction::UNINTERPRETABLE_REQUEST_DIGEST_REASON
        || error.kind() == StoreErrorKind::UpgradeRequired
    {
        return error.with_store_slot(identity.slot());
    }
    if error.kind() == StoreErrorKind::StoreQuarantined {
        return error.with_store_slot(identity.slot());
    }
    if error.reason_code() == "store-identity-mismatch" {
        return quarantine(identity, "migration-store-identity-mismatch");
    }
    quarantine(identity, "migration-database-corrupt")
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
    if entry_type(parent, STAGED_PREPARED_MARKER_FILE_NAME)?.is_some() {
        return Err(quarantine(
            identity,
            "migration-stage-marker-already-present",
        ));
    }
    if entry_type(parent, STAGED_PREPARED_MARKER_TEMP_FILE_NAME)?.is_some() {
        return Err(quarantine(
            identity,
            "migration-stage-marker-temp-already-present",
        ));
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

#[cfg(test)]
fn sync_named_stage(parent: &File, identity: &StoreIdentity) -> Result<(), StoreError> {
    sync_named_stage_with_fault(parent, identity, None)
}

fn sync_named_stage_with_fault(
    parent: &File,
    identity: &StoreIdentity,
    fault: Option<PublicationBoundary>,
) -> Result<(), StoreError> {
    let staged = open_named_file(parent, DEFAULT_STAGED_FILE_NAME, identity)?;
    backup::sync_staged_file(&staged, parent)
        .map_err(|_| quarantine(identity, "migration-staged-sync-failed"))?;
    inject_boundary_fault(fault, PublicationBoundary::StageSync, identity)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StageMarkerState {
    Absent,
    Prepared(StoreMeta),
    Invalid,
    Unsafe,
}

fn inspect_stage_prepared_marker(
    parent: &File,
    identity: &StoreIdentity,
) -> Result<StageMarkerState, StoreError> {
    let Some(file_type) = entry_type(parent, STAGED_PREPARED_MARKER_FILE_NAME)? else {
        return Ok(StageMarkerState::Absent);
    };
    if file_type != FileType::RegularFile {
        return Err(quarantine(identity, "migration-stage-marker-not-regular"));
    }
    let marker = open_named_file(parent, STAGED_PREPARED_MARKER_FILE_NAME, identity)?;
    let mut bytes = Vec::new();
    marker
        .take(4096)
        .read_to_end(&mut bytes)
        .map_err(|_| quarantine(identity, "migration-stage-marker-read-failed"))?;
    Ok(parse_stage_prepared_marker(&bytes, identity))
}

fn parse_stage_prepared_marker(bytes: &[u8], identity: &StoreIdentity) -> StageMarkerState {
    let mut lines = bytes.splitn(3, |byte| *byte == b'\n');
    if lines.next() != Some(b"d2b-redb-stage-prepared/v1") {
        return StageMarkerState::Invalid;
    }
    let Some(slot) = lines.next().and_then(|line| line.strip_prefix(b"slot=")) else {
        return StageMarkerState::Invalid;
    };
    let Ok(slot) = std::str::from_utf8(slot) else {
        return StageMarkerState::Invalid;
    };
    let Ok(slot) = slot.parse::<u32>() else {
        return StageMarkerState::Invalid;
    };
    if slot != identity.slot().get() {
        return StageMarkerState::Unsafe;
    }
    let Some(mut metadata) = lines.next() else {
        return StageMarkerState::Invalid;
    };
    if let Some(stripped) = metadata.strip_suffix(b"\n") {
        metadata = stripped;
    }
    let Ok(meta) = serde_json::from_slice::<StoreMeta>(metadata) else {
        return StageMarkerState::Invalid;
    };
    if meta.store_uuid != identity.store_uuid().as_str()
        || meta.zone_name != identity.zone().as_str()
        || meta.zone_uid != identity.zone_uid().as_str()
        || meta.created_at != identity.created_at()
    {
        return StageMarkerState::Unsafe;
    }
    if meta.schema_version != CURRENT_PHYSICAL_SCHEMA_VERSION {
        return StageMarkerState::Invalid;
    }
    StageMarkerState::Prepared(meta)
}

fn discard_marker_residue(parent: &File, identity: &StoreIdentity) -> Result<(), StoreError> {
    let mut changed = false;
    if entry_type(parent, STAGED_PREPARED_MARKER_FILE_NAME)?.is_some() {
        remove_stage_prepared_marker(parent, identity)?;
        changed = true;
    }
    if entry_type(parent, STAGED_PREPARED_MARKER_TEMP_FILE_NAME)?.is_some() {
        remove_stage_prepared_marker_temp(parent, identity)?;
        changed = true;
    }
    if changed {
        sync_parent(parent, identity)?;
    }
    Ok(())
}

fn discard_unpublished_stage(parent: &File, identity: &StoreIdentity) -> Result<(), StoreError> {
    discard_marker_residue(parent, identity)?;
    if entry_type(parent, DEFAULT_STAGED_FILE_NAME)?.is_some() {
        remove_regular(parent, DEFAULT_STAGED_FILE_NAME, identity)?;
        sync_parent(parent, identity)?;
    }
    Ok(())
}

fn remove_stage_prepared_marker_temp(
    parent: &File,
    identity: &StoreIdentity,
) -> Result<(), StoreError> {
    remove_regular_if_present(parent, STAGED_PREPARED_MARKER_TEMP_FILE_NAME, identity)
}

fn remove_regular_if_present(
    parent: &File,
    name: &str,
    identity: &StoreIdentity,
) -> Result<(), StoreError> {
    match entry_type(parent, name)? {
        None => Ok(()),
        Some(FileType::RegularFile) => unlinkat(parent, name, AtFlags::empty())
            .map_err(|_| quarantine(identity, "migration-remove-failed")),
        Some(_) => Err(quarantine(identity, "migration-remove-nonregular")),
    }
}

#[cfg(test)]
fn mark_stage_prepared(parent: &File, identity: &StoreIdentity) -> Result<(), StoreError> {
    mark_stage_prepared_with_fault(parent, identity, None)
}

fn mark_stage_prepared_with_fault(
    parent: &File,
    identity: &StoreIdentity,
    fault: Option<PublicationBoundary>,
) -> Result<(), StoreError> {
    let staged = open_named_database(parent, DEFAULT_STAGED_FILE_NAME, identity)?;
    let meta = read_meta(&staged, identity)?;
    let marker_bytes = stage_prepared_marker_bytes(identity, &meta)?;
    drop(staged);

    let fd = openat(
        parent,
        STAGED_PREPARED_MARKER_TEMP_FILE_NAME,
        OFlags::CREATE | OFlags::EXCL | OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|_| quarantine(identity, "migration-stage-marker-temp-create-failed"))?;
    let mut marker = File::from(fd);
    crate::validate_owned_file(&marker)
        .map_err(|_| quarantine(identity, "migration-stage-marker-temp-posture-invalid"))?;
    inject_boundary_fault(fault, PublicationBoundary::MarkerTempCreate, identity)?;
    marker
        .write_all(&marker_bytes)
        .map_err(|_| quarantine(identity, "migration-stage-marker-temp-write-failed"))?;
    inject_boundary_fault(fault, PublicationBoundary::MarkerTempWrite, identity)?;
    marker
        .sync_all()
        .map_err(|_| quarantine(identity, "migration-stage-marker-temp-sync-failed"))?;
    inject_boundary_fault(fault, PublicationBoundary::MarkerTempSync, identity)?;
    drop(marker);
    renameat_with(
        parent,
        STAGED_PREPARED_MARKER_TEMP_FILE_NAME,
        parent,
        STAGED_PREPARED_MARKER_FILE_NAME,
        RenameFlags::NOREPLACE,
    )
    .map_err(|_| quarantine(identity, "migration-stage-marker-rename-failed"))?;
    inject_boundary_fault(fault, PublicationBoundary::MarkerRename, identity)?;
    sync_parent(parent, identity)?;
    inject_boundary_fault(fault, PublicationBoundary::MarkerSync, identity)
}

fn validate_stage_prepared_marker(
    parent: &File,
    identity: &StoreIdentity,
    meta: &StoreMeta,
) -> Result<(), StoreError> {
    match inspect_stage_prepared_marker(parent, identity)? {
        StageMarkerState::Prepared(actual) if actual == *meta => Ok(()),
        StageMarkerState::Unsafe => Err(quarantine(
            identity,
            "migration-stage-marker-identity-mismatch",
        )),
        StageMarkerState::Absent | StageMarkerState::Prepared(_) | StageMarkerState::Invalid => {
            Err(quarantine(identity, "migration-stage-marker-invalid"))
        }
    }
}

fn stage_prepared_marker_bytes(
    identity: &StoreIdentity,
    meta: &StoreMeta,
) -> Result<Vec<u8>, StoreError> {
    let metadata = serde_json::to_vec(meta)
        .map_err(|_| quarantine(identity, "migration-stage-marker-invalid"))?;
    let mut bytes = format!("d2b-redb-stage-prepared/v1\nslot={}\n", identity.slot()).into_bytes();
    bytes.extend_from_slice(&metadata);
    bytes.push(b'\n');
    Ok(bytes)
}

fn remove_stage_prepared_marker(parent: &File, identity: &StoreIdentity) -> Result<(), StoreError> {
    match entry_type(parent, STAGED_PREPARED_MARKER_FILE_NAME)? {
        None => Ok(()),
        Some(FileType::RegularFile) => {
            unlinkat(parent, STAGED_PREPARED_MARKER_FILE_NAME, AtFlags::empty())
                .map_err(|_| quarantine(identity, "migration-stage-marker-remove-failed"))
        }
        Some(_) => Err(quarantine(identity, "migration-stage-marker-not-regular")),
    }
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
    StageCreate,
    StageDatabaseCreate,
    StageDatabaseCommit,
    StageSync,
    MarkerTempCreate,
    MarkerTempWrite,
    MarkerTempSync,
    MarkerRename,
    MarkerSync,
    MarkerTempRemove,
    MarkerRemove,
    StageRemove,
    PriorRename,
    ActiveRename,
    FinalSync,
}

fn inject_boundary_fault(
    fault: Option<PublicationBoundary>,
    boundary: PublicationBoundary,
    identity: &StoreIdentity,
) -> Result<(), StoreError> {
    if fault == Some(boundary) {
        return Err(injected_fault(
            "migration-fault-at-publication-boundary",
            identity,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_resource::v3::{ConfigurationGeneration, ResourceUid, Timestamp, ZoneId};
    use d2b_resource_store::PolicySnapshot;
    use std::fs::OpenOptions;

    use crate::transaction::STORE_META;

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
        empty_backup_for(&identity())
    }

    fn pre_upgrade_backup() -> LogicalBackup {
        let mut backup = empty_backup();
        backup.schema_version = 1;
        let table = backup
            .tables
            .iter_mut()
            .find(|table| table.name == "store_meta")
            .unwrap();
        let row = table.rows.first_mut().unwrap();
        let mut meta: crate::transaction::StoreMeta =
            crate::transaction::decode(crate::ValueKind::StoreMetaScalar, &row.value).unwrap();
        meta.schema_version = 1;
        row.value = crate::transaction::encode(crate::ValueKind::StoreMetaScalar, &meta).unwrap();
        table.checksum = crate::backup::checksum_rows(&table.rows);
        backup
    }

    fn empty_backup_for(store_identity: &StoreIdentity) -> LogicalBackup {
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
        crate::transaction::initialize(&database, store_identity).unwrap();
        LogicalBackup::from_database(&database, store_identity).unwrap()
    }

    fn create_current_file(directory: &tempfile::TempDir, name: &str) {
        create_current_file_with_identity(directory, name, &identity());
    }

    fn create_current_file_with_identity(
        directory: &tempfile::TempDir,
        name: &str,
        store_identity: &StoreIdentity,
    ) {
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
        crate::transaction::initialize(&database, store_identity).unwrap();
        drop(database);
    }

    fn other_identity() -> StoreIdentity {
        StoreIdentity::new(
            d2b_resource_store::StoreSlot::new(0).unwrap(),
            ResourceUid::parse("33333333-3333-4333-8333-333333333333").unwrap(),
            ZoneId::parse("work").unwrap(),
            ResourceUid::parse("22222222-2222-4222-8222-222222222222").unwrap(),
            Timestamp::parse("2026-07-31T00:00:00.000Z").unwrap(),
            identity().revisions,
        )
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

    fn insert_operation(directory: &tempfile::TempDir, operation_id: &str, request_digest: &str) {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(directory.path().join(DEFAULT_ACTIVE_FILE_NAME))
            .unwrap();
        let database = Database::builder()
            .set_cache_size(REDB_CACHE_SIZE)
            .create_with_backend(FileBackend::new(file).unwrap())
            .unwrap();
        let operation = OperationRecord {
            request_digest: request_digest.to_owned(),
            resource_uids: Vec::new(),
            resources: Vec::new(),
            outcome: "committed".to_owned(),
            error_code: None,
            accepted_revision: 0,
            finished_revision: 0,
            audit_outbox: None,
            authority: None,
        };
        let key = crate::keys::encode_key(
            crate::keys::KeySpace::Operations,
            &[crate::keys::KeyComponent::Text(operation_id)],
        )
        .unwrap();
        let value = crate::transaction::encode(ValueKind::OperationRecord, &operation).unwrap();
        let mut write = database.begin_write().unwrap();
        write.set_durability(Durability::Immediate).unwrap();
        write
            .open_table(crate::transaction::OPERATIONS)
            .unwrap()
            .insert(key.as_bytes(), value.as_slice())
            .unwrap();
        write.commit().unwrap();
    }

    fn insert_legacy_outbox(directory: &tempfile::TempDir, operation_id: &str) {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(directory.path().join(DEFAULT_ACTIVE_FILE_NAME))
            .unwrap();
        let database = Database::builder()
            .set_cache_size(REDB_CACHE_SIZE)
            .create_with_backend(FileBackend::new(file).unwrap())
            .unwrap();
        let operation = OperationRecord {
            request_digest: format!("sha256:{}", "a".repeat(64)),
            resource_uids: Vec::new(),
            resources: Vec::new(),
            outcome: "committed".to_owned(),
            error_code: None,
            accepted_revision: 0,
            finished_revision: 0,
            audit_outbox: Some(crate::transaction::AuditOutboxRecord {
                zone: identity().zone.as_str().to_owned(),
                operation_id: String::new(),
                operation_identity: None,
                correlation_id: "legacy-correlation".to_owned(),
                subject_digest: "legacy-subject".to_owned(),
                policy_revision: 7,
                resulting_revision: 0,
                requires_broker: false,
                mutations: vec![crate::transaction::AuditOutboxMutation {
                    verb: "create".to_owned(),
                    resource_type: "Host".to_owned(),
                    resource_uid: None,
                    target_digest: "legacy-target".to_owned(),
                    generation: 1,
                    expected_revision: 0,
                    mutation_id: String::new(),
                    ordinal: 9,
                    timestamp_ms: 0,
                    outcome: String::new(),
                    error_code: None,
                    previous_hash: None,
                    record_hash: None,
                }],
            }),
            authority: None,
        };
        let key = crate::keys::encode_key(
            crate::keys::KeySpace::Operations,
            &[crate::keys::KeyComponent::Text(operation_id)],
        )
        .unwrap();
        let value = crate::transaction::encode(ValueKind::OperationRecord, &operation).unwrap();
        let mut write = database.begin_write().unwrap();
        write.set_durability(Durability::Immediate).unwrap();
        write
            .open_table(crate::transaction::OPERATIONS)
            .unwrap()
            .insert(key.as_bytes(), value.as_slice())
            .unwrap();
        write.commit().unwrap();
    }

    fn legacy_backup() -> LogicalBackup {
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
        crate::transaction::initialize(&database, &identity()).unwrap();
        let operation = OperationRecord {
            request_digest: format!("sha256:{}", "a".repeat(64)),
            resource_uids: Vec::new(),
            resources: Vec::new(),
            outcome: "committed".to_owned(),
            error_code: None,
            accepted_revision: 0,
            finished_revision: 0,
            audit_outbox: None,
            authority: None,
        };
        let key = crate::keys::encode_key(
            crate::keys::KeySpace::Operations,
            &[crate::keys::KeyComponent::Text("audit-outbox")],
        )
        .unwrap();
        let value = crate::transaction::encode(ValueKind::OperationRecord, &operation).unwrap();
        let mut write = database.begin_write().unwrap();
        write.set_durability(Durability::Immediate).unwrap();
        write
            .open_table(crate::transaction::OPERATIONS)
            .unwrap()
            .insert(key.as_bytes(), value.as_slice())
            .unwrap();
        write.commit().unwrap();
        let mut backup = LogicalBackup::from_database(&database, &identity()).unwrap();
        let legacy = OperationRecord {
            audit_outbox: Some(crate::transaction::AuditOutboxRecord {
                zone: identity().zone.as_str().to_owned(),
                operation_id: String::new(),
                operation_identity: None,
                correlation_id: "legacy-correlation".to_owned(),
                subject_digest: "legacy-subject".to_owned(),
                policy_revision: 7,
                resulting_revision: 0,
                requires_broker: false,
                mutations: vec![crate::transaction::AuditOutboxMutation {
                    verb: "create".to_owned(),
                    resource_type: "Host".to_owned(),
                    resource_uid: None,
                    target_digest: "legacy-target".to_owned(),
                    generation: 1,
                    expected_revision: 0,
                    mutation_id: String::new(),
                    ordinal: 9,
                    timestamp_ms: 0,
                    outcome: String::new(),
                    error_code: None,
                    previous_hash: None,
                    record_hash: None,
                }],
            }),
            ..operation
        };
        let value = crate::transaction::encode(ValueKind::OperationRecord, &legacy).unwrap();
        let table = backup
            .tables
            .iter_mut()
            .find(|table| table.name == "operations")
            .unwrap();
        let row = table
            .rows
            .iter_mut()
            .find(|row| row.key == key.as_bytes())
            .unwrap();
        row.value = value;
        table.checksum = backup::checksum_rows(&table.rows);
        backup.validate().unwrap();
        backup
    }

    fn invalid_digest_backup() -> LogicalBackup {
        let mut backup = legacy_backup();
        let table = backup
            .tables
            .iter_mut()
            .find(|table| table.name == "operations")
            .unwrap();
        let row = table.rows.first_mut().unwrap();
        let mut operation: OperationRecord =
            crate::transaction::decode(ValueKind::OperationRecord, row.value.as_slice()).unwrap();
        operation.request_digest = "not-a-digest".to_owned();
        row.value = crate::transaction::encode(ValueKind::OperationRecord, &operation).unwrap();
        table.checksum = backup::checksum_rows(&table.rows);
        backup.validate().unwrap();
        backup
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
        mark_stage_prepared(&parent, &identity()).unwrap();
    }

    #[test]
    fn registered_chain_is_explicit_and_unknown_versions_refuse() {
        assert_eq!(REGISTERED_MIGRATIONS.len(), 2);
        assert_eq!(
            migration_chain(0).unwrap(),
            vec![
                MigrationStep {
                    from: 0,
                    to: 1,
                    name: "physical-schema-v0-to-v1",
                },
                MigrationStep {
                    from: 1,
                    to: 2,
                    name: "physical-schema-v1-to-v2-authority-lifecycle",
                },
            ]
        );
        assert!(migration_chain(3).unwrap_err().kind() == StoreErrorKind::UpgradeRequired);
    }

    #[test]
    fn version_one_authority_shape_migrates_atomically_to_current() {
        let (directory, parent_fd, mut marker) = parent();
        create_current_file(&directory, DEFAULT_ACTIVE_FILE_NAME);
        set_schema_version(&directory, 1);
        let outcome = upgrade_owned(&parent_fd, &mut marker, &identity()).unwrap();
        assert_eq!(
            outcome,
            MigrationOutcome::Upgraded {
                from: 1,
                to: CURRENT_PHYSICAL_SCHEMA_VERSION
            }
        );
        let file = open_named_database(&parent_fd, DEFAULT_ACTIVE_FILE_NAME, &identity()).unwrap();
        validate_database(&file, &identity(), Some(CURRENT_PHYSICAL_SCHEMA_VERSION)).unwrap();
    }

    #[test]
    fn schema_upgrade_crosses_the_backup_boundary_before_publication() {
        let (directory, parent_fd, mut marker) = parent();
        create_current_file(&directory, DEFAULT_ACTIVE_FILE_NAME);
        set_schema_version(&directory, 1);
        let backup = pre_upgrade_backup();
        let outcome =
            upgrade_owned_after_backup(&parent_fd, &mut marker, &identity(), backup).unwrap();
        assert_eq!(
            outcome,
            MigrationOutcome::Upgraded {
                from: 1,
                to: CURRENT_PHYSICAL_SCHEMA_VERSION
            }
        );
        assert!(
            !directory.path().join(DEFAULT_STAGED_FILE_NAME).exists(),
            "staged image must not remain after a validated publication"
        );
    }

    #[test]
    fn restore_rejects_a_pre_upgrade_backup_as_upgrade_required() {
        let (directory, parent_fd, mut marker) = parent();
        let error =
            restore_owned(&parent_fd, &mut marker, &pre_upgrade_backup(), &identity()).unwrap_err();
        assert_eq!(error.kind(), StoreErrorKind::UpgradeRequired);
        assert_eq!(error.reason_code(), "backup-schema-version-unsupported");
        assert_eq!(
            backup::publication_state(
                &parent_fd,
                DEFAULT_STAGED_FILE_NAME,
                DEFAULT_ACTIVE_FILE_NAME,
                DEFAULT_PRIOR_FILE_NAME
            )
            .unwrap(),
            PublicationState::Empty
        );
        drop(directory);
    }

    #[test]
    fn schema_upgrade_rejects_a_backup_for_another_store_before_staging() {
        let (directory, parent_fd, mut marker) = parent();
        create_current_file(&directory, DEFAULT_ACTIVE_FILE_NAME);
        set_schema_version(&directory, 1);
        let backup = empty_backup_for(&other_identity());
        let error =
            upgrade_owned_after_backup(&parent_fd, &mut marker, &identity(), backup).unwrap_err();
        assert_eq!(error.reason_code(), "backup-store-identity-mismatch");
        assert!(
            !directory.path().join(DEFAULT_STAGED_FILE_NAME).exists(),
            "identity refusal must happen before staging"
        );
    }

    #[test]
    fn upgrade_normalizes_legacy_outbox_on_staged_copy_before_publish() {
        let (directory, parent_fd, mut marker) = parent();
        create_current_file(&directory, DEFAULT_ACTIVE_FILE_NAME);
        set_schema_version(&directory, 1);
        insert_legacy_outbox(&directory, "legacy-staged-upgrade");

        assert_eq!(
            upgrade_owned(&parent_fd, &mut marker, &identity()).unwrap(),
            MigrationOutcome::Upgraded { from: 1, to: 2 }
        );
        let active =
            open_named_database(&parent_fd, DEFAULT_ACTIVE_FILE_NAME, &identity()).unwrap();
        let read = active.begin_read().unwrap();
        let key = crate::keys::encode_key(
            crate::keys::KeySpace::Operations,
            &[crate::keys::KeyComponent::Text("legacy-staged-upgrade")],
        )
        .unwrap();
        let value = read
            .open_table(crate::transaction::OPERATIONS)
            .unwrap()
            .get(key.as_bytes())
            .unwrap()
            .unwrap();
        let operation: OperationRecord =
            crate::transaction::decode(ValueKind::OperationRecord, value.value()).unwrap();
        let outbox = operation.audit_outbox.expect("normalized outbox");
        assert_eq!(outbox.operation_id, "legacy-staged-upgrade");
        assert!(outbox.operation_identity.is_some());
        assert!(!outbox.mutations[0].mutation_id.is_empty());
        assert!(outbox.mutations[0].timestamp_ms > 0);
        assert_eq!(
            backup::publication_state(
                &parent_fd,
                DEFAULT_STAGED_FILE_NAME,
                DEFAULT_ACTIVE_FILE_NAME,
                DEFAULT_PRIOR_FILE_NAME
            )
            .unwrap(),
            PublicationState::ActiveOnly
        );
        drop(directory);
    }

    #[test]
    fn unmarked_stage_after_copy_is_discarded_and_upgrade_retries() {
        let (directory, parent_fd, mut marker) = parent();
        create_current_file(&directory, DEFAULT_ACTIVE_FILE_NAME);
        set_schema_version(&directory, 1);
        insert_legacy_outbox(&directory, "legacy-copy");
        let source =
            open_named_database(&parent_fd, DEFAULT_ACTIVE_FILE_NAME, &identity()).unwrap();
        let staged = create_stage(&parent_fd, &identity()).unwrap();
        let backend = FileBackend::new(staged).unwrap();
        let target = Database::builder()
            .set_cache_size(REDB_CACHE_SIZE)
            .create_with_backend(backend)
            .unwrap();
        copy_registered_step(&source, &target, 1, &identity()).unwrap();
        drop(target);
        drop(source);

        assert_eq!(
            recover_owned(&parent_fd, &mut marker, &identity()).unwrap(),
            RecoveryOutcome::Clean
        );
        assert_eq!(
            backup::publication_state(
                &parent_fd,
                DEFAULT_STAGED_FILE_NAME,
                DEFAULT_ACTIVE_FILE_NAME,
                DEFAULT_PRIOR_FILE_NAME
            )
            .unwrap(),
            PublicationState::ActiveOnly
        );
        assert_eq!(
            upgrade_owned(&parent_fd, &mut marker, &identity()).unwrap(),
            MigrationOutcome::Upgraded { from: 1, to: 2 }
        );
        drop(directory);
    }

    #[test]
    fn current_active_normalizes_before_discarding_unmarked_stage() {
        let (directory, parent_fd, mut marker) = parent();
        create_current_file(&directory, DEFAULT_ACTIVE_FILE_NAME);
        insert_legacy_outbox(&directory, "current-recovery");
        create_current_file(&directory, DEFAULT_STAGED_FILE_NAME);

        assert_eq!(
            recover_owned(&parent_fd, &mut marker, &identity()).unwrap(),
            RecoveryOutcome::Clean
        );
        validate_named_current(&parent_fd, DEFAULT_ACTIVE_FILE_NAME, &identity()).unwrap();
        assert_eq!(
            backup::publication_state(
                &parent_fd,
                DEFAULT_STAGED_FILE_NAME,
                DEFAULT_ACTIVE_FILE_NAME,
                DEFAULT_PRIOR_FILE_NAME
            )
            .unwrap(),
            PublicationState::ActiveOnly
        );
        drop(directory);
    }

    #[test]
    fn unmarked_stage_after_normalization_is_discarded_and_upgrade_retries() {
        let (directory, parent_fd, mut marker) = parent();
        create_current_file(&directory, DEFAULT_ACTIVE_FILE_NAME);
        set_schema_version(&directory, 1);
        insert_legacy_outbox(&directory, "legacy-normalized");
        let source =
            open_named_database(&parent_fd, DEFAULT_ACTIVE_FILE_NAME, &identity()).unwrap();
        let staged = create_stage(&parent_fd, &identity()).unwrap();
        let backend = FileBackend::new(staged).unwrap();
        let target = Database::builder()
            .set_cache_size(REDB_CACHE_SIZE)
            .create_with_backend(backend)
            .unwrap();
        copy_registered_step(&source, &target, 1, &identity()).unwrap();
        drop(target);
        drop(source);
        let staged_database =
            open_named_database(&parent_fd, DEFAULT_STAGED_FILE_NAME, &identity()).unwrap();
        crate::transaction::normalize_audit_outboxes(&staged_database).unwrap();
        drop(staged_database);
        sync_named_stage(&parent_fd, &identity()).unwrap();

        assert_eq!(
            recover_owned(&parent_fd, &mut marker, &identity()).unwrap(),
            RecoveryOutcome::Clean
        );
        assert_eq!(
            upgrade_owned(&parent_fd, &mut marker, &identity()).unwrap(),
            MigrationOutcome::Upgraded { from: 1, to: 2 }
        );
        drop(directory);
    }

    #[test]
    fn uninterpretable_legacy_request_digest_quarantines_before_publication() {
        let (directory, parent_fd, mut marker) = parent();
        create_current_file(&directory, DEFAULT_ACTIVE_FILE_NAME);
        set_schema_version(&directory, 1);
        insert_operation(&directory, "legacy-invalid-digest", "not-a-digest");

        let error = upgrade_owned(&parent_fd, &mut marker, &identity()).unwrap_err();
        assert_eq!(error.kind(), StoreErrorKind::StoreQuarantined);
        assert_eq!(
            error.reason_code(),
            "operation-request-digest-uninterpretable"
        );
        assert_eq!(
            backup::publication_state(
                &parent_fd,
                DEFAULT_STAGED_FILE_NAME,
                DEFAULT_ACTIVE_FILE_NAME,
                DEFAULT_PRIOR_FILE_NAME
            )
            .unwrap(),
            PublicationState::ActiveOnly
        );

        let active =
            open_named_database(&parent_fd, DEFAULT_ACTIVE_FILE_NAME, &identity()).unwrap();
        let meta = read_meta(&active, &identity()).unwrap();
        assert_eq!(meta.schema_version, 1);
        let read = active.begin_read().unwrap();
        let key = crate::keys::encode_key(
            crate::keys::KeySpace::Operations,
            &[crate::keys::KeyComponent::Text("legacy-invalid-digest")],
        )
        .unwrap();
        let value = read
            .open_table(crate::transaction::OPERATIONS)
            .unwrap()
            .get(key.as_bytes())
            .unwrap()
            .unwrap();
        let operation: OperationRecord =
            crate::transaction::decode(ValueKind::OperationRecord, value.value()).unwrap();
        assert_eq!(operation.request_digest, "not-a-digest");
        assert!(
            entry_type(&parent_fd, DEFAULT_STAGED_FILE_NAME)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn restore_uses_normalize_then_validate_for_legacy_outboxes() {
        let (directory, parent_fd, mut marker) = parent();
        let backup = legacy_backup();
        assert_eq!(
            restore_owned(&parent_fd, &mut marker, &backup, &identity()).unwrap(),
            MigrationOutcome::Restored
        );
        let active =
            open_named_database(&parent_fd, DEFAULT_ACTIVE_FILE_NAME, &identity()).unwrap();
        validate_database(&active, &identity(), Some(CURRENT_PHYSICAL_SCHEMA_VERSION)).unwrap();
        drop(directory);
    }

    #[test]
    fn restore_invalid_digest_leaves_active_only_and_quarantines() {
        let (directory, parent_fd, mut marker) = parent();
        create_current_file(&directory, DEFAULT_ACTIVE_FILE_NAME);
        let error = restore_owned(
            &parent_fd,
            &mut marker,
            &invalid_digest_backup(),
            &identity(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), StoreErrorKind::StoreQuarantined);
        assert_eq!(
            error.reason_code(),
            "operation-request-digest-uninterpretable"
        );
        assert_eq!(
            backup::publication_state(
                &parent_fd,
                DEFAULT_STAGED_FILE_NAME,
                DEFAULT_ACTIVE_FILE_NAME,
                DEFAULT_PRIOR_FILE_NAME
            )
            .unwrap(),
            PublicationState::ActiveOnly
        );
        drop(directory);
    }

    #[test]
    fn restore_publishes_only_after_staged_validation() {
        let (directory, parent_fd, mut marker) = parent();
        let backup = empty_backup();
        let outcome = restore_owned(&parent_fd, &mut marker, &backup, &identity()).unwrap();
        assert_eq!(outcome, MigrationOutcome::Restored);
        assert_eq!(
            backup::publication_state(
                &parent_fd,
                DEFAULT_STAGED_FILE_NAME,
                DEFAULT_ACTIVE_FILE_NAME,
                DEFAULT_PRIOR_FILE_NAME
            )
            .unwrap(),
            PublicationState::ActiveOnly
        );
        let file = open_named_database(&parent_fd, DEFAULT_ACTIVE_FILE_NAME, &identity()).unwrap();
        validate_database(&file, &identity(), Some(CURRENT_PHYSICAL_SCHEMA_VERSION)).unwrap();
        drop(file);
        drop(directory);
    }

    #[test]
    fn version_zero_upgrade_uses_a_new_file_and_is_idempotent() {
        let (directory, parent_fd, mut marker) = parent();
        create_current_file(&directory, DEFAULT_ACTIVE_FILE_NAME);
        set_schema_version(&directory, 0);
        let outcome = upgrade_owned(&parent_fd, &mut marker, &identity()).unwrap();
        assert_eq!(
            outcome,
            MigrationOutcome::Upgraded {
                from: 0,
                to: CURRENT_PHYSICAL_SCHEMA_VERSION
            }
        );
        assert_eq!(
            upgrade_owned(&parent_fd, &mut marker, &identity()).unwrap(),
            MigrationOutcome::AlreadyCurrent
        );
        assert_eq!(
            backup::publication_state(
                &parent_fd,
                DEFAULT_STAGED_FILE_NAME,
                DEFAULT_ACTIVE_FILE_NAME,
                DEFAULT_PRIOR_FILE_NAME
            )
            .unwrap(),
            PublicationState::ActiveOnly
        );
    }

    #[test]
    fn unknown_active_and_backup_versions_refuse_without_publication() {
        let (directory, parent_fd, mut marker) = parent();
        create_current_file(&directory, DEFAULT_ACTIVE_FILE_NAME);
        set_schema_version(&directory, CURRENT_PHYSICAL_SCHEMA_VERSION + 1);
        let error = upgrade_owned(&parent_fd, &mut marker, &identity()).unwrap_err();
        assert_eq!(error.kind(), StoreErrorKind::UpgradeRequired);
        assert_eq!(
            backup::publication_state(
                &parent_fd,
                DEFAULT_STAGED_FILE_NAME,
                DEFAULT_ACTIVE_FILE_NAME,
                DEFAULT_PRIOR_FILE_NAME
            )
            .unwrap(),
            PublicationState::ActiveOnly
        );

        let (backup_directory, backup_parent, mut backup_marker) = parent();
        let mut backup = empty_backup();
        backup.schema_version = CURRENT_PHYSICAL_SCHEMA_VERSION + 1;
        let error =
            restore_owned(&backup_parent, &mut backup_marker, &backup, &identity()).unwrap_err();
        assert_eq!(error.kind(), StoreErrorKind::UpgradeRequired);
        assert_eq!(
            backup::publication_state(
                &backup_parent,
                DEFAULT_STAGED_FILE_NAME,
                DEFAULT_ACTIVE_FILE_NAME,
                DEFAULT_PRIOR_FILE_NAME
            )
            .unwrap(),
            PublicationState::Empty
        );
        drop(backup_directory);
    }

    #[test]
    fn every_publication_boundary_rolls_back_without_changing_active() {
        for boundary in [
            PublicationBoundary::StageSync,
            PublicationBoundary::PriorRename,
            PublicationBoundary::ActiveRename,
            PublicationBoundary::FinalSync,
        ] {
            let (directory, parent_fd, _marker) = parent();
            create_current_file(&directory, DEFAULT_ACTIVE_FILE_NAME);
            create_staged_current(&directory);
            let error = publish_with_rollback(&parent_fd, Some(boundary), &identity()).unwrap_err();
            assert!(error.reason_code().contains("migration-fault"));
            assert_eq!(
                backup::publication_state(
                    &parent_fd,
                    DEFAULT_STAGED_FILE_NAME,
                    DEFAULT_ACTIVE_FILE_NAME,
                    DEFAULT_PRIOR_FILE_NAME
                )
                .unwrap(),
                PublicationState::ActiveOnly
            );
            validate_named_current(&parent_fd, DEFAULT_ACTIVE_FILE_NAME, &identity()).unwrap();
        }
    }

    #[test]
    fn crash_states_resume_idempotently_and_corruption_quarantines() {
        let (directory, parent_fd, mut marker) = parent();
        create_current_file(&directory, DEFAULT_ACTIVE_FILE_NAME);
        create_staged_current(&directory);
        renameat_with(
            &parent_fd,
            DEFAULT_ACTIVE_FILE_NAME,
            &parent_fd,
            DEFAULT_PRIOR_FILE_NAME,
            RenameFlags::NOREPLACE,
        )
        .unwrap();
        sync_parent(&parent_fd, &identity()).unwrap();
        assert_eq!(
            recover_owned(&parent_fd, &mut marker, &identity()).unwrap(),
            RecoveryOutcome::Resumed
        );
        assert_eq!(
            recover_owned(&parent_fd, &mut marker, &identity()).unwrap(),
            RecoveryOutcome::Clean
        );

        let (corrupt_directory, corrupt_parent_fd, mut corrupt_marker) = parent();
        let corrupt = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(corrupt_directory.path().join(DEFAULT_STAGED_FILE_NAME))
            .unwrap();
        corrupt.sync_all().unwrap();
        assert_eq!(
            recover_owned(&corrupt_parent_fd, &mut corrupt_marker, &identity()).unwrap(),
            RecoveryOutcome::Clean
        );
        assert_eq!(
            backup::publication_state(
                &corrupt_parent_fd,
                DEFAULT_STAGED_FILE_NAME,
                DEFAULT_ACTIVE_FILE_NAME,
                DEFAULT_PRIOR_FILE_NAME
            )
            .unwrap(),
            PublicationState::Empty
        );
    }

    #[test]
    fn empty_target_unprepared_stage_is_discarded_and_restore_retries() {
        let (directory, parent_fd, mut marker) = parent();
        create_current_file(&directory, DEFAULT_STAGED_FILE_NAME);
        assert_eq!(
            backup::publication_state(
                &parent_fd,
                DEFAULT_STAGED_FILE_NAME,
                DEFAULT_ACTIVE_FILE_NAME,
                DEFAULT_PRIOR_FILE_NAME
            )
            .unwrap(),
            PublicationState::StagedOnly
        );

        assert_eq!(
            recover_owned(&parent_fd, &mut marker, &identity()).unwrap(),
            RecoveryOutcome::Clean
        );
        assert_eq!(
            backup::publication_state(
                &parent_fd,
                DEFAULT_STAGED_FILE_NAME,
                DEFAULT_ACTIVE_FILE_NAME,
                DEFAULT_PRIOR_FILE_NAME
            )
            .unwrap(),
            PublicationState::Empty
        );
        assert_eq!(
            restore_owned(&parent_fd, &mut marker, &empty_backup(), &identity()).unwrap(),
            MigrationOutcome::Restored
        );
        drop(directory);
    }

    #[test]
    fn marker_only_cleanup_residue_heals_idempotently() {
        let (directory, parent_fd, mut marker) = parent();
        create_staged_current(&directory);
        std::fs::remove_file(directory.path().join(DEFAULT_STAGED_FILE_NAME)).unwrap();
        assert_eq!(
            recover_owned(&parent_fd, &mut marker, &identity()).unwrap(),
            RecoveryOutcome::Finalized
        );
        assert_eq!(
            entry_type(&parent_fd, STAGED_PREPARED_MARKER_FILE_NAME).unwrap(),
            None
        );
        assert_eq!(
            recover_owned(&parent_fd, &mut marker, &identity()).unwrap(),
            RecoveryOutcome::Clean
        );
        drop(directory);
    }

    #[test]
    fn partial_marker_is_not_authority_when_active_is_valid() {
        let (directory, parent_fd, mut marker) = parent();
        create_current_file(&directory, DEFAULT_ACTIVE_FILE_NAME);
        create_staged_current(&directory);
        let marker_path = directory.path().join(STAGED_PREPARED_MARKER_FILE_NAME);
        let partial = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&marker_path)
            .unwrap();
        partial.set_len(0).unwrap();
        partial.sync_all().unwrap();

        assert_eq!(
            recover_owned(&parent_fd, &mut marker, &identity()).unwrap(),
            RecoveryOutcome::Clean
        );
        assert_eq!(
            backup::publication_state(
                &parent_fd,
                DEFAULT_STAGED_FILE_NAME,
                DEFAULT_ACTIVE_FILE_NAME,
                DEFAULT_PRIOR_FILE_NAME
            )
            .unwrap(),
            PublicationState::ActiveOnly
        );
        validate_named_current(&parent_fd, DEFAULT_ACTIVE_FILE_NAME, &identity()).unwrap();
        drop(directory);
    }

    #[test]
    fn unsafe_marker_identity_refuses_without_mutating_publication() {
        let (directory, parent_fd, mut marker) = parent();
        create_current_file(&directory, DEFAULT_ACTIVE_FILE_NAME);
        create_staged_current(&directory);
        let marker_path = directory.path().join(STAGED_PREPARED_MARKER_FILE_NAME);
        let mut partial = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&marker_path)
            .unwrap();
        partial.set_len(0).unwrap();
        partial
            .write_all(b"d2b-redb-stage-prepared/v1\nslot=1\n")
            .unwrap();
        partial.sync_all().unwrap();
        let active_before = std::fs::read(directory.path().join(DEFAULT_ACTIVE_FILE_NAME)).unwrap();

        let error = recover_owned(&parent_fd, &mut marker, &identity()).unwrap_err();
        assert_eq!(
            error.reason_code(),
            "migration-stage-marker-identity-mismatch"
        );
        assert_eq!(
            std::fs::read(directory.path().join(DEFAULT_ACTIVE_FILE_NAME)).unwrap(),
            active_before
        );
        assert_eq!(
            backup::publication_state(
                &parent_fd,
                DEFAULT_STAGED_FILE_NAME,
                DEFAULT_ACTIVE_FILE_NAME,
                DEFAULT_PRIOR_FILE_NAME
            )
            .unwrap(),
            PublicationState::ActiveAndStaged
        );
        drop(directory);
    }

    #[test]
    fn foreign_active_is_rejected_before_legacy_repair() {
        let (directory, parent_fd, mut marker) = parent();
        let foreign = other_identity();
        create_current_file_with_identity(&directory, DEFAULT_ACTIVE_FILE_NAME, &foreign);
        insert_legacy_outbox(&directory, "foreign-legacy");
        let foreign_db =
            open_named_database(&parent_fd, DEFAULT_ACTIVE_FILE_NAME, &foreign).unwrap();
        let key = crate::keys::encode_key(
            crate::keys::KeySpace::Operations,
            &[crate::keys::KeyComponent::Text("foreign-legacy")],
        )
        .unwrap();
        let before = foreign_db
            .begin_read()
            .unwrap()
            .open_table(crate::transaction::OPERATIONS)
            .unwrap()
            .get(key.as_bytes())
            .unwrap()
            .unwrap()
            .value()
            .to_vec();
        drop(foreign_db);

        let error = recover_owned(&parent_fd, &mut marker, &identity()).unwrap_err();
        assert_eq!(error.reason_code(), "migration-store-identity-mismatch");
        let foreign_db =
            open_named_database(&parent_fd, DEFAULT_ACTIVE_FILE_NAME, &foreign).unwrap();
        let after = foreign_db
            .begin_read()
            .unwrap()
            .open_table(crate::transaction::OPERATIONS)
            .unwrap()
            .get(key.as_bytes())
            .unwrap()
            .unwrap()
            .value()
            .to_vec();
        assert_eq!(after, before);
        drop(directory);
    }

    #[test]
    fn stage_creation_and_marker_boundaries_are_recoverable() {
        let boundaries = [
            PublicationBoundary::StageCreate,
            PublicationBoundary::StageDatabaseCreate,
            PublicationBoundary::StageDatabaseCommit,
            PublicationBoundary::StageSync,
            PublicationBoundary::MarkerTempCreate,
            PublicationBoundary::MarkerTempWrite,
            PublicationBoundary::MarkerTempSync,
            PublicationBoundary::MarkerRename,
            PublicationBoundary::MarkerSync,
            PublicationBoundary::MarkerRemove,
            PublicationBoundary::MarkerTempRemove,
            PublicationBoundary::StageRemove,
        ];
        for boundary in boundaries {
            let (directory, parent_fd, mut marker) = parent();
            let backup = empty_backup();
            let next_generation = next_backup_generation(&parent_fd, &backup, &identity()).unwrap();
            let result = if matches!(
                boundary,
                PublicationBoundary::MarkerRemove
                    | PublicationBoundary::MarkerTempRemove
                    | PublicationBoundary::StageRemove
            ) {
                prepare_restore_stage_with_fault(
                    &parent_fd,
                    &backup,
                    &identity(),
                    next_generation,
                    None,
                )
                .unwrap();
                cleanup_failed_stage_with_fault(
                    &parent_fd,
                    &identity(),
                    Err::<(), _>(injected_fault("migration-fault-cleanup", &identity())),
                    Some(boundary),
                )
            } else {
                prepare_restore_stage_with_fault(
                    &parent_fd,
                    &backup,
                    &identity(),
                    next_generation,
                    Some(boundary),
                )
            };
            assert!(result.is_err(), "boundary {boundary:?} did not inject");
            let recovery = recover_owned(&parent_fd, &mut marker, &identity()).unwrap();
            assert!(
                matches!(
                    recovery,
                    RecoveryOutcome::Clean | RecoveryOutcome::Finalized
                ),
                "boundary {boundary:?} left {recovery:?}"
            );
            assert_eq!(
                backup::publication_state(
                    &parent_fd,
                    DEFAULT_STAGED_FILE_NAME,
                    DEFAULT_ACTIVE_FILE_NAME,
                    DEFAULT_PRIOR_FILE_NAME
                )
                .unwrap(),
                PublicationState::Empty
            );
            assert_eq!(
                restore_owned(&parent_fd, &mut marker, &backup, &identity()).unwrap(),
                MigrationOutcome::Restored
            );
            drop(directory);
        }
    }

    #[test]
    fn prepared_stage_corruption_refuses_recovery() {
        let (directory, parent_fd, mut marker) = parent();
        create_current_file(&directory, DEFAULT_ACTIVE_FILE_NAME);
        create_staged_current(&directory);
        let staged = OpenOptions::new()
            .read(true)
            .write(true)
            .open(directory.path().join(DEFAULT_STAGED_FILE_NAME))
            .unwrap();
        staged.set_len(0).unwrap();
        drop(staged);

        let error = recover_owned(&parent_fd, &mut marker, &identity()).unwrap_err();
        assert_eq!(error.kind(), StoreErrorKind::StoreQuarantined);
        assert_eq!(
            backup::publication_state(
                &parent_fd,
                DEFAULT_STAGED_FILE_NAME,
                DEFAULT_ACTIVE_FILE_NAME,
                DEFAULT_PRIOR_FILE_NAME
            )
            .unwrap(),
            PublicationState::ActiveAndStaged
        );
        drop(directory);
    }

    #[test]
    fn ambiguous_prior_only_and_all_present_states_remain_fail_closed() {
        let (prior_only_directory, prior_only_parent, mut prior_only_marker) = parent();
        create_current_file(&prior_only_directory, DEFAULT_ACTIVE_FILE_NAME);
        renameat_with(
            &prior_only_parent,
            DEFAULT_ACTIVE_FILE_NAME,
            &prior_only_parent,
            DEFAULT_PRIOR_FILE_NAME,
            RenameFlags::NOREPLACE,
        )
        .unwrap();
        let error =
            recover_owned(&prior_only_parent, &mut prior_only_marker, &identity()).unwrap_err();
        assert_eq!(error.reason_code(), "migration-publication-state-ambiguous");

        let (all_directory, all_parent, mut all_marker) = parent();
        create_current_file(&all_directory, DEFAULT_ACTIVE_FILE_NAME);
        create_staged_current(&all_directory);
        renameat_with(
            &all_parent,
            DEFAULT_ACTIVE_FILE_NAME,
            &all_parent,
            DEFAULT_PRIOR_FILE_NAME,
            RenameFlags::NOREPLACE,
        )
        .unwrap();
        create_current_file(&all_directory, DEFAULT_ACTIVE_FILE_NAME);
        let error = recover_owned(&all_parent, &mut all_marker, &identity()).unwrap_err();
        assert_eq!(error.reason_code(), "migration-publication-state-ambiguous");
    }

    #[test]
    fn marker_and_identity_mismatch_quarantine_before_stage_creation() {
        let (directory, parent_fd, mut marker) = parent();
        let other = StoreIdentity::new(
            d2b_resource_store::StoreSlot::new(0).unwrap(),
            ResourceUid::parse("33333333-3333-4333-8333-333333333333").unwrap(),
            ZoneId::parse("work").unwrap(),
            ResourceUid::parse("22222222-2222-4222-8222-222222222222").unwrap(),
            Timestamp::parse("2026-07-31T00:00:00.000Z").unwrap(),
            identity().revisions,
        );
        let error = restore_owned(&parent_fd, &mut marker, &empty_backup(), &other).unwrap_err();
        assert_eq!(error.kind(), StoreErrorKind::StoreQuarantined);
        assert_eq!(
            backup::publication_state(
                &parent_fd,
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
