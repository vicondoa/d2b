#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use serde_json::{Map, Value};

const EXPECTED_BAZEL_VERSION: &str = "bazel 9.2.0";

const NON_RUST_SURFACES: [(&str, &str); 13] = [
    ("layer1:tier0", "//bazel/checks/meta:tier0"),
    ("layer1:test-lint", "//bazel/checks/policy:lint"),
    ("layer1:check-inventory", "//bazel/checks/meta:inventory"),
    ("layer1:test-changelog", "//bazel/checks/policy:changelog"),
    ("layer1:bazel-check", "//:bazel_check"),
    ("layer1:test-proofs", "//bazel/checks/policy:proofs"),
    ("layer1:test-flake", "//bazel/checks/nix:flake"),
    ("layer1:test-nix-unit", "//bazel/checks/nix:nix_unit"),
    ("layer1:test-policy", "//bazel/checks/policy:policy"),
    ("layer1:test-drift", "//bazel/checks/policy:drift"),
    (
        "layer1:test-runtime-ledger",
        "//bazel/checks/policy:runtime_ledger",
    ),
    (
        "layer1:test-performance-budgets",
        "//bazel/checks/meta:performance_budgets",
    ),
    (
        "layer1:test-fixture-contracts",
        "//bazel/checks/fixtures:contracts",
    ),
];

const REQUIRED_LOCAL_REASONS: [&str; 7] = [
    "preflight-gate",
    "nix-realization",
    "generated-artifact-drift",
    "stable-self-hosted-runner",
    "fixture-realization",
    "host-or-device-required",
    "provider-evidence-unavailable",
];

const REQUIRED_INTEGRATION_SUCCESSORS: [&str; 5] = [
    "integration:containers",
    "integration:kvm",
    "integration:live-host",
    "integration:hardware",
    "integration:host-state",
];

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
        if path.is_file() {
            path.pop();
        }
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
    panic!("repository root with Cargo.toml, BUILD.bazel, and flake.nix is not discoverable");
}

fn bazel_binary() -> PathBuf {
    static BAZEL: OnceLock<PathBuf> = OnceLock::new();
    BAZEL.get_or_init(resolve_bazel_binary).clone()
}

fn bazel_version(path: &Path) -> Option<String> {
    let output = Command::new(path).arg("--version").output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn assert_exact_bazel(path: &Path, source: &str) {
    let version = bazel_version(path)
        .unwrap_or_else(|| panic!("Bazel provider from {source} could not execute"));
    assert_eq!(
        version, EXPECTED_BAZEL_VERSION,
        "Bazel provider from {source} must be exactly {EXPECTED_BAZEL_VERSION}"
    );
}

fn resolve_bazel_binary() -> PathBuf {
    if let Some(path) = std::env::var_os("D2B_BAZEL_BIN").map(PathBuf::from) {
        assert_exact_bazel(&path, "D2B_BAZEL_BIN");
        return path;
    }

    let ambient = PathBuf::from("bazel");
    if bazel_version(&ambient).as_deref() == Some(EXPECTED_BAZEL_VERSION) {
        return ambient;
    }

    let system = match std::env::consts::ARCH {
        "x86_64" => "x86_64-linux",
        "aarch64" => "aarch64-linux",
        _ => panic!("unsupported host architecture for the Bazel provider"),
    };
    let attribute = format!(".#packages.{system}.bazel-9_2_0");
    let output = Command::new("nix")
        .args([
            "build",
            "--no-link",
            "--no-write-lock-file",
            "--print-out-paths",
            &attribute,
        ])
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|error| panic!("resolve the Bazel provider: {error}"));
    assert!(
        output.status.success(),
        "resolve the exact Bazel provider:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bazel_output = String::from_utf8_lossy(&output.stdout);
    let store_path = bazel_output
        .lines()
        .rfind(|line| !line.trim().is_empty())
        .expect("Nix emitted a Bazel store path");
    let path = PathBuf::from(store_path.trim()).join("bin/bazel");
    assert_exact_bazel(&path, "the repository Bazel provider");
    path
}

fn bazel_output_user_root() -> PathBuf {
    if let Some(path) = std::env::var_os("D2B_BAZEL_OUTPUT_USER_ROOT") {
        return PathBuf::from(path);
    }
    if let Some(path) = std::env::var_os("XDG_CACHE_HOME") {
        return PathBuf::from(path).join("d2b-bazel-non-rust-parity-output");
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|path| path.join(".cache/d2b-bazel-non-rust-parity-output"))
        .unwrap_or_else(|| panic!("D2B_BAZEL_OUTPUT_USER_ROOT or HOME is required"))
}

fn run_bazel_query(expression: &str) -> std::process::Output {
    Command::new(bazel_binary())
        .arg(format!(
            "--output_user_root={}",
            bazel_output_user_root().display()
        ))
        .args([
            "query",
            "--noshow_progress",
            "--lockfile_mode=error",
            "--repo_contents_cache=",
            "--output=label",
            expression,
        ])
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|error| panic!("run Bazel query: {error}"))
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

fn array<'a>(value: &'a Map<String, Value>, key: &str, context: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{context}.{key} must be an array"))
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

fn source_exists(relative: &str, context: &str) {
    let path = repo_root().join(relative);
    assert!(
        path.is_file() || path.is_dir(),
        "{context} source does not exist: {relative}"
    );
}

fn validate_label(label: &str, context: &str) {
    assert!(
        label.starts_with("//") && label.contains(':'),
        "{context} must be a canonical Bazel label: {label}"
    );
}

fn validate_reason_registry(coverage: &Map<String, Value>) -> BTreeSet<String> {
    let reasons = array(coverage, "localOnlyReasons", "coverage");
    let mut ids = BTreeSet::new();
    for (index, reason) in reasons.iter().enumerate() {
        let context = format!("coverage.localOnlyReasons[{index}]");
        let reason = object(reason, &context);
        let id = string(reason, "id", &context);
        assert!(
            ids.insert(id.to_owned()),
            "duplicate local-only reason: {id}"
        );
        assert!(
            REQUIRED_LOCAL_REASONS.contains(&id),
            "unknown local-only reason: {id}"
        );
        assert!(
            !string(reason, "description", &context).is_empty(),
            "{context}.description must not be empty"
        );
    }
    for required in REQUIRED_LOCAL_REASONS {
        assert!(
            ids.contains(required),
            "missing local-only reason: {required}"
        );
    }
    ids
}

fn validate_surface_carrier(surface: &Map<String, Value>, reason_ids: &BTreeSet<String>) {
    let id = string(surface, "id", "surface");
    let context = format!("coverage surface {id}");
    let carrier = object(
        surface
            .get("carrier")
            .unwrap_or_else(|| panic!("{context}.carrier is missing")),
        &format!("{context}.carrier"),
    );
    assert_eq!(
        string(carrier, "kind", &context),
        "bazel-test",
        "{context}.carrier.kind must be bazel-test"
    );
    let label = string(carrier, "label", &context);
    validate_label(label, &context);
    assert_eq!(
        label,
        string(surface, "bazelLabel", &context),
        "{context}.carrier.label must match bazelLabel"
    );
    let sources = array(carrier, "sources", &context);
    assert!(
        !sources.is_empty(),
        "{context}.carrier.sources must not be empty"
    );
    for (index, source) in sources.iter().enumerate() {
        let source = source
            .as_str()
            .unwrap_or_else(|| panic!("{context}.carrier.sources[{index}] must be a string"));
        source_exists(source, &format!("{context}.carrier.sources[{index}]"));
    }

    let eligibility = object(
        surface
            .get("eligibility")
            .unwrap_or_else(|| panic!("{context}.eligibility is missing")),
        &format!("{context}.eligibility"),
    );
    let eligible = bool_field(eligibility, "eligible", &context);
    let remote = bool_field(carrier, "remoteCandidate", &context);
    assert_eq!(
        eligible, remote,
        "{context} eligibility and carrier remoteCandidate disagree"
    );
    if eligible {
        assert!(
            eligibility
                .get("localOnlyReason")
                .is_none_or(Value::is_null),
            "{context} eligible surface must not have a local-only reason"
        );
    } else {
        let reason = string(eligibility, "localOnlyReason", &context);
        assert!(
            reason_ids.contains(reason),
            "{context} has an unregistered local-only reason: {reason}"
        );
    }
}

fn validate_integration_successors(coverage: &Map<String, Value>) {
    let successors = array(coverage, "integrationSuccessors", "coverage");
    let mut ids = BTreeSet::new();
    for (index, successor) in successors.iter().enumerate() {
        let context = format!("coverage.integrationSuccessors[{index}]");
        let successor = object(successor, &context);
        let id = string(successor, "id", &context);
        assert!(
            ids.insert(id.to_owned()),
            "duplicate integration successor: {id}"
        );
        assert!(
            REQUIRED_INTEGRATION_SUCCESSORS.contains(&id),
            "unknown integration successor: {id}"
        );
        source_exists(string(successor, "source", &context), &context);
        assert_eq!(
            string(successor, "localOnlyReason", &context),
            "host-or-device-required",
            "{context} must remain local-only"
        );
        assert!(
            !bool_field(successor, "remoteAggregate", &context),
            "{context} must not enter the remote aggregate"
        );
    }
    for required in REQUIRED_INTEGRATION_SUCCESSORS {
        assert!(
            ids.contains(required),
            "missing integration successor: {required}"
        );
    }
}

fn validate_nix_policy(coverage: &Map<String, Value>) {
    let nix = object(
        coverage
            .get("nixPolicy")
            .unwrap_or_else(|| panic!("coverage.nixPolicy is missing")),
        "coverage.nixPolicy",
    );
    let pure = object(
        nix.get("pureEvaluation")
            .unwrap_or_else(|| panic!("coverage.nixPolicy.pureEvaluation is missing")),
        "coverage.nixPolicy.pureEvaluation",
    );
    validate_label(
        string(pure, "label", "coverage.nixPolicy.pureEvaluation"),
        "coverage.nixPolicy.pureEvaluation",
    );
    assert!(
        bool_field(pure, "remoteEligible", "coverage.nixPolicy.pureEvaluation"),
        "pure locked evaluation is remote-eligible only after its proof"
    );
    assert_eq!(
        string(pure, "proofStatus", "coverage.nixPolicy.pureEvaluation"),
        "proven",
        "pure locked evaluation must carry a hermetic proof"
    );
    assert!(
        bool_field(pure, "lockedInputs", "coverage.nixPolicy.pureEvaluation"),
        "pure evaluation proof must require locked inputs"
    );
    source_exists(
        string(pure, "hermeticFixture", "coverage.nixPolicy.pureEvaluation"),
        "coverage.nixPolicy.pureEvaluation",
    );

    let realization = object(
        nix.get("realization")
            .unwrap_or_else(|| panic!("coverage.nixPolicy.realization is missing")),
        "coverage.nixPolicy.realization",
    );
    assert!(
        !bool_field(
            realization,
            "remoteEligible",
            "coverage.nixPolicy.realization"
        ),
        "Nix realization must remain local without the worker proof"
    );
    assert_eq!(
        string(
            realization,
            "localOnlyReason",
            "coverage.nixPolicy.realization"
        ),
        "nix-realization"
    );
    let proof = object(
        realization
            .get("workerImageExperiment")
            .unwrap_or_else(|| panic!("Nix realization worker-image experiment is missing")),
        "coverage.nixPolicy.realization.workerImageExperiment",
    );
    validate_label(
        string(proof, "label", "Nix realization worker-image experiment"),
        "Nix realization worker-image experiment",
    );
    assert!(!bool_field(
        proof,
        "baseline",
        "Nix realization worker-image experiment"
    ));
    assert!(!bool_field(
        proof,
        "privileged",
        "Nix realization worker-image experiment"
    ));
    assert!(bool_field(
        proof,
        "immutable",
        "Nix realization worker-image experiment"
    ));
    assert_eq!(
        string(proof, "status", "Nix realization worker-image experiment"),
        "experimental"
    );
    let requirements = array(
        realization,
        "requiredProof",
        "coverage.nixPolicy.realization",
    );
    let required = requirements
        .iter()
        .map(|value| {
            value
                .as_str()
                .unwrap_or_else(|| panic!("Nix realization proof fields must be strings"))
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        required,
        [
            "uid",
            "privilege",
            "store",
            "closure",
            "output",
            "image",
            "system",
            "target",
        ]
        .into_iter()
        .collect()
    );
}

fn validate_fixture_contract(coverage: &Map<String, Value>) {
    let surface = array(coverage, "surfaces", "coverage")
        .iter()
        .find(|surface| {
            object(surface, "coverage surface")
                .get("id")
                .and_then(Value::as_str)
                == Some("layer1:test-fixture-contracts")
        })
        .unwrap_or_else(|| panic!("fixture contract surface is missing"));
    let surface = object(surface, "layer1:test-fixture-contracts");
    let carrier = object(
        surface
            .get("carrier")
            .expect("fixture contract carrier is missing"),
        "fixture contract carrier",
    );
    assert_eq!(
        string(carrier, "fixtureManifest", "fixture contract carrier"),
        "tests/fixtures/BUILD.bazel"
    );
    assert_ne!(
        string(carrier, "fixtureState", "fixture contract carrier"),
        "stale",
        "stale fixture contracts must fail closed"
    );
    assert_eq!(
        string(carrier, "fixtureState", "fixture contract carrier"),
        "declared"
    );
}

fn bazel_test_carriers(coverage: &Map<String, Value>) -> BTreeMap<String, String> {
    let mut carriers = BTreeMap::new();
    for (index, surface) in array(coverage, "surfaces", "coverage").iter().enumerate() {
        let context = format!("coverage.surfaces[{index}]");
        let surface = object(surface, &context);
        let Some(carrier_value) = surface.get("carrier") else {
            continue;
        };
        let carrier = object(carrier_value, &format!("{context}.carrier"));
        if string(carrier, "kind", &context) != "bazel-test" {
            continue;
        }
        let id = string(surface, "id", &context).to_owned();
        let label = string(carrier, "label", &context).to_owned();
        assert!(
            carriers.insert(id.clone(), label).is_none(),
            "duplicate Bazel-test carrier surface: {id}"
        );
    }
    carriers
}

#[test]
fn non_rust_layer1_surfaces_have_declared_bazel_carriers() {
    let coverage_value = read_json("tests/golden/bazel/check-coverage.json");
    let coverage = object(&coverage_value, "coverage");
    let reason_ids = validate_reason_registry(coverage);
    let surfaces = array(coverage, "surfaces", "coverage");
    let by_id = surfaces
        .iter()
        .map(|surface| {
            let surface = object(surface, "coverage surface");
            (string(surface, "id", "coverage surface"), surface)
        })
        .collect::<BTreeMap<_, _>>();

    for (id, label) in NON_RUST_SURFACES {
        let surface = by_id
            .get(id)
            .unwrap_or_else(|| panic!("missing non-Rust Layer-1 surface: {id}"));
        assert_eq!(
            string(surface, "bazelLabel", id),
            label,
            "{id} has the wrong Bazel carrier label"
        );
        validate_surface_carrier(surface, &reason_ids);
    }
    validate_integration_successors(coverage);
    validate_nix_policy(coverage);
    validate_fixture_contract(coverage);
}

#[test]
fn every_bazel_test_carrier_is_queryable_as_a_test() {
    let coverage_value = read_json("tests/golden/bazel/check-coverage.json");
    let coverage = object(&coverage_value, "coverage");
    let carriers = bazel_test_carriers(coverage);
    assert!(
        !carriers.is_empty(),
        "coverage must declare at least one Bazel-test carrier"
    );

    for (id, label) in carriers {
        let expression = format!("kind(\"test\", {label})");
        let output = run_bazel_query(&expression);
        assert!(
            output.status.success(),
            "Bazel query failed for {id} ({label}):\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let query_stdout = String::from_utf8_lossy(&output.stdout);
        let matches = query_stdout
            .lines()
            .filter(|line| line.starts_with("//"))
            .collect::<Vec<_>>();
        assert!(
            !matches.is_empty(),
            "Bazel-test carrier {id} ({label}) is not a Bazel test rule; \
             query `{expression}` returned no labels"
        );
    }
}

#[test]
fn remote_aggregate_contains_only_proven_eligible_non_rust_surfaces() {
    let coverage_value = read_json("tests/golden/bazel/check-coverage.json");
    let coverage = object(&coverage_value, "coverage");
    let aggregate = array(coverage, "remoteAggregate", "coverage");
    let mut ids = BTreeSet::new();
    for (index, entry) in aggregate.iter().enumerate() {
        let context = format!("coverage.remoteAggregate[{index}]");
        let entry = object(entry, &context);
        let id = string(entry, "id", &context);
        assert!(
            ids.insert(id.to_owned()),
            "duplicate remote aggregate entry: {id}"
        );
        let surface = array(coverage, "surfaces", "coverage")
            .iter()
            .find(|surface| {
                object(surface, "coverage surface")
                    .get("id")
                    .and_then(Value::as_str)
                    == Some(id)
            })
            .unwrap_or_else(|| panic!("remote aggregate references unknown surface: {id}"));
        let surface = object(surface, id);
        let eligibility = object(
            surface.get("eligibility").expect("surface eligibility"),
            &format!("{id}.eligibility"),
        );
        assert!(
            bool_field(eligibility, "eligible", id),
            "remote aggregate contains local-only surface: {id}"
        );
        assert_eq!(
            string(entry, "label", &context),
            string(surface, "bazelLabel", id),
            "remote aggregate label differs from coverage"
        );
    }
    assert!(ids.contains("layer1:test-changelog"));
    assert!(ids.contains("layer1:test-nix-unit"));
    assert!(ids.contains("layer1:test-proofs"));
    assert!(ids.contains("layer1:test-runtime-ledger"));
    assert!(!ids.contains("layer1:test-flake"));
    assert!(!ids.contains("layer1:test-fixture-contracts"));
}

#[test]
#[should_panic(expected = "carrier is missing")]
fn planted_negative_missing_carrier_is_rejected() {
    let surface = serde_json::json!({
        "id": "layer1:test-lint",
        "bazelLabel": "//bazel/checks/policy:lint",
        "eligibility": {
            "eligible": false,
            "localOnlyReason": "preflight-gate"
        }
    });
    validate_surface_carrier(
        object(&surface, "surface"),
        &BTreeSet::from(["preflight-gate".into()]),
    );
}

#[test]
#[should_panic(expected = "duplicate local-only reason")]
fn planted_negative_duplicate_local_only_reason_is_rejected() {
    let coverage = serde_json::json!({
        "localOnlyReasons": [
            {"id": "preflight-gate", "description": "one"},
            {"id": "preflight-gate", "description": "two"},
            {"id": "nix-realization", "description": "three"},
            {"id": "generated-artifact-drift", "description": "four"},
            {"id": "stable-self-hosted-runner", "description": "five"},
            {"id": "fixture-realization", "description": "six"},
            {"id": "host-or-device-required", "description": "seven"},
            {"id": "provider-evidence-unavailable", "description": "eight"}
        ]
    });
    validate_reason_registry(object(&coverage, "coverage"));
}

#[test]
#[should_panic(expected = "unknown local-only reason")]
fn planted_negative_unknown_local_only_reason_is_rejected() {
    let coverage = serde_json::json!({
        "localOnlyReasons": [
            {"id": "preflight-gate", "description": "one"},
            {"id": "nix-realization", "description": "two"},
            {"id": "generated-artifact-drift", "description": "three"},
            {"id": "stable-self-hosted-runner", "description": "four"},
            {"id": "fixture-realization", "description": "five"},
            {"id": "host-or-device-required", "description": "six"},
            {"id": "provider-evidence-unavailable", "description": "seven"},
            {"id": "future-local-mode", "description": "unknown"}
        ]
    });
    validate_reason_registry(object(&coverage, "coverage"));
}

#[test]
#[should_panic(expected = "must not enter the remote aggregate")]
fn planted_negative_integration_surface_in_remote_aggregate_is_rejected() {
    let successor = serde_json::json!({
        "id": "integration:kvm",
        "source": "tests/host-integration",
        "localOnlyReason": "host-or-device-required",
        "remoteAggregate": true
    });
    let coverage = serde_json::json!({
        "integrationSuccessors": [successor]
    });
    validate_integration_successors(object(&coverage, "coverage"));
}

#[test]
#[should_panic(expected = "worker-image experiment is missing")]
fn planted_negative_realization_without_proof_is_rejected() {
    let coverage = serde_json::json!({
        "nixPolicy": {
            "pureEvaluation": {
                "label": "//bazel/checks/nix:pure_locked_evaluation",
                "remoteEligible": true,
                "proofStatus": "proven",
                "lockedInputs": true,
                "hermeticFixture": "tests/unit/nix/eval-jobs.nix"
            },
            "realization": {
                "remoteEligible": false,
                "localOnlyReason": "nix-realization",
                "requiredProof": ["uid", "privilege", "store", "closure", "output", "image", "system", "target"]
            }
        }
    });
    validate_nix_policy(object(&coverage, "coverage"));
}

#[test]
#[should_panic(expected = "stale fixture contracts")]
fn planted_negative_missing_or_stale_fixture_is_rejected() {
    let coverage = serde_json::json!({
        "surfaces": [{
            "id": "layer1:test-fixture-contracts",
            "carrier": {
                "fixtureManifest": "tests/fixtures/BUILD.bazel",
                "fixtureState": "stale"
            }
        }]
    });
    validate_fixture_contract(object(&coverage, "coverage"));
}
