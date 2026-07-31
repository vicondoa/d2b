use d2b_contracts::v3::volume::EntryType;
use d2b_contracts::v3::zone_routing::{ZoneLabelId, ZonePath};
use d2b_contracts::v3::{
    MigrationPolicy, PersistenceClass, ResourceGeneration, ResourceRef, SchemaVersion,
    VolumeStateSchemaId,
};
use d2b_provider_volume_local::audit::{
    VolumeAuditDigest, VolumeAuditEvent, VolumeAuditKind, VolumeAuditOutcome, VolumeAuditReason,
    VolumeAuditResultClass, VolumeBrokerAuditKind, VolumeRepairAction,
};
use d2b_provider_volume_local::otel::{
    METRICS, MetricAccess, MetricKind, MetricLabelKey, MetricLabelValue, MetricOutcome,
    MetricTrigger, MetricUnit, MetricView, OperationLabel, PersistenceLabel, ProviderLabel,
    RESOURCE_ATTRIBUTE_KEYS, ResourceAttributeKey, SchemaLabel, SchemaVersionLabel,
    SourceKindLabel, validate_labels,
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
        r#"{"kind":"volume-provisioned","zone":["work"],"volumeRef":"Volume/controller-state","outcome":"succeeded","schemaId":"example-provider.d2bus.org/controller/main-state","schemaVersion":"1.0","persistenceClass":"persistent"}"#
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

#[test]
fn audit_catalogues_match_the_provider_contract() {
    let provider_kinds: Vec<String> = VolumeAuditKind::ALL
        .iter()
        .map(|kind| serde_json::to_string(kind).unwrap())
        .collect();
    assert_eq!(
        provider_kinds,
        [
            r#""volume-provisioned""#,
            r#""volume-layout-repaired""#,
            r#""volume-migration-start""#,
            r#""volume-migration-committed""#,
            r#""volume-migration-failed""#,
            r#""volume-migration-rolled-back""#,
            r#""volume-snapshot-created""#,
            r#""volume-relocation-start""#,
            r#""volume-relocation-committed""#,
            r#""volume-incident-hold-set""#,
            r#""volume-incident-hold-cleared""#,
            r#""volume-sealing-rotation-start""#,
            r#""volume-sealing-rotation-failed""#,
            r#""volume-sealing-rotation-committed""#,
            r#""volume-destroyed""#,
            r#""volume-marker-check""#,
            r#""volume-quota-exceeded""#,
            r#""volume-store-sync-complete""#,
        ]
    );
    assert_eq!(
        VolumeBrokerAuditKind::ALL.map(VolumeBrokerAuditKind::as_str),
        [
            "PrepareSwtpmDir",
            "ProvisionLayoutEntry",
            "RepairLayoutEntry",
            "CleanupLayoutEntry",
            "StoreSyncComplete",
            "RotateSealingKey",
        ]
    );
}

#[test]
fn new_audit_event_payloads_are_typed_and_path_free() {
    let event = |kind, outcome| {
        VolumeAuditEvent::new(
            kind,
            ZonePath::local_root(),
            ResourceRef::parse("Volume/controller-state").unwrap(),
            outcome,
        )
    };
    let generation = |value| ResourceGeneration::new(value).unwrap();
    let actor_digest = VolumeAuditDigest::actor(b"User/alice");
    let operation_digest = VolumeAuditDigest::operation_id(b"private-operation-id");
    assert_eq!(format!("{actor_digest:?}"), "VolumeAuditDigest(<redacted>)");
    assert_eq!(actor_digest.to_string(), "VolumeAuditDigest(<redacted>)");
    let cases = [
        (
            event(
                VolumeAuditKind::VolumeLayoutRepaired,
                VolumeAuditOutcome::Succeeded,
            )
            .with_layout_repair(EntryType::Directory, VolumeRepairAction::Combined),
            &["entryType", "actionClass"] as &[_],
        ),
        (
            event(
                VolumeAuditKind::VolumeSealingRotationFailed,
                VolumeAuditOutcome::Failed,
            )
            .with_generation_transition(generation(3), generation(4))
            .with_reason(VolumeAuditReason::SealingFailed),
            &["fromGeneration", "toGeneration", "reason"],
        ),
        (
            event(
                VolumeAuditKind::VolumeMarkerCheck,
                VolumeAuditOutcome::Succeeded,
            )
            .with_result_class(VolumeAuditResultClass::Verified),
            &["resultClass"],
        ),
        (
            event(
                VolumeAuditKind::VolumeQuotaExceeded,
                VolumeAuditOutcome::Failed,
            ),
            &[],
        ),
        (
            event(
                VolumeAuditKind::VolumeIncidentHoldSet,
                VolumeAuditOutcome::Succeeded,
            )
            .with_actor_digest(actor_digest),
            &["actorDigest"],
        ),
        (
            event(
                VolumeAuditKind::VolumeSealingRotationStart,
                VolumeAuditOutcome::Succeeded,
            )
            .with_generation_transition(generation(3), generation(4))
            .with_operation_id_digest(operation_digest),
            &["fromGeneration", "toGeneration", "operationIdDigest"],
        ),
        (
            event(
                VolumeAuditKind::VolumeStoreSyncComplete,
                VolumeAuditOutcome::Succeeded,
            )
            .with_generation_number(generation(4)),
            &["generationNumber"],
        ),
    ];

    for (event, required) in cases {
        let value = serde_json::to_value(event).unwrap();
        for field in required {
            assert!(value.get(field).is_some(), "missing audit field {field}");
        }
        for digest in [value.get("actorDigest"), value.get("operationIdDigest")]
            .into_iter()
            .flatten()
        {
            let digest = digest.as_str().unwrap();
            assert_eq!(digest.len(), 71);
            assert!(digest.starts_with("sha256:"));
            assert!(!digest.contains("alice"));
            assert!(!digest.contains("operation"));
        }
        for key in value.as_object().unwrap().keys() {
            let lower = key.to_ascii_lowercase();
            assert!(
                FORBIDDEN_KEYS
                    .iter()
                    .all(|fragment| !lower.contains(fragment)),
                "audit key {key:?} is forbidden"
            );
        }
    }
}

#[test]
fn metric_catalogue_matches_the_provider_contract() {
    let actual: Vec<(&str, MetricKind, MetricUnit, Vec<MetricLabelKey>)> = METRICS
        .iter()
        .map(|metric| {
            (
                metric.name,
                metric.kind,
                metric.unit,
                metric.labels.to_vec(),
            )
        })
        .collect();
    assert_eq!(
        actual,
        [
            (
                "d2b_volume_provision_total",
                MetricKind::Counter,
                MetricUnit::Count,
                vec![
                    MetricLabelKey::Provider,
                    MetricLabelKey::PersistenceClass,
                    MetricLabelKey::SourceKind,
                    MetricLabelKey::Outcome
                ]
            ),
            (
                "d2b_volume_provision_duration_ms",
                MetricKind::Histogram,
                MetricUnit::Milliseconds,
                vec![MetricLabelKey::Provider, MetricLabelKey::SourceKind]
            ),
            (
                "d2b_volume_layout_repair_total",
                MetricKind::Counter,
                MetricUnit::Count,
                vec![MetricLabelKey::Provider, MetricLabelKey::Outcome]
            ),
            (
                "d2b_volume_state_size_bytes",
                MetricKind::Gauge,
                MetricUnit::Bytes,
                vec![MetricLabelKey::Provider, MetricLabelKey::SchemaId]
            ),
            (
                "d2b_volume_state_migration_total",
                MetricKind::Counter,
                MetricUnit::Count,
                vec![
                    MetricLabelKey::Provider,
                    MetricLabelKey::SchemaId,
                    MetricLabelKey::Outcome
                ]
            ),
            (
                "d2b_volume_state_migration_duration_ms",
                MetricKind::Histogram,
                MetricUnit::Milliseconds,
                vec![MetricLabelKey::Provider, MetricLabelKey::SchemaId]
            ),
            (
                "d2b_volume_state_snapshot_total",
                MetricKind::Counter,
                MetricUnit::Count,
                vec![
                    MetricLabelKey::Provider,
                    MetricLabelKey::SchemaId,
                    MetricLabelKey::Trigger
                ]
            ),
            (
                "d2b_volume_state_marker_check_total",
                MetricKind::Counter,
                MetricUnit::Count,
                vec![MetricLabelKey::Provider, MetricLabelKey::Outcome]
            ),
            (
                "d2b_volume_state_quota_exceeded_total",
                MetricKind::Counter,
                MetricUnit::Count,
                vec![MetricLabelKey::Provider]
            ),
            (
                "d2b_volume_store_sync_total",
                MetricKind::Counter,
                MetricUnit::Count,
                vec![MetricLabelKey::Provider, MetricLabelKey::Outcome]
            ),
            (
                "d2b_volume_store_sync_duration_ms",
                MetricKind::Histogram,
                MetricUnit::Milliseconds,
                vec![MetricLabelKey::Provider]
            ),
            (
                "d2b_volume_relocation_total",
                MetricKind::Counter,
                MetricUnit::Count,
                vec![MetricLabelKey::Provider, MetricLabelKey::Outcome]
            ),
            (
                "d2b_volume_sealing_rotation_total",
                MetricKind::Counter,
                MetricUnit::Count,
                vec![MetricLabelKey::Provider, MetricLabelKey::Outcome]
            ),
            (
                "d2b_volume_unclaimed_gc_total",
                MetricKind::Counter,
                MetricUnit::Count,
                vec![MetricLabelKey::Provider, MetricLabelKey::PersistenceClass]
            ),
            (
                "d2b_volume_fd_handoff_total",
                MetricKind::Counter,
                MetricUnit::Count,
                vec![
                    MetricLabelKey::Provider,
                    MetricLabelKey::View,
                    MetricLabelKey::Access,
                    MetricLabelKey::Outcome
                ]
            ),
        ]
    );
}

#[test]
fn metric_validation_accepts_only_closed_value_types_in_descriptor_order() {
    let provision = METRICS
        .iter()
        .find(|metric| metric.name == "d2b_volume_provision_total")
        .unwrap();
    assert!(validate_labels(
        provision,
        &[
            MetricLabelValue::Provider(ProviderLabel::VolumeLocal),
            MetricLabelValue::Persistence(PersistenceLabel::Persistent),
            MetricLabelValue::SourceKind(SourceKindLabel::LocalPath),
            MetricLabelValue::Outcome(MetricOutcome::Succeeded),
        ]
    ));
    assert!(!validate_labels(
        provision,
        &[
            MetricLabelValue::Provider(ProviderLabel::VolumeLocal),
            MetricLabelValue::SourceKind(SourceKindLabel::LocalPath),
            MetricLabelValue::Persistence(PersistenceLabel::Persistent),
            MetricLabelValue::Outcome(MetricOutcome::Succeeded),
        ]
    ));

    let handoff = METRICS
        .iter()
        .find(|metric| metric.name == "d2b_volume_fd_handoff_total")
        .unwrap();
    let labels = [
        MetricLabelValue::Provider(ProviderLabel::VolumeLocal),
        MetricLabelValue::View(MetricView::Subtree),
        MetricLabelValue::Access(MetricAccess::ReadOnly),
        MetricLabelValue::Outcome(MetricOutcome::Succeeded),
    ];
    assert!(validate_labels(handoff, &labels));
    assert_eq!(
        labels.map(MetricLabelValue::as_str),
        ["volume-local", "subtree", "read-only", "succeeded"]
    );
}

#[test]
fn metric_label_keys_and_values_are_exact_closed_sets() {
    assert_eq!(
        MetricLabelKey::ALL.map(MetricLabelKey::as_str),
        [
            "provider",
            "schema_id",
            "schema_version",
            "persistence_class",
            "source_kind",
            "operation",
            "trigger",
            "view",
            "access",
            "outcome",
        ]
    );

    assert_eq!(
        ProviderLabel::ALL.map(ProviderLabel::as_str),
        ["volume-local"]
    );
    assert_eq!(
        SchemaLabel::ALL.map(SchemaLabel::as_str),
        ["provider-state", "store-view", "swtpm-state"]
    );
    assert_eq!(
        SchemaVersionLabel::ALL.map(SchemaVersionLabel::as_str),
        ["current", "migration-required"]
    );
    assert_eq!(
        PersistenceLabel::ALL.map(PersistenceLabel::as_str),
        ["persistent", "ephemeral", "cache", "config"]
    );
    assert_eq!(
        SourceKindLabel::ALL.map(SourceKindLabel::as_str),
        ["local-path", "block-image", "tmpfs"]
    );
    assert_eq!(
        OperationLabel::ALL.map(OperationLabel::as_str),
        [
            "provision",
            "layout-repair",
            "migration",
            "snapshot",
            "marker-check",
            "store-sync",
            "relocation",
            "sealing-rotation",
            "unclaimed-gc",
            "fd-handoff",
        ]
    );
    assert_eq!(
        MetricTrigger::ALL.map(MetricTrigger::as_str),
        ["manual", "pre-migration", "pre-relocation"]
    );
    assert_eq!(MetricView::ALL.map(MetricView::as_str), ["root", "subtree"]);
    assert_eq!(
        MetricAccess::ALL.map(MetricAccess::as_str),
        ["read-only", "read-write", "shared-write"]
    );
    assert_eq!(
        MetricOutcome::ALL.map(MetricOutcome::as_str),
        [
            "succeeded",
            "failed",
            "verified",
            "missing",
            "replaced",
            "retryable",
            "recovered",
        ]
    );
}
