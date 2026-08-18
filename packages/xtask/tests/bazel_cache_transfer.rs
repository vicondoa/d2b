#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

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

fn fixture(relative: &str) -> PathBuf {
    repo_root()
        .join("tests/fixtures/bazel/cache-transfer")
        .join(relative)
}

fn scratch_root() -> PathBuf {
    std::env::var_os("TEST_TMPDIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("TEST_UNDECLARED_OUTPUTS_DIR").map(PathBuf::from))
        .unwrap_or_else(std::env::temp_dir)
}

fn run_xtask(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(args)
        .env("D2B_REPO_ROOT", repo_root())
        .output()
        .expect("run xtask")
}

fn read_json(path: &Path) -> Value {
    let bytes =
        std::fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn run_with_log_value(value: &Value, name: &str) -> std::process::Output {
    run_with_log_bytes(
        &serde_json::to_vec_pretty(value).expect("serialize invalid log"),
        name,
    )
}

fn run_with_log_bytes(bytes: &[u8], name: &str) -> std::process::Output {
    let directory = scratch_root().join(format!(
        ".scratch/bazel-cache-transfer-invalid-{}-{}",
        std::process::id(),
        name
    ));
    std::fs::create_dir_all(&directory).expect("create invalid-log scratch");
    let log_path = directory.join("execution-log.json");
    let output_path = directory.join("report.json");
    std::fs::write(&log_path, bytes).expect("write invalid log");
    let output = run_xtask(&[
        "bazel-cache-transfer",
        "--execution-log",
        log_path.to_str().expect("log path"),
        "--eligibility",
        fixture("eligibility.json")
            .to_str()
            .expect("eligibility path"),
        "--output",
        output_path.to_str().expect("output path"),
        "--configuration",
        "local",
        "--platform",
        "linux-x86_64",
        "--toolchain",
        "rules_rust-fixture",
    ]);
    let _ = std::fs::remove_dir_all(directory);
    output
}

fn analyze_value(value: &Value, name: &str) -> Value {
    let directory = scratch_root().join(format!(
        ".scratch/bazel-cache-transfer-report-{}-{}",
        std::process::id(),
        name
    ));
    std::fs::create_dir_all(&directory).expect("create report scratch");
    let log_path = directory.join("execution-log.json");
    let output_path = directory.join("report.json");
    std::fs::write(
        &log_path,
        serde_json::to_vec_pretty(value).expect("serialize report log"),
    )
    .expect("write report log");
    let output = run_xtask(&[
        "bazel-cache-transfer",
        "--execution-log",
        log_path.to_str().expect("log path"),
        "--eligibility",
        fixture("eligibility.json")
            .to_str()
            .expect("eligibility path"),
        "--configuration",
        "local",
        "--platform",
        "linux-x86_64",
        "--toolchain",
        "rules_rust-fixture",
        "--output",
        output_path.to_str().expect("report path"),
    ]);
    assert!(
        output.status.success(),
        "analyzer failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = read_json(&output_path);
    let _ = std::fs::remove_dir_all(directory);
    report
}

fn object<'a>(value: &'a Value, context: &str) -> &'a serde_json::Map<String, Value> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("{context} must be an object"))
}

fn u64_field(value: &Value, key: &str, context: &str) -> u64 {
    object(value, context)
        .get(key)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("{context}.{key} must be an unsigned integer"))
}

#[test]
fn reports_action_mnemonic_graph_and_boundary_metrics() {
    let output_path = scratch_root().join(format!(
        ".scratch/bazel-cache-transfer-test-{}/baseline.json",
        std::process::id()
    ));
    std::fs::create_dir_all(output_path.parent().expect("output parent")).expect("create scratch");

    let output = run_xtask(&[
        "bazel-cache-transfer",
        "--execution-log",
        fixture("baseline-execution-log.json")
            .to_str()
            .expect("fixture path"),
        "--eligibility",
        fixture("eligibility.json")
            .to_str()
            .expect("eligibility path"),
        "--output",
        output_path.to_str().expect("output path"),
        "--configuration",
        "local",
        "--platform",
        "linux-x86_64",
        "--toolchain",
        "rules_rust-fixture",
    ]);
    assert!(
        output.status.success(),
        "analyzer failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let report = read_json(&output_path);
    assert_eq!(u64_field(&report, "schemaVersion", "report"), 1);
    let semantics = object(
        object(&report, "report")
            .get("semantics")
            .expect("report.semantics"),
        "report.semantics",
    );
    assert!(
        !semantics
            .get("providerBilling")
            .and_then(Value::as_bool)
            .expect("providerBilling")
    );

    let whole_graph = object(
        object(&report, "report")
            .get("wholeGraph")
            .expect("report.wholeGraph"),
        "report.wholeGraph",
    );
    assert_eq!(
        u64_field(
            &Value::Object(whole_graph.clone()),
            "actionCount",
            "wholeGraph"
        ),
        6
    );

    let classes = object(
        object(&report, "report")
            .get("classes")
            .expect("report.classes"),
        "report.classes",
    );
    let rbe = classes.get("rbe").expect("rbe summary");
    assert_eq!(u64_field(rbe, "actionCount", "rbe"), 2);
    assert_eq!(u64_field(rbe, "grossInputBytes", "rbe"), 180);
    assert_eq!(u64_field(rbe, "uniqueInputBytes", "rbe"), 160);
    assert_eq!(u64_field(rbe, "outputBytes", "rbe"), 35);
    assert!(
        rbe.get("largestInputs")
            .and_then(Value::as_array)
            .expect("rbe largest inputs")
            .iter()
            .any(|artifact| artifact.get("digest").and_then(Value::as_str) == Some("local-large"))
    );
    assert_eq!(
        rbe.get("responsibleTargets")
            .and_then(Value::as_array)
            .expect("rbe responsible targets")
            .len(),
        2
    );

    let cache_only = classes
        .get("remote-cache-only")
        .expect("remote-cache-only summary");
    assert_eq!(u64_field(cache_only, "actionCount", "remote-cache-only"), 1);
    assert_eq!(
        u64_field(cache_only, "grossInputBytes", "remote-cache-only"),
        20
    );

    let local = classes.get("fully-local").expect("fully-local summary");
    assert_eq!(u64_field(local, "actionCount", "fully-local"), 3);
    assert_eq!(u64_field(local, "outputBytes", "fully-local"), 1103);
    assert!(
        !local
            .get("largestInputs")
            .and_then(Value::as_array)
            .expect("local largest inputs")
            .iter()
            .any(|artifact| artifact.get("digest").and_then(Value::as_str) == Some("local-large"))
    );

    let mnemonics = object(&report, "report")
        .get("mnemonics")
        .and_then(Value::as_array)
        .expect("report.mnemonics");
    let compact = mnemonics
        .iter()
        .find(|mnemonic| {
            mnemonic.get("mnemonic").and_then(Value::as_str) == Some("ExtractCargoTomlEnvVars")
        })
        .expect("ExtractCargoTomlEnvVars mnemonic");
    assert_eq!(
        u64_field(compact, "grossInputBytes", "ExtractCargoTomlEnvVars"),
        150
    );
    assert_eq!(
        u64_field(compact, "uniqueInputBytes", "ExtractCargoTomlEnvVars"),
        130
    );

    let boundaries = object(&report, "report")
        .get("boundaryCrossings")
        .and_then(Value::as_array)
        .expect("report.boundaryCrossings");
    assert_eq!(boundaries.len(), 2);
    assert!(
        boundaries.iter().any(|boundary| {
            let boundary = object(boundary, "boundary");
            boundary.get("direction").and_then(Value::as_str) == Some("local-to-remote")
                && boundary.get("digest").and_then(Value::as_str) == Some("local-large")
        }),
        "local artifact crossing must be reported"
    );
    assert!(
        boundaries.iter().any(|boundary| {
            let boundary = object(boundary, "boundary");
            boundary.get("direction").and_then(Value::as_str) == Some("remote-to-local")
                && boundary.get("digest").and_then(Value::as_str) == Some("test-log")
        }),
        "remote artifact crossing must be reported"
    );

    let largest = object(
        object(&report, "report")
            .get("largestArtifacts")
            .expect("report.largestArtifacts"),
        "report.largestArtifacts",
    );
    let highest_exposure = largest
        .get("highestExposure")
        .and_then(Value::as_array)
        .expect("highest exposure");
    assert_eq!(
        highest_exposure[0]
            .get("digest")
            .and_then(Value::as_str)
            .expect("highest exposure digest"),
        "shared-rmeta"
    );

    let schema = read_json(&repo_root().join("tests/golden/bazel/cache-transfer-schema.json"));
    let schema = object(&schema, "schema");
    let report_object = object(&report, "report");
    for field in schema
        .get("requiredTopLevelFields")
        .and_then(Value::as_array)
        .expect("schema top-level fields")
    {
        let field = field.as_str().expect("schema top-level field");
        assert!(
            report_object.contains_key(field),
            "report is missing schema field {field}"
        );
    }
    let source = report_object.get("source").expect("report.source");
    for field in schema
        .get("requiredSourceFields")
        .and_then(Value::as_array)
        .expect("schema source fields")
    {
        let field = field.as_str().expect("schema source field");
        assert!(
            object(source, "report.source").contains_key(field),
            "report.source is missing schema field {field}"
        );
    }
    let scope_fields = schema
        .get("requiredScopeFields")
        .and_then(Value::as_array)
        .expect("schema scope fields");
    let mut scopes = vec![report_object.get("wholeGraph").expect("whole graph")];
    scopes.extend(
        report_object
            .get("classes")
            .and_then(Value::as_object)
            .expect("classes")
            .values(),
    );
    for scope in scopes {
        let scope = object(scope, "scope");
        for field in scope_fields {
            let field = field.as_str().expect("schema scope field");
            assert!(
                scope.contains_key(field),
                "scope is missing schema field {field}"
            );
        }
    }

    let _ = std::fs::remove_file(&output_path);
    let _ = std::fs::remove_dir(output_path.parent().expect("test directory"));
}

#[test]
fn compares_compatible_reports_and_rejects_graph_mismatch() {
    let directory = scratch_root().join(format!(
        ".scratch/bazel-cache-transfer-compare-{}/",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).expect("create scratch");
    let baseline = directory.join("baseline.json");
    let optimized = directory.join("optimized.json");
    let delta = directory.join("delta.json");

    for (log, report) in [
        ("baseline-execution-log.json", &baseline),
        ("optimized-execution-log.json", &optimized),
    ] {
        let output = run_xtask(&[
            "bazel-cache-transfer",
            "--execution-log",
            fixture(log).to_str().expect("fixture path"),
            "--eligibility",
            fixture("eligibility.json")
                .to_str()
                .expect("eligibility path"),
            "--output",
            report.to_str().expect("report path"),
            "--configuration",
            "local",
            "--platform",
            "linux-x86_64",
            "--toolchain",
            "rules_rust-fixture",
        ]);
        assert!(
            output.status.success(),
            "analyzer failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let output = run_xtask(&[
        "bazel-cache-transfer",
        "compare",
        "--baseline",
        baseline.to_str().expect("baseline path"),
        "--optimized",
        optimized.to_str().expect("optimized path"),
        "--output",
        delta.to_str().expect("delta path"),
    ]);
    assert!(
        output.status.success(),
        "comparison failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let comparison = read_json(&delta);
    assert!(
        comparison
            .get("compatible")
            .and_then(Value::as_bool)
            .expect("comparison.compatible")
    );
    assert_eq!(
        comparison
            .get("delta")
            .and_then(|value| value.get("rbe"))
            .and_then(|value| value.get("grossInputBytes"))
            .and_then(Value::as_i64)
            .expect("rbe gross delta"),
        -20
    );

    let optimized_report = read_json(&optimized);
    let mut topology_log = read_json(&fixture("optimized-execution-log.json"));
    topology_log
        .get_mut("records")
        .and_then(Value::as_array_mut)
        .and_then(|records| {
            records.iter_mut().find(|record| {
                record
                    .get("spawnExec")
                    .and_then(|payload| payload.get("actualOutputs"))
                    .and_then(Value::as_array)
                    .is_some_and(|outputs| !outputs.is_empty())
            })
        })
        .and_then(|record| record.get_mut("spawnExec"))
        .and_then(Value::as_object_mut)
        .and_then(|payload| payload.get_mut("actualOutputs"))
        .and_then(Value::as_array_mut)
        .and_then(|outputs| outputs.first_mut())
        .and_then(Value::as_object_mut)
        .expect("optimized action output")
        .insert(
            "path".to_owned(),
            Value::String("different/output.txt".to_owned()),
        );
    let topology_log_path = directory.join("topology-execution-log.json");
    std::fs::write(
        &topology_log_path,
        serde_json::to_vec_pretty(&topology_log).expect("serialize topology log"),
    )
    .expect("write topology log");
    let output = run_xtask(&[
        "bazel-cache-transfer",
        "--execution-log",
        topology_log_path.to_str().expect("topology log path"),
        "--eligibility",
        fixture("eligibility.json")
            .to_str()
            .expect("eligibility path"),
        "--configuration",
        "local",
        "--platform",
        "linux-x86_64",
        "--toolchain",
        "rules_rust-fixture",
        "--output",
        optimized.to_str().expect("optimized path"),
    ]);
    assert!(
        output.status.success(),
        "topology analyzer failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = run_xtask(&[
        "bazel-cache-transfer",
        "compare",
        "--baseline",
        baseline.to_str().expect("baseline path"),
        "--optimized",
        optimized.to_str().expect("optimized path"),
    ]);
    assert!(
        !output.status.success(),
        "output topology mismatch must fail closed"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("graph digest"),
        "topology mismatch should name the graph digest: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut mismatched = optimized_report;
    object_mut(&mut mismatched, "report")
        .get_mut("source")
        .and_then(Value::as_object_mut)
        .expect("report.source")
        .insert(
            "graphDigest".to_owned(),
            Value::String("sha256:mismatch".to_owned()),
        );
    std::fs::write(
        &optimized,
        serde_json::to_vec_pretty(&mismatched).expect("serialize mismatch"),
    )
    .expect("write mismatch");
    let output = run_xtask(&[
        "bazel-cache-transfer",
        "compare",
        "--baseline",
        baseline.to_str().expect("baseline path"),
        "--optimized",
        optimized.to_str().expect("optimized path"),
    ]);
    assert!(!output.status.success(), "graph mismatch must fail closed");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("graph digest"),
        "mismatch should name the graph digest: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut missing_identity = read_json(&baseline);
    object_mut(&mut missing_identity, "report")
        .get_mut("source")
        .and_then(Value::as_object_mut)
        .expect("report.source")
        .insert("configuration".to_owned(), Value::Null);
    std::fs::write(
        &optimized,
        serde_json::to_vec_pretty(&missing_identity).expect("serialize missing identity"),
    )
    .expect("write missing identity");
    let output = run_xtask(&[
        "bazel-cache-transfer",
        "compare",
        "--baseline",
        baseline.to_str().expect("baseline path"),
        "--optimized",
        optimized.to_str().expect("optimized path"),
    ]);
    assert!(
        !output.status.success(),
        "missing identity must fail closed"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("configuration"),
        "missing identity should name configuration: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn rejects_missing_fields_duplicate_records_and_overflow() {
    let mut baseline = read_json(&fixture("baseline-execution-log.json"));
    let records = baseline
        .get_mut("records")
        .and_then(Value::as_array_mut)
        .expect("fixture records");
    let first_payload = records[0]
        .get_mut("spawnExec")
        .and_then(Value::as_object_mut)
        .expect("first payload");
    first_payload.remove("targetLabel");
    let output = run_with_log_value(&baseline, "missing-target");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("target label"));

    let mut baseline = read_json(&fixture("baseline-execution-log.json"));
    let records = baseline
        .get_mut("records")
        .and_then(Value::as_array_mut)
        .expect("fixture records");
    let first_input = records[0]
        .get_mut("spawnExec")
        .and_then(Value::as_object_mut)
        .and_then(|payload| payload.get_mut("inputs"))
        .and_then(Value::as_array_mut)
        .and_then(|inputs| inputs.first_mut())
        .and_then(Value::as_object_mut)
        .expect("first input");
    first_input.remove("digest");
    first_input.insert("sizeBytes".to_owned(), Value::from(1));
    let output = run_with_log_value(&baseline, "missing-digest");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("digest"));

    let mut baseline = read_json(&fixture("baseline-execution-log.json"));
    let records = baseline
        .get_mut("records")
        .and_then(Value::as_array_mut)
        .expect("fixture records");
    let first_digest = records[0]
        .get_mut("spawnExec")
        .and_then(Value::as_object_mut)
        .and_then(|payload| payload.get_mut("inputs"))
        .and_then(Value::as_array_mut)
        .and_then(|inputs| inputs.first_mut())
        .and_then(Value::as_object_mut)
        .and_then(|input| input.get_mut("digest"))
        .and_then(Value::as_object_mut)
        .expect("first digest");
    first_digest.remove("sizeBytes");
    let output = run_with_log_value(&baseline, "missing-size");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("size"));

    let mut baseline = read_json(&fixture("baseline-execution-log.json"));
    let first_input = baseline
        .get_mut("records")
        .and_then(Value::as_array_mut)
        .expect("fixture records")[0]
        .get_mut("spawnExec")
        .and_then(Value::as_object_mut)
        .and_then(|payload| payload.get_mut("inputs"))
        .and_then(Value::as_array_mut)
        .and_then(|inputs| inputs.first_mut())
        .and_then(Value::as_object_mut)
        .expect("first input");
    first_input.insert("sizeBytes".to_owned(), Value::from(6));
    first_input
        .get_mut("digest")
        .and_then(Value::as_object_mut)
        .expect("first digest")
        .insert("hash".to_owned(), Value::String(String::new()));
    let output = run_with_log_value(&baseline, "conflicting-empty-size");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("conflicting"));

    let mut baseline = read_json(&fixture("baseline-execution-log.json"));
    let records = baseline
        .get_mut("records")
        .and_then(Value::as_array_mut)
        .expect("fixture records");
    let duplicate = records[0].clone();
    records.push(duplicate);
    let output = run_with_log_value(&baseline, "duplicate");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("duplicate"));

    let mut baseline = read_json(&fixture("baseline-execution-log.json"));
    let records = baseline
        .get_mut("records")
        .and_then(Value::as_array_mut)
        .expect("fixture records");
    let compile_payload = records[3]
        .get_mut("spawnExec")
        .and_then(Value::as_object_mut)
        .expect("compile payload");
    compile_payload.insert("inputs".to_owned(), Value::Array(Vec::new()));
    compile_payload.remove("remotable");
    compile_payload.remove("cacheable");
    compile_payload.remove("remoteCacheable");
    compile_payload.remove("runner");
    let output = run_with_log_value(&baseline, "missing-class");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("execution-class"));

    let mut baseline = read_json(&fixture("baseline-execution-log.json"));
    let records = baseline
        .get_mut("records")
        .and_then(Value::as_array_mut)
        .expect("fixture records");
    let first_size = records[0]
        .get_mut("spawnExec")
        .and_then(Value::as_object_mut)
        .and_then(|payload| payload.get_mut("inputs"))
        .and_then(Value::as_array_mut)
        .and_then(|inputs| inputs.first_mut())
        .and_then(Value::as_object_mut)
        .and_then(|input| input.get_mut("digest"))
        .and_then(Value::as_object_mut)
        .expect("first digest");
    first_size.insert(
        "hash".to_owned(),
        Value::String("overflow-source".to_owned()),
    );
    first_size.insert("sizeBytes".to_owned(), Value::from(u64::MAX));
    records[0]
        .get_mut("spawnExec")
        .and_then(Value::as_object_mut)
        .and_then(|payload| payload.get_mut("inputs"))
        .and_then(Value::as_array_mut)
        .expect("first inputs")
        .push(serde_json::json!({
            "path": "fixtures/overflow.txt",
            "digest": {
                "hash": "overflow-input",
                "sizeBytes": 1
            }
        }));
    let output = run_with_log_value(&baseline, "overflow");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("overflow"));

    let mut baseline = read_json(&fixture("baseline-execution-log.json"));
    baseline
        .get_mut("records")
        .and_then(Value::as_array_mut)
        .expect("fixture records")[0]
        .get_mut("spawnExec")
        .and_then(Value::as_object_mut)
        .expect("first payload")
        .extend([
            (
                "status".to_owned(),
                Value::String("NON_ZERO_EXIT".to_owned()),
            ),
            ("exitCode".to_owned(), Value::from(1)),
        ]);
    let output = run_with_log_value(&baseline, "failed-action");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed"));

    let output = run_with_log_bytes(b"{", "malformed");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("parse execution log"));
}

#[test]
fn accepts_direct_spawnexec_arrays_and_json_lines() {
    let wrapped = read_json(&fixture("baseline-execution-log.json"));
    let wrapped_records = wrapped
        .get("records")
        .and_then(Value::as_array)
        .expect("fixture records");
    let direct_records = wrapped_records
        .iter()
        .map(|record| {
            let payload = record
                .get("spawnExec")
                .and_then(Value::as_object)
                .expect("wrapped payload")
                .clone();
            let mut value = Value::Object(payload);
            stringify_sizes(&mut value);
            value
        })
        .collect::<Vec<_>>();
    let output = run_with_log_value(&Value::Array(direct_records.clone()), "direct-array");
    assert!(
        output.status.success(),
        "direct SpawnExec array failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut lines = Vec::new();
    for record in direct_records {
        lines.extend_from_slice(
            serde_json::to_vec_pretty(&record)
                .expect("serialize SpawnExec")
                .as_slice(),
        );
    }
    let output = run_with_log_bytes(&lines, "concatenated-json");
    assert!(
        output.status.success(),
        "concatenated SpawnExec JSON failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn keeps_empty_symlink_and_path_identity_semantics() {
    let log = serde_json::json!({
        "records": [
            {
                "type": "SpawnExec",
                "id": "local-shared",
                "spawnExec": {
                    "mnemonic": "Genrule",
                    "targetLabel": "//demo:local",
                    "runner": "linux-sandbox",
                    "actualOutputs": [{
                        "path": "out/shared",
                        "digest": {"hash": "same-content", "sizeBytes": 4}
                    }]
                }
            },
            {
                "type": "SpawnExec",
                "id": "remote-different-path",
                "spawnExec": {
                    "mnemonic": "ExtractCargoTomlEnvVars",
                    "targetLabel": "//demo:rbe",
                    "remotable": true,
                    "inputs": [{
                        "path": "different/shared",
                        "digest": {"hash": "same-content", "sizeBytes": 4}
                    }]
                }
            },
            {
                "type": "SpawnExec",
                "id": "local-cross",
                "spawnExec": {
                    "mnemonic": "Genrule",
                    "targetLabel": "//demo:local",
                    "runner": "linux-sandbox",
                    "actualOutputs": [{
                        "path": "out/cross",
                        "digest": {"hash": "cross-content", "sizeBytes": 6}
                    }]
                }
            },
            {
                "type": "SpawnExec",
                "id": "remote-cross",
                "spawnExec": {
                    "mnemonic": "ExtractCargoTomlEnvVars",
                    "targetLabel": "//demo:rbe",
                    "remotable": true,
                    "inputs": [{
                        "path": "out/cross",
                        "digest": {"hash": "cross-content", "sizeBytes": 6}
                    }]
                }
            },
            {
                "type": "SpawnExec",
                "id": "local-empty",
                "spawnExec": {
                    "mnemonic": "Empty",
                    "targetLabel": "//demo:local",
                    "runner": "linux-sandbox",
                    "inputs": [{"path": "empty", "sizeBytes": 0}],
                    "actualOutputs": [{"path": "out/empty"}]
                }
            },
            {
                "type": "SpawnExec",
                "id": "cache-local",
                "spawnExec": {
                    "mnemonic": "Rustc",
                    "targetLabel": "//demo:cache",
                    "cacheable": true,
                    "remoteCacheable": false,
                    "inputs": [{
                        "path": "src/cache.rs",
                        "digest": {"hash": "cache-input", "sizeBytes": 3}
                    }]
                }
            },
            {
                "type": "SpawnExec",
                "id": "local-symlink",
                "spawnExec": {
                    "mnemonic": "Symlink",
                    "targetLabel": "//demo:local",
                    "runner": "linux-sandbox",
                    "actualOutputs": [{
                        "path": "out/link",
                        "symlinkTargetPath": "target"
                    }]
                }
            }
        ],
        "metadata": {
            "configuration": "local",
            "platform": "linux-x86_64",
            "toolchain": "rules_rust-fixture"
        }
    });
    let report = analyze_value(&log, "artifact-semantics");
    let classes = object(&report, "report")
        .get("classes")
        .and_then(Value::as_object)
        .expect("classes");
    assert_eq!(
        u64_field(classes.get("rbe").expect("rbe"), "actionCount", "rbe"),
        2
    );
    assert_eq!(
        u64_field(
            classes.get("remote-cache-only").expect("remote-cache-only"),
            "actionCount",
            "remote-cache-only"
        ),
        0
    );
    assert_eq!(
        u64_field(
            classes.get("fully-local").expect("fully-local"),
            "actionCount",
            "fully-local"
        ),
        5
    );
    let boundaries = object(&report, "report")
        .get("boundaryCrossings")
        .and_then(Value::as_array)
        .expect("boundaries");
    assert_eq!(boundaries.len(), 1);
    assert_eq!(
        boundaries[0]
            .get("digest")
            .and_then(Value::as_str)
            .expect("boundary digest"),
        "cross-content"
    );
}

#[test]
fn records_unlisted_dependency_owners_from_bazel_signals() {
    let log = serde_json::json!({
        "records": [{
            "type": "SpawnExec",
            "spawnExec": {
                "mnemonic": "ExtractCargoTomlEnvVars",
                "targetLabel": "//demo:dependency",
                "remotable": true,
                "inputs": [{
                    "path": "src/dependency.rs",
                    "digest": {"hash": "dependency-source", "sizeBytes": 3}
                }]
            }
        }],
        "metadata": {
            "configuration": "local",
            "platform": "linux-x86_64",
            "toolchain": "rules_rust-fixture"
        }
    });
    let report = analyze_value(&log, "unlisted-owner");
    let source = object(&report, "report")
        .get("source")
        .and_then(Value::as_object)
        .expect("source");
    assert_eq!(
        source
            .get("unlistedTargets")
            .and_then(Value::as_array)
            .expect("unlisted targets")[0]
            .as_str(),
        Some("//demo:dependency")
    );
    assert_eq!(
        u64_field(
            object(&report, "report")
                .get("classes")
                .and_then(Value::as_object)
                .expect("classes")
                .get("rbe")
                .expect("rbe"),
            "actionCount",
            "rbe"
        ),
        1
    );
}

#[test]
fn representative_local_summary_records_measured_two_crate_bounds() {
    let summary =
        read_json(&repo_root().join("tests/golden/bazel/cache-transfer-representative.json"));
    let summary = summary.as_object().expect("representative summary");
    assert_eq!(
        summary.get("reportKind").and_then(Value::as_str),
        Some("representative-summary")
    );
    assert_eq!(
        u64_field(
            summary.get("wholeGraph").expect("wholeGraph"),
            "actionCount",
            "wholeGraph"
        ),
        207
    );
    assert_eq!(
        u64_field(
            summary.get("wholeGraph").expect("wholeGraph"),
            "grossInputBytes",
            "wholeGraph"
        ),
        162_901_404_939
    );
    let rejected = summary
        .get("pipeliningRejected")
        .and_then(Value::as_object)
        .expect("pipeliningRejected");
    assert!(
        rejected
            .get("pipelinedGrossInputBytes")
            .and_then(Value::as_u64)
            .expect("pipelined gross")
            > 162_901_404_939
    );

    let classes_value = summary.get("classes").expect("classes");
    let class = |key: &str| -> &Value {
        if let Some(object) = classes_value.as_object() {
            return object.get(key).unwrap_or_else(|| panic!("{key} class"));
        }
        classes_value
            .as_array()
            .expect("representative classes")
            .iter()
            .find_map(|entry| {
                let object = entry.as_object()?;
                (object.get("key").and_then(Value::as_str) == Some(key))
                    .then(|| object.get("value"))
                    .flatten()
            })
            .unwrap_or_else(|| panic!("{key} class"))
    };
    let remote_unique = ["rbe", "remote-cache-only"]
        .iter()
        .map(|key| u64_field(class(key), "uniqueInputBytes", key))
        .sum::<u64>();
    let remote_gross = ["rbe", "remote-cache-only"]
        .iter()
        .map(|key| u64_field(class(key), "grossInputBytes", key))
        .sum::<u64>();
    assert!(
        remote_unique <= 96 * 1024 * 1024,
        "remote-class unique inputs {remote_unique} exceed the 96 MiB compact budget"
    );
    assert!(
        remote_gross < 1_000_000_000,
        "compact-only remote class must stay far below the 80 GB working budget, got {remote_gross}"
    );
    let mnemonics = summary
        .get("mnemonics")
        .and_then(Value::as_array)
        .expect("mnemonics");
    for mnemonic_name in ["Rustc", "CargoBuildScriptRun"] {
        let mnemonic = mnemonics
            .iter()
            .find(|entry| entry.get("mnemonic").and_then(Value::as_str) == Some(mnemonic_name))
            .unwrap_or_else(|| panic!("{mnemonic_name} mnemonic"));
        let remote_actions = ["rbe", "remote-cache-only"]
            .iter()
            .map(|class_name| {
                mnemonic
                    .get("byClass")
                    .and_then(Value::as_object)
                    .and_then(|by_class| by_class.get(*class_name))
                    .and_then(|class| class.get("actionCount"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            })
            .sum::<u64>();
        assert_eq!(
            remote_actions, 0,
            "{mnemonic_name} must stay fully local, not {remote_actions} remote-class actions"
        );
    }
}

#[test]
fn high_io_remotable_rustc_stays_fully_local() {
    let report = analyze_value(
        &serde_json::json!({
            "metadata": {
                "configuration": "local",
                "platform": "linux-x86_64",
                "toolchain": "rules_rust-fixture"
            },
            "records": [{
                "type": "SpawnExec",
                "id": "rustc-heavy",
                "spawnExec": {
                    "mnemonic": "Rustc",
                    "targetLabel": "//demo:remote-consumer",
                    "remotable": true,
                    "remoteCacheable": true,
                    "inputs": [{
                        "path": "large.rlib",
                        "digest": { "hash": "large", "sizeBytes": 2_000_000 }
                    }],
                    "actualOutputs": [{
                        "path": "out.rlib",
                        "digest": { "hash": "out", "sizeBytes": 1 }
                    }]
                }
            }]
        }),
        "high-io-rustc",
    );
    let classes = object(report.get("classes").expect("classes"), "classes");
    assert_eq!(
        u64_field(classes.get("rbe").expect("rbe"), "actionCount", "rbe"),
        0
    );
    assert_eq!(
        u64_field(
            classes.get("fully-local").expect("fully-local"),
            "actionCount",
            "fully-local"
        ),
        1
    );
}

fn stringify_sizes(value: &mut Value) {
    match value {
        Value::Array(values) => values.iter_mut().for_each(stringify_sizes),
        Value::Object(object) => {
            if let Some(size) = object.get_mut("sizeBytes")
                && let Some(number) = size.as_u64()
            {
                *size = Value::String(number.to_string());
            }
            object.values_mut().for_each(stringify_sizes);
        }
        _ => {}
    }
}

fn object_mut<'a>(value: &'a mut Value, context: &str) -> &'a mut serde_json::Map<String, Value> {
    value
        .as_object_mut()
        .unwrap_or_else(|| panic!("{context} must be an object"))
}
