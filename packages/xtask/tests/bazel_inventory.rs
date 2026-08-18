#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::Command;

use serde_json::{Map, Value};

fn repo_root() -> PathBuf {
    let mut candidates = Vec::new();
    if let Some(root) = std::env::var_os("D2B_REPO_ROOT") {
        candidates.push(PathBuf::from(root));
    }
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
    panic!("repository root with Cargo.toml and BUILD.bazel is not discoverable");
}

fn read_json(relative: &str) -> Value {
    let bytes = std::fs::read(repo_root().join(relative))
        .unwrap_or_else(|error| panic!("read {relative}: {error}"));
    serde_json::from_slice(&bytes).unwrap_or_else(|error| panic!("parse {relative}: {error}"))
}

fn object<'a>(value: &'a Value, context: &str) -> &'a Map<String, Value> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("{context} must be an object"))
}

fn string<'a>(value: &'a Map<String, Value>, key: &str, context: &str) -> &'a str {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{context}.{key} must be a string"))
}

fn bool_field(value: &Map<String, Value>, key: &str, context: &str) -> bool {
    value
        .get(key)
        .and_then(Value::as_bool)
        .unwrap_or_else(|| panic!("{context}.{key} must be a boolean"))
}

fn array<'a>(value: &'a Map<String, Value>, key: &str, context: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{context}.{key} must be an array"))
}

fn layer1_job_ids() -> BTreeSet<String> {
    let manifest = read_json("tests/layer1-jobs.json");
    let local = object(
        object(&manifest, "layer1 manifest").get("local").unwrap(),
        "layer1 manifest.local",
    );
    let phases = array(local, "phases", "layer1 manifest.local");
    phases
        .iter()
        .flat_map(|phase| {
            let phase = object(phase, "layer1 phase");
            array(phase, "jobs", "layer1 phase")
                .iter()
                .map(|job| {
                    job.as_str()
                        .unwrap_or_else(|| panic!("layer1 job id must be a string"))
                        .to_owned()
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn layer1_job_enforcement() -> BTreeMap<String, String> {
    let manifest = read_json("tests/layer1-jobs.json");
    let jobs = object(
        object(&manifest, "layer1 manifest")
            .get("jobs")
            .unwrap_or_else(|| panic!("layer1 manifest.jobs is missing")),
        "layer1 manifest.jobs",
    );
    layer1_job_ids()
        .into_iter()
        .map(|id| {
            let job = object(
                jobs.get(&id)
                    .unwrap_or_else(|| panic!("layer1 manifest.jobs.{id} is missing")),
                &format!("layer1 manifest.jobs.{id}"),
            );
            let enforcement = job
                .get("enforcement")
                .and_then(Value::as_str)
                .unwrap_or("enforcing")
                .to_owned();
            (id, enforcement)
        })
        .collect()
}

fn source_text(relative: &str) -> String {
    std::fs::read_to_string(repo_root().join(relative))
        .unwrap_or_else(|error| panic!("read {relative}: {error}"))
}

fn validate_label(label: &str, context: &str) {
    assert!(
        label.starts_with("//") && label.contains(':'),
        "{context} must use a canonical Bazel label: {label}"
    );
    assert!(
        !label.contains("//") || label[2..].contains(':'),
        "{context} must have one package and target component: {label}"
    );
}

fn validate_local_reason(reason: &str, context: &str) {
    const ALLOWED: &[&str] = &[
        "preflight-gate",
        "nix-evaluation",
        "nix-realization",
        "generated-artifact-drift",
        "fixture-realization",
        "stable-self-hosted-runner",
        "host-or-device-required",
        "provider-evidence-unavailable",
    ];
    assert!(
        ALLOWED.contains(&reason),
        "{context} has unknown local-only reason `{reason}`; eligibility must fail closed"
    );
}

#[test]
#[should_panic(expected = "unknown local-only reason")]
fn unknown_eligibility_reason_fails_closed() {
    validate_local_reason("future-provider-mode", "synthetic surface");
}

fn expected_rust_surface_ids() -> BTreeSet<&'static str> {
    [
        "rust:main-workspace",
        "rust:broker-default",
        "rust:broker-layer1",
        "rust:broker-fakebackends",
        "rust:guest-shell-runner-real-libshpool",
        "rust:no-bash-ast",
        "rust:schema-reproducibility",
        "rust:inventory-stub",
        "rust:supply-chain",
        "rust:fixture-contracts",
        "rust:cli-contracts",
        "rust:doctests",
        "rust:harness-free-targets",
        "rust:proof-workspaces",
        "rust:benches",
    ]
    .into_iter()
    .collect()
}

fn expected_harness_free_sources() -> BTreeSet<String> {
    let mut sources = BTreeSet::new();
    for root in ["packages", "proofs"] {
        let mut stack = vec![repo_root().join(root)];
        while let Some(path) = stack.pop() {
            let entries = std::fs::read_dir(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
            for entry in entries {
                let entry = entry.expect("read directory entry");
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.file_name().and_then(|name| name.to_str()) != Some("Cargo.toml") {
                    continue;
                }
                let text = std::fs::read_to_string(&path)
                    .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
                if !text.contains("harness = false") {
                    continue;
                }
                let relative = path
                    .strip_prefix(repo_root())
                    .expect("Cargo manifest is under repository root")
                    .display()
                    .to_string();
                sources.insert(relative);
            }
        }
    }
    sources
}

fn expected_doctest_markers() -> [&'static str; 2] {
    ["cargo test --jobs", "--doc"]
}

fn assert_no_credential_material(value: &Value) {
    let normalized = value
        .to_string()
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    for forbidden in ["apikey", "token", "authorization"] {
        assert!(
            !normalized.contains(forbidden),
            "BuildBuddy evidence must never contain credential material"
        );
    }
}

#[test]
fn coverage_map_is_complete_and_bidirectional_against_layer1_manifest() {
    let coverage_value = read_json("tests/golden/bazel/check-coverage.json");
    let coverage = object(&coverage_value, "coverage");
    let surfaces = array(coverage, "surfaces", "coverage");

    let mut by_id = BTreeMap::new();
    for (index, surface) in surfaces.iter().enumerate() {
        let context = format!("coverage.surfaces[{index}]");
        let surface = object(surface, &context);
        let id = string(surface, "id", &context);
        assert!(
            by_id.insert(id.to_owned(), surface).is_none(),
            "duplicate coverage surface id: {id}"
        );
        validate_label(string(surface, "bazelLabel", &context), &context);
        assert!(
            !string(surface, "source", &context).is_empty(),
            "{context}.source must not be empty"
        );
        assert!(
            !array(surface, "architecture", &context).is_empty(),
            "{context}.architecture must not be empty"
        );

        let eligible = object(
            surface
                .get("eligibility")
                .unwrap_or_else(|| panic!("{context}.eligibility is missing")),
            &format!("{context}.eligibility"),
        );
        let eligible_value = bool_field(eligible, "eligible", &format!("{context}.eligibility"));
        if eligible_value {
            assert!(
                eligible.get("localOnlyReason").is_none()
                    || eligible.get("localOnlyReason") == Some(&Value::Null),
                "{context} eligible surface must not carry a local-only reason"
            );
        } else {
            let reason = string(
                eligible,
                "localOnlyReason",
                &format!("{context}.eligibility"),
            );
            validate_local_reason(reason, &format!("{context}.eligibility"));
        }
    }

    let manifest_jobs = layer1_job_ids();
    let manifest_enforcement = layer1_job_enforcement();
    let mapped_jobs = by_id
        .keys()
        .filter_map(|id| id.strip_prefix("layer1:"))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        mapped_jobs, manifest_jobs,
        "coverage must map every current local Layer-1 job exactly once"
    );
    for (job, enforcement) in manifest_enforcement {
        let surface = by_id
            .get(&format!("layer1:{job}"))
            .expect("mapped Layer-1 surface");
        assert_eq!(
            string(surface, "enforcement", &format!("layer1:{job}")),
            enforcement,
            "coverage must preserve the current Layer-1 enforcement class"
        );
    }
}

#[test]
fn coverage_map_retains_every_current_rust_execution_context() {
    let coverage_value = read_json("tests/golden/bazel/check-coverage.json");
    let coverage = object(&coverage_value, "coverage");
    let surfaces = array(coverage, "surfaces", "coverage");
    let ids = surfaces
        .iter()
        .map(|surface| {
            string(
                object(surface, "coverage surface"),
                "id",
                "coverage surface",
            )
        })
        .collect::<BTreeSet<_>>();

    for required in expected_rust_surface_ids() {
        assert!(
            ids.contains(required),
            "Rust coverage context is missing from the map: {required}"
        );
    }

    let rust_driver = source_text("tests/test-rust.sh");
    for marker in [
        "features layer1-bootstrap",
        "features fake-backends",
        "features real-libshpool",
        "cargo test --doc",
        "harness-free targets",
        "D2B_ENABLE_FIXTURE_BUILD",
        "run_fixture_contract_tests",
    ] {
        assert!(
            rust_driver.contains(marker),
            "current Rust driver no longer contains the inventory marker `{marker}`"
        );
    }
    for marker in expected_doctest_markers() {
        assert!(
            rust_driver.contains(marker),
            "doctest companion coverage marker is missing: {marker}"
        );
    }

    let harness_surface = surfaces
        .iter()
        .find(|surface| {
            string(
                object(surface, "coverage surface"),
                "id",
                "coverage surface",
            ) == "rust:harness-free-targets"
        })
        .expect("harness-free surface");
    let harness_surface = object(harness_surface, "rust:harness-free-targets");
    let sources = array(harness_surface, "sources", "rust:harness-free-targets")
        .iter()
        .map(|source| {
            source
                .as_str()
                .unwrap_or_else(|| panic!("harness source must be a string"))
                .to_owned()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        sources,
        expected_harness_free_sources(),
        "harness-free Cargo manifests must be covered bidirectionally"
    );
}

#[test]
fn eligibility_projection_is_exactly_derived_from_coverage() {
    let coverage_value = read_json("tests/golden/bazel/check-coverage.json");
    let coverage = object(&coverage_value, "coverage");
    let eligibility_value = read_json("tests/golden/bazel/eligibility.json");
    let eligibility = object(&eligibility_value, "eligibility");
    assert_eq!(
        string(eligibility, "source", "eligibility"),
        "tests/golden/bazel/check-coverage.json"
    );

    let expected = array(coverage, "surfaces", "coverage")
        .iter()
        .map(|surface| {
            let surface = object(surface, "coverage surface");
            let eligibility = object(
                surface.get("eligibility").expect("coverage eligibility"),
                "coverage surface eligibility",
            );
            serde_json::json!({
                "id": string(surface, "id", "coverage surface"),
                "bazelLabel": string(surface, "bazelLabel", "coverage surface"),
                "eligible": bool_field(eligibility, "eligible", "coverage surface eligibility"),
                "localOnlyReason": eligibility.get("localOnlyReason").cloned().unwrap_or(Value::Null),
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        array(eligibility, "entries", "eligibility"),
        expected.as_slice(),
        "eligibility.json must remain a generated projection of the canonical coverage map"
    );
}

#[test]
fn buildbuddy_evidence_is_machine_readable_and_non_qualifying_without_provider_proof() {
    let eligibility_value = read_json("tests/golden/bazel/eligibility.json");
    let eligibility = object(&eligibility_value, "eligibility");
    let evidence = object(
        eligibility
            .get("buildBuddy")
            .expect("eligibility.buildBuddy"),
        "eligibility.buildBuddy",
    );
    assert_eq!(
        string(evidence, "provider", "eligibility.buildBuddy"),
        "buildbuddy"
    );
    let probe = object(
        evidence.get("probe").expect("eligibility.buildBuddy.probe"),
        "eligibility.buildBuddy.probe",
    );
    assert_eq!(
        string(probe, "kind", "eligibility.buildBuddy.probe"),
        "credential-isolated-command"
    );
    assert_eq!(
        string(probe, "command", "eligibility.buildBuddy.probe"),
        "xtask buildbuddy-probe"
    );
    assert_eq!(
        string(probe, "input", "eligibility.buildBuddy.probe"),
        "D2B_BUILDBUDDY_EVIDENCE_FILE"
    );
    assert_eq!(
        string(probe, "credentialMode", "eligibility.buildBuddy.probe"),
        "none"
    );
    assert!(
        matches!(
            string(probe, "credentialMode", "eligibility.buildBuddy.probe"),
            "none" | "credential-helper"
        ),
        "BuildBuddy authentication must use the closed credential-helper contract"
    );
    assert!(
        bool_field(probe, "readOnly", "eligibility.buildBuddy.probe"),
        "BuildBuddy entitlement probing must be read-only"
    );
    assert!(
        bool_field(probe, "fixtureSafe", "eligibility.buildBuddy.probe"),
        "BuildBuddy probe fixtures must not carry provider credentials"
    );
    assert!(
        matches!(
            string(evidence, "status", "eligibility.buildBuddy"),
            "unavailable" | "non-qualifying" | "qualified"
        ),
        "BuildBuddy evidence status must use the closed result contract"
    );

    let evidence_file = std::env::var_os("D2B_BUILDBUDDY_EVIDENCE_FILE");
    let evidence_value = if let Some(path) = evidence_file {
        let path = PathBuf::from(path);
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|error| panic!("read BuildBuddy evidence {}: {error}", path.display()));
        serde_json::from_slice::<Value>(&bytes)
            .unwrap_or_else(|error| panic!("parse BuildBuddy evidence {}: {error}", path.display()))
    } else {
        Value::Object(evidence.clone())
    };
    let evidence = object(&evidence_value, "BuildBuddy evidence");
    assert_eq!(
        string(evidence, "provider", "BuildBuddy evidence"),
        "buildbuddy"
    );
    let evidence_probe = object(
        evidence.get("probe").expect("BuildBuddy evidence.probe"),
        "BuildBuddy evidence.probe",
    );
    assert!(
        matches!(
            string(
                evidence_probe,
                "credentialMode",
                "BuildBuddy evidence.probe"
            ),
            "none" | "credential-helper"
        ),
        "BuildBuddy evidence must use the closed authentication mode contract"
    );
    assert_no_credential_material(&evidence_value);

    let qualified = string(evidence, "status", "BuildBuddy evidence") == "qualified";
    let required_proof = [
        "authenticated",
        "executionEntitled",
        "cacheReadEvidence",
        "cacheWriteEvidence",
        "readOnlyProbe",
        "transferBytes",
        "qualificationMetrics",
        "workerArchitectures",
        "uploadsDisabled",
        "secretRedaction",
        "trustedSeed",
        "invocationId",
        "dispatchEvidence",
    ];
    for field in required_proof {
        assert!(
            evidence.contains_key(field),
            "BuildBuddy evidence contract is missing `{field}`"
        );
    }
    let uploads_disabled = evidence
        .get("uploadsDisabled")
        .expect("BuildBuddy evidence.uploadsDisabled");
    assert!(
        uploads_disabled.is_null() || uploads_disabled.is_boolean(),
        "uploadsDisabled must be an explicit boolean or null when provider evidence is unavailable"
    );
    if qualified {
        for field in [
            "authenticated",
            "executionEntitled",
            "cacheReadEvidence",
            "cacheWriteEvidence",
            "readOnlyProbe",
            "secretRedaction",
            "trustedSeed",
            "dispatchEvidence",
        ] {
            assert!(
                bool_field(evidence, field, "BuildBuddy evidence"),
                "qualified BuildBuddy evidence requires `{field}=true`"
            );
        }
        let bytes = object(
            evidence.get("transferBytes").unwrap(),
            "BuildBuddy evidence.transferBytes",
        );
        assert!(bytes.get("uploaded").and_then(Value::as_u64).is_some());
        assert!(bytes.get("downloaded").and_then(Value::as_u64).is_some());
        let metrics = object(
            evidence.get("qualificationMetrics").unwrap(),
            "BuildBuddy evidence.qualificationMetrics",
        );
        for field in [
            "wallTimeMillis",
            "actionCacheHits",
            "actionCacheMisses",
            "casHits",
            "casMisses",
            "remoteExecutions",
            "repositoryTrafficBytes",
            "besTrafficBytes",
            "retryTrafficBytes",
            "localNixMillis",
        ] {
            assert!(metrics.get(field).and_then(Value::as_u64).is_some());
        }
        assert!(
            uploads_disabled.is_boolean(),
            "qualified BuildBuddy evidence must state whether uploads were disabled"
        );
        assert!(!array(evidence, "workerArchitectures", "BuildBuddy evidence").is_empty());
        assert!(!string(evidence, "invocationId", "BuildBuddy evidence").is_empty());
    } else if string(evidence, "status", "BuildBuddy evidence") == "unavailable" {
        assert!(
            !bool_field(evidence, "executionEntitled", "BuildBuddy evidence"),
            "missing or incomplete provider proof must remain non-qualifying"
        );
    } else {
        assert!(
            evidence
                .get("secretRedaction")
                .is_some_and(Value::is_boolean),
            "provider evidence must state secret-redaction proof"
        );
        assert!(
            evidence
                .get("uploadsDisabled")
                .is_some_and(Value::is_boolean),
            "provider evidence must state whether uploads were disabled"
        );
    }
}

#[test]
fn buildbuddy_probe_command_emits_the_default_non_qualifying_contract() {
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("buildbuddy-probe")
        .env_remove("D2B_BUILDBUDDY_EVIDENCE_FILE")
        .output()
        .expect("run BuildBuddy probe command");
    assert!(
        output.status.success(),
        "BuildBuddy probe command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "BuildBuddy probe must not print diagnostics on a successful result"
    );
    let actual: Value =
        serde_json::from_slice(&output.stdout).expect("BuildBuddy probe emits JSON");
    let eligibility_value = read_json("tests/golden/bazel/eligibility.json");
    let eligibility = object(&eligibility_value, "eligibility");
    assert_eq!(
        actual,
        eligibility
            .get("buildBuddy")
            .expect("eligibility.buildBuddy")
            .clone(),
        "BuildBuddy command output must own the checked-in default evidence contract"
    );
}

#[test]
#[should_panic(expected = "credential material")]
fn buildbuddy_evidence_rejects_credential_fields() {
    assert_no_credential_material(&serde_json::json!({
        "provider": "buildbuddy",
        "api_key": "sentinel",
    }));
}
