#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Mutex;

use serde_json::Value;

static BAZEL_QUERY_LOCK: Mutex<()> = Mutex::new(());

fn repo_root() -> PathBuf {
    let mut candidate = std::env::var_os("D2B_REPO_ROOT")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .expect("repository root");
    loop {
        if candidate.join("Cargo.toml").is_file()
            && candidate.join("BUILD.bazel").is_file()
            && candidate.join("flake.nix").is_file()
        {
            return candidate;
        }
        assert!(candidate.pop(), "repository root is not discoverable");
    }
}

fn read_repo_file(relative: &str) -> String {
    std::fs::read_to_string(repo_root().join(relative))
        .unwrap_or_else(|error| panic!("read {relative}: {error}"))
}

fn repo_path_exists(relative: &str) -> bool {
    repo_root().join(relative).exists()
}

fn cargo_program() -> std::ffi::OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}

fn cargo_metadata() -> Value {
    let output = Command::new(cargo_program())
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
        .expect("run cargo metadata");
    assert!(
        output.status.success(),
        "cargo metadata failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("cargo metadata JSON")
}

fn workspace_packages() -> Vec<(String, Value)> {
    let metadata = cargo_metadata();
    let root = repo_root();
    let members = metadata["workspace_members"]
        .as_array()
        .expect("workspace_members")
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    metadata["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .filter(|package| {
            package["id"]
                .as_str()
                .map(|id| members.contains(id))
                .unwrap_or(false)
        })
        .map(|package| {
            let manifest = package["manifest_path"]
                .as_str()
                .expect("package manifest path")
                .strip_prefix("file://")
                .unwrap_or_else(|| {
                    package["manifest_path"]
                        .as_str()
                        .expect("package manifest path")
                });
            (
                Path::new(manifest)
                    .parent()
                    .expect("package manifest parent")
                    .strip_prefix(&root)
                    .expect("package is under repository root")
                    .display()
                    .to_string(),
                package.clone(),
            )
        })
        .collect()
}

fn bazel_binary() -> PathBuf {
    std::env::var_os("D2B_BAZEL_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("bazel"))
}

fn run_bazel_query(expression: &str) -> Output {
    let _guard = BAZEL_QUERY_LOCK.lock().expect("Bazel query lock");
    let output_root = std::env::var_os("TEST_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join(".scratch/bazel-rust-parity"));
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    expression.hash(&mut hasher);
    let output_root = output_root.join(format!("{:x}", hasher.finish()));
    Command::new(bazel_binary())
        .arg(format!("--output_user_root={}", output_root.display()))
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
        .expect("run Bazel query")
}

fn bazel_labels(expression: &str) -> BTreeSet<String> {
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

fn target_names(package: &Value, kind: &str) -> BTreeSet<String> {
    package["targets"]
        .as_array()
        .expect("package targets")
        .iter()
        .filter(|target| {
            target["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|value| value.as_str() == Some(kind)))
        })
        .filter_map(|target| target["name"].as_str().map(str::to_owned))
        .collect()
}

fn package_build(package: &str) -> String {
    read_repo_file(&format!("{package}/BUILD.bazel"))
}

#[test]
fn cargo_packages_have_explicit_bazel_ownership() {
    for (package, metadata) in workspace_packages() {
        assert!(
            repo_path_exists(&format!("{package}/BUILD.bazel")),
            "Cargo workspace package has no BUILD ownership: {package}"
        );
        let build = package_build(&package);
        if !target_names(&metadata, "lib").is_empty() {
            assert!(
                build.contains("rust_library("),
                "library package has no explicit rust_library target: {package}"
            );
        }
        if !target_names(&metadata, "bin").is_empty() {
            assert!(
                build.contains("rust_binary("),
                "binary package has no explicit rust_binary target: {package}"
            );
        }
    }
}

#[test]
fn cargo_target_names_have_explicit_bazel_rules() {
    for (package, metadata) in workspace_packages() {
        let build = package_build(&package);
        for name in target_names(&metadata, "lib") {
            assert!(
                build.contains(&format!("name = \"{name}\"")) && build.contains("rust_library("),
                "Cargo library target {package}:{name} has no explicit Bazel rust_library rule"
            );
        }
        for name in target_names(&metadata, "bin") {
            assert!(
                build.contains(&format!("name = \"{name}\"")) && build.contains("rust_binary("),
                "Cargo binary target {package}:{name} has no explicit Bazel rust_binary rule"
            );
        }
        let cargo_tests = target_names(&metadata, "test");
        let compile_only_tests = cargo_tests
            .iter()
            .filter(|name| {
                build.contains(&format!("name = \"{name}\""))
                    && build.contains(&format!("rust_library(\n    name = \"{name}\""))
            })
            .count();
        assert!(
            build.matches("rust_test(").count() + compile_only_tests >= cargo_tests.len(),
            "Bazel test inventory is smaller than Cargo's for {package}: Bazel={} Cargo={}",
            build.matches("rust_test(").count() + compile_only_tests,
            cargo_tests.len()
        );
    }
}

#[test]
fn first_party_builds_use_rules_rs_helpers_and_source_globs() {
    for (package, metadata) in workspace_packages() {
        let build = package_build(&package);
        assert!(
            build.contains("all_crate_deps("),
            "first-party package must derive third-party deps from rules_rs: {package}"
        );
        if !target_names(&metadata, "lib").is_empty() {
            assert!(
                build.contains("src/**/*.rs"),
                "library package must use a standard source glob: {package}"
            );
        }
        assert!(
            !build.contains("# gazelle:"),
            "first-party BUILD must not retain Gazelle directives: {package}"
        );
    }
}

#[test]
fn doctest_and_harness_free_companions_are_native_bazel_targets() {
    let doctest_labels = bazel_labels(r#"kind("rust_doc_test rule", //bazel/checks/rust:*)"#);
    let harness_free = workspace_packages()
        .into_iter()
        .map(|(package, _)| package_build(&package))
        .filter(|build| build.contains("use_libtest_harness = False"))
        .count();
    let doctested_crates = workspace_packages()
        .into_iter()
        .filter(|(_, package)| {
            package["targets"].as_array().is_some_and(|targets| {
                targets.iter().any(|target| {
                    target["kind"]
                        .as_array()
                        .is_some_and(|kinds| kinds.iter().any(|kind| kind == "lib"))
                        && target["doctest"].as_bool() == Some(true)
                })
            })
        })
        .count();
    assert!(
        doctest_labels.len() >= doctested_crates,
        "Bazel doctest targets are missing: Bazel={} Cargo={doctested_crates}",
        doctest_labels.len()
    );
    assert!(
        harness_free > 0,
        "no harness-free Bazel companion target exists"
    );
}

#[test]
fn feature_contexts_are_explicit_and_define_free() {
    let rust_build = read_repo_file("bazel/checks/rust/BUILD.bazel");
    assert!(
        !rust_build.contains("define_values"),
        "feature contexts must not depend on global Bazel --define selectors"
    );
    let broker = read_repo_file("packages/d2b-priv-broker/BUILD.bazel");
    assert!(
        broker.contains("name = \"d2b_priv_broker_test\""),
        "broker default feature context is missing from explicit Bazel targets"
    );
    for feature in ["layer1-bootstrap", "fake-backends"] {
        assert!(
            broker.contains(feature),
            "broker feature context is missing from explicit Bazel targets: {feature}"
        );
    }
    let guest = read_repo_file("packages/d2b-guest-shell-runner/BUILD.bazel");
    assert!(
        guest.contains("real-libshpool"),
        "guest real-libshpool context is missing from explicit Bazel targets"
    );
}

#[test]
fn root_cargo_authority_has_no_bazel_only_workspace() {
    assert!(!repo_path_exists("bazel/checks/rust/Cargo.toml"));
    assert!(!repo_path_exists("bazel/checks/rust/Cargo.lock"));
    assert!(!repo_path_exists("tests/golden/bazel/rust-coverage.json"));
    let module = read_repo_file("MODULE.bazel");
    assert_eq!(module.matches("cargo_lock = \"//:Cargo.lock\"").count(), 1);
    assert!(module.contains("cargo_toml = \"//:Cargo.toml\""));
}
