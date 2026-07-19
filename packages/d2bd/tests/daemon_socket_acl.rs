use d2bd::daemon_service::{DaemonMethod, DaemonPeerRole};

#[test]
fn launcher_role_allows_configured_terminal_openers_but_not_lifecycle_mutation() {
    for method in [
        DaemonMethod::Resolve,
        DaemonMethod::ListRealms,
        DaemonMethod::ListWorkloads,
        DaemonMethod::Inspect,
        DaemonMethod::Exec,
        DaemonMethod::OpenConsole,
    ] {
        assert!(DaemonPeerRole::Launcher.permits(method), "{method:?}");
    }
    for method in [
        DaemonMethod::Apply,
        DaemonMethod::Start,
        DaemonMethod::Stop,
        DaemonMethod::Restart,
        DaemonMethod::Shell,
        DaemonMethod::ExportAudit,
    ] {
        assert!(!DaemonPeerRole::Launcher.permits(method), "{method:?}");
    }
}

#[test]
fn admin_role_can_dispatch_every_daemon_operation() {
    for method in [
        DaemonMethod::Resolve,
        DaemonMethod::ListRealms,
        DaemonMethod::ListWorkloads,
        DaemonMethod::Inspect,
        DaemonMethod::Apply,
        DaemonMethod::Start,
        DaemonMethod::Stop,
        DaemonMethod::Restart,
        DaemonMethod::Exec,
        DaemonMethod::Shell,
        DaemonMethod::OpenConsole,
        DaemonMethod::ExportAudit,
    ] {
        assert!(DaemonPeerRole::Admin.permits(method), "{method:?}");
    }
}

#[test]
fn shutdown_role_is_scoped_to_stop() {
    assert!(DaemonPeerRole::HostShutdown.permits(DaemonMethod::Stop));
    assert!(!DaemonPeerRole::HostShutdown.permits(DaemonMethod::Start));
    assert!(!DaemonPeerRole::HostShutdown.permits(DaemonMethod::Exec));
    assert!(!DaemonPeerRole::HostShutdown.permits(DaemonMethod::ExportAudit));
}
