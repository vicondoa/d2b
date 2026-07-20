//! Typed top-level `d2b list` daemon-unavailable contract.

use std::process::Command;

use serde_json::Value;

#[test]
fn list_json_missing_daemon_emits_typed_daemon_down() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = Command::new(env!("CARGO_BIN_EXE_d2b"))
        .args(["list", "--json"])
        .env("D2B_PUBLIC_SOCKET", tmp.path().join("missing-public.sock"))
        .output()
        .expect("spawn d2b list --json");

    assert_eq!(out.status.code(), Some(1));
    assert!(
        out.stderr.is_empty(),
        "JSON daemon-down must not write stderr"
    );
    let envelope: Value =
        serde_json::from_slice(&out.stdout).expect("typed daemon-down JSON envelope");
    assert_eq!(envelope["code"], "daemon-down");
    assert_eq!(envelope["exitCode"], 1);
    assert!(
        envelope.get("items").is_none(),
        "daemon-down must not resemble a static inventory"
    );
}
