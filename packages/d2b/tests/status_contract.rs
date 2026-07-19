//! Local `d2b status --check-bridges` contract.

use std::process::Command;

use d2b_contracts::cli_output::{StatusBridgeCheckOutputV2, StatusOutputV2};

#[test]
fn status_check_bridges_returns_frozen_not_yet_implemented_envelope() {
    let out = Command::new(env!("CARGO_BIN_EXE_d2b"))
        .args(["status", "--check-bridges", "--json"])
        .output()
        .expect("spawn d2b status --check-bridges --json");
    assert!(
        out.status.success(),
        "status --check-bridges exited {:?}; stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );

    let classified: StatusOutputV2 =
        serde_json::from_slice(&out.stdout).expect("typed status output");
    assert!(matches!(classified, StatusOutputV2::CheckBridges(_)));

    let envelope: StatusBridgeCheckOutputV2 =
        serde_json::from_slice(&out.stdout).expect("strict check-bridges output");
    assert_eq!(envelope.mode, "check-bridges");
    assert_eq!(envelope.status, "not-yet-implemented");
}
