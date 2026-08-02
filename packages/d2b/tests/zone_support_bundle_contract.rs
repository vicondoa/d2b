#[path = "../src/zone_doctor.rs"]
mod zone_doctor;
#[path = "../src/zone_support_bundle.rs"]
mod zone_support_bundle;

#[test]
fn support_bundle_is_bounded_and_partial_on_quarantine() {
    let input = zone_doctor::DoctorInput {
        zone_phase: zone_doctor::ZonePhase::Failed,
        store_health: zone_doctor::StoreHealth {
            phase: "quarantined".to_owned(),
            revision: 1,
            compaction_floor: 1,
            watch_active: 0,
        },
        controllers: Vec::new(),
        providers: Vec::new(),
        process_counts: zone_doctor::ProcessCounts {
            active: 0,
            failed: 1,
        },
        audit: zone_doctor::AuditHealth {
            phase: "ok".to_owned(),
            segments: 1,
            drop_privileged: 0,
            drop_total: 0,
        },
        telemetry: zone_doctor::TelemetryHealth {
            phase: "unavailable".to_owned(),
            drop_total: 1,
        },
        schema_catalog_consistent: true,
        watch_quota_headroom: true,
        audit_hash_chain_clean: true,
        user_only_hosts: 0,
        user_only_hosts_declared: 0,
    };
    let doctor = zone_doctor::build_report("work", input);
    let bundle = zone_support_bundle::build_bundle(
        doctor,
        true,
        (0..600)
            .map(|index| zone_support_bundle::ResourceStatusSnapshot {
                uid: format!("Provider:uid-{index}"),
                zone: "work".to_owned(),
                generation: 1,
                revision: 1,
                observed_at: "tick".to_owned(),
                phase: "degraded".to_owned(),
                conditions: vec!["store-quarantined".to_owned()],
                observed_generation: 1,
                outcome: None,
            })
            .collect(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        None,
        Vec::new(),
    );
    assert_eq!(bundle.bundle_completeness, "partial");
    assert!(bundle.resource_status.len() <= zone_support_bundle::MAX_SNAPSHOTS_PER_TYPE);
    let json = zone_support_bundle::render_ndjson(&bundle).unwrap();
    assert!(!json.contains("\"spec\""));
    assert!(!json.contains("\"metadata.name\""));
}
