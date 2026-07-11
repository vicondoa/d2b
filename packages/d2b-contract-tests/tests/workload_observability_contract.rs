#[test]
fn overview_dashboard_covers_workload_provider_signals() {
    let dashboard: serde_json::Value = serde_json::from_str(include_str!(
        "../../../nixos-modules/components/observability/dashboards/01-d2b-overview.json"
    ))
    .expect("overview dashboard is valid JSON");
    let rendered = serde_json::to_string(&dashboard).expect("dashboard serializes");
    assert!(rendered.contains("d2b_daemon_workload_availability"));
    assert!(rendered.contains("d2b_daemon_workload_lifecycle_total"));
    for forbidden in ["argv", "environment", "cwd", "process_id", "unit_name"] {
        assert!(!rendered.contains(forbidden));
    }
}
