#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

const CONTRACTS_CRATE: &str = "d2b-contracts";
const EXCLUDED_WORKSPACES: &[&str] = &["d2b-priv-broker", "d2b-guest-shell-runner"];
const API_SURFACE_CRATE: &str = "packages/d2b-api-surface";
const RUST_DRIVER: &str = "tests/test-rust.sh";
const RUST_DAG_LEAVES: &[&str] = &[
    "test-rust-leaf-api-surface",
    "test-rust-leaf-main-workspace",
    "test-rust-leaf-broker",
    "test-rust-leaf-guest-shell-runner",
    "test-rust-leaf-no-bash-ast",
    "test-rust-leaf-supply-chain",
];
const RUST_SHARED_TARGET_EDGES: &[(&str, &str)] = &[];
const RUST_LEAF_MODES: &[&str] = &[
    "api-surface",
    "main-workspace",
    "broker",
    "guest-shell-runner",
    "no-bash-ast",
    "schema-reproducibility",
    "supply-chain",
    "inventory-stub",
    "fixture-contracts",
];
const RUST_BASELINE_LEAF_IDS: &[&str] = &[
    "rust-api-surface",
    "rust-main-format",
    "rust-main-clippy",
    "rust-main-workspace-tests",
    "rust-contract-tests",
    "rust-cli-contract-tests",
    "rust-no-bash-ast",
    "rust-broker-default",
    "rust-broker-layer1",
    "rust-broker-fakebackends",
    "rust-guest-shell-runner",
    "rust-schema-reproducibility",
    "rust-deny-main",
    "rust-deny-broker",
    "rust-deny-guest",
    "rust-audit-main",
    "rust-audit-broker",
    "rust-audit-guest",
    "rust-stub-no-socket",
    "rust-assert-pinned",
];
const BROKER_FEATURE_PASSES: &[&str] = &["default", "layer1-bootstrap", "fake-backends"];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("xtask lives under packages/xtask")
        .to_path_buf()
}

fn read_repo_file(rel: &str) -> String {
    std::fs::read_to_string(repo_root().join(rel)).expect("read repo file")
}

fn git_tracked_files() -> Vec<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root())
        .args(["ls-files", "-z"])
        .output()
        .expect("run git ls-files");
    assert!(
        output.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| String::from_utf8(entry.to_vec()).expect("tracked paths are UTF-8"))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[test]
fn workspace_names_contract_crate_by_role() {
    let workspace = read_repo_file("packages/Cargo.toml");
    assert!(
        workspace.contains(&format!("\"{CONTRACTS_CRATE}\"")),
        "main workspace must include the contract/DTO crate by role"
    );
    assert!(
        !workspace.contains(&format!("\"{}{}\"", "d2b", "-ipc")),
        "main workspace must not reintroduce the old transport-shaped contract crate name"
    );

    let manifest = read_repo_file("packages/d2b-contracts/Cargo.toml");
    assert!(
        manifest.contains(&format!("name = \"{CONTRACTS_CRATE}\"")),
        "contract crate manifest must use the role-based package name"
    );
}

#[test]
fn stale_ipc_crate_name_is_absent_from_current_sources() {
    let old_hyphen = format!("{}{}", "d2b", "-ipc");
    let old_underscore = format!("{}{}", "d2b", "_ipc");
    let self_path = "packages/xtask/tests/policy_workspace.rs";
    let mut violations = Vec::new();
    for rel in git_tracked_files() {
        if rel == self_path {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(repo_root().join(&rel)) else {
            continue;
        };
        if content.contains(&old_hyphen) || content.contains(&old_underscore) {
            violations.push(rel);
        }
    }
    assert!(
        violations.is_empty(),
        "stale contract crate references remain:\n{}",
        violations.join("\n")
    );
}

fn assert_fast_dev_profile(manifest: &str, workspace: &str) {
    for required in [
        "[profile.dev]",
        "[profile.dev.package.\"*\"]",
        "[profile.test]",
        "[profile.test.package.\"*\"]",
        "[profile.debugging]",
        "inherits = \"dev\"",
        "debug = \"line-tables-only\"",
        "debug = false",
        "debug = 2",
    ] {
        assert!(
            manifest.contains(required),
            "{workspace} must keep the measured fast-development profile and full-debug escape hatch: missing {required}"
        );
    }
}

#[test]
fn every_tested_workspace_uses_fast_debug_profiles() {
    assert_fast_dev_profile(&read_repo_file("packages/Cargo.toml"), "main workspace");
    for workspace in EXCLUDED_WORKSPACES {
        assert_fast_dev_profile(
            &read_repo_file(&format!("packages/{workspace}/Cargo.toml")),
            workspace,
        );
    }
}

#[test]
fn compiler_derived_api_surface_is_pinned_and_enforcing() {
    let root = repo_root();
    let workspace = read_repo_file("packages/Cargo.toml");
    let driver = read_repo_file("tests/test-rust.sh");
    let api_driver = read_repo_file("tests/tools/api-surface-json.sh");
    let policy_manifest = read_repo_file(&format!("{API_SURFACE_CRATE}/Cargo.toml"));
    let toolchain = read_repo_file(&format!("{API_SURFACE_CRATE}/rust-toolchain.toml"));

    assert!(workspace.contains("\"d2b-api-surface\""));
    assert!(driver.contains("tests/tools/api-surface-json.sh"));
    assert!(api_driver.contains("--document-private-items --document-hidden-items"));
    assert!(api_driver.contains("--workspace --lib --no-deps"));
    assert!(policy_manifest.contains("public-api = { version = \"=0.52.0\""));
    assert!(policy_manifest.contains("rustdoc-types = \"=0.57.4\""));
    assert!(toolchain.contains("nightly-2026-02-16"));
    for required in [
        "workspace-metadata.json",
        "roots.json",
        "public-api.txt",
        "capability-api.txt",
        "hidden-public-api.txt",
        "capability-trait-impls.txt",
    ] {
        assert!(
            root.join("tests/golden/api-surface")
                .join(required)
                .is_file()
        );
    }
}

#[test]
fn excluded_workspaces_keep_own_lock_and_supply_chain_policy() {
    let root = repo_root();
    let main_workspace = read_repo_file("packages/Cargo.toml");
    let flake = read_repo_file("flake.nix");
    let driver = read_repo_file(RUST_DRIVER);
    let violations = excluded_workspace_violations(&main_workspace, &flake, &driver);
    assert!(
        violations.is_empty(),
        "excluded Rust workspaces must retain independent lock, deny, and driver policy:\n{}",
        violations.join("\n")
    );
    for workspace in EXCLUDED_WORKSPACES {
        assert!(
            main_workspace.contains(&format!("\"{workspace}\"")),
            "main workspace exclude list must mention {workspace}"
        );
        for required in ["Cargo.toml", "Cargo.lock", "deny.toml"] {
            let path = root.join("packages").join(workspace).join(required);
            assert!(path.exists(), "{} must exist", path.display());
        }
        assert!(
            flake.contains(&format!("packages/{workspace}/Cargo.lock"))
                && flake.contains(&format!("packages/{workspace}/deny.toml")),
            "flake supply-chain gates must cover {workspace}"
        );
    }
}

fn non_comment_lines(source: &str) -> Vec<&str> {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect()
}

fn discovered_package_manifests() -> Vec<(String, String)> {
    let mut manifests = git_tracked_files()
        .into_iter()
        .filter(|rel| {
            rel.starts_with("packages/")
                && rel.ends_with("/Cargo.toml")
                && rel != "packages/Cargo.toml"
        })
        .map(|rel| {
            let content = std::fs::read_to_string(repo_root().join(&rel)).unwrap_or_else(|error| {
                panic!("failed to read tracked package manifest {rel}: {error}")
            });
            (rel, content)
        })
        .collect::<Vec<_>>();
    manifests.sort_by(|left, right| left.0.cmp(&right.0));
    manifests
}

fn quoted_key(block: &str, key: &str) -> Option<String> {
    let prefix = format!("{key} = \"");
    block.lines().find_map(|line| {
        let value = line.trim().strip_prefix(&prefix)?;
        Some(value.strip_suffix('"').unwrap_or(value).to_owned())
    })
}

fn discovered_harness_false_targets(manifests: &[(String, String)]) -> Vec<String> {
    let mut targets = Vec::new();
    for (rel, content) in manifests {
        for block in content.split("[[test]]").skip(1) {
            let block = block.split("[[").next().unwrap_or(block);
            if !block.lines().any(|line| line.trim() == "harness = false") {
                continue;
            }
            let name = quoted_key(block, "name").unwrap_or_else(|| {
                panic!("harness=false test target in {rel} is missing its name")
            });
            targets.push(format!("{rel}:{name}"));
        }
    }
    targets.sort();
    targets
}

fn discovered_broker_features(manifest: &str) -> BTreeSet<String> {
    let Some(features) = manifest.split("[features]").nth(1) else {
        return BTreeSet::new();
    };
    let features = features.split("\n[").next().unwrap_or(features);
    features
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (name, _) = line.split_once('=')?;
            Some(name.trim().to_owned())
        })
        .collect()
}

fn discovered_workspace_excludes(manifest: &str) -> BTreeSet<String> {
    let Some(exclude) = manifest.split("exclude = [").nth(1) else {
        return BTreeSet::new();
    };
    let exclude = exclude.split(']').next().unwrap_or(exclude);
    exclude
        .split('"')
        .enumerate()
        .filter_map(|(index, value)| (index % 2 == 1).then_some(value.to_owned()))
        .collect()
}

fn make_rule_prerequisites(source: &str, target: &str) -> Option<BTreeSet<String>> {
    let prefix = format!("{target}:");
    for (index, line) in source.lines().enumerate() {
        if !line.starts_with(&prefix) {
            continue;
        }
        let mut rhs = line[prefix.len()..].trim_end().to_owned();
        let mut next = index + 1;
        while rhs.trim_end().ends_with('\\') {
            rhs = rhs.trim_end_matches('\\').trim_end().to_owned();
            let continuation = source.lines().nth(next)?;
            rhs.push(' ');
            rhs.push_str(continuation.trim());
            next += 1;
        }
        return Some(
            rhs.split_whitespace()
                .filter(|item| *item != "\\")
                .map(str::to_owned)
                .collect(),
        );
    }
    None
}

fn discovered_make_targets(source: &str) -> BTreeSet<String> {
    source
        .lines()
        .filter(|line| !line.chars().next().is_some_and(char::is_whitespace))
        .filter_map(|line| line.split_once(':').map(|(lhs, _)| lhs))
        .flat_map(|lhs| lhs.split_whitespace())
        .filter(|target| !target.starts_with('.') && !target.contains('='))
        .map(str::to_owned)
        .collect()
}

fn rust_dag_violations(makefile: &str) -> Vec<String> {
    let mut violations = Vec::new();
    let Some(root_prerequisites) = make_rule_prerequisites(makefile, "test-rust") else {
        return vec!["test-rust has no explicit Rust orchestration rule".to_owned()];
    };

    for leaf in RUST_DAG_LEAVES {
        if !root_prerequisites.contains(*leaf) {
            violations.push(format!("test-rust must schedule the Rust leaf `{leaf}`"));
        }
        if make_rule_prerequisites(makefile, leaf).is_none() {
            violations.push(format!("Rust leaf `{leaf}` has no Make rule"));
        }
    }

    for (parent, child) in RUST_SHARED_TARGET_EDGES {
        let Some(prerequisites) = make_rule_prerequisites(makefile, parent) else {
            violations.push(format!("shared-target parent `{parent}` has no Make rule"));
            continue;
        };
        if !prerequisites.contains(*child) {
            violations.push(format!(
                "shared-target edge `{parent} -> {child}` is missing"
            ));
        }
    }
    for required in [
        "D2B_RUST_MAIN_PREREQS_aggregate := test-rust-leaf-schema",
        "D2B_RUST_MAIN_PREREQS_cold :=",
        "D2B_RUST_MAIN_PREREQS_main :=",
        "test-rust-leaf-main-workspace: $(D2B_RUST_MAIN_PREREQS)",
        "D2B_RUST_SCHEMA_PREREQS_aggregate := test-rust-leaf-inventory",
        "D2B_RUST_SCHEMA_PREREQS_cold :=",
        "D2B_RUST_SCHEMA_PREREQS_schema :=",
        "test-rust-leaf-schema: $(D2B_RUST_SCHEMA_PREREQS)",
        "D2B_RUST_BROKER_PREREQS_aggregate := test-rust-leaf-inventory",
        "D2B_RUST_BROKER_PREREQS_cold :=",
        "D2B_RUST_BROKER_PREREQS_broker :=",
        "test-rust-leaf-broker: $(D2B_RUST_BROKER_PREREQS)",
    ] {
        if !makefile.contains(required) {
            violations.push(format!(
                "profile-aware main-workspace dependency contract is missing `{required}`"
            ));
        }
    }
    violations
}

fn empty_harness_discovery_guard_present(driver: &str) -> bool {
    let lines = non_comment_lines(driver);
    for (index, line) in lines.iter().enumerate() {
        let checks_empty_targets = line.contains("-z") && line.contains("targets");
        let checks_empty_count = line.contains("ran")
            && (line.contains("-eq 0") || line.contains("== 0") || line.contains("= 0"));
        if !(checks_empty_targets || checks_empty_count) {
            continue;
        }
        let window = lines
            .iter()
            .skip(index)
            .take(5)
            .copied()
            .collect::<Vec<_>>();
        if window.iter().any(|candidate| {
            candidate.contains("fail") || candidate.contains("exit") || candidate.contains("return")
        }) {
            return true;
        }
    }
    false
}

fn rust_companion_violations(driver: &str, harness_targets: &[String]) -> Vec<String> {
    let mut violations = Vec::new();
    if harness_targets.is_empty() {
        violations.push("harness=false discovery returned no governed targets".to_owned());
    }
    if !driver.contains("cargo test --doc") {
        violations.push("Rust doctests are not explicitly retained".to_owned());
    }
    for required in [
        "cargo nextest list",
        "kind == \"test\"",
        "testcases | length",
        "cargo test",
        "--test",
    ] {
        if !driver.contains(required) {
            violations.push(format!(
                "harness=false discovery is missing required input `{required}`"
            ));
        }
    }
    if !empty_harness_discovery_guard_present(driver) {
        violations.push(
            "harness=false discovery must fail explicitly when its result is empty".to_owned(),
        );
    }
    if non_comment_lines(driver).iter().any(|line| {
        line.contains("cargo test") && line.contains("--test") && line.contains("--test-threads")
    }) {
        violations.push("harness=false binaries must not receive libtest arguments".to_owned());
    }
    violations
}

fn ordered_contains(source: &str, needles: &[&str]) -> bool {
    let mut rest = source;
    for needle in needles {
        let Some(index) = rest.find(needle) else {
            return false;
        };
        rest = &rest[index + needle.len()..];
    }
    true
}

fn has_background_job(line: &str) -> bool {
    line.split_whitespace()
        .any(|word| word == "&" || word.ends_with('&'))
}

fn broker_serial_violations(driver: &str, broker_features: &BTreeSet<String>) -> Vec<String> {
    let mut violations = Vec::new();
    for feature in BROKER_FEATURE_PASSES {
        if !broker_features.contains(*feature) {
            violations.push(format!("broker feature `{feature}` is not governed"));
        }
    }
    for required in [
        "broker_stream_default",
        "broker_stream_layer1",
        "broker_stream_fakebackends",
        "layer1-bootstrap",
        "fake-backends",
    ] {
        if !driver.contains(required) {
            violations.push(format!("broker serial chain is missing `{required}`"));
        }
    }

    let Some(start) = driver
        .find("for _stream in")
        .or_else(|| driver.find("broker_stream_default()"))
    else {
        violations.push("broker feature passes have no bounded execution block".to_owned());
        return violations;
    };
    let end = driver[start..]
        .find("\ncleanup_cargo_special_files")
        .map(|offset| start + offset)
        .unwrap_or(driver.len());
    let block = &driver[start..end];
    let code = non_comment_lines(block);
    if !ordered_contains(
        driver,
        &[
            "broker_stream_default",
            "broker_stream_layer1",
            "broker_stream_fakebackends",
        ],
    ) {
        violations.push("broker feature passes are not ordered as one chain".to_owned());
    }
    if !code
        .iter()
        .any(|line| line.contains("for ") && line.contains("broker_stream"))
    {
        violations.push("broker feature passes have no serial dispatch loop".to_owned());
    }
    if code.iter().any(|line| has_background_job(line)) {
        violations.push("broker feature passes must not be backgrounded".to_owned());
    }
    violations
}

fn runtime_frontier_quota_violations(makefile: &str) -> Vec<String> {
    let code = non_comment_lines(makefile);
    let mut violations = Vec::new();
    for required in [
        "D2B_RUST_BUDGET",
        "--jobs",
        "--test-threads",
        "frontier",
        "quota",
    ] {
        if !code.iter().any(|line| line.contains(required)) {
            violations.push(format!("runtime quota contract is missing `{required}`"));
        }
    }
    if !code
        .iter()
        .any(|line| line.contains("active") && line.contains("lane"))
    {
        violations.push("runtime quota contract has no active-lane bound".to_owned());
    }
    if !code.iter().any(|line| {
        line.contains("-lt 1")
            || line.contains("< 1")
            || (line.contains("positive") && line.contains("budget"))
    }) {
        violations.push("runtime quota contract does not reject budgets below one".to_owned());
    }
    if !code
        .iter()
        .any(|line| line.contains("-eq 1") || (line.contains("budget") && line.contains("= 1")))
    {
        violations.push("runtime quota contract has no explicit budget-one path".to_owned());
    }
    if !code.iter().any(|line| {
        let bounded = line.contains("-le") || line.contains("<=") || line.contains("-gt");
        bounded && line.contains("frontier") && line.contains("budget")
    }) {
        violations.push("runtime frontier quota is not checked against the budget".to_owned());
    }
    violations
}

fn rust_manifest_policy_violations(makefile: &str, driver: &str) -> Vec<String> {
    let mut violations = Vec::new();
    for leaf in RUST_BASELINE_LEAF_IDS {
        if !driver.contains(leaf) {
            violations.push(format!(
                "Rust execution manifest is missing baseline leaf `{leaf}`"
            ));
        }
    }
    if driver.contains(r#"--leaf "$rust_mode""#) {
        violations.push("Rust execution manifest emits a coarse driver mode".to_owned());
    }
    let emitter = driver
        .split("publish_manifest_fragment()")
        .nth(1)
        .and_then(|region| region.split("rust_surface_start()").next());
    match emitter {
        Some(region) if region.contains(">/dev/null") || region.contains("|| true") => {
            violations.push("Rust manifest fragment publication suppresses errors".to_owned());
        }
        None => violations.push("Rust manifest fragment emitter is missing".to_owned()),
        Some(_) => {}
    }
    if !makefile.contains("D2B_SKIP_FIXTURE_BUILD") {
        violations.push("Rust aggregate lost its conditional fixture behavior".to_owned());
    }
    violations
}

fn rust_profile_violations(makefile: &str, driver: &str, api_driver: &str) -> Vec<String> {
    let mut violations = Vec::new();
    for profile in [
        "aggregate",
        "api",
        "main",
        "broker",
        "guest",
        "no-bash",
        "schema",
        "inventory",
        "supply",
    ] {
        if !makefile.contains(&format!(",{profile})")) {
            violations.push(format!(
                "Rust Make target does not select `{profile}` profile"
            ));
        }
    }
    for required in [
        "profile=cold",
        "D2B_RUST_COLD_PROFILE=$$cold_profile",
        "D2B_RUST_QUOTA_MAIN=$$quota_main",
        "D2B_RUST_QUOTA_BROKER=$$quota_broker",
    ] {
        if !makefile.contains(required) {
            violations.push(format!("Rust profile contract is missing `{required}`"));
        }
    }
    if !driver.contains("${D2B_RUST_COLD_PROFILE:-0}")
        || !driver.contains("fixture_target_dir=\"$workspace_target_dir\"")
    {
        violations.push("fixture target does not restore the shared CI/cold target".to_owned());
    }
    if !api_driver.contains("${D2B_RUST_COLD_PROFILE:-0}")
        || !api_driver.contains("shared_census=1")
        || !api_driver.contains("public_target=\"$target_root/census\"")
    {
        violations.push("API census does not restore the shared CI/cold target".to_owned());
    }
    violations
}

fn rust_mode_violations(driver: &str) -> Vec<String> {
    let code = non_comment_lines(driver);
    let mut violations = Vec::new();
    for mode in RUST_LEAF_MODES {
        if !code.iter().any(|line| line.contains(mode)) {
            violations.push(format!("test-rust.sh has no leaf mode `{mode}`"));
        }
    }
    if code
        .iter()
        .any(|line| line.trim_start().starts_with("all)") || line.contains("${1:-all}"))
    {
        violations.push("test-rust.sh still exposes the removed `all` scheduler".to_owned());
    }
    if code
        .iter()
        .any(|line| line.trim_start().starts_with("remaining-suite)"))
    {
        violations
            .push("test-rust.sh still exposes the aggregate `remaining-suite` mode".to_owned());
    }
    let actionable_no_arg_rejection = code.iter().enumerate().any(|(index, line)| {
        if !line.contains("make test-rust") {
            return false;
        }
        code.iter()
            .skip(index)
            .take(5)
            .any(|candidate| candidate.contains("fail") || candidate.contains("exit 2"))
    });
    if !actionable_no_arg_rejection {
        violations.push(
            "the no-argument test-rust.sh rejection must direct callers to `make test-rust`"
                .to_owned(),
        );
    }
    violations
}

fn excluded_workspace_violations(main_workspace: &str, flake: &str, driver: &str) -> Vec<String> {
    let discovered = discovered_workspace_excludes(main_workspace);
    let expected = EXCLUDED_WORKSPACES
        .iter()
        .copied()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let mut violations = Vec::new();
    if discovered.is_empty() {
        violations.push("main Cargo workspace exclusion discovery is empty".to_owned());
    } else if discovered != expected {
        violations.push(format!(
            "main Cargo workspace exclusions changed: expected {expected:?}, found {discovered:?}"
        ));
    }
    for workspace in EXCLUDED_WORKSPACES {
        for suffix in ["Cargo.toml", "Cargo.lock", "deny.toml"] {
            let rel = format!("packages/{workspace}/{suffix}");
            if !driver.contains(&rel) {
                violations.push(format!(
                    "test-rust.sh no longer governs excluded workspace input `{rel}`"
                ));
            }
        }
        for suffix in ["Cargo.lock", "deny.toml"] {
            let rel = format!("packages/{workspace}/{suffix}");
            if !flake.contains(&rel) {
                violations.push(format!(
                    "flake supply-chain policy no longer covers excluded workspace input `{rel}`"
                ));
            }
        }
    }
    violations
}

#[test]
fn rust_companion_surfaces_are_retained_and_fail_closed() {
    let manifests = discovered_package_manifests();
    assert!(
        !manifests.is_empty(),
        "Rust policy discovery must find package Cargo manifests"
    );
    let harness_targets = discovered_harness_false_targets(&manifests);
    assert!(
        !harness_targets.is_empty(),
        "Rust policy discovery must find at least one harness=false test target"
    );

    let driver = read_repo_file(RUST_DRIVER);
    let violations = rust_companion_violations(&driver, &harness_targets);
    assert!(
        violations.is_empty(),
        "Rust must retain doctests and every discovered harness=false binary:\n{}",
        violations.join("\n")
    );
    for workspace in ["main workspace", "guest shell runner"] {
        assert!(
            driver.contains(&format!("\"{workspace}\"")),
            "Rust companion execution must retain doctest and harness=false coverage for {workspace}"
        );
    }
}

#[test]
fn rust_companion_policy_rejects_mutated_or_empty_discovery_fixtures() {
    let targets = vec!["packages/example/Cargo.toml:smoke".to_owned()];
    let good = r#"
run_companions() {
  cargo test --doc
  listing=$(cargo nextest list --message-format json)
  jq -r '.["rust-suites"][] | select(.kind == "test") | select((.testcases | length) == 0)'
  cargo test --test smoke
  targets="discovered"
  if [ -z "$targets" ]; then
    fail "empty harness-free discovery"
    exit 1
  fi
}
"#;
    assert!(
        rust_companion_violations(good, &targets).is_empty(),
        "the positive companion fixture must satisfy the policy"
    );

    for (needle, label) in [
        ("cargo test --doc", "doctest"),
        ("cargo nextest list", "harness discovery"),
        (
            r#"if [ -z "$targets" ]; then
    fail "empty harness-free discovery"
    exit 1
  fi"#,
            "empty-discovery failure",
        ),
    ] {
        let mutated = good.replacen(needle, "", 1);
        let violations = rust_companion_violations(&mutated, &targets);
        assert!(
            !violations.is_empty(),
            "removing the {label} contract must be rejected"
        );
    }
    let mutated = good.replace(
        "cargo test --test smoke",
        "cargo test --test smoke -- --test-threads 4",
    );
    let violations = rust_companion_violations(&mutated, &targets);
    assert!(
        violations
            .iter()
            .any(|violation| violation.contains("libtest arguments")),
        "passing libtest arguments to a harness-free binary must be rejected"
    );
}

#[test]
fn broker_feature_passes_remain_serial() {
    let manifest = read_repo_file("packages/d2b-priv-broker/Cargo.toml");
    let features = discovered_broker_features(&manifest);
    assert!(
        !features.is_empty(),
        "broker policy discovery must find a non-empty feature table"
    );
    let driver = read_repo_file(RUST_DRIVER);
    let violations = broker_serial_violations(&driver, &features);
    assert!(
        violations.is_empty(),
        "broker default, layer1-bootstrap, and fake-backends passes must remain serial:\n{}",
        violations.join("\n")
    );
}

#[test]
fn broker_serial_policy_rejects_a_backgrounded_feature_fixture() {
    let features = BROKER_FEATURE_PASSES
        .iter()
        .copied()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let good = r#"
broker_stream_default() { cargo test; }
broker_stream_layer1() { cargo test --features layer1-bootstrap; }
broker_stream_fakebackends() { cargo test --features fake-backends; }
broker_streams=(default layer1 fakebackends)
for _stream in "${broker_streams[@]}"; do
  "broker_stream_$_stream"
done
guest_shell_runner_gate() { :; }
"#;
    assert!(
        broker_serial_violations(good, &features).is_empty(),
        "the positive serial broker fixture must satisfy the policy"
    );
    let mutated = good.replace(
        r#""broker_stream_$_stream""#,
        r#""broker_stream_$_stream" &"#,
    );
    let violations = broker_serial_violations(&mutated, &features);
    assert!(
        violations
            .iter()
            .any(|violation| violation.contains("backgrounded")),
        "a backgrounded broker feature pass must be rejected: {violations:?}"
    );
}

#[test]
fn rust_dag_orders_leaves_that_share_the_cargo_target_directory() {
    let makefile = read_repo_file("Makefile");
    let targets = discovered_make_targets(&makefile);
    assert!(
        !targets.is_empty(),
        "Rust policy discovery must find Make targets"
    );
    let violations = rust_dag_violations(&makefile);
    assert!(
        violations.is_empty(),
        "Rust Make DAG leaves and shared-target dependency edges are incomplete:\n{}",
        violations.join("\n")
    );
}

#[test]
fn rust_dag_policy_rejects_a_missing_shared_target_edge_fixture() {
    let good = r#"
test-rust: test-rust-leaf-api-surface test-rust-leaf-main-workspace test-rust-leaf-broker test-rust-leaf-guest-shell-runner test-rust-leaf-no-bash-ast test-rust-leaf-supply-chain
test-rust-leaf-api-surface:
D2B_RUST_MAIN_PREREQS_aggregate := test-rust-leaf-schema
D2B_RUST_MAIN_PREREQS_cold :=
D2B_RUST_MAIN_PREREQS_main :=
test-rust-leaf-main-workspace: $(D2B_RUST_MAIN_PREREQS)
D2B_RUST_BROKER_PREREQS_aggregate := test-rust-leaf-inventory
D2B_RUST_BROKER_PREREQS_cold :=
D2B_RUST_BROKER_PREREQS_broker :=
test-rust-leaf-broker: $(D2B_RUST_BROKER_PREREQS)
test-rust-leaf-guest-shell-runner:
test-rust-leaf-no-bash-ast:
test-rust-leaf-supply-chain:
D2B_RUST_SCHEMA_PREREQS_aggregate := test-rust-leaf-inventory
D2B_RUST_SCHEMA_PREREQS_cold :=
D2B_RUST_SCHEMA_PREREQS_schema :=
test-rust-leaf-schema: $(D2B_RUST_SCHEMA_PREREQS)
test-rust-leaf-inventory:
"#;
    assert!(
        rust_dag_violations(good).is_empty(),
        "the positive Rust DAG fixture must satisfy the policy"
    );
    let mutated = good.replace(
        "D2B_RUST_MAIN_PREREQS_aggregate := test-rust-leaf-schema",
        "D2B_RUST_MAIN_PREREQS_aggregate :=",
    );
    let violations = rust_dag_violations(&mutated);
    assert!(
        violations
            .iter()
            .any(|violation| violation.contains("profile-aware main-workspace")),
        "removing the aggregate-only dependency edge must be rejected: {violations:?}"
    );
}

#[test]
fn rust_runtime_frontier_quota_is_bounded_for_constrained_budgets() {
    let makefile = read_repo_file("Makefile");
    let driver = read_repo_file(RUST_DRIVER);
    let governed_source = format!("{makefile}\n{driver}");
    let violations = runtime_frontier_quota_violations(&governed_source);
    assert!(
        violations.is_empty(),
        "Rust runtime quotas must bound every frontier, including budget 1:\n{}",
        violations.join("\n")
    );
}

#[test]
fn rust_runtime_quota_policy_rejects_an_unbounded_frontier_fixture() {
    let good = r#"
D2B_RUST_BUDGET ?= 4
rust-runtime-quota:
	@budget="$(D2B_RUST_BUDGET)"; \
	active_lanes=1; \
	if [ "$$budget" -lt 1 ]; then exit 2; fi; \
	if [ "$$budget" -eq 1 ]; then active_lanes=1; fi; \
	frontier_quota=$$(printf '%s' "$$budget"); \
	test "$$frontier_quota" -le "$$budget"; \
	cargo test --jobs "$$frontier_quota" --test-threads "$$frontier_quota"
"#;
    assert!(
        runtime_frontier_quota_violations(good).is_empty(),
        "the positive runtime-quota fixture must satisfy the policy"
    );
    let mutated = good.replace(r#" -le "$$budget""#, r#" "$$budget""#);
    let violations = runtime_frontier_quota_violations(&mutated);
    assert!(
        violations
            .iter()
            .any(|violation| violation.contains("frontier quota")),
        "removing the frontier bound must be rejected: {violations:?}"
    );
}

#[test]
fn rust_execution_manifest_policy_is_fail_closed() {
    let makefile = read_repo_file("Makefile");
    let driver = read_repo_file(RUST_DRIVER);
    let violations = rust_manifest_policy_violations(&makefile, &driver);
    assert!(
        violations.is_empty(),
        "Rust execution-manifest policy drifted:\n{}",
        violations.join("\n")
    );
}

#[test]
fn rust_execution_manifest_policy_rejects_negative_mutations() {
    let makefile = "D2B_SKIP_FIXTURE_BUILD=1";
    let driver = format!(
        "publish_manifest_fragment() {{ perl helper; }}\n\
         rust_surface_start() {{ :; }}\n{}",
        RUST_BASELINE_LEAF_IDS.join("\n")
    );
    assert!(
        rust_manifest_policy_violations(makefile, &driver).is_empty(),
        "the positive execution-manifest policy fixture must pass"
    );

    let mutated = driver
        .replacen(RUST_BASELINE_LEAF_IDS[0], "", 1)
        .replace("perl helper;", "perl helper >/dev/null || true;")
        + "\n--leaf \"$rust_mode\"";
    let violations = rust_manifest_policy_violations("", &mutated);
    for expected in [
        "missing baseline leaf",
        "coarse driver mode",
        "suppresses errors",
        "conditional fixture behavior",
    ] {
        assert!(
            violations
                .iter()
                .any(|violation| violation.contains(expected)),
            "negative execution-manifest fixture did not reject {expected}: {violations:?}"
        );
    }
}

#[test]
fn rust_local_ci_and_cold_profiles_are_explicit() {
    let makefile = read_repo_file("Makefile");
    let driver = read_repo_file(RUST_DRIVER);
    let api_driver = read_repo_file("tests/tools/api-surface-json.sh");
    let violations = rust_profile_violations(&makefile, &driver, &api_driver);
    assert!(
        violations.is_empty(),
        "Rust local, CI, and cold profiles drifted:\n{}",
        violations.join("\n")
    );
}

#[test]
fn rust_profile_policy_rejects_a_missing_ci_profile() {
    let makefile = r#"
$(call D2B_RUST_DISPATCH,leaves,aggregate)
$(call D2B_RUST_DISPATCH,leaves,api)
$(call D2B_RUST_DISPATCH,leaves,main)
$(call D2B_RUST_DISPATCH,leaves,broker)
$(call D2B_RUST_DISPATCH,leaves,guest)
$(call D2B_RUST_DISPATCH,leaves,no-bash)
$(call D2B_RUST_DISPATCH,leaves,schema)
$(call D2B_RUST_DISPATCH,leaves,inventory)
$(call D2B_RUST_DISPATCH,leaves,supply)
profile=cold
D2B_RUST_COLD_PROFILE=$$cold_profile
D2B_RUST_QUOTA_MAIN=$$quota_main
D2B_RUST_QUOTA_BROKER=$$quota_broker
"#;
    let driver = r#"
${D2B_RUST_COLD_PROFILE:-0}
fixture_target_dir="$workspace_target_dir"
"#;
    let api_driver = r#"
${D2B_RUST_COLD_PROFILE:-0}
shared_census=1
public_target="$target_root/census"
"#;
    assert!(
        rust_profile_violations(makefile, driver, api_driver).is_empty(),
        "positive Rust profile fixture must pass"
    );
    let mutated = makefile.replace(",main)", ")");
    let violations = rust_profile_violations(&mutated, driver, api_driver);
    assert!(
        violations
            .iter()
            .any(|violation| violation.contains("`main` profile")),
        "removing the main CI profile must fail: {violations:?}"
    );
}

#[test]
fn rust_driver_exposes_leaf_only_modes() {
    let driver = read_repo_file(RUST_DRIVER);
    let violations = rust_mode_violations(&driver);
    assert!(
        violations.is_empty(),
        "tests/test-rust.sh must be a leaf dispatcher, not a second scheduler:\n{}",
        violations.join("\n")
    );
}

#[test]
fn rust_driver_policy_rejects_an_aggregate_mode_fixture() {
    let good = r#"
rust_mode="${1:-}"
if [ "$#" -eq 0 ]; then
  fail "select a leaf mode; run make test-rust"
  exit 2
fi
case "$rust_mode" in
  api-surface) run_api_surface ;;
  main-workspace) run_main ;;
  broker) run_broker ;;
  guest-shell-runner) run_guest ;;
  no-bash-ast) run_ast ;;
  schema-reproducibility) run_schema ;;
  supply-chain) run_supply ;;
  inventory-stub) run_inventory ;;
  fixture-contracts) run_fixture ;;
esac
"#;
    assert!(
        rust_mode_violations(good).is_empty(),
        "the positive leaf-mode fixture must satisfy the policy"
    );
    let mutated = good.replace("fixture-contracts) run_fixture", "all) run_all");
    let violations = rust_mode_violations(&mutated);
    assert!(
        violations
            .iter()
            .any(|violation| violation.contains("removed `all` scheduler")),
        "reintroducing an aggregate all mode must be rejected: {violations:?}"
    );
}

#[test]
fn excluded_workspace_policy_rejects_mutated_governed_inputs() {
    let main_workspace = read_repo_file("packages/Cargo.toml");
    let flake = read_repo_file("flake.nix");
    let driver = read_repo_file(RUST_DRIVER);
    let removed_exclude = main_workspace.replacen("\"d2b-priv-broker\"", "", 1);
    let violations = excluded_workspace_violations(&removed_exclude, &flake, &driver);
    assert!(
        !violations.is_empty(),
        "removing an excluded workspace from the governed manifest must be rejected"
    );

    let removed_supply_chain = flake.replace("packages/d2b-priv-broker/Cargo.lock", "");
    let violations = excluded_workspace_violations(&main_workspace, &removed_supply_chain, &driver);
    assert!(
        !violations.is_empty(),
        "removing an excluded workspace supply-chain input must be rejected"
    );
}
