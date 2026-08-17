#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

use serde_json::{Map, Value};

const EXPECTED_BAZEL_VERSION: &str = "bazel 9.2.0";
const MAIN_EXCLUDES: [&str; 3] = [
    "d2b-contract-tests",
    "d2b-priv-broker",
    "d2b-guest-shell-runner",
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
    if let Some(manifest_dir) = option_env!("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from))
    {
        candidates.push(manifest_dir);
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

fn required_object<'a>(
    value: &'a Map<String, Value>,
    key: &str,
    context: &str,
) -> &'a Map<String, Value> {
    object(
        value
            .get(key)
            .unwrap_or_else(|| panic!("{context}.{key} is missing")),
        &format!("{context}.{key}"),
    )
}

fn array<'a>(value: &'a Map<String, Value>, key: &str, context: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{context}.{key} must be an array"))
}

fn required_string<'a>(value: &'a Map<String, Value>, key: &str, context: &str) -> &'a str {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{context}.{key} must be a string"))
}

fn required_bool(value: &Map<String, Value>, key: &str, context: &str) -> bool {
    value
        .get(key)
        .and_then(Value::as_bool)
        .unwrap_or_else(|| panic!("{context}.{key} must be a boolean"))
}

fn required_string_set(value: &Map<String, Value>, key: &str, context: &str) -> BTreeSet<String> {
    array(value, key, context)
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .unwrap_or_else(|| panic!("{context}.{key} entries must be strings"))
                .to_owned()
        })
        .collect()
}

fn source_exists(relative: &str, context: &str) {
    assert!(
        !relative.is_empty(),
        "{context} source path must not be empty"
    );
    let path = repo_root().join(relative);
    assert!(
        path.is_file() || path.is_dir(),
        "{context} source does not exist: {relative}"
    );
}

fn repository_relative_path(path: &Path) -> String {
    if let Ok(relative) = path.strip_prefix(repo_root()) {
        return relative.display().to_string();
    }
    let mut components = path.components();
    while let Some(component) = components.next() {
        if component.as_os_str() == "_main" {
            let relative = components.as_path();
            return relative.display().to_string();
        }
    }
    panic!(
        "Cargo target source is outside the repository: {}",
        path.display()
    );
}

fn canonical_target_key(package: &str, name: &str, kinds: &[String]) -> String {
    let mut kinds = kinds.to_vec();
    kinds.sort();
    format!("{package}:{name}:{}", kinds.join("+"))
}

fn cargo_program() -> std::ffi::OsString {
    let raw = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let raw = PathBuf::from(raw);
    if let Ok(relative) = raw.strip_prefix("${pwd}") {
        return std::env::current_dir()
            .expect("parity test current directory")
            .join(relative.strip_prefix("/").unwrap_or(relative))
            .into_os_string();
    }
    if raw.is_relative() {
        let mut bases = vec![std::env::current_dir().expect("parity test current directory")];
        for variable in ["TEST_SRCDIR", "RUNFILES_DIR"] {
            if let Some(base) = std::env::var_os(variable).map(PathBuf::from) {
                bases.push(base.clone());
                if let Some(workspace) = std::env::var_os("TEST_WORKSPACE") {
                    bases.push(base.join(workspace));
                }
                bases.push(base.join("_main"));
            }
        }
        for base in bases {
            let candidate = base.join(&raw);
            if candidate.is_file() {
                return candidate.into_os_string();
            }
        }
    }
    raw.into_os_string()
}

fn cargo_command() -> Command {
    let mut command = Command::new(cargo_program());
    if let Some(test_tmpdir) = std::env::var_os("TEST_TMPDIR") {
        command.env(
            "CARGO_TARGET_DIR",
            PathBuf::from(test_tmpdir).join("d2b-bazel-rust-parity-target"),
        );
    }
    command
}

fn cargo_metadata() -> Value {
    let output = cargo_command()
        .args([
            "metadata",
            "--locked",
            "--offline",
            "--format-version",
            "1",
            "--no-deps",
        ])
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|error| panic!("run cargo metadata via {:?}: {error}", cargo_program()));
    assert!(
        output.status.success(),
        "cargo metadata failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("cargo metadata JSON")
}

fn cargo_nextest(arguments: &[String]) -> Value {
    let nextest_args = [
        "nextest".to_owned(),
        "list".to_owned(),
        "--locked".to_owned(),
    ]
    .into_iter()
    .chain(arguments.iter().cloned())
    .chain(["--message-format".to_owned(), "json".to_owned()])
    .collect::<Vec<_>>();
    let root = repo_root();
    let output = {
        let mut command = cargo_command();
        command
            .args(&nextest_args)
            .current_dir(&root)
            .output()
            .unwrap_or_else(|error| {
                panic!("run cargo nextest list via {:?}: {error}", cargo_program())
            })
    };
    let output = if output.status.success()
        || !String::from_utf8_lossy(&output.stderr).contains("no such command: `nextest`")
    {
        output
    } else {
        let nix_root = root
            .join("flake.nix")
            .canonicalize()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| root.clone());
        let mut command = Command::new("nix");
        command
            .args(["shell", "--quiet", "--inputs-from"])
            .arg(nix_root)
            .arg("nixpkgs#cargo-nextest")
            .arg("--command")
            .arg(cargo_program())
            .args(&nextest_args)
            .current_dir(&root);
        if let Some(test_tmpdir) = std::env::var_os("TEST_TMPDIR") {
            command.env(
                "CARGO_TARGET_DIR",
                PathBuf::from(test_tmpdir).join("d2b-bazel-rust-parity-target"),
            );
        }
        command.output().unwrap_or_else(|error| {
            panic!(
                "run cargo nextest through nix via {:?}: {error}",
                cargo_program()
            )
        })
    };
    assert!(
        output.status.success(),
        "cargo nextest list failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("cargo nextest JSON")
}

fn nextest_suite_keys(value: &Value) -> BTreeMap<String, usize> {
    let suites = required_object(object(value, "nextest"), "rust-suites", "nextest");
    suites
        .iter()
        .map(|(_, suite)| {
            let suite = object(suite, "nextest suite");
            let package = required_string(suite, "package-name", "nextest suite");
            let binary = required_string(suite, "binary-name", "nextest suite");
            let kind = required_string(suite, "kind", "nextest suite");
            let testcases = suite
                .get("testcases")
                .and_then(Value::as_object)
                .unwrap_or_else(|| panic!("nextest suite.testcases must be an object"));
            (format!("{package}:{binary}:{kind}"), testcases.len())
        })
        .collect()
}

fn metadata_target_keys(value: &Value) -> BTreeMap<String, (String, bool, bool)> {
    let metadata = object(value, "cargo metadata");
    let workspace_members = required_string_set(metadata, "workspace_members", "cargo metadata");
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("cargo metadata.packages must be an array"));
    let mut targets = BTreeMap::new();
    for package in packages {
        let package = object(package, "cargo package");
        let package_id = required_string(package, "id", "cargo package");
        if !workspace_members.contains(package_id) {
            continue;
        }
        let package_name = required_string(package, "name", "cargo package");
        for target in package
            .get("targets")
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("cargo package.targets must be an array"))
        {
            let target = object(target, "cargo target");
            let name = required_string(target, "name", "cargo target");
            let kinds = target
                .get("kind")
                .and_then(Value::as_array)
                .unwrap_or_else(|| panic!("cargo target.kind must be an array"))
                .iter()
                .map(|kind| {
                    kind.as_str()
                        .unwrap_or_else(|| panic!("cargo target.kind entries must be strings"))
                        .to_owned()
                })
                .collect::<Vec<_>>();
            let key = canonical_target_key(package_name, name, &kinds);
            let src_path = required_string(target, "src_path", "cargo target");
            let relative = repository_relative_path(Path::new(src_path));
            let doc = target
                .get("doctest")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let test = target.get("test").and_then(Value::as_bool).unwrap_or(false);
            targets.insert(key, (relative, doc, test));
        }
    }
    targets
}

fn metadata_bench_keys(value: &Value) -> BTreeSet<String> {
    let metadata = object(value, "cargo metadata");
    let workspace_members = required_string_set(metadata, "workspace_members", "cargo metadata");
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("cargo metadata.packages must be an array"));
    packages
        .iter()
        .filter_map(|package| {
            let package = object(package, "cargo package");
            let package_id = required_string(package, "id", "cargo package");
            workspace_members.contains(package_id).then_some(package)
        })
        .flat_map(|package| {
            let package_name = required_string(package, "name", "cargo package").to_owned();
            package
                .get("targets")
                .and_then(Value::as_array)
                .unwrap_or_else(|| panic!("cargo package.targets must be an array"))
                .iter()
                .filter_map(move |target| {
                    let target = object(target, "cargo target");
                    let is_bench = target
                        .get("kind")
                        .and_then(Value::as_array)
                        .unwrap_or_else(|| panic!("cargo target.kind must be an array"))
                        .iter()
                        .any(|kind| kind.as_str() == Some("bench"));
                    is_bench.then(|| {
                        format!(
                            "{package_name}:{}",
                            required_string(target, "name", "cargo target")
                        )
                    })
                })
        })
        .collect()
}

fn validate_carrier_inventory(coverage: &Map<String, Value>) -> BTreeSet<String> {
    let carriers = array(coverage, "carriers", "rust coverage");
    let mut ids = BTreeSet::new();
    let mut labels = BTreeSet::new();
    for (index, value) in carriers.iter().enumerate() {
        let context = format!("rust coverage.carriers[{index}]");
        let carrier = object(value, &context);
        let id = required_string(carrier, "id", &context);
        assert!(ids.insert(id.to_owned()), "duplicate carrier id: {id}");
        let label = required_string(carrier, "label", &context);
        assert!(
            labels.insert(label.to_owned()),
            "duplicate carrier label: {label}"
        );
        assert!(
            label.starts_with("//") && label.contains(':'),
            "{context}.label must be a canonical Bazel label: {label}"
        );
        let sources = array(carrier, "sources", &context);
        assert!(!sources.is_empty(), "{context}.sources must not be empty");
        for (source_index, source) in sources.iter().enumerate() {
            let source = source
                .as_str()
                .unwrap_or_else(|| panic!("{context}.sources[{source_index}] must be a string"));
            source_exists(source, &format!("{context}.sources[{source_index}]"));
        }
        let execution = required_string(carrier, "execution", &context);
        assert!(
            matches!(
                execution,
                "run" | "compile-only" | "doctest" | "bench" | "proof" | "policy"
            ),
            "{context}.execution has unknown value: {execution}"
        );
    }
    labels
}

fn validate_target_references(
    coverage: &Map<String, Value>,
    carrier_labels: &BTreeSet<String>,
    metadata_targets: &BTreeMap<String, (String, bool, bool)>,
) {
    let targets = array(coverage, "cargoTargets", "rust coverage");
    let mut mapped = BTreeMap::new();
    for (index, value) in targets.iter().enumerate() {
        let context = format!("rust coverage.cargoTargets[{index}]");
        let target = object(value, &context);
        let package = required_string(target, "package", &context);
        let name = required_string(target, "name", &context);
        let kinds = array(target, "kinds", &context)
            .iter()
            .map(|kind| {
                kind.as_str()
                    .unwrap_or_else(|| panic!("{context}.kinds entries must be strings"))
                    .to_owned()
            })
            .collect::<Vec<_>>();
        let key = canonical_target_key(package, name, &kinds);
        assert!(
            mapped.insert(key.clone(), target).is_none(),
            "duplicate Cargo target in coverage: {key}"
        );
        let source = required_string(target, "source", &context);
        source_exists(source, &format!("{context}.source"));
        let carrier = required_string(target, "carrier", &context);
        assert!(
            carrier_labels.contains(carrier),
            "{context}.carrier is not a declared carrier: {carrier}"
        );
    }
    let expected = metadata_targets.keys().cloned().collect::<BTreeSet<_>>();
    let actual = mapped.keys().cloned().collect::<BTreeSet<_>>();
    assert_eq!(
        actual, expected,
        "Cargo metadata targets and rust coverage.cargoTargets differ"
    );
}

fn validate_nextest_references(coverage: &Map<String, Value>) {
    let nextest = required_object(coverage, "nextest", "rust coverage");
    for stream in [
        "main",
        "broker-default",
        "broker-layer1",
        "broker-fakebackends",
        "guest-real-libshpool",
    ] {
        let suites = array(nextest, stream, "rust coverage.nextest");
        let mut ids = BTreeSet::new();
        for (index, value) in suites.iter().enumerate() {
            let context = format!("rust coverage.nextest.{stream}[{index}]");
            let suite = object(value, &context);
            let package = required_string(suite, "package", &context);
            let binary = required_string(suite, "binary", &context);
            let kind = required_string(suite, "kind", &context);
            let id = format!("{package}:{binary}:{kind}");
            assert!(ids.insert(id.clone()), "duplicate nextest suite: {id}");
            let test_count = suite
                .get("testCount")
                .and_then(Value::as_u64)
                .unwrap_or_else(|| panic!("{context}.testCount must be an integer"));
            let execution = required_string(suite, "execution", &context);
            if test_count == 0 {
                assert_ne!(
                    execution, "run",
                    "{context} is compile-only but is marked as execution"
                );
            } else {
                assert_eq!(
                    execution, "run",
                    "{context} has test cases but is not marked as execution"
                );
            }
            let carrier = required_string(suite, "carrier", &context);
            assert!(
                carrier.starts_with("//") && carrier.contains(':'),
                "{context}.carrier must be a Bazel label"
            );
        }
    }
}

fn coverage_nextest_keys(coverage: &Map<String, Value>, stream: &str) -> BTreeMap<String, usize> {
    array(
        required_object(coverage, "nextest", "rust coverage"),
        stream,
        "rust coverage.nextest",
    )
    .iter()
    .map(|value| {
        let suite = object(value, "rust coverage nextest entry");
        let key = format!(
            "{}:{}:{}",
            required_string(suite, "package", "rust coverage nextest entry"),
            required_string(suite, "binary", "rust coverage nextest entry"),
            required_string(suite, "kind", "rust coverage nextest entry")
        );
        let count = suite
            .get("testCount")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| panic!("rust coverage nextest entry.testCount must be an integer"))
            as usize;
        (key, count)
    })
    .collect()
}

fn validate_nextest_inventory(
    coverage: &Map<String, Value>,
    stream: &str,
    expected: BTreeMap<String, usize>,
) {
    assert_eq!(
        coverage_nextest_keys(coverage, stream),
        expected,
        "nextest inventory and rust coverage.nextest.{stream} differ"
    );
}

fn validate_contexts(coverage: &Map<String, Value>) {
    let expected = [
        ("broker-default", true, None, "exclusive-serialized"),
        (
            "broker-layer1",
            true,
            Some("layer1-bootstrap"),
            "exclusive-serialized",
        ),
        (
            "broker-fakebackends",
            true,
            Some("fake-backends"),
            "exclusive-serialized",
        ),
        (
            "guest-shell-runner-real-libshpool",
            false,
            Some("real-libshpool"),
            "dedicated",
        ),
        ("no-bash-ast", false, None, "dedicated"),
        ("schema-reproducibility", false, None, "dedicated"),
        ("inventory-stub", false, None, "dedicated"),
        ("supply-chain", false, None, "dedicated"),
        ("fixture-contracts", false, None, "fixture-excluded"),
        ("cli-contracts", false, None, "fixture-excluded"),
        ("doctests", false, None, "companion"),
        ("harness-free-targets", false, None, "companion"),
        ("proof-workspaces", false, None, "dedicated"),
        ("benches", false, None, "companion"),
    ];
    let contexts = array(coverage, "contexts", "rust coverage");
    let mut actual = BTreeSet::new();
    for (index, value) in contexts.iter().enumerate() {
        let context = format!("rust coverage.contexts[{index}]");
        let context_value = object(value, &context);
        let id = required_string(context_value, "id", &context);
        assert!(actual.insert(id.to_owned()), "duplicate Rust context: {id}");
        required_string(context_value, "carrier", &context);
        let sources = array(context_value, "sources", &context);
        assert!(!sources.is_empty(), "{context}.sources must not be empty");
        for source in sources {
            source_exists(
                source
                    .as_str()
                    .unwrap_or_else(|| panic!("{context}.sources must contain strings")),
                &format!("{context}.sources"),
            );
        }
        let exclusive = required_bool(context_value, "exclusive", &context);
        let serial_group = context_value
            .get("serialGroup")
            .and_then(Value::as_str)
            .unwrap_or("");
        let feature = context_value.get("feature").and_then(Value::as_str);
        let expected_context = expected
            .iter()
            .find(|(expected_id, _, _, _)| *expected_id == id)
            .unwrap_or_else(|| panic!("unknown Rust context: {id}"));
        assert_eq!(
            required_string(context_value, "aggregate", &context),
            expected_context.3,
            "{context} has the wrong aggregate"
        );
        assert_eq!(
            feature, expected_context.2,
            "{context} has the wrong feature selector"
        );
        if id.starts_with("broker-") {
            assert!(exclusive, "{context} must be exclusive");
            assert_eq!(
                serial_group, "broker-process-global",
                "{context} must use the broker process-global serial group"
            );
        }
        if id == "guest-shell-runner-real-libshpool" {
            assert_eq!(feature, Some("real-libshpool"));
        }
    }
    let expected_ids = expected
        .into_iter()
        .map(|(id, _, _, _)| id.to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual, expected_ids,
        "Rust feature and companion context inventory differs"
    );
}

fn validate_special_inventory(coverage: &Map<String, Value>, expected_benches: &BTreeSet<String>) {
    let harness_free = array(coverage, "harnessFree", "rust coverage");
    assert!(
        !harness_free.is_empty(),
        "harness-free inventory must not be empty"
    );
    for (index, value) in harness_free.iter().enumerate() {
        let context = format!("rust coverage.harnessFree[{index}]");
        let entry = object(value, &context);
        required_string(entry, "package", &context);
        required_string(entry, "target", &context);
        source_exists(required_string(entry, "manifest", &context), &context);
        source_exists(required_string(entry, "source", &context), &context);
        required_string(entry, "carrier", &context);
    }

    let benches = array(coverage, "benches", "rust coverage");
    assert!(!benches.is_empty(), "bench inventory must not be empty");
    let mut actual_benches = BTreeSet::new();
    for (index, value) in benches.iter().enumerate() {
        let context = format!("rust coverage.benches[{index}]");
        let entry = object(value, &context);
        let key = format!(
            "{}:{}",
            required_string(entry, "package", &context),
            required_string(entry, "target", &context)
        );
        assert!(
            actual_benches.insert(key.clone()),
            "duplicate bench target: {key}"
        );
        source_exists(required_string(entry, "source", &context), &context);
        required_string(entry, "carrier", &context);
    }
    assert_eq!(
        actual_benches, *expected_benches,
        "Cargo bench targets and rust coverage.benches differ"
    );

    validate_ui_inventory(coverage);
}

fn validate_doctests(
    coverage: &Map<String, Value>,
    metadata_targets: &BTreeMap<String, (String, bool, bool)>,
) {
    let doctests = array(coverage, "doctests", "rust coverage");
    let mut actual = BTreeSet::new();
    for (index, value) in doctests.iter().enumerate() {
        let context = format!("rust coverage.doctests[{index}]");
        let entry = object(value, &context);
        let package = required_string(entry, "package", &context);
        let target = required_string(entry, "target", &context);
        let key = format!("{package}:{target}");
        assert!(
            actual.insert(key.clone()),
            "duplicate doctest target: {key}"
        );
        required_string(entry, "carrier", &context);
        let source = required_string(entry, "source", &context);
        source_exists(source, &context);
        assert!(
            entry.get("compileFail").and_then(Value::as_bool).is_some(),
            "{context}.compileFail must be a boolean"
        );
    }
    let expected = metadata_targets
        .iter()
        .filter(|(_, (_, doctest, _))| *doctest)
        .map(|(key, _)| {
            let (package, target, _) = key
                .split_once(':')
                .and_then(|(package, rest)| {
                    rest.split_once(':')
                        .map(|(target, kinds)| (package, target, kinds))
                })
                .unwrap_or_else(|| panic!("invalid Cargo target key: {key}"));
            (package.to_owned(), target.to_owned())
        })
        .map(|(package, target)| format!("{package}:{target}"))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual, expected,
        "Cargo doctest targets and rust coverage.doctests differ"
    );

    let compile_fail_sources = required_string_set(coverage, "compileFailSources", "rust coverage");
    let expected_compile_fail = discover_compile_fail_sources();
    assert_eq!(
        compile_fail_sources, expected_compile_fail,
        "compile_fail capability seals are not covered bidirectionally"
    );
}

fn discover_compile_fail_sources() -> BTreeSet<String> {
    let mut files = BTreeSet::new();
    discover_compile_fail_under(&repo_root().join("packages"), &mut files);
    discover_compile_fail_under(&repo_root().join("proofs"), &mut files);
    files
}

fn discover_compile_fail_under(path: &Path, files: &mut BTreeSet<String>) {
    if !path.exists() {
        return;
    }
    if path.is_file() {
        if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
            return;
        }
        if path.strip_prefix(repo_root()).ok().and_then(Path::to_str)
            == Some("packages/xtask/tests/bazel_rust_parity.rs")
        {
            return;
        }
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        if text.contains("compile_fail") {
            files.insert(
                path.strip_prefix(repo_root())
                    .expect("compile-fail source is under repository")
                    .display()
                    .to_string(),
            );
        }
        return;
    }
    for entry in
        std::fs::read_dir(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
    {
        discover_compile_fail_under(
            &entry
                .unwrap_or_else(|error| panic!("read directory entry: {error}"))
                .path(),
            files,
        );
    }
}

fn validate_ui_inventory(coverage: &Map<String, Value>) {
    let ui = required_object(coverage, "ui", "rust coverage");
    let inputs = required_string_set(ui, "inputs", "rust coverage.ui");
    let expected = discover_ui_files();
    if inputs != expected {
        let missing = expected.difference(&inputs).cloned().collect::<Vec<_>>();
        let extra = inputs.difference(&expected).cloned().collect::<Vec<_>>();
        panic!(
            "UI and trybuild fixtures are not covered bidirectionally; missing={missing:?}; extra={extra:?}"
        );
    }
    required_string(ui, "carrier", "rust coverage.ui");
}

fn discover_ui_files() -> BTreeSet<String> {
    let mut files = BTreeSet::new();
    for root in [
        repo_root().join("packages/d2b-bus/tests/ui"),
        repo_root().join("packages/d2b-controller-toolkit/tests/ui"),
        repo_root().join("packages/d2b-resource-api/tests/ui"),
    ] {
        discover_files(&root, &mut files);
    }
    files
}

fn discover_files(path: &Path, files: &mut BTreeSet<String>) {
    if !path.exists() {
        return;
    }
    if path.is_file() {
        let relative = path
            .strip_prefix(repo_root())
            .expect("discovered file is under the repository")
            .display()
            .to_string();
        files.insert(relative);
        return;
    }
    for entry in std::fs::read_dir(path)
        .unwrap_or_else(|error| panic!("read UI directory {}: {error}", path.display()))
    {
        discover_files(
            &entry
                .unwrap_or_else(|error| panic!("read UI directory entry: {error}"))
                .path(),
            files,
        );
    }
}

fn validate_rust_coverage(value: &Value) -> BTreeSet<String> {
    let coverage = object(value, "rust coverage");
    assert_eq!(
        coverage.get("schemaVersion").and_then(Value::as_u64),
        Some(1),
        "rust coverage schemaVersion must be 1"
    );
    let labels = validate_carrier_inventory(coverage);
    let metadata = cargo_metadata();
    let metadata_targets = metadata_target_keys(&metadata);
    validate_target_references(coverage, &labels, &metadata_targets);
    validate_doctests(coverage, &metadata_targets);
    validate_nextest_references(coverage);
    validate_contexts(coverage);
    validate_special_inventory(coverage, &metadata_bench_keys(&metadata));
    labels
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

fn bazel_output_user_root() -> PathBuf {
    if let Some(path) = std::env::var_os("D2B_BAZEL_OUTPUT_USER_ROOT") {
        return PathBuf::from(path);
    }
    if let Some(path) = std::env::var_os("XDG_CACHE_HOME") {
        return PathBuf::from(path).join("d2b-bazel-rust-parity-output");
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|path| path.join(".cache/d2b-bazel-rust-parity-output"))
        .unwrap_or_else(|| panic!("D2B_BAZEL_OUTPUT_USER_ROOT or HOME is required"))
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
    let output_stdout = String::from_utf8_lossy(&output.stdout);
    let store_path = output_stdout
        .lines()
        .rfind(|line| !line.trim().is_empty())
        .expect("Nix emitted a Bazel store path");
    let path = PathBuf::from(store_path.trim()).join("bin/bazel");
    assert_exact_bazel(&path, "the repository Bazel provider");
    path
}

fn run_bazel_query(expression: &str) -> Output {
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

fn query_labels(expression: &str) -> BTreeSet<String> {
    let output = run_bazel_query(expression);
    assert!(
        output.status.success(),
        "Bazel query failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.starts_with("//"))
        .map(str::to_owned)
        .collect()
}

fn run_bazel_cquery(expression: &str) -> Output {
    Command::new(bazel_binary())
        .arg(format!(
            "--output_user_root={}",
            bazel_output_user_root().display()
        ))
        .args([
            "cquery",
            "--noshow_progress",
            "--lockfile_mode=error",
            "--repo_contents_cache=",
            "--output=label",
            expression,
        ])
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|error| panic!("run Bazel cquery: {error}"))
}

fn cquery_labels(expression: &str) -> BTreeSet<String> {
    let output = run_bazel_cquery(expression);
    assert!(
        output.status.success(),
        "Bazel cquery failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|label| label.starts_with("//"))
        .map(str::to_owned)
        .collect()
}

fn validate_d2bd_test_support_dependencies(labels: &BTreeSet<String>) {
    // d2bd's test carrier deliberately reuses the production first-party graph
    // so d2b_contracts has one crate identity. Empty test-support features on
    // the production libraries expose the APIs needed by the libtest harness.
    let required = [
        "//packages/d2b-audit:d2b_audit",
        "//packages/d2b-contracts:d2b_contracts",
        "//packages/d2b-controller-toolkit:d2b_controller_toolkit",
        "//packages/d2b-core:d2b_core",
        "//packages/d2b-core-controller:d2b_core_controller",
        "//packages/d2b-host:d2b_host",
        "//packages/d2b-provider:d2b_provider",
        "//packages/d2b-provider-device-tpm:d2b_provider_device_tpm",
        "//packages/d2b-resource-api:d2b_resource_api",
        "//packages/d2b-resource-store:d2b_resource_store",
        "//packages/d2b-resource-store-redb:d2b_resource_store_redb",
        "//packages/d2b-telemetry:d2b_telemetry",
        "//packages/d2b-zone-routing:d2b_zone_routing",
    ];
    let missing = required
        .into_iter()
        .filter(|label| !labels.contains(*label))
        .collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "d2bd test carriers are missing test-support dependencies: {missing:?}"
    );
}

fn synthetic_carrier(label: &str, sources: &[&str]) -> Value {
    serde_json::json!({
        "schemaVersion": 1,
        "carriers": [{
            "id": "synthetic",
            "label": label,
            "sources": sources,
            "execution": "run"
        }]
    })
}

#[test]
fn rust_coverage_map_is_bidirectional_against_cargo_and_nextest() {
    let coverage = read_json("tests/golden/bazel/rust-coverage.json");
    let labels = validate_rust_coverage(&coverage);
    let coverage = object(&coverage, "rust coverage");
    let nextest = required_object(coverage, "nextest", "rust coverage");

    let mut main_args = vec!["--workspace".to_owned()];
    for package in MAIN_EXCLUDES {
        main_args.push("--exclude".to_owned());
        main_args.push(package.to_owned());
    }
    validate_nextest_inventory(
        coverage,
        "main",
        nextest_suite_keys(&cargo_nextest(&main_args)),
    );

    for (stream, args) in [
        (
            "broker-default",
            vec![
                "--package".to_owned(),
                "d2b-priv-broker".to_owned(),
                "--no-default-features".to_owned(),
            ],
        ),
        (
            "broker-layer1",
            vec![
                "--package".to_owned(),
                "d2b-priv-broker".to_owned(),
                "--no-default-features".to_owned(),
                "--features".to_owned(),
                "layer1-bootstrap".to_owned(),
            ],
        ),
        (
            "broker-fakebackends",
            vec![
                "--package".to_owned(),
                "d2b-priv-broker".to_owned(),
                "--no-default-features".to_owned(),
                "--features".to_owned(),
                "fake-backends".to_owned(),
            ],
        ),
        (
            "guest-real-libshpool",
            vec![
                "--package".to_owned(),
                "d2b-guest-shell-runner".to_owned(),
                "--no-default-features".to_owned(),
                "--features".to_owned(),
                "real-libshpool".to_owned(),
            ],
        ),
    ] {
        validate_nextest_inventory(coverage, stream, nextest_suite_keys(&cargo_nextest(&args)));
    }

    for stream in [
        "main",
        "broker-default",
        "broker-layer1",
        "broker-fakebackends",
        "guest-real-libshpool",
    ] {
        for value in array(nextest, stream, "rust coverage.nextest") {
            let suite = object(value, "rust coverage nextest entry");
            assert!(
                labels.contains(required_string(
                    suite,
                    "carrier",
                    "rust coverage nextest entry"
                )),
                "nextest suite carrier must be in the carrier inventory"
            );
        }
    }
}

#[test]
fn rust_carrier_labels_are_exactly_the_rust_rules_in_bazel_query() {
    let coverage = read_json("tests/golden/bazel/rust-coverage.json");
    let mapped = validate_rust_coverage(&coverage);
    let queried = query_labels(r#"kind("rust_.* rule", //...)"#);
    if mapped != queried {
        let missing = queried.difference(&mapped).cloned().collect::<Vec<_>>();
        let extra = mapped.difference(&queried).cloned().collect::<Vec<_>>();
        panic!(
            "Rust carrier inventory and Bazel's rust rule query differ; missing={missing:?}; extra={extra:?}"
        );
    }
}

#[test]
fn d2bd_test_carriers_preserve_cargo_dev_dependency_features() {
    let labels = cquery_labels("deps(//packages/d2bd:d2bd_lib_test)");
    validate_d2bd_test_support_dependencies(&labels);
}

#[test]
#[should_panic(expected = "sources must not be empty")]
fn planted_negative_empty_carrier_is_rejected() {
    validate_carrier_inventory(object(
        &synthetic_carrier("//synthetic:empty", &[]),
        "synthetic coverage",
    ));
}

#[test]
#[should_panic(expected = "source does not exist")]
fn planted_negative_missing_source_is_rejected() {
    validate_carrier_inventory(object(
        &synthetic_carrier("//synthetic:missing-source", &["does/not/exist.rs"]),
        "synthetic coverage",
    ));
}

#[test]
#[should_panic(expected = "duplicate carrier label")]
fn planted_negative_duplicate_carrier_is_rejected() {
    let value = serde_json::json!({
        "schemaVersion": 1,
        "carriers": [
            {"id": "one", "label": "//synthetic:carrier", "sources": ["Cargo.toml"], "execution": "run"},
            {"id": "two", "label": "//synthetic:carrier", "sources": ["Cargo.toml"], "execution": "run"}
        ]
    });
    validate_carrier_inventory(object(&value, "synthetic coverage"));
}

#[test]
#[should_panic(expected = "not a declared carrier")]
fn planted_negative_wrong_carrier_reference_is_rejected() {
    let value = serde_json::json!({
        "schemaVersion": 1,
        "carriers": [{"id": "one", "label": "//synthetic:carrier", "sources": ["Cargo.toml"], "execution": "run"}],
        "cargoTargets": [{
            "package": "synthetic",
            "name": "synthetic",
            "kinds": ["lib"],
            "source": "Cargo.toml",
            "carrier": "//synthetic:wrong"
        }]
    });
    let metadata = BTreeMap::from([(
        "synthetic:synthetic:lib".to_owned(),
        ("Cargo.toml".to_owned(), false, false),
    )]);
    validate_target_references(
        object(&value, "synthetic coverage"),
        &BTreeSet::from(["//synthetic:carrier".to_owned()]),
        &metadata,
    );
}

#[test]
#[should_panic(expected = "must be exclusive")]
fn planted_negative_broker_context_without_exclusivity_is_rejected() {
    let value = serde_json::json!({
        "schemaVersion": 1,
        "contexts": [{
            "id": "broker-default",
            "carrier": "//synthetic:broker",
            "sources": ["Cargo.toml"],
            "exclusive": false,
            "serialGroup": "broker-process-global",
            "aggregate": "exclusive-serialized"
        }]
    });
    validate_contexts(object(&value, "synthetic coverage"));
}

#[test]
#[should_panic(expected = "d2bd test carriers are missing test-support dependencies")]
fn planted_negative_d2bd_test_carrier_without_dev_dependency_features_is_rejected() {
    validate_d2bd_test_support_dependencies(&BTreeSet::from([
        "//packages/d2b-core:d2b_core_test_support".to_owned(),
    ]));
}

#[test]
#[should_panic(expected = "real-libshpool")]
fn planted_negative_guest_context_without_real_feature_is_rejected() {
    let value = serde_json::json!({
        "schemaVersion": 1,
        "contexts": [{
            "id": "guest-shell-runner-real-libshpool",
            "carrier": "//synthetic:guest",
            "sources": ["Cargo.toml"],
            "exclusive": false,
            "feature": "default",
            "aggregate": "dedicated"
        }]
    });
    validate_contexts(object(&value, "synthetic coverage"));
}

#[test]
#[should_panic(expected = "fixture-excluded")]
fn planted_negative_fixture_context_in_main_aggregate_is_rejected() {
    let value = serde_json::json!({
        "schemaVersion": 1,
        "contexts": [{
            "id": "fixture-contracts",
            "carrier": "//synthetic:fixtures",
            "sources": ["Cargo.toml"],
            "exclusive": false,
            "aggregate": "main-workspace"
        }]
    });
    let result =
        std::panic::catch_unwind(|| validate_contexts(object(&value, "synthetic coverage")));
    assert!(result.is_err());
    panic!("fixture-excluded");
}

#[test]
#[should_panic(expected = "UI and trybuild fixtures")]
fn planted_negative_missing_ui_fixture_is_rejected() {
    let value = serde_json::json!({
        "schemaVersion": 1,
        "ui": {"inputs": [], "carrier": "//synthetic:ui"}
    });
    validate_ui_inventory(object(&value, "synthetic coverage"));
}

#[test]
#[should_panic(expected = "nextest inventory and rust coverage.nextest.main differ")]
fn planted_negative_missing_test_is_rejected() {
    let value = serde_json::json!({
        "nextest": {"main": []}
    });
    validate_nextest_inventory(
        object(&value, "synthetic coverage"),
        "main",
        BTreeMap::from([("synthetic:test:test".to_owned(), 1)]),
    );
}

#[test]
#[should_panic(expected = "Cargo bench targets and rust coverage.benches differ")]
fn planted_negative_missing_bench_is_rejected() {
    let value = serde_json::json!({
        "harnessFree": [{
            "package": "synthetic",
            "target": "smoke",
            "manifest": "Cargo.toml",
            "source": "Cargo.toml",
            "carrier": "//synthetic:smoke"
        }],
        "benches": [{
            "package": "synthetic",
            "target": "one",
            "source": "Cargo.toml",
            "carrier": "//synthetic:bench"
        }],
        "ui": {"inputs": ["Cargo.toml"], "carrier": "//synthetic:ui"}
    });
    validate_special_inventory(
        object(&value, "synthetic coverage"),
        &BTreeSet::from(["synthetic:one".to_owned(), "synthetic:two".to_owned()]),
    );
}
