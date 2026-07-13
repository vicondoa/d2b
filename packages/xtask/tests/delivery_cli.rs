#![forbid(unsafe_code)]

use std::process::Command;

#[test]
fn wave_help_lists_the_end_to_end_machine_readable_workflow() {
    let binary = env!("CARGO_BIN_EXE_xtask");
    let output = Command::new(binary)
        .args(["delivery", "wave", "help"])
        .output()
        .expect("run xtask");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("machine-readable help JSON");
    assert_eq!(value["operation"], "help");
    let stages = value["stages"].as_array().expect("stages");
    for stage in [
        "snapshot",
        "validation-run",
        "validation-import",
        "panel-request",
        "panel-attest",
        "seal",
        "eligibility",
        "history-proof",
        "retarget-preflight",
        "merge",
    ] {
        assert!(stages.iter().any(|value| value == stage), "missing {stage}");
    }
    assert!(
        value["integration_points"]
            .as_array()
            .expect("integration points")
            .iter()
            .any(|value| value == "external-layer1-renderer")
    );
    let commands = value["commands"].as_array().expect("command help");
    let merge = commands
        .iter()
        .find(|command| command["name"] == "merge")
        .expect("merge help");
    assert!(
        merge["purpose"]
            .as_str()
            .expect("purpose")
            .contains("expected-head")
    );
}

#[test]
fn old_caller_authored_stack_manifest_surface_is_not_accepted() {
    let binary = env!("CARGO_BIN_EXE_xtask");
    let output = Command::new(binary)
        .args(["stack", "validate", "--manifest", "caller.json"])
        .output()
        .expect("run xtask");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage:"));
}
