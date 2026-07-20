use std::process::Command;

#[test]
fn standalone_startup_fails_closed_without_systemd_activation() {
    let output = Command::new(env!("CARGO_BIN_EXE_d2b-clipd"))
        .output()
        .expect("spawn d2b-clipd");

    assert_eq!(output.status.code(), Some(78));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "d2b-clipd: clipboard-activation-invalid\n"
    );
}

#[test]
fn legacy_startup_arguments_cannot_reenable_removed_composition() {
    let output = Command::new(env!("CARGO_BIN_EXE_d2b-clipd"))
        .args([
            "--config",
            "/etc/d2b/clipboard.json",
            "--bridge-root",
            "/run/d2b/clipd",
            "--picker",
            "/run/current-system/sw/bin/d2b-clip-picker",
            "--oneshot",
        ])
        .output()
        .expect("spawn d2b-clipd");

    assert_eq!(output.status.code(), Some(78));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "d2b-clipd: clipboard-activation-invalid\n"
    );
}

#[test]
fn binary_has_no_legacy_socket_picker_or_wayland_composition() {
    let source = include_str!("../src/main.rs");
    for forbidden in [
        "UnixListener",
        "UnixStream",
        "CommandPickerSpawner",
        "DataControlClient",
        "thread::park()",
        "--bridge-root",
        "--picker",
        "--oneshot",
    ] {
        assert!(
            !source.contains(forbidden),
            "legacy composition marker remains: {forbidden}"
        );
    }
    assert!(source.contains("d2b_clipd::daemon::run()"));
}

#[test]
fn daemon_composition_names_only_the_three_systemd_endpoints() {
    let source = include_str!("../src/daemon.rs");
    for required in [
        "\"clipboard-control\"",
        "\"clipboard-picker\"",
        "\"clipboard-bridge\"",
        "ActivatedSeqpacketListeners::from_systemd",
        "serve_ttrpc_services",
    ] {
        assert!(
            source.contains(required),
            "missing daemon marker: {required}"
        );
    }
    for forbidden in ["UnixListener", "bind(", "connect(", "read_line", "lines()"] {
        assert!(
            !source.contains(forbidden),
            "ambient or newline transport marker remains: {forbidden}"
        );
    }
}
