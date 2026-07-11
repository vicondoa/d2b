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

#[test]
fn overview_dashboard_covers_redacted_shell_lifecycle() {
    let dashboard: serde_json::Value = serde_json::from_str(include_str!(
        "../../../nixos-modules/components/observability/dashboards/01-d2b-overview.json"
    ))
    .expect("overview dashboard is valid JSON");
    let panel = dashboard["panels"]
        .as_array()
        .expect("dashboard panels")
        .iter()
        .find(|panel| {
            serde_json::to_string(panel)
                .expect("panel serializes")
                .contains("d2b_daemon_shell_lifecycle_total")
        })
        .expect("shell lifecycle panel");
    let rendered = format!(
        "{} {}",
        panel["targets"][0]["expr"].as_str().expect("shell query"),
        panel["targets"][0]["legendFormat"]
            .as_str()
            .expect("shell legend")
    );
    for label in ["provider", "operation", "outcome", "error_kind"] {
        assert!(rendered.contains(label), "{label}: {rendered}");
    }

    for forbidden in [
        "uid",
        "target",
        "name",
        "session",
        "supervisor",
        "terminal_bytes",
        "argv",
        "environment",
        "cwd",
        "path",
        "pid",
        "unit",
    ] {
        assert!(!rendered.contains(forbidden), "{forbidden}: {rendered}");
    }
}

#[test]
fn runtime_emits_only_provider_neutral_shell_audit_events() {
    let daemon = include_str!("../../d2bd/src/lib.rs");
    let audit = include_str!("../../d2bd/src/daemon_audit.rs");
    assert!(
        !daemon.contains("DaemonEvent::GuestControlShellAttached"),
        "runtime must not dual-emit legacy shell attach audit"
    );
    assert!(
        !daemon.contains("DaemonEvent::GuestControlShellDetached"),
        "runtime must not dual-emit legacy shell detach audit"
    );
    assert!(daemon.contains("DaemonEvent::ShellLifecycle"));
    assert!(!audit.contains("GuestControlShellAttached"));
    assert!(!audit.contains("GuestControlShellDetached"));
}
