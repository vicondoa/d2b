#![forbid(unsafe_code)]
#![recursion_limit = "256"]

use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

const TEST_NONCE: &str = "nonce-1";

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
    panic!("repository root is not discoverable");
}

fn current_commit() -> String {
    if let Some(commit) = std::env::var_os("D2B_QUALIFICATION_TEST_COMMIT") {
        return commit.to_string_lossy().into_owned();
    }
    String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(repo_root())
            .output()
            .expect("read commit")
            .stdout,
    )
    .expect("commit is utf8")
    .trim()
    .to_owned()
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after the Unix epoch")
        .as_millis() as u64
}

fn scratch(name: &str) -> PathBuf {
    let path = repo_root().join(format!(
        ".scratch/bazel-qualification-test-{}-{name}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).expect("create qualification scratch");
    path
}

fn write_json(path: &Path, value: &Value) {
    std::fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("serialize qualification fixture"),
    )
    .expect("write qualification fixture");
}

fn run_xtask(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(args)
        .env("D2B_REPO_ROOT", repo_root())
        .env("D2B_QUALIFICATION_TEST_MODE", "1")
        .env("D2B_QUALIFICATION_TEST_AUTH", "1")
        .output()
        .expect("run xtask")
}

fn run_untrusted_xtask(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(args)
        .env("D2B_REPO_ROOT", repo_root())
        .env_remove("D2B_QUALIFICATION_TEST_MODE")
        .env_remove("D2B_QUALIFICATION_TEST_AUTH")
        .output()
        .expect("run untrusted xtask")
}

fn candidate(commit: &str) -> Value {
    json!({
        "schemaVersion": 1,
        "commit": commit,
        "targetSet": ["//..."],
        "configuration": ".bazelrc",
        "selectedClosure": "tests/golden/bazel/eligibility.json",
        "namespace": "d2b/qualification/linux-x86_64/rules_rust/worker-v1/minimal/lock-v1",
        "toolchain": "rules_rust",
        "coverage": {
            "currentScheduler": "pass",
            "bazel": "pass",
            "seedFailuresObserved": true,
            "equivalentTargetSet": true
        },
        "cache": {
            "trustedSeedComplete": true,
            "asyncUploadsDrained": true,
            "unchangedCacheableExecutions": 0,
            "approvedUncacheableReasons": [],
            "cacheMatrix": {
                "warm": true,
                "unchanged": true,
                "sourceInvalidation": true,
                "toolchainInvalidation": true,
                "featureInvalidation": true,
                "lockInvalidation": true,
                "platformInvalidation": true,
                "agedCache": true,
                "eviction": true,
                "compression": true,
                "architecture": true,
                "empty": true,
                "workerImageInvalidation": true,
                "crossMachine": true,
                "retry": true,
                "fallback": true
            }
        },
        "fallback": {
            "status": "not-used",
            "localRetryCount": 0,
            "maxLocalRetries": 1,
            "identicalTargetSet": true
        }
    })
}

fn provider_evidence(observed_at: u64) -> Value {
    let commit = current_commit();
    let sample = json!({
        "schemaVersion": 1,
        "provider": "buildbuddy",
        "projection": "xtask-buildbuddy-probe/v1",
        "source": "credential-helper-probe",
        "status": "qualified",
        "providerAccountedTransfer": true,
        "probe": {
            "kind": "credential-isolated-command",
            "command": "xtask buildbuddy-probe",
            "input": "D2B_BUILDBUDDY_EVIDENCE_FILE",
            "readOnly": true,
            "fixtureSafe": true,
            "credentialMode": "credential-helper",
            "nonce": TEST_NONCE
        },
        "authenticated": true,
        "executionEntitled": true,
        "cacheReadEvidence": true,
        "cacheWriteEvidence": true,
        "readOnlyProbe": true,
        "uploadsDisabled": false,
        "secretRedaction": true,
        "trustedSeed": true,
        "dispatchEvidence": true,
        "invocationId": "invocation-1",
        "sampleId": "sample-1",
        "observedAtMillis": observed_at,
        "workerArchitecture": "linux-x86_64",
        "workerArchitectures": ["linux-x86_64"],
        "workerImage": "d2b-bazel-worker/v1",
        "sampleClass": "fresh-worktree",
        "freshWorktree": true,
        "isolatedServer": true,
        "localDiskCacheDisabled": true,
        "cacheState": "populated",
        "worktreeId": "worktree-1",
        "outputRootId": "output-root-1",
        "outputBaseId": "output-base-1",
        "bazelServerId": "bazel-server-1",
        "localCacheId": "local-cache-1",
        "commit": commit,
        "identity": {
            "commit": commit,
            "targetSetDigest": "sha256:576bbb5fd15ccdd2ae7db72515aefdf66b2413a60687921d1077f7dab5593dae",
            "configurationDigest": "sha256:235901475fb814988c5b6a5672cae92fb6c091dab92ff2ae8abfc33fb41b3436",
            "selectedClosureDigest": "sha256:3e54856cbb0b16d56c8a5482450ab66b9e725c7141c87d2c47a5ab5c80395898",
            "namespace": "d2b/qualification/linux-x86_64/rules_rust/worker-v1/minimal/lock-v1",
            "toolchain": "rules_rust",
            "platform": "linux-x86_64"
        },
        "transferBytes": {
            "uploaded": 1034798612u64,
            "downloaded": 355161088u64
        },
        "qualificationMetrics": {
            "wallTimeMillis": 1200,
            "actionCacheHits": 10,
            "actionCacheMisses": 0,
            "casHits": 20,
            "casMisses": 0,
            "remoteExecutions": 4,
            "repositoryTrafficBytes": 30,
            "besTrafficBytes": 40,
            "retryTrafficBytes": 0,
            "localNixMillis": 50
        }
    });
    let samples = (0..5)
        .map(|index| {
            let mut sample = sample.clone();
            sample["invocationId"] = json!(format!("invocation-{index}"));
            sample["sampleId"] = json!(format!("sample-{index}"));
            sample["observedAtMillis"] = json!(observed_at);
            sample["qualificationMetrics"]["wallTimeMillis"] = json!(1200 + index * 10);
            sample["worktreeId"] = json!(format!("worktree-{index}"));
            sample["outputRootId"] = json!(format!("output-root-{index}"));
            sample["outputBaseId"] = json!(format!("output-base-{index}"));
            sample["bazelServerId"] = json!(format!("bazel-server-{index}"));
            sample["localCacheId"] = json!(format!("local-cache-{index}"));
            sample
        })
        .collect::<Vec<_>>();
    json!({ "samples": samples })
}

fn for_each_sample(provider: &mut Value, mut update: impl FnMut(&mut Value)) {
    for sample in provider["samples"]
        .as_array_mut()
        .expect("provider samples")
    {
        update(sample);
    }
}

fn run_acceptance(candidate_value: &Value, provider_value: &Value, name: &str) -> Output {
    let directory = scratch(name);
    let candidate_path = directory.join("candidate.json");
    let provider_path = directory.join("provider.json");
    write_json(&candidate_path, candidate_value);
    write_json(&provider_path, provider_value);
    let now = now_millis().to_string();
    let output = run_xtask(&[
        "bazel-qualification",
        "acceptance",
        "--candidate",
        candidate_path.to_str().expect("candidate path"),
        "--provider-evidence",
        provider_path.to_str().expect("provider path"),
        "--now-millis",
        &now,
        "--nonce",
        TEST_NONCE,
        "--report-only",
    ]);
    let _ = std::fs::remove_dir_all(directory);
    output
}

#[test]
fn typed_fallback_retries_only_pre_dispatch_infrastructure_once() {
    for class in [
        "missing-credentials",
        "authentication",
        "endpoint",
        "worker",
        "transport",
    ] {
        let output = run_xtask(&[
            "bazel-qualification",
            "typed-fallback",
            "--class",
            class,
            "--dispatch-started",
            "false",
            "--attempt",
            "0",
        ]);
        assert!(output.status.success(), "{class}: {:?}", output);
        let value: Value = serde_json::from_slice(&output.stdout).expect("fallback JSON");
        assert_eq!(value["retryLocally"], true, "{class}");
        assert_eq!(value["maxLocalRetries"], 1, "{class}");
    }

    for (class, dispatch_started, attempt) in [
        ("authentication", "true", "0"),
        ("transport", "false", "1"),
        ("test", "false", "0"),
        ("analysis", "false", "0"),
    ] {
        let output = run_xtask(&[
            "bazel-qualification",
            "typed-fallback",
            "--class",
            class,
            "--dispatch-started",
            dispatch_started,
            "--attempt",
            attempt,
        ]);
        assert!(output.status.success(), "{class}: {:?}", output);
        let value: Value = serde_json::from_slice(&output.stdout).expect("fallback JSON");
        assert_eq!(value["retryLocally"], false, "{class}");
    }
}

#[test]
fn non_qualifying_reports_require_explicit_report_only_mode() {
    let output = run_xtask(&["bazel-qualification", "acceptance"]);
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).expect("non-qualifying report");
    assert_eq!(report["status"], "non-qualifying");

    let output = run_xtask(&["bazel-qualification", "acceptance", "--report-only"]);
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).expect("report-only report");
    assert_eq!(report["status"], "non-qualifying");
}

#[test]
fn caller_authored_candidate_evidence_is_quarantined_outside_fixture_mode() {
    let directory = scratch("untrusted-candidate");
    let candidate_path = directory.join("candidate.json");
    write_json(&candidate_path, &candidate(&current_commit()));
    let output = run_untrusted_xtask(&[
        "bazel-qualification",
        "acceptance",
        "--candidate",
        candidate_path.to_str().expect("candidate path"),
        "--report-only",
    ]);
    let _ = std::fs::remove_dir_all(directory);
    assert!(
        output.status.success(),
        "quarantine is a report: {:?}",
        output
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("quarantine report");
    assert_eq!(report["status"], "non-qualifying");
    assert_eq!(report["candidateEvidenceOriginTrusted"], false);
    assert_eq!(report["coverage"], json!({}));
    assert!(
        report["reasons"]
            .as_array()
            .expect("qualification reasons")
            .iter()
            .any(|reason| reason == "candidate-evidence-origin-untrusted")
    );
}

#[test]
fn caller_authored_provider_evidence_is_quarantined_outside_fixture_mode() {
    let directory = scratch("untrusted-provider");
    let provider_path = directory.join("provider.json");
    write_json(&provider_path, &provider_evidence(now_millis()));
    let output = run_untrusted_xtask(&[
        "bazel-qualification",
        "acceptance",
        "--provider-evidence",
        provider_path.to_str().expect("provider path"),
        "--report-only",
    ]);
    let _ = std::fs::remove_dir_all(directory);
    assert!(
        output.status.success(),
        "quarantine is a report: {:?}",
        output
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("quarantine report");
    assert_eq!(report["status"], "non-qualifying");
    assert_eq!(report["provider"]["evidenceOriginTrusted"], false);
    assert_eq!(report["provider"]["sampleCount"], 0);
    assert_eq!(report["transfer"]["p99Bytes"], Value::Null);
    assert!(
        report["reasons"]
            .as_array()
            .expect("qualification reasons")
            .iter()
            .any(|reason| reason == "provider-evidence-origin-untrusted")
    );
}

#[test]
fn candidate_fallback_rejects_retrying_product_failures() {
    let commit = current_commit();
    let mut candidate_value = candidate(&commit);
    candidate_value["fallback"] = json!({
        "status": "used",
        "localRetryCount": 1,
        "maxLocalRetries": 1,
        "identicalTargetSet": true,
        "failureClass": "test",
        "dispatchStarted": false,
        "attempt": 0,
        "retryLocally": true
    });
    let output = run_acceptance(
        &candidate_value,
        &provider_evidence(now_millis()),
        "fallback-product-failure",
    );
    assert!(output.status.success(), "fallback result is a report");
    let report: Value = serde_json::from_slice(&output.stdout).expect("fallback report");
    assert_eq!(report["status"], "non-qualifying");
    assert!(
        report["reasons"]
            .as_array()
            .expect("reasons")
            .iter()
            .any(|reason| reason == "fallback-retry-state-mismatch")
    );
    assert!(
        report["reasons"]
            .as_array()
            .expect("reasons")
            .iter()
            .any(|reason| reason == "fallback-used-for-non-retriable-failure")
    );
}

#[test]
fn malformed_used_fallback_is_non_qualifying() {
    let commit = current_commit();
    let mut candidate_value = candidate(&commit);
    candidate_value["fallback"] = json!({
        "status": "used",
        "localRetryCount": 0,
        "maxLocalRetries": 1,
        "identicalTargetSet": true,
        "failureClass": "authentication",
        "dispatchStarted": false,
        "attempt": "zero",
        "retryLocally": true
    });
    let output = run_acceptance(
        &candidate_value,
        &provider_evidence(now_millis()),
        "fallback-malformed",
    );
    assert!(output.status.success(), "fallback result is a report");
    let report: Value = serde_json::from_slice(&output.stdout).expect("fallback report");
    assert_eq!(report["status"], "non-qualifying");
    assert!(report["reasons"].as_array().is_some_and(|reasons| {
        reasons
            .iter()
            .any(|reason| reason == "fallback-attempt-invalid")
    }));
}

#[test]
fn provider_transfer_is_required_and_missing_metrics_are_not_zeroed() {
    let commit = current_commit();
    let candidate_value = candidate(&commit);
    let mut provider = provider_evidence(now_millis());
    for_each_sample(&mut provider, |sample| {
        sample["status"] = json!("non-qualifying");
        sample["providerAccountedTransfer"] = json!(false);
        sample["transferBytes"] = json!({
            "uploaded": null,
            "downloaded": null
        });
        sample["qualificationMetrics"] = json!({
            "wallTimeMillis": null,
            "actionCacheHits": null,
            "actionCacheMisses": null,
            "casHits": null,
            "casMisses": null,
            "remoteExecutions": null,
            "repositoryTrafficBytes": null,
            "besTrafficBytes": null,
            "retryTrafficBytes": null,
            "localNixMillis": null
        });
    });

    let output = run_acceptance(&candidate_value, &provider, "missing-transfer");
    assert!(
        output.status.success(),
        "missing transfer is a result: {:?}",
        output
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("qualification report");
    assert_eq!(report["status"], "non-qualifying");
    assert_eq!(report["reason"], "provider-accounted-transfer-missing");
    assert_eq!(report["transfer"]["p99Bytes"], Value::Null);
    assert_eq!(report["transfer"]["monthlyRuns"], Value::Null);
}

#[test]
fn fewer_than_the_independent_sample_set_is_non_qualifying() {
    let commit = current_commit();
    let mut provider = provider_evidence(now_millis());
    let sample = provider["samples"][0].clone();
    provider["samples"] = json!([sample]);
    let output = run_acceptance(&candidate(&commit), &provider, "sample-count");
    assert!(output.status.success(), "sample count is a report");
    let report: Value = serde_json::from_slice(&output.stdout).expect("sample-count report");
    assert_eq!(report["status"], "non-qualifying");
    assert_eq!(report["transfer"]["p99Bytes"], Value::Null);
    assert!(
        report["reasons"]
            .as_array()
            .expect("reasons")
            .iter()
            .any(|reason| reason == "provider-evidence-independent-samples-insufficient:required=5")
    );
}

#[test]
fn qualified_report_binds_identity_compares_u9_and_calculates_budget() {
    let commit = current_commit();
    let output = run_acceptance(
        &candidate(&commit),
        &provider_evidence(now_millis()),
        "qualified",
    );
    assert!(
        output.status.success(),
        "qualification failed: {:?}",
        output
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("qualification report");
    assert_eq!(report["status"], "qualified");
    assert_eq!(report["candidate"]["commit"], commit);
    assert!(
        report["candidate"]["targetSetDigest"]
            .as_str()
            .is_some_and(|value| value.starts_with("sha256:"))
    );
    assert!(
        report["candidate"]["configurationDigest"]
            .as_str()
            .is_some_and(|value| value.starts_with("sha256:"))
    );
    assert!(
        report["candidate"]["selectedClosureDigest"]
            .as_str()
            .is_some_and(|value| value.starts_with("sha256:"))
    );
    assert_eq!(
        report["candidate"]["namespace"],
        "d2b/qualification/linux-x86_64/rules_rust/worker-v1/minimal/lock-v1"
    );
    assert_eq!(report["candidate"]["toolchain"], "rules_rust");
    assert_eq!(
        report["provider"]["projection"],
        "xtask-buildbuddy-probe/v1"
    );
    assert_eq!(report["transfer"]["p99Bytes"], 1_389_959_700u64);
    assert_eq!(report["transfer"]["monthlyRuns"], 57);
    assert_eq!(report["transfer"]["workingBudgetBytes"], 80_000_000_000u64);
    assert_eq!(report["transfer"]["headroomBytes"], 20_000_000_000u64);
    assert_eq!(report["latency"]["p95WallTimeMillis"], 1240);
}

#[test]
fn cache_command_keeps_the_same_candidate_and_provider_contract() {
    let commit = current_commit();
    let directory = scratch("cache-command");
    let candidate_path = directory.join("candidate.json");
    let provider_path = directory.join("provider.json");
    write_json(&candidate_path, &candidate(&commit));
    write_json(&provider_path, &provider_evidence(now_millis()));
    let output = run_xtask(&[
        "bazel-qualification",
        "cache",
        "--candidate",
        candidate_path.to_str().expect("candidate path"),
        "--provider-evidence",
        provider_path.to_str().expect("provider path"),
        "--now-millis",
        &now_millis().to_string(),
        "--nonce",
        TEST_NONCE,
        "--report-only",
    ]);
    assert!(
        output.status.success(),
        "cache command failed: {:?}",
        output
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("cache report");
    assert_eq!(report["mode"], "cache");
    assert_eq!(report["candidate"]["commit"], commit);
    assert_eq!(report["transfer"]["providerAccounted"], true);
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn incomplete_cache_matrix_is_non_qualifying() {
    let mut candidate_value = candidate(&current_commit());
    candidate_value["cache"]["cacheMatrix"]["eviction"] = Value::Bool(false);
    let output = run_acceptance(
        &candidate_value,
        &provider_evidence(now_millis()),
        "cache-matrix",
    );
    assert!(
        output.status.success(),
        "qualification command failed: {:?}",
        output
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("qualification report");
    assert_eq!(report["status"], "non-qualifying");
    assert!(report["reasons"].as_array().is_some_and(|reasons| {
        reasons
            .iter()
            .any(|reason| reason == "cache-matrix-eviction-incomplete")
    }));
}

#[test]
fn missing_uncacheable_reasons_are_non_qualifying() {
    let commit = current_commit();
    let mut candidate_value = candidate(&commit);
    candidate_value["cache"]
        .as_object_mut()
        .expect("cache object")
        .remove("approvedUncacheableReasons");
    let output = run_acceptance(
        &candidate_value,
        &provider_evidence(now_millis()),
        "uncacheable-reasons",
    );
    assert!(output.status.success(), "cache result is a report");
    let report: Value = serde_json::from_slice(&output.stdout).expect("cache report");
    assert_eq!(report["status"], "non-qualifying");
    assert!(report["reasons"].as_array().is_some_and(|reasons| {
        reasons
            .iter()
            .any(|reason| reason == "cache-uncacheable-reasons-missing")
    }));
}

#[test]
fn malformed_unchanged_cacheable_execution_count_is_non_qualifying() {
    let commit = current_commit();
    let mut candidate_value = candidate(&commit);
    candidate_value["cache"]["unchangedCacheableExecutions"] = json!("zero");
    let output = run_acceptance(
        &candidate_value,
        &provider_evidence(now_millis()),
        "uncacheable-execution-type",
    );
    assert!(output.status.success(), "cache result is a report");
    let report: Value = serde_json::from_slice(&output.stdout).expect("cache report");
    assert_eq!(report["status"], "non-qualifying");
    assert!(report["reasons"].as_array().is_some_and(|reasons| {
        reasons
            .iter()
            .any(|reason| reason == "cache-unchanged-run-invalid")
    }));
}

#[test]
fn unchanged_cacheable_execution_count_blocks_qualification() {
    let commit = current_commit();
    let mut candidate_value = candidate(&commit);
    candidate_value["cache"]["unchangedCacheableExecutions"] = json!(1);
    let output = run_acceptance(
        &candidate_value,
        &provider_evidence(now_millis()),
        "unchanged-cacheable-execution",
    );
    assert!(output.status.success(), "cache result is a report");
    let report: Value = serde_json::from_slice(&output.stdout).expect("cache report");
    assert_eq!(report["status"], "non-qualifying");
    assert!(report["reasons"].as_array().is_some_and(|reasons| {
        reasons
            .iter()
            .any(|reason| reason == "cache-unchanged-execution")
    }));
}

#[test]
fn conflicting_candidate_aliases_are_rejected() {
    let commit = current_commit();
    let mut candidate_value = candidate(&commit);
    candidate_value["cache"]["trustedSeedComplete"] = Value::Bool(true);
    candidate_value["cache"]["seedComplete"] = Value::Bool(false);
    let output = run_acceptance(
        &candidate_value,
        &provider_evidence(now_millis()),
        "conflicting-aliases",
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("alias-conflict"));
}

#[test]
fn reused_independence_identity_is_rejected() {
    let commit = current_commit();
    let mut provider = provider_evidence(now_millis());
    provider["samples"][1]["worktreeId"] = provider["samples"][0]["worktreeId"].clone();
    let output = run_acceptance(&candidate(&commit), &provider, "reused-independence");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("provider-evidence-independent-samples-reused")
    );
}

#[test]
fn transfer_above_u9_pessimistic_bound_is_non_qualifying() {
    let commit = current_commit();
    let mut provider = provider_evidence(now_millis());
    for_each_sample(&mut provider, |sample| {
        sample["transferBytes"]["uploaded"] = json!(200_000_000_000u64);
    });
    let output = run_acceptance(&candidate(&commit), &provider, "u9-bound");
    assert!(output.status.success(), "bound result is a report");
    let report: Value = serde_json::from_slice(&output.stdout).expect("bound report");
    assert_eq!(report["status"], "non-qualifying");
    assert_eq!(report["reason"], "provider-transfer-outside-u9-bounds");
    assert_eq!(
        report["u9Comparison"]["details"]["comparison"],
        "above-pessimistic-bound"
    );
    assert_eq!(report["transfer"]["monthlyRuns"], json!(0));
}

#[test]
fn transfer_above_working_budget_is_non_qualifying() {
    let commit = current_commit();
    let mut provider = provider_evidence(now_millis());
    for_each_sample(&mut provider, |sample| {
        sample["transferBytes"]["uploaded"] = json!(80_000_000_001u64);
    });
    let output = run_acceptance(&candidate(&commit), &provider, "working-budget");
    assert!(output.status.success(), "budget result is a report");
    let report: Value = serde_json::from_slice(&output.stdout).expect("budget report");
    assert_eq!(report["status"], "non-qualifying");
    assert_eq!(report["reason"], "provider-transfer-over-working-budget");
    assert_eq!(report["transfer"]["monthlyRuns"], json!(0));
}

#[test]
fn exact_warm_samples_allow_zero_remote_execution_and_low_upload() {
    let commit = current_commit();
    let mut provider = provider_evidence(now_millis());
    for_each_sample(&mut provider, |sample| {
        sample["transferBytes"]["uploaded"] = json!(1u64);
        sample["transferBytes"]["downloaded"] = json!(1u64);
        sample["qualificationMetrics"]["remoteExecutions"] = json!(0);
        sample["qualificationMetrics"]["actionCacheHits"] = json!(10);
        sample["qualificationMetrics"]["actionCacheMisses"] = json!(0);
        sample["qualificationMetrics"]["casHits"] = json!(20);
        sample["qualificationMetrics"]["casMisses"] = json!(0);
    });
    let output = run_acceptance(&candidate(&commit), &provider, "exact-warm");
    assert!(output.status.success(), "warm result is a report");
    let report: Value = serde_json::from_slice(&output.stdout).expect("warm report");
    assert_eq!(report["status"], "qualified");
    assert_eq!(
        report["transfer"]["u9Comparison"]["comparison"],
        "below-optimistic-bound"
    );
}

#[test]
fn wall_time_at_three_minutes_is_non_qualifying() {
    let commit = current_commit();
    let mut provider = provider_evidence(now_millis());
    for_each_sample(&mut provider, |sample| {
        sample["qualificationMetrics"]["wallTimeMillis"] = json!(180_000u64);
    });
    let output = run_acceptance(&candidate(&commit), &provider, "wall-time-boundary");
    assert!(output.status.success(), "latency result is a report");
    let report: Value = serde_json::from_slice(&output.stdout).expect("latency report");
    assert_eq!(report["status"], "non-qualifying");
    assert_eq!(
        report["latency"]["freshWorktreeP95UnderThreeMinutes"],
        false
    );
    assert_eq!(report["reason"], "fresh-worktree-p95-over-three-minutes");
}

#[test]
fn empty_provider_counters_are_non_qualifying() {
    let commit = current_commit();
    let mut provider = provider_evidence(now_millis());
    for_each_sample(&mut provider, |sample| {
        sample["qualificationMetrics"]["remoteExecutions"] = json!(0);
        sample["qualificationMetrics"]["actionCacheHits"] = json!(0);
        sample["qualificationMetrics"]["actionCacheMisses"] = json!(0);
        sample["qualificationMetrics"]["casHits"] = json!(0);
        sample["qualificationMetrics"]["casMisses"] = json!(0);
    });
    let output = run_acceptance(&candidate(&commit), &provider, "empty-counters");
    assert!(output.status.success(), "counter result is a report");
    let report: Value = serde_json::from_slice(&output.stdout).expect("counter report");
    assert_eq!(report["status"], "non-qualifying");
    assert!(
        report["reasons"]
            .as_array()
            .expect("reasons")
            .iter()
            .any(|reason| reason == "provider-evidence-counters-empty")
    );
}

#[test]
fn candidate_secrets_and_client_latency_are_rejected() {
    let commit = current_commit();
    let mut secret_candidate = candidate(&commit);
    secret_candidate["apiKey"] = json!("sentinel");
    let output = run_acceptance(
        &secret_candidate,
        &provider_evidence(now_millis()),
        "candidate-secret",
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("candidate-field"));

    let mut latency_candidate = candidate(&commit);
    latency_candidate["latency"] = json!({"wallTimeMillis": [1]});
    let output = run_acceptance(
        &latency_candidate,
        &provider_evidence(now_millis()),
        "candidate-latency",
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("latency"));
}

#[test]
fn schema_golden_declares_the_fail_closed_budget_contract() {
    let schema: Value = serde_json::from_str(
        &std::fs::read_to_string(repo_root().join("tests/golden/bazel/qualification-schema.json"))
            .expect("read qualification schema"),
    )
    .expect("parse qualification schema");
    assert_eq!(schema["schemaVersion"], 1);
    assert_eq!(schema["reportKind"], "bazel-qualification");
    assert_eq!(schema["transfer"]["workingBudgetBytes"], 80_000_000_000u64);
    assert_eq!(schema["transfer"]["headroomBytes"], 20_000_000_000u64);
    assert_eq!(schema["transfer"]["missingMetricValue"], Value::Null);
    assert!(
        schema["rejectedEvidence"]
            .as_array()
            .expect("rejection list")
            .iter()
            .any(|value| value == "client-supplied")
    );
}

#[test]
fn stale_cross_commit_replayed_and_secret_bearing_provider_evidence_is_rejected() {
    let commit = current_commit();
    let candidate_value = candidate(&commit);

    let mut stale = provider_evidence(1);
    let output = run_acceptance(&candidate_value, &stale, "stale");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("stale"));

    stale = provider_evidence(now_millis());
    for_each_sample(&mut stale, |sample| {
        sample["commit"] = json!("0000000000000000000000000000000000000000");
    });
    let output = run_acceptance(&candidate_value, &stale, "cross-commit");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cross-commit"));

    let mut replayed = provider_evidence(now_millis());
    let duplicate = replayed["samples"][0].clone();
    replayed["samples"] = json!([
        duplicate.clone(),
        duplicate.clone(),
        duplicate.clone(),
        duplicate.clone(),
        duplicate
    ]);
    let output = run_acceptance(&candidate_value, &replayed, "replayed");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("replay"));

    let mut secret = provider_evidence(now_millis());
    secret["apiKey"] = json!("sentinel");
    let output = run_acceptance(&candidate_value, &secret, "secret");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("credential"));
}
