#[path = "../src/zone_doctor.rs"]
mod zone_doctor;

fn ready_input() -> zone_doctor::DoctorInput {
    zone_doctor::DoctorInput {
        zone_phase: zone_doctor::ZonePhase::Ready,
        store_health: zone_doctor::StoreHealth {
            phase: "ready".to_owned(),
            revision: 2,
            compaction_floor: 1,
            watch_active: 1,
        },
        controllers: vec![zone_doctor::ControllerHealth {
            handler: "provider".to_owned(),
            phase: "ready".to_owned(),
            queue_depth: 0,
            last_reconciled_at: "tick".to_owned(),
        }],
        providers: vec![zone_doctor::ProviderHealth {
            provider: "system-core".to_owned(),
            phase: "ready".to_owned(),
            component_phases: Default::default(),
        }],
        process_counts: zone_doctor::ProcessCounts {
            active: 1,
            failed: 0,
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
    }
}

#[test]
fn doctor_uses_zone_phase_and_omits_legacy_broker_ready() {
    let report = zone_doctor::build_report("work", ready_input());
    let json = zone_doctor::render_json(&report).unwrap();
    assert!(json.contains("\"zone_phase\""));
    assert!(!json.contains("\"broker_ready\""));
    assert_eq!(zone_doctor::exit_code(&report), 1);
}

#[test]
fn isolation_posture_check_is_conditional() {
    let mut input = ready_input();
    let report = zone_doctor::build_report("work", input.clone());
    assert!(
        !report
            .checks
            .iter()
            .any(|check| check.name == "isolation-posture-declared")
    );
    input.user_only_hosts = 1;
    let report = zone_doctor::build_report("work", input);
    assert!(
        report
            .checks
            .iter()
            .any(|check| check.name == "isolation-posture-declared")
    );
}
