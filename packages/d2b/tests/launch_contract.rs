use std::process::Command;

#[test]
fn launch_has_no_static_or_ssh_fallback_when_daemon_is_missing() {
    let dir = tempfile::tempdir().expect("test dir");
    let output = Command::new(env!("CARGO_BIN_EXE_d2b"))
        .args(["launch", "browser.host.d2b", "--item", "browser"])
        .env("D2B_PUBLIC_SOCKET", dir.path().join("missing-public.sock"))
        .env("D2B_BUNDLE_PATH", dir.path().join("missing-bundle.json"))
        .output()
        .expect("spawn d2b launch");
    assert_eq!(output.status.code(), Some(69));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no static or provider fallback"));
    assert!(!stderr.contains("ssh"));
    assert!(!stderr.contains("sudo"));
}

#[test]
fn launch_rejects_public_command_arguments() {
    let output = Command::new(env!("CARGO_BIN_EXE_d2b"))
        .args([
            "launch",
            "browser.host.d2b",
            "--item",
            "browser",
            "--",
            "private-canary",
        ])
        .output()
        .expect("spawn d2b launch");
    assert_eq!(output.status.code(), Some(2));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("private-canary"));
}
