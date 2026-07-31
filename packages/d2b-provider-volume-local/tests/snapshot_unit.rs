use d2b_contracts::v3::{SchemaVersion, Timestamp};
use d2b_provider_volume_local::audit::{SnapshotTrigger, VolumeAuditKind};
use d2b_provider_volume_local::snapshot::{
    SnapshotCatalog, SnapshotError, SnapshotId, SnapshotPhase, SnapshotPolicy, SnapshotStatus,
};

fn status(id: &str, hour: u64, phase: SnapshotPhase) -> SnapshotStatus {
    SnapshotStatus::new(
        SnapshotId::parse(id).unwrap(),
        Timestamp::parse(format!("2026-07-{:02}T00:00:00.000Z", hour + 1)).unwrap(),
        hour,
        SchemaVersion::new(1, 0).unwrap(),
        4096,
        SnapshotTrigger::PreMigration,
        phase,
    )
    .unwrap()
}

#[test]
fn policy_gates_automatic_snapshot_dispatch() {
    let policy = SnapshotPolicy::new(3, 168, false, true).unwrap();
    let catalog = SnapshotCatalog::new(policy);
    assert_eq!(
        catalog.plan_dispatch(SnapshotTrigger::PreMigration),
        Err(SnapshotError::TriggerDisabled)
    );
    let dispatch = catalog
        .plan_dispatch(SnapshotTrigger::PreRelocation)
        .unwrap();
    assert_eq!(
        dispatch.success_audit,
        VolumeAuditKind::VolumeSnapshotCreated
    );
    assert!(catalog.plan_dispatch(SnapshotTrigger::Manual).is_ok());
}

#[test]
fn retention_expires_by_count_and_ttl_without_exposing_snapshot_paths() {
    let mut catalog = SnapshotCatalog::new(SnapshotPolicy::new(2, 5, true, true).unwrap());
    catalog
        .record(status("snap-a", 1, SnapshotPhase::Ready))
        .unwrap();
    catalog
        .record(status("snap-b", 4, SnapshotPhase::Ready))
        .unwrap();
    catalog
        .record(status("snap-c", 8, SnapshotPhase::Ready))
        .unwrap();

    let expired = catalog.retention_plan(9).unwrap();
    assert_eq!(
        expired,
        [
            SnapshotId::parse("snap-a").unwrap(),
            SnapshotId::parse("snap-b").unwrap()
        ]
    );
    catalog.apply_expired(&expired);
    assert_eq!(catalog.records()[0].phase(), SnapshotPhase::Expired);
    assert_eq!(catalog.records()[1].phase(), SnapshotPhase::Expired);
    assert_eq!(catalog.records()[2].phase(), SnapshotPhase::Ready);
    assert_eq!(
        format!("{:?}", catalog.records()[0]),
        "SnapshotStatus { phase: Expired, trigger: PreMigration, .. }"
    );
    catalog.remove_expired();
    assert_eq!(catalog.records().len(), 1);
}

#[test]
fn pre_migration_snapshot_status_survives_an_interrupted_migration() {
    let mut catalog = SnapshotCatalog::new(SnapshotPolicy::new(3, 0, true, false).unwrap());
    catalog
        .plan_dispatch(SnapshotTrigger::PreMigration)
        .unwrap();
    catalog
        .record(status("protective", 2, SnapshotPhase::Ready))
        .unwrap();
    assert_eq!(catalog.records().len(), 1);
    assert_eq!(catalog.records()[0].phase(), SnapshotPhase::Ready);
    let rendered = serde_json::to_value(&catalog.records()[0]).unwrap();
    assert_eq!(rendered["phase"], "Ready");
    assert!(rendered.get("createdHour").is_none());
    assert!(rendered.get("path").is_none());
}

#[test]
fn snapshot_catalogue_rejects_duplicate_identity_and_invalid_policy() {
    assert_eq!(
        SnapshotPolicy::new(0, 0, false, false),
        Err(SnapshotError::PolicyInvalid)
    );
    let mut catalog = SnapshotCatalog::new(SnapshotPolicy::new(1, 0, true, true).unwrap());
    catalog
        .record(status("same", 1, SnapshotPhase::Ready))
        .unwrap();
    assert_eq!(
        catalog.record(status("same", 2, SnapshotPhase::Failed)),
        Err(SnapshotError::DuplicateSnapshot)
    );
}

#[test]
fn equal_time_count_boundary_fails_closed_without_an_invented_tie_break() {
    let policy = SnapshotPolicy::new(1, 0, true, true).unwrap();
    let mut catalog = SnapshotCatalog::new(policy);
    let first = SnapshotStatus::new(
        SnapshotId::parse("first").unwrap(),
        Timestamp::parse("2026-07-01T00:00:00.000Z").unwrap(),
        1,
        SchemaVersion::new(1, 0).unwrap(),
        1,
        SnapshotTrigger::Manual,
        SnapshotPhase::Ready,
    )
    .unwrap();
    let second = SnapshotStatus::new(
        SnapshotId::parse("second").unwrap(),
        Timestamp::parse("2026-07-01T00:00:00.000Z").unwrap(),
        1,
        SchemaVersion::new(1, 0).unwrap(),
        1,
        SnapshotTrigger::Manual,
        SnapshotPhase::Ready,
    )
    .unwrap();
    catalog.record(first).unwrap();
    catalog.record(second).unwrap();
    assert_eq!(
        catalog.retention_plan(1),
        Err(SnapshotError::RetentionOrderAmbiguous)
    );
}
