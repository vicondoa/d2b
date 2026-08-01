use d2b_contracts::v3::SchemaVersion;
use d2b_contracts::v3::credential::CredentialLeaseState;
use d2b_provider_volume_local::marker::MarkerDisposition;
use d2b_provider_volume_local::migration::{
    MigrationAction, MigrationError, MigrationPhase, MigrationState, recover_after_restart,
};
use d2b_provider_volume_local::relocation::{
    RelocationAction, RelocationError, RelocationPhase, RelocationState,
};
use d2b_provider_volume_local::sealing::{
    BoundRotationResult, RotationPhase, SealingAction, SealingError, SealingState,
};

fn version(major: u32) -> SchemaVersion {
    SchemaVersion::new(major, 0).unwrap()
}

fn migration_ready_to_commit() -> MigrationState {
    let mut migration =
        MigrationState::new(version(1), version(2), MarkerDisposition::Verified).unwrap();
    assert_eq!(
        migration.prepare().unwrap().action,
        MigrationAction::CommitPrepare
    );
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
    migration
}

fn relocation_at_copy(has_guest_attachments: bool) -> RelocationState {
    let mut relocation =
        RelocationState::new(MarkerDisposition::Verified, has_guest_attachments).unwrap();
    assert_eq!(
        relocation.begin().unwrap().action,
        RelocationAction::AddSourceFinalizer
    );
    assert_eq!(
        relocation.finalizer_committed().unwrap().action,
        RelocationAction::DrainSourceMounts
    );
    assert_eq!(
        relocation.source_drained().unwrap().action,
        RelocationAction::CreateDestination
    );
    assert_eq!(
        relocation
            .destination_ready(MarkerDisposition::Verified)
            .unwrap()
            .action,
        RelocationAction::DispatchCopyWorker
    );
    relocation
}

#[test]
fn migration_and_relocation_refuse_unverified_source_markers() {
    assert_eq!(
        MigrationState::new(version(1), version(2), MarkerDisposition::Unprovisioned).unwrap_err(),
        MigrationError::MarkerNotVerified
    );
    assert_eq!(
        recover_after_restart(
            MarkerDisposition::Unprovisioned,
            version(1),
            version(2),
            true,
        ),
        Err(MigrationError::MarkerNotVerified)
    );
    assert_eq!(
        RelocationState::new(MarkerDisposition::Unprovisioned, true).unwrap_err(),
        RelocationError::MarkerNotVerified
    );
}

#[test]
fn migration_commit_requires_verified_target_version_evidence() {
    let mut migration = migration_ready_to_commit();
    assert_eq!(
        migration.commit(MarkerDisposition::Verified, version(3)),
        Err(MigrationError::TargetNotCommitted)
    );
    assert_eq!(migration.phase(), MigrationPhase::ReadyToCommit);
    assert_eq!(migration.installed_version(), version(1));

    let committed = migration
        .commit(MarkerDisposition::Verified, version(2))
        .unwrap();
    assert_eq!(committed.phase, MigrationPhase::Current);
    assert_eq!(committed.action, MigrationAction::CleanupStaging);
    assert_eq!(migration.installed_version(), version(2));
}

#[test]
fn precommit_worker_failure_preserves_installed_schema_and_rolls_back_staging() {
    let mut migration =
        MigrationState::new(version(1), version(2), MarkerDisposition::Verified).unwrap();
    migration.prepare().unwrap();
    migration.writers_ready().unwrap();
    migration.staging_ready().unwrap();

    let failed = migration.worker_failed().unwrap();
    assert_eq!(failed.phase, MigrationPhase::RollingBack);
    assert_eq!(failed.action, MigrationAction::RollbackStaging);
    assert_eq!(migration.installed_version(), version(1));
    assert_eq!(
        migration.rollback_completed().unwrap().phase,
        MigrationPhase::Failed
    );
    assert_eq!(
        migration.commit(MarkerDisposition::Verified, version(2)),
        Err(MigrationError::InvalidTransition)
    );
}

#[test]
fn guest_relocation_repoints_attachments_before_finalizer_or_source_removal() {
    let mut relocation = relocation_at_copy(true);
    assert_eq!(
        relocation.copy_succeeded().unwrap().action,
        RelocationAction::ActivateDestination
    );
    let activated = relocation.destination_activated().unwrap();
    assert_eq!(activated.phase, RelocationPhase::AttachmentRepointPending);
    assert_eq!(activated.action, RelocationAction::RepointAttachments);
    assert_eq!(
        relocation.finalizer_removed(),
        Err(RelocationError::InvalidTransition)
    );
    assert_eq!(
        relocation.source_deleted(),
        Err(RelocationError::InvalidTransition)
    );

    let repointed = relocation.attachments_repointed().unwrap();
    assert_eq!(repointed.action, RelocationAction::RemoveSourceFinalizer);
    assert_eq!(
        relocation.source_deleted(),
        Err(RelocationError::InvalidTransition)
    );
    assert_eq!(
        relocation.finalizer_removed().unwrap().action,
        RelocationAction::DeleteSource
    );
    assert_eq!(
        relocation.source_deleted().unwrap().phase,
        RelocationPhase::Committed
    );
}

#[test]
fn relocation_copy_failure_preserves_source_and_finalizer() {
    let mut relocation = relocation_at_copy(false);
    let failed = relocation.copy_failed().unwrap();
    assert_eq!(failed.phase, RelocationPhase::Failed);
    assert_eq!(failed.action, RelocationAction::PreserveSource);
    assert_eq!(
        relocation.finalizer_removed(),
        Err(RelocationError::InvalidTransition)
    );
    assert_eq!(
        relocation.source_deleted(),
        Err(RelocationError::InvalidTransition)
    );
}

#[test]
fn sealing_releases_no_rotation_effect_before_pending_status_commit() {
    let mut sealing = SealingState::sealed(4).unwrap();
    let observed = sealing
        .observe_credential(CredentialLeaseState::Active, 5)
        .unwrap();
    assert_eq!(observed.phase, RotationPhase::StatusCommitRequired);
    assert_eq!(observed.action, SealingAction::CommitPendingStatus);
    assert_eq!(
        sealing.apply_bound_result(BoundRotationResult::Rotated),
        Err(SealingError::InvalidTransition)
    );

    let ready = sealing.pending_status_committed().unwrap();
    assert_eq!(ready.phase, RotationPhase::EffectReady);
    assert_eq!(ready.action, SealingAction::InvokeBoundRotation);
    let complete = sealing
        .apply_bound_result(BoundRotationResult::Rotated)
        .unwrap();
    assert_eq!(complete.action, SealingAction::CommitSealedStatus);
    assert_eq!(sealing.active_generation(), 5);
}

#[test]
fn sealing_retries_the_same_pending_generation_until_audit_is_durable() {
    let mut sealing = SealingState::sealed(7).unwrap();
    sealing
        .observe_credential(CredentialLeaseState::Active, 8)
        .unwrap();
    sealing.pending_status_committed().unwrap();

    let pending = sealing
        .apply_bound_result(BoundRotationResult::CommitPendingAudit)
        .unwrap();
    assert_eq!(pending.phase, RotationPhase::CommitPendingAudit);
    assert_eq!(pending.action, SealingAction::RetryIdenticalRotation);
    assert_eq!(sealing.target_generation(), Some(8));
    assert_eq!(sealing.active_generation(), 7);

    let recovered = sealing
        .apply_bound_result(BoundRotationResult::RecoveredCommitted)
        .unwrap();
    assert_eq!(recovered.action, SealingAction::CommitSealedStatus);
    assert_eq!(sealing.target_generation(), None);
    assert_eq!(sealing.active_generation(), 8);
}
