#[test]
fn overview_dashboard_covers_workload_provider_signals() {
    let dashboard: serde_json::Value = serde_json::from_str(include_str!(
        "../../../nixos-modules/components/observability/dashboards/01-d2b-overview.json"
    ))
    .expect("overview dashboard is valid JSON");
    let workload_panels = dashboard["panels"]
        .as_array()
        .expect("dashboard panels")
        .iter()
        .filter(|panel| {
            let rendered = serde_json::to_string(panel).expect("panel serializes");
            rendered.contains("d2b_daemon_workload_")
        })
        .collect::<Vec<_>>();
    assert_eq!(workload_panels.len(), 2);
    let rendered = serde_json::to_string(&workload_panels).expect("workload panels serialize");
    assert!(rendered.contains("d2b_daemon_workload_availability"));
    assert!(rendered.contains("d2b_daemon_workload_lifecycle_total"));
    for forbidden in ["argv", "environment", "cwd", "process_id", "unit_name"] {
        assert!(!rendered.contains(forbidden), "{forbidden}: {rendered}");
    }
}
