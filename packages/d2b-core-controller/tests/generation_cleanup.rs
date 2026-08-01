use std::collections::BTreeSet;

use d2b_contracts::v3::{ResourceBundleGenerationId, ResourceName, ResourceTypeName, Timestamp};
use d2b_core_controller::{
    cleanup::{
        CleanupZonePhase, PendingCleanupState, PriorGenerationBundle, pending_cleanup_condition,
        prunable_prior_bundles,
    },
    configuration::{ResourceKey, RetainedGenerations},
    resource_store::{PersistedResourceMetadata, PersistedResourceRecord},
};

fn generation(byte: char) -> ResourceBundleGenerationId {
    ResourceBundleGenerationId::parse(format!("sha256:{}", byte.to_string().repeat(64))).unwrap()
}

fn key(name: &str) -> ResourceKey {
    ResourceKey::new(
        ResourceTypeName::parse("Network").unwrap(),
        ResourceName::parse(name).unwrap(),
    )
}

#[test]
fn pending_cleanup_tracks_only_configuration_owned_deletion_requests() {
    let mut removed = PersistedResourceMetadata::configuration(1).unwrap();
    assert!(removed.schedule_deletion(&Timestamp::parse("2026-07-31T00:01:00.000Z").unwrap()));
    let resources = vec![
        PersistedResourceRecord::new(key("removed"), removed),
        PersistedResourceRecord::new(key("child"), PersistedResourceMetadata::controller()),
        PersistedResourceRecord::new(key("api"), PersistedResourceMetadata::api()),
    ];
    let condition = pending_cleanup_condition(&resources);
    assert_eq!(condition.name(), "PendingCleanup");
    assert_eq!(condition.state(), PendingCleanupState::True);
    assert_eq!(condition.phase(), CleanupZonePhase::Degraded);
    assert_eq!(condition.pending_count(), 1);

    let clean = pending_cleanup_condition(&resources[1..]);
    assert_eq!(clean.state(), PendingCleanupState::False);
    assert_eq!(clean.phase(), CleanupZonePhase::Ready);
}

#[test]
fn prior_bundle_pruning_is_count_based_and_waits_for_cleanup() {
    let prior = vec![
        PriorGenerationBundle::new(generation('1'), [key("removed")]),
        PriorGenerationBundle::new(generation('2'), [key("unchanged")]),
        PriorGenerationBundle::new(generation('3'), [key("current")]),
    ];
    let current = vec![
        PersistedResourceRecord::new(
            key("removed"),
            PersistedResourceMetadata::configuration(1).unwrap(),
        ),
        PersistedResourceRecord::new(
            key("unchanged"),
            PersistedResourceMetadata::configuration(4).unwrap(),
        ),
        PersistedResourceRecord::new(
            key("current"),
            PersistedResourceMetadata::configuration(4).unwrap(),
        ),
    ];
    let unchanged = BTreeSet::from([key("unchanged")]);
    let retained = RetainedGenerations::new(1).unwrap();

    let blocked = prunable_prior_bundles(&prior, retained, &current, &unchanged);
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0].content_hash(), &generation('2'));

    let after_delete = prunable_prior_bundles(&prior, retained, &current[1..], &unchanged);
    assert_eq!(after_delete.len(), 2);
    assert_eq!(after_delete[0].content_hash(), &generation('1'));
    assert_eq!(after_delete[1].content_hash(), &generation('2'));
}

#[test]
fn retained_generation_default_and_range_are_frozen() {
    assert_eq!(RetainedGenerations::default_value().get(), 3);
    assert!(RetainedGenerations::new(1).is_ok());
    assert!(RetainedGenerations::new(16).is_ok());
    assert!(RetainedGenerations::new(0).is_err());
    assert!(RetainedGenerations::new(17).is_err());
}
