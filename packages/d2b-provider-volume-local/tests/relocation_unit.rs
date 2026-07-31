use d2b_provider_volume_local::audit::VolumeAuditKind;
use d2b_provider_volume_local::marker::MarkerDisposition;
use d2b_provider_volume_local::relocation::{
    RelocationAction, RelocationError, RelocationPhase, RelocationState,
};

#[test]
fn successful_guest_attached_relocation_repoints_before_source_deletion() {
    let mut relocation = RelocationState::new(MarkerDisposition::Verified, true).unwrap();
    let begin = relocation.begin().unwrap();
    assert_eq!(begin.action, RelocationAction::AddSourceFinalizer);
    assert_eq!(begin.audit, Some(VolumeAuditKind::VolumeRelocationStart));
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
    assert_eq!(
        relocation.copy_succeeded().unwrap().action,
        RelocationAction::ActivateDestination
    );
    assert_eq!(
        relocation.destination_activated().unwrap().action,
        RelocationAction::RepointAttachments
    );
    assert_eq!(
        relocation.attachments_repointed().unwrap().action,
        RelocationAction::RemoveSourceFinalizer
    );
    assert_eq!(
        relocation.finalizer_removed().unwrap().action,
        RelocationAction::DeleteSource
    );
    let committed = relocation.source_deleted().unwrap();
    assert_eq!(committed.phase, RelocationPhase::Committed);
    assert_eq!(
        committed.audit,
        Some(VolumeAuditKind::VolumeRelocationCommitted)
    );
}

#[test]
fn midpoint_copy_failure_preserves_source_and_finalizer() {
    let mut relocation = RelocationState::new(MarkerDisposition::Verified, false).unwrap();
    relocation.begin().unwrap();
    relocation.finalizer_committed().unwrap();
    relocation.source_drained().unwrap();
    relocation
        .destination_ready(MarkerDisposition::Verified)
        .unwrap();
    let failed = relocation.copy_failed().unwrap();
    assert_eq!(failed.phase, RelocationPhase::Failed);
    assert_eq!(failed.action, RelocationAction::PreserveSource);
    assert_eq!(
        relocation.source_deleted(),
        Err(RelocationError::InvalidTransition)
    );
}

#[test]
fn relocation_never_uses_missing_state_as_an_empty_source_or_destination() {
    assert_eq!(
        RelocationState::new(MarkerDisposition::Unprovisioned, false).unwrap_err(),
        RelocationError::MarkerNotVerified
    );

    let mut relocation = RelocationState::new(MarkerDisposition::Verified, false).unwrap();
    relocation.begin().unwrap();
    relocation.finalizer_committed().unwrap();
    relocation.source_drained().unwrap();
    assert_eq!(
        relocation.destination_ready(MarkerDisposition::Unprovisioned),
        Err(RelocationError::MarkerNotVerified)
    );
    assert_eq!(relocation.phase(), RelocationPhase::DestinationPending);
}

#[test]
fn source_cannot_be_deleted_before_destination_activation() {
    let mut relocation = RelocationState::new(MarkerDisposition::Verified, false).unwrap();
    relocation.begin().unwrap();
    assert_eq!(
        relocation.source_deleted(),
        Err(RelocationError::InvalidTransition)
    );
}
