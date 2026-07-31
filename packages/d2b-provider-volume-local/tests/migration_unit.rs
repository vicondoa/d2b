use d2b_contracts::v3::SchemaVersion;
use d2b_provider_volume_local::audit::VolumeAuditKind;
use d2b_provider_volume_local::marker::MarkerDisposition;
use d2b_provider_volume_local::migration::{
    MigrationAction, MigrationError, MigrationPhase, MigrationState, recover_after_restart,
};

fn version(major: u32) -> SchemaVersion {
    SchemaVersion::new(major, 0).unwrap()
}

#[test]
fn prepare_worker_commit_and_cleanup_are_ordered_and_idempotent() {
    let mut migration =
        MigrationState::new(version(1), version(2), MarkerDisposition::Verified).unwrap();
    let prepared = migration.prepare().unwrap();
    assert_eq!(prepared.action, MigrationAction::CommitPrepare);
    assert_eq!(prepared.audit, Some(VolumeAuditKind::VolumeMigrationStart));
    assert_eq!(
        migration.writers_ready().unwrap().action,
        MigrationAction::CreateStaging
    );
    assert_eq!(
        migration.staging_ready().unwrap().action,
        MigrationAction::DispatchWorker
    );
    assert_eq!(
        migration.worker_succeeded().unwrap().action,
        MigrationAction::CommitStaging
    );
    let committed = migration
        .commit(MarkerDisposition::Verified, version(2))
        .unwrap();
    assert_eq!(committed.phase, MigrationPhase::Current);
    assert_eq!(committed.action, MigrationAction::CleanupStaging);
    assert_eq!(
        committed.audit,
        Some(VolumeAuditKind::VolumeMigrationCommitted)
    );
    assert_eq!(
        migration.commit(MarkerDisposition::Verified, version(2)),
        Err(MigrationError::InvalidTransition)
    );
}

#[test]
fn failed_worker_rolls_back_only_before_commit_and_preserves_installed_version() {
    let mut migration =
        MigrationState::new(version(1), version(2), MarkerDisposition::Verified).unwrap();
    migration.prepare().unwrap();
    migration.writers_ready().unwrap();
    migration.staging_ready().unwrap();
    let failed = migration.worker_failed().unwrap();
    assert_eq!(failed.phase, MigrationPhase::RollingBack);
    assert_eq!(failed.action, MigrationAction::RollbackStaging);
    assert_eq!(migration.installed_version(), version(1));
    let rolled_back = migration.rollback_completed().unwrap();
    assert_eq!(rolled_back.phase, MigrationPhase::Failed);
    assert_eq!(
        rolled_back.audit,
        Some(VolumeAuditKind::VolumeMigrationRolledBack)
    );
    assert_eq!(
        migration.commit(MarkerDisposition::Verified, version(2)),
        Err(MigrationError::InvalidTransition)
    );
}

#[test]
fn restart_rolls_forward_from_target_marker_and_cleans_orphan_staging() {
    let recovered =
        recover_after_restart(MarkerDisposition::Verified, version(2), version(2), true).unwrap();
    assert_eq!(recovered.phase, MigrationPhase::Current);
    assert_eq!(recovered.action, MigrationAction::CleanupStaging);

    let resume =
        recover_after_restart(MarkerDisposition::Verified, version(1), version(2), true).unwrap();
    assert_eq!(resume.phase, MigrationPhase::Migrating);
    assert_eq!(resume.action, MigrationAction::DispatchWorker);
}

#[test]
fn migration_never_treats_unverified_marker_state_as_first_provision() {
    assert_eq!(
        MigrationState::new(version(1), version(2), MarkerDisposition::Unprovisioned).unwrap_err(),
        MigrationError::MarkerNotVerified
    );
    assert_eq!(
        recover_after_restart(
            MarkerDisposition::Unprovisioned,
            version(1),
            version(2),
            false,
        ),
        Err(MigrationError::MarkerNotVerified)
    );
}
