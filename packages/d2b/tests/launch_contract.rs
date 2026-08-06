use std::process::Command;

#[test]
fn retired_launch_is_rejected_without_fallback_or_argument_leakage() {
    let dir = tempfile::tempdir().expect("test dir");
    let output = Command::new(env!("CARGO_BIN_EXE_d2b"))
        .args(["launch", "browser.host.d2b", "--item", "browser"])
        .env("D2B_PUBLIC_SOCKET", dir.path().join("missing-public.sock"))
        .env("D2B_BUNDLE_PATH", dir.path().join("missing-bundle.json"))
        .output()
        .expect("spawn d2b launch");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unrecognized subcommand 'launch'"));
    assert!(stderr.contains("Usage: d2b"));
    assert!(!stderr.contains("browser.host.d2b"));
    assert!(!stderr.contains("browser"));
    assert!(!stderr.contains("static"));
    assert!(!stderr.contains("provider"));
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
    assert!(!String::from_utf8_lossy(&output.stderr).contains("private-canary"));
}
