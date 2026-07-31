use d2b_contracts::v3::zone_routing::{ZoneLabelId, ZonePath};
use d2b_contracts::v3::{
    MigrationPolicy, PersistenceClass, ResourceRef, SchemaVersion, VolumeStateSchemaId,
};
use d2b_provider_volume_local::audit::{VolumeAuditEvent, VolumeAuditKind, VolumeAuditOutcome};
use d2b_provider_volume_local::otel::{
    METRICS, MetricLabelKey, RESOURCE_ATTRIBUTE_KEYS, ResourceAttributeKey,
};

const FORBIDDEN_KEYS: [&str; 14] = [
    "pid",
    "pidfd",
    "unit",
    "invocation",
    "cgroup",
    "path",
    "argv",
    "command",
    "binary",
    "env",
    "uid",
    "gid",
    "credential",
    "content",
];

fn provisioned_event() -> VolumeAuditEvent {
    VolumeAuditEvent::new(
        VolumeAuditKind::VolumeProvisioned,
        ZonePath::new(vec![ZoneLabelId::parse("work").unwrap()]).unwrap(),
        ResourceRef::parse("Volume/controller-state").unwrap(),
        VolumeAuditOutcome::Succeeded,
    )
    .with_schema(
        VolumeStateSchemaId::parse("example-provider.d2bus.org/controller/main-state").unwrap(),
        SchemaVersion::new(1, 0).unwrap(),
    )
    .with_persistence(PersistenceClass::Persistent)
}

#[test]
fn audit_golden_record_is_bounded_and_carries_no_sensitive_field_class() {
    let event = provisioned_event();
    let rendered = serde_json::to_string(&event).unwrap();
    assert_eq!(
        rendered,
        concat!(
            r#"{"kind":"volume-provisioned","zone":["work"],"volumeRef":"Volume/controller-state","outcome":"succeeded","schemaId":"example-provider.d2bus.org/controller/main-state","schemaVersion":"1.0","persistenceClass":"persistent"}"#
        )
    );
    let value: serde_json::Value = serde_json::from_str(&rendered).unwrap();
    for key in value.as_object().unwrap().keys() {
        let lower = key.to_ascii_lowercase();
        assert!(
            FORBIDDEN_KEYS
                .iter()
                .all(|fragment| !lower.contains(fragment)),
            "audit key {key:?} is forbidden"
        );
    }
    let diagnostic = format!("{event:?}").to_ascii_lowercase();
    assert!(!diagnostic.contains("work"));
    assert!(!diagnostic.contains("controller-state"));
    assert!(!diagnostic.contains("example-provider"));
}

#[test]
fn migration_record_uses_closed_fields_only() {
    let event = VolumeAuditEvent::new(
        VolumeAuditKind::VolumeMigrationStart,
        ZonePath::local_root(),
        ResourceRef::parse("Volume/controller-state").unwrap(),
        VolumeAuditOutcome::Succeeded,
    )
    .with_migration(
        SchemaVersion::new(1, 0).unwrap(),
        SchemaVersion::new(2, 0).unwrap(),
        MigrationPolicy::PreLaunchRequired,
    );
    let value = serde_json::to_value(event).unwrap();
    let mut keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "fromVersion",
            "kind",
            "migrationPolicy",
            "outcome",
            "toVersion",
            "volumeRef",
            "zone",
        ]
    );
}

#[test]
fn telemetry_labels_are_structurally_closed_and_zone_is_resource_only() {
    for metric in METRICS {
        let mut keys = metric.labels.to_vec();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), metric.labels.len());
        for key in metric.labels {
            let rendered = key.as_str();
            assert!(!matches!(rendered, "vm" | "zone" | "zone_id" | "zone_uid"));
            assert!(!rendered.contains("resource_name"));
            assert!(
                !FORBIDDEN_KEYS
                    .iter()
                    .any(|fragment| rendered.contains(fragment))
            );
        }
    }
    assert!(RESOURCE_ATTRIBUTE_KEYS.contains(&ResourceAttributeKey::D2bZone));
    assert_eq!(ResourceAttributeKey::D2bZone.as_str(), "d2b.zone");
    assert!(
        METRICS
            .iter()
            .all(|metric| !metric.labels.contains(&MetricLabelKey::Provider)
                || metric.labels.first() == Some(&MetricLabelKey::Provider))
    );
}
