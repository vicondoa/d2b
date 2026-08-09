//! Fixture-independent supply-chain policy for the Spec 003 root workspace.

use std::collections::BTreeSet;

use d2b_contract_tests::{read_repo_file, repo_path_exists};
use serde_json::Value;

const ROOT_LOCK: &str = "packages/Cargo.lock";
const GUEST_LOCK: &str = "packages/Cargo.guest.lock";
const BROKER_NESTED_LOCK: &str = "packages/d2b-priv-broker/Cargo.lock";
const GUEST_NESTED_LOCK: &str = "packages/d2b-guest-shell-runner/Cargo.lock";
const NATIVE_MANIFEST: &str = "tests/golden/native-policy-check-manifest.json";

#[test]
fn aggregate_flake_supply_chain_checks_are_independent_and_root_selected() {
    let flake = read_repo_file("flake.nix");
    let test_rust = read_repo_file("tests/test-rust.sh");

    assert!(flake.contains(ROOT_LOCK));
    assert!(flake.contains(GUEST_LOCK));
    assert!(!flake.contains(BROKER_NESTED_LOCK));
    assert!(!flake.contains(GUEST_NESTED_LOCK));
    assert!(flake.contains("lockFile = ./packages/Cargo.lock;"));
    assert!(flake.contains("lockFile = ./packages/Cargo.guest.lock;"));
    assert!(flake.contains("run_audit ${rustPackagesSrc}/packages/Cargo.lock"));
    assert!(flake.contains("run_audit ${rustPackagesSrc}/packages/Cargo.guest.lock"));
    assert!(test_rust.contains(ROOT_LOCK));
    assert!(test_rust.contains(GUEST_LOCK));
    assert!(!test_rust.contains(BROKER_NESTED_LOCK));
    assert!(!test_rust.contains(GUEST_NESTED_LOCK));
}

#[test]
fn selected_policy_inputs_and_no_fetch_audits_are_pinned_without_retry() {
    let flake = read_repo_file("flake.nix");
    let test_rust = read_repo_file("tests/test-rust.sh");

    assert!(test_rust.contains(NATIVE_MANIFEST));
    assert!(flake.contains(NATIVE_MANIFEST));
    for path in [
        "x86_64-linux/x86_64-unknown-linux-gnu/broker-production",
        "x86_64-linux/x86_64-unknown-linux-musl/guest-real-libshpool",
        "aarch64-linux/aarch64-unknown-linux-gnu/broker-production",
        "aarch64-linux/aarch64-unknown-linux-musl/guest-real-libshpool",
    ] {
        assert!(
            read_repo_file(NATIVE_MANIFEST).contains(path),
            "missing selected policy input {path}"
        );
    }
    assert!(test_rust.contains("policy_metadata_path"));
    assert!(test_rust.contains("policy_lock_path"));
    assert!(flake.contains("--no-fetch"));
    assert!(test_rust.contains("--no-fetch"));
    assert!(!flake.contains("retry"));
    let policy_audit_helpers = test_rust
        .split_once("run_policy_audit()")
        .and_then(|(_, rest)| rest.split_once("run_inventory_stub_gate()"))
        .map(|(section, _)| section)
        .expect("policy audit helper section must exist");
    assert!(!policy_audit_helpers.contains("retry"));
    assert!(flake.contains("policyContextRoot"));
    assert!(flake.contains("/production/closure.json"));
    assert!(flake.contains("/production/Cargo.lock"));
    assert!(!flake.contains("guest-shell-runner/Cargo.lock"));
}

#[test]
fn guest_license_policy_has_exactly_six_package_scoped_exceptions() {
    let deny = read_repo_file("packages/d2b-guest-shell-runner/deny.toml");
    let pairs = exception_pairs(&deny);
    let expected = BTreeSet::from([
        ("bindgen".to_owned(), "BSD-3-Clause".to_owned()),
        ("instant".to_owned(), "BSD-3-Clause".to_owned()),
        ("inotify".to_owned(), "ISC".to_owned()),
        ("inotify-sys".to_owned(), "ISC".to_owned()),
        ("libloading".to_owned(), "ISC".to_owned()),
        ("notify".to_owned(), "CC0-1.0".to_owned()),
    ]);
    assert_eq!(pairs, expected);
    let global_licenses = deny
        .split_once("exceptions = [")
        .map(|(global, _)| global)
        .expect("guest exceptions must follow the global license table");
    assert!(!global_licenses.contains("\"BSD-3-Clause\""));
    assert!(!global_licenses.contains("\"ISC\""));
    assert!(!global_licenses.contains("\"CC0-1.0\""));
    assert!(deny.contains("[licenses.exceptions]") || deny.contains("exceptions = ["));
}

fn exception_pairs(text: &str) -> BTreeSet<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let name = quoted_after(line, "name = ")?;
            let license = quoted_after(line, "allow = [")?;
            Some((name, license))
        })
        .collect()
}

fn quoted_after(line: &str, marker: &str) -> Option<String> {
    let value = line.split_once(marker)?.1;
    let start = value.find('"')? + 1;
    let rest = &value[start..];
    Some(rest[..rest.find('"')?].to_owned())
}

#[test]
fn package_policy_contexts_keep_root_dev_policy_separate_from_production() {
    let generator = read_repo_file("packages/xtask/src/package_policy.rs");
    let flake = read_repo_file("flake.nix");
    assert!(generator.contains("\"normal,build\""));
    assert!(generator.contains("\"normal,build,dev\""));
    assert!(generator.contains("--no-default-features"));
    assert!(generator.contains("--features"));
    assert!(generator.contains("--target"));
    assert!(generator.contains("--locked"));
    assert!(generator.contains("--offline"));
    assert!(generator.contains("metadata_command"));
    assert!(generator.contains("policy_tree_command"));
    assert!(generator.contains("selected_source_census"));
    assert!(generator.contains("verify_git_archive_pin"));
    for check in [
        "broker-production-package-policy",
        "guest-real-libshpool-package-policy",
    ] {
        assert!(
            flake.contains(check),
            "missing package policy check {check}"
        );
    }
}

#[test]
fn policy_diagnostics_do_not_emit_store_paths_or_deleted_lock_inputs() {
    for path in [
        "flake.nix",
        "tests/test-rust.sh",
        "packages/d2b-guest-shell-runner/deny.toml",
    ] {
        let text = read_repo_file(path);
        assert!(!text.contains("/nix/store/"), "{path} emits a store path");
        assert!(
            !text.contains(BROKER_NESTED_LOCK),
            "{path} uses deleted broker lock"
        );
        assert!(
            !text.contains(GUEST_NESTED_LOCK),
            "{path} uses deleted guest lock"
        );
    }
}

#[test]
fn generated_policy_inputs_are_not_forged_as_a_second_workspace_authority() {
    if repo_path_exists("packages/policy-inputs") {
        let manifest: Value =
            serde_json::from_str(&read_repo_file(NATIVE_MANIFEST)).expect("native manifest JSON");
        for context in manifest["contexts"].as_array().expect("contexts") {
            let path = context["policyInput"].as_str().expect("policy input");
            for variant in ["production", "policy"] {
                assert!(
                    repo_path_exists(&format!("{path}/{variant}/Cargo.lock")),
                    "missing generated policy lock {path}/{variant}"
                );
            }
        }
    }
}

#[test]
fn policy_mutation_tests_use_the_production_validator() {
    let nix_case = read_repo_file("tests/unit/nix/cases/bazel-package-policy.nix");
    let validator = read_repo_file("nixos-modules/policy-artifact-validator.nix");
    assert!(nix_case.contains("policy-artifact-validator.nix"));
    assert!(nix_case.contains("policyArtifactShapeOk"));
    assert!(!nix_case.contains("legacyPolicyValid"));
    assert!(validator.contains("resolveNodeEdgesClosed"));
    assert!(validator.contains("lockMatches"));
    assert!(validator.contains("reachableNodeIds"));
}
