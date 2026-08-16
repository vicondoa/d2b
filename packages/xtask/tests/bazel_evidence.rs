#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

fn repo_root() -> PathBuf {
    let mut candidates = Vec::new();
    if let Some(root) = std::env::var_os("D2B_REPO_ROOT") {
        candidates.push(PathBuf::from(root));
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("xtask lives under packages/xtask")
            .to_path_buf(),
    );
    for variable in ["TEST_SRCDIR", "RUNFILES_DIR"] {
        if let Some(base) = std::env::var_os(variable).map(PathBuf::from) {
            candidates.push(base.clone());
            if let Some(workspace) = std::env::var_os("TEST_WORKSPACE") {
                candidates.push(base.join(workspace));
            }
            candidates.push(base.join("_main"));
        }
    }
    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir);
    }
    for candidate in candidates {
        let mut path = candidate;
        loop {
            if path.join("Cargo.toml").is_file()
                && path.join("BUILD.bazel").is_file()
                && path.join("flake.nix").is_file()
            {
                return path;
            }
            if !path.pop() {
                break;
            }
        }
    }
    panic!("repository root is not discoverable")
}

fn run_xtask(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(args)
        .env("D2B_REPO_ROOT", repo_root())
        .output()
        .expect("run xtask")
}

fn scratch(name: &str) -> PathBuf {
    let base = std::env::var_os("TEST_UNDECLARED_OUTPUTS_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("TEST_TMPDIR").map(PathBuf::from))
        .unwrap_or_else(std::env::temp_dir);
    let path = base.join(format!("bazel-evidence-test-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&path).expect("create evidence scratch");
    path
}

#[test]
fn current_u9_evidence_is_required_before_remote_use() {
    let output = run_xtask(&["bazel-evidence", "check-u9"]);
    assert!(
        output.status.success(),
        "U9 evidence check failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("U9 check emits JSON evidence");
    assert_eq!(value["status"], "pass");
    assert_eq!(
        value["eligibilityDigest"],
        "sha256:62b4a9685445237db70b69d673b35205a1a18d835cf7ce7aed55e0edf43a8813"
    );
}

#[test]
fn stale_u9_digest_blocks_remote_profiles() {
    let directory = scratch("stale");
    let report_path = directory.join("representative.json");
    let report: Value = serde_json::from_str(
        &std::fs::read_to_string(
            repo_root().join("tests/golden/bazel/cache-transfer-representative.json"),
        )
        .expect("read representative report"),
    )
    .expect("parse representative report");
    let mut stale = report;
    stale["source"]["eligibilityDigest"] = json!("sha256:stale");
    std::fs::write(
        &report_path,
        serde_json::to_vec_pretty(&stale).expect("serialize stale report"),
    )
    .expect("write stale report");

    let output = run_xtask(&[
        "bazel-evidence",
        "check-u9",
        "--report",
        report_path.to_str().expect("report path"),
    ]);
    assert!(!output.status.success(), "stale evidence must fail closed");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("eligibility-digest"), "{stderr}");
}

#[test]
fn only_pre_dispatch_infrastructure_failures_allow_one_local_retry() {
    let directory = scratch("classification");
    let pre_dispatch = directory.join("pre-dispatch.log");
    std::fs::write(
        &pre_dispatch,
        "remote authentication failed: UNAUTHENTICATED\n",
    )
    .expect("write pre-dispatch log");
    let output = run_xtask(&[
        "bazel-evidence",
        "classify-failure",
        "--log",
        pre_dispatch.to_str().expect("pre-dispatch path"),
    ]);
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).expect("classification JSON");
    assert_eq!(value["kind"], "authentication");
    assert_eq!(value["retryLocally"], true);
    assert_eq!(value["maxLocalRetries"], 1);

    let post_dispatch = directory.join("post-dispatch.log");
    std::fs::write(
        &post_dispatch,
        "remote execution started\nremote authentication failed: UNAUTHENTICATED\n",
    )
    .expect("write post-dispatch log");
    let output = run_xtask(&[
        "bazel-evidence",
        "classify-failure",
        "--log",
        post_dispatch.to_str().expect("post-dispatch path"),
    ]);
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).expect("classification JSON");
    assert_eq!(value["kind"], "post-dispatch-uncertainty");
    assert_eq!(value["retryLocally"], false);
}

#[test]
fn redaction_removes_plain_encoded_and_split_sentinels_from_evidence() {
    let directory = scratch("redaction");
    let input = directory.join("bep.json");
    let output_path = directory.join("redacted.json");
    let plain = "plain-buildbuddy-secret";
    let encoded = "cGxhaW4tYnVpbGRidWRkeS1zZWNyZXQ=";
    let split = "x-buildbuddy-api-key=split-buildbuddy-secret";
    std::fs::write(
        &input,
        format!("plain={plain}\nencoded={encoded}\n{split}\n"),
    )
    .expect("write sentinel evidence");

    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args([
            "bazel-evidence",
            "redact-log",
            "--input",
            input.to_str().expect("input path"),
            "--output",
            output_path.to_str().expect("output path"),
        ])
        .env("D2B_REPO_ROOT", repo_root())
        .env(
            "D2B_BUILDBUDDY_SENTINELS",
            format!("{plain}|{encoded}|{split}"),
        )
        .output()
        .expect("run redaction");
    assert!(output.status.success(), "redaction failed");
    let redacted = std::fs::read_to_string(output_path).expect("read redacted evidence");
    for sentinel in [plain, encoded, split] {
        assert!(
            !redacted.contains(sentinel),
            "sentinel leaked into redacted evidence: {sentinel}"
        );
    }
}
