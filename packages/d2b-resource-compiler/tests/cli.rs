//! Deterministic CLI integration tests for the Nix build entry point.

use std::{fs, process::Command};

use serde_json::json;
use tempfile::tempdir;

fn run(input: &serde_json::Value, strict: bool) -> std::process::Output {
    let directory = tempdir().expect("temporary compiler directory");
    let input_path = directory.path().join("input.json");
    let output_path = directory.path().join("bundle.json");
    let catalog_path = directory.path().join("artifact-catalog.json");
    fs::write(
        &catalog_path,
        br#"{"catalogDigest":"sha256:0000000000000000000000000000000000000000000000000000000000000001","entries":[],"schemaVersion":3}"#,
    )
    .expect("write catalog");
    let mut input = input.clone();
    input["artifactCatalogPath"] = json!(catalog_path);
    input["expectedArtifactCatalogDigest"] =
        json!("sha256:0000000000000000000000000000000000000000000000000000000000000001");
    input["strictSecrets"] = json!(strict);
    fs::write(
        &input_path,
        serde_json::to_vec(&input).expect("serialize compiler input"),
    )
    .expect("write compiler input");
    Command::new(env!("CARGO_BIN_EXE_d2b-resource-compiler"))
        .args([
            "compile",
            "--input",
            input_path.to_str().expect("input path"),
            "--output",
            output_path.to_str().expect("output path"),
        ])
        .output()
        .expect("run resource compiler")
}

fn empty_input() -> serde_json::Value {
    json!({
        "zone": "local-root",
        "resources": [],
        "providerSchemaDigests": {},
    })
}

#[test]
fn cli_emits_a_stable_bundle_from_declared_inputs() {
    let input = empty_input();
    let directory = tempdir().expect("temporary compiler directory");
    let input_path = directory.path().join("input.json");
    let catalog_path = directory.path().join("artifact-catalog.json");
    let output_a = directory.path().join("bundle-a.json");
    let output_b = directory.path().join("bundle-b.json");
    let mut declared = input;
    declared["artifactCatalogPath"] = json!(catalog_path);
    declared["expectedArtifactCatalogDigest"] =
        json!("sha256:0000000000000000000000000000000000000000000000000000000000000001");
    fs::write(
        &catalog_path,
        br#"{"catalogDigest":"sha256:0000000000000000000000000000000000000000000000000000000000000001","entries":[],"schemaVersion":3}"#,
    )
    .expect("write catalog");
    fs::write(
        &input_path,
        serde_json::to_vec(&declared).expect("serialize input"),
    )
    .expect("write input");
    for output in [&output_a, &output_b] {
        let result = Command::new(env!("CARGO_BIN_EXE_d2b-resource-compiler"))
            .args([
                "compile",
                "--input",
                input_path.to_str().expect("input path"),
                "--output",
                output.to_str().expect("output path"),
            ])
            .output()
            .expect("run compiler");
        assert!(result.status.success(), "{result:?}");
    }
    assert_eq!(
        fs::read(&output_a).expect("read first output"),
        fs::read(&output_b).expect("read second output")
    );
}

#[test]
fn cli_strict_secret_mode_fails_closed() {
    let mut input = empty_input();
    input["resources"] = json!([{
        "apiVersion": "resources.d2bus.org/v3",
        "type": "User",
        "metadata": {"name": "user", "zone": "local-root"},
        "spec": {"token": "inline-secret"}
    }]);
    let output = run(&input, true);
    assert!(!output.status.success());
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(message.contains("resource-compiler-inline-secret"));
    assert!(!message.contains("contains inline-secret"), "{message}");
}

#[test]
fn cli_rejects_unsorted_resources() {
    let mut input = empty_input();
    input["resources"] = json!([
        {
            "apiVersion": "resources.d2bus.org/v3",
            "type": "User",
            "metadata": {"name": "user", "zone": "local-root"},
            "spec": {}
        },
        {
            "apiVersion": "resources.d2bus.org/v3",
            "type": "User",
            "metadata": {"name": "a", "zone": "local-root"},
            "spec": {}
        }
    ]);
    let output = run(&input, false);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("resource-compiler-resource-order-invalid"),
        "{output:?}"
    );
}
