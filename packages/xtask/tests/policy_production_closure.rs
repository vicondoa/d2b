#![forbid(unsafe_code)]

//! Fail-closed checks for the Cargo production-closure authority.
//!
//! These checks deliberately validate the checked-in projections rather than
//! trusting that the generator can only emit valid data. The planted negative
//! cases exercise the same structural failures that would otherwise let a
//! shared Cargo.lock approve an unrelated or unreviewed edge.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

mod common;

use common::repo_root;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const APPROVAL_MARKER: &str = "d2b-production-closure-approved/v1";
const PROTECTED_CODEOWNERS_RULES: &[&str] = &[
    "/.github/CODEOWNERS @vicondoa",
    "/Cargo.toml @vicondoa",
    "/Cargo.lock @vicondoa",
    "/packages/Cargo.guest.lock @vicondoa",
    "/packages/d2b-priv-broker/Cargo.toml @vicondoa",
    "/packages/d2b-guest-shell-runner/Cargo.toml @vicondoa",
    "/packages/policy-inputs/** @vicondoa",
    "/packages/policy-inputs/advisory-policy.json @vicondoa",
    "/packages/xtask/Cargo.toml @vicondoa",
    "/packages/xtask/src/main.rs @vicondoa",
    "/packages/xtask/src/production_closure.rs @vicondoa",
    "/packages/xtask/tests/policy_production_closure.rs @vicondoa",
    "/Makefile @vicondoa",
    "/flake.nix @vicondoa",
    "/nixos-modules/guest-control.nix @vicondoa",
    "/nixos-modules/host-activation.nix @vicondoa",
    "/nixos-modules/host-broker.nix @vicondoa",
    "/nixos-modules/host-daemon.nix @vicondoa",
    "/nixos-modules/processes-json.nix @vicondoa",
    "/nixos-modules/resource-compiler.nix @vicondoa",
    "/packages/d2b-provider-network-local/nix/** @vicondoa",
    "/packages/d2b-provider-volume-local/nix/** @vicondoa",
    "/packages/d2b-provider-activation-nixos/nix/** @vicondoa",
    "/nixos-modules/unsafe-local-helper.nix @vicondoa",
    "/tests/lib.sh @vicondoa",
    "/tests/tools/guest-workspace-drift.py @vicondoa",
    "/tests/integration/containers/images/ubuntu-host-check.nix @vicondoa",
    "/tests/unit/smoke/guest-static-consumption-eval.nix @vicondoa",
];
const CONTEXT_ROOT: &str = "packages/policy-inputs";

fn read_repo_file(rel: &str) -> String {
    std::fs::read_to_string(repo_root().join(rel)).expect("read repo file")
}

fn closure_paths() -> Vec<PathBuf> {
    let root = repo_root().join(CONTEXT_ROOT);
    let mut paths = Vec::new();
    for system in fs::read_dir(&root).expect("policy systems") {
        let system = system.expect("policy system entry").path();
        if !system.is_dir() {
            continue;
        }
        for target in fs::read_dir(system).expect("policy targets") {
            let target = target.expect("policy target entry").path();
            if !target.is_dir() {
                continue;
            }
            for context in fs::read_dir(target).expect("policy contexts") {
                let context = context.expect("policy context entry").path();
                let closure = context.join("production/closure.json");
                if closure.is_file() {
                    paths.push(closure);
                }
            }
        }
    }
    paths.sort();
    paths
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(
        &fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display())),
    )
    .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn package_ids(closure: &Value) -> BTreeSet<String> {
    closure["packages"]
        .as_array()
        .expect("closure packages")
        .iter()
        .map(|package| package["id"].as_str().expect("package id").to_owned())
        .collect()
}

fn validate_production_closure(closure: &Value) -> Result<(), String> {
    if closure["schema_version"] != 1
        || closure["authority"] != "cargo-locked-metadata"
        || closure["packages"].as_array().is_none()
        || closure["packages"].as_array().is_some_and(Vec::is_empty)
    {
        return Err("empty or unknown production closure".to_owned());
    }
    let ids = package_ids(closure);
    if ids.len() != closure["packages"].as_array().expect("packages").len() {
        return Err("duplicate production package entry".to_owned());
    }
    let roots = closure["roots"]
        .as_array()
        .ok_or("closure roots are missing")?
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    let mut reachable = closure["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .filter(|package| roots.contains(package["name"].as_str().unwrap_or_default()))
        .map(|package| package["id"].as_str().expect("root id").to_owned())
        .collect::<BTreeSet<_>>();
    let mut edges = BTreeSet::new();
    for edge in closure["edges"].as_array().expect("closure edges") {
        let from = edge["from"].as_str().ok_or("edge has no source")?;
        let to = edge["to"].as_str().ok_or("edge has no target")?;
        let kind = edge["kind"].as_str().ok_or("edge has no kind")?;
        if !ids.contains(from) || !ids.contains(to) {
            return Err("edge points outside production closure".to_owned());
        }
        if !matches!(kind, "normal" | "build" | "proc-macro") {
            return Err("production closure contains a non-production edge".to_owned());
        }
        let target = edge
            .get("target")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if target.contains("missing") {
            return Err("edge target expression is invalid".to_owned());
        }
        if !edges.insert((
            from.to_owned(),
            to.to_owned(),
            kind.to_owned(),
            target.to_owned(),
        )) {
            return Err("duplicate production edge".to_owned());
        }
    }
    loop {
        let before = reachable.len();
        for (from, to, _, _) in &edges {
            if reachable.contains(from) {
                reachable.insert(to.clone());
            }
        }
        if reachable.len() == before {
            break;
        }
    }
    if reachable != ids {
        return Err("production closure contains an unreachable package".to_owned());
    }
    let target = closure["target"]
        .as_str()
        .ok_or("closure target is missing")?;
    if !matches!(
        target,
        "x86_64-unknown-linux-gnu"
            | "aarch64-unknown-linux-gnu"
            | "x86_64-unknown-linux-musl"
            | "aarch64-unknown-linux-musl"
    ) {
        return Err("production closure target is invalid".to_owned());
    }
    for package in closure["packages"].as_array().expect("packages") {
        for field in ["id", "name", "version", "target"] {
            if package[field].as_str().is_none_or(str::is_empty) {
                return Err(format!("package entry has no {field}"));
            }
        }
        if package["source"]
            .as_str()
            .is_some_and(|source| source.starts_with("registry+"))
            && package["checksum"].is_null()
        {
            return Err("registry package has no checksum".to_owned());
        }
        if package["target"] != target {
            return Err("package target disagrees with closure target".to_owned());
        }
        let lock_path = closure["source_authority"].as_str().unwrap_or("Cargo.lock");
        let lock = read_repo_file(lock_path);
        let block = lock
            .split("[[package]]")
            .find(|block| {
                block.contains(&format!(
                    "name = \"{}\"",
                    package["name"].as_str().unwrap_or_default()
                )) && block.contains(&format!(
                    "version = \"{}\"",
                    package["version"].as_str().unwrap_or_default()
                ))
            })
            .ok_or("package is not present in authoritative lock")?;
        if let Some(source) = package["source"].as_str() {
            if !block.contains(&format!("source = \"{source}\"")) {
                return Err("package source disagrees with authoritative lock".to_owned());
            }
            let checksum = package["checksum"].as_str().unwrap_or_default();
            if source.starts_with("registry+")
                && !block.contains(&format!("checksum = \"{checksum}\""))
            {
                return Err("package checksum disagrees with authoritative lock".to_owned());
            }
        }
    }
    Ok(())
}

fn validate_policy(policy: &Value) -> Result<(), String> {
    if policy["schemaVersion"] != 1
        || policy["authority"] != "context-scoped-advisory-policy"
        || policy["requiresProtectedReview"] != true
        || policy["ignore"].is_object()
        || policy["ignores"].is_array()
    {
        return Err("policy is not context-scoped and protected".to_owned());
    }
    let recomputation = policy["recomputation"]
        .as_object()
        .ok_or("missing recomputation metadata")?;
    if recomputation["trustedApprovalRequired"] != true {
        return Err("independent approval recomputation is not required".to_owned());
    }
    if policy["recomputation"]["command"] != "cargo xtask gen-package-policy-inputs --check"
        || policy["recomputation"]["mode"] != "independent-locked-metadata-recompute"
    {
        return Err("recomputation metadata is not exact".to_owned());
    }
    if policy["protectedOwnership"]["path"] != ".github/CODEOWNERS"
        || policy["protectedOwnership"]["owner"] != "@vicondoa"
    {
        return Err("protected ownership metadata is not exact".to_owned());
    }
    let codeowners = read_repo_file(".github/CODEOWNERS");
    let rules = codeowners
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<BTreeSet<_>>();
    for required_rule in PROTECTED_CODEOWNERS_RULES {
        if !rules.contains(required_rule) {
            return Err(format!(
                "CODEOWNERS is missing exact protected rule {required_rule}"
            ));
        }
    }
    let contexts = policy["contexts"]
        .as_object()
        .ok_or("missing policy contexts")?;
    let mut owned = BTreeSet::new();
    for (context_name, context) in contexts {
        let approval = context["approval"].as_object().ok_or("missing approval")?;
        if approval["marker"] != APPROVAL_MARKER
            || approval["owner"] != "@vicondoa"
            || approval["rationale"].as_str().is_none_or(str::is_empty)
            || approval["expiresAt"]
                .as_str()
                .is_none_or(|expiry| expiry < "2026-01-01")
        {
            return Err(format!("invalid approval for {context_name}"));
        }
        for advisory in context["advisories"]
            .as_array()
            .ok_or("advisories is not an array")?
        {
            let id = advisory["id"].as_str().ok_or("advisory has no id")?;
            if !owned.insert(id.to_owned()) {
                return Err(format!("advisory {id} is owned by multiple contexts"));
            }
            if advisory["owner"] != "@vicondoa"
                || advisory["approvalMarker"] != APPROVAL_MARKER
                || advisory["rationale"].as_str().is_none_or(str::is_empty)
                || advisory["expiresAt"]
                    .as_str()
                    .is_none_or(|expiry| expiry < "2026-01-01")
            {
                return Err(format!("invalid advisory approval for {context_name}"));
            }
        }
    }
    Ok(())
}

#[test]
fn checked_in_contexts_are_nonempty_and_structurally_valid() {
    let paths = closure_paths();
    assert_eq!(paths.len(), 14, "expected both systems and all U2 contexts");
    for path in paths {
        validate_production_closure(&read_json(&path))
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    }
}

#[test]
fn unrelated_lock_only_package_does_not_enter_production() {
    for path in closure_paths() {
        let closure = read_json(&path);
        assert!(
            !package_ids(&closure)
                .iter()
                .any(|id| id.contains("unrelated-lock-only")),
            "{} approved an unrelated lock-only package",
            path.display()
        );
    }
}

#[test]
fn normal_build_and_proc_macro_edges_are_approved_only_when_present() {
    let mut kinds = BTreeSet::new();
    for path in closure_paths() {
        let closure = read_json(&path);
        for edge in closure["edges"].as_array().expect("edges") {
            kinds.insert(edge["kind"].as_str().expect("edge kind").to_owned());
        }
    }
    assert!(kinds.is_subset(&BTreeSet::from([
        "normal".to_owned(),
        "build".to_owned(),
        "proc-macro".to_owned(),
    ])));
    assert!(kinds.contains("normal"));
}

#[test]
fn dev_edge_move_is_rejected_from_production() {
    let mut closure = read_json(&closure_paths()[0]);
    closure["edges"][0]["kind"] = Value::String("dev".to_owned());
    assert!(validate_production_closure(&closure).is_err());
}

#[test]
fn missing_stale_extra_and_duplicate_entries_are_rejected() {
    let original = read_json(&closure_paths()[0]);

    let mut missing = original.clone();
    missing["packages"]
        .as_array_mut()
        .expect("packages")
        .clear();
    assert!(validate_production_closure(&missing).is_err());

    let mut stale = original.clone();
    stale["edges"][0]["from"] = Value::String("missing@0#path".to_owned());
    assert!(validate_production_closure(&stale).is_err());

    let mut extra = original.clone();
    extra["packages"]
        .as_array_mut()
        .expect("packages")
        .push(json!({
            "id": "extra@1#path",
            "name": "extra",
            "version": "1",
            "source": null,
            "checksum": null,
            "target": "x86_64-unknown-linux-gnu"
        }));
    assert!(validate_production_closure(&extra).is_err());
    let duplicate = extra["packages"][0].clone();
    extra["packages"]
        .as_array_mut()
        .expect("packages")
        .push(duplicate);
    assert!(validate_production_closure(&extra).is_err());
}

#[test]
fn wrong_source_checksum_target_and_edge_are_rejected() {
    let original = read_json(&closure_paths()[0]);

    let mut wrong_source = original.clone();
    let package = wrong_source["packages"]
        .as_array_mut()
        .expect("packages")
        .iter_mut()
        .find(|package| package["source"].is_string())
        .expect("registry package");
    package["source"] = Value::String("registry+https://wrong.invalid".to_owned());
    package["checksum"] = Value::Null;
    assert!(validate_production_closure(&wrong_source).is_err());

    let mut wrong_checksum = original.clone();
    let package = wrong_checksum["packages"]
        .as_array_mut()
        .expect("packages")
        .iter_mut()
        .find(|package| package["source"].is_string())
        .expect("registry package");
    package["checksum"] = Value::String("wrong".to_owned());
    assert!(validate_production_closure(&wrong_checksum).is_err());

    let mut wrong_target = original.clone();
    wrong_target["target"] = Value::String("invalid-target".to_owned());
    assert!(validate_production_closure(&wrong_target).is_err());
    wrong_target["packages"][0]["target"] = Value::String("invalid-target".to_owned());
    assert!(validate_production_closure(&wrong_target).is_err());

    let mut wrong_edge = original;
    wrong_edge["edges"][0]["target"] = Value::String("cfg(missing)".to_owned());
    assert!(validate_production_closure(&wrong_edge).is_err());
}

#[test]
fn target_cfg_and_feature_contexts_are_not_collapsed() {
    let mut contexts = BTreeSet::new();
    for path in closure_paths() {
        let closure = read_json(&path);
        contexts.insert((
            closure["system"].as_str().expect("system").to_owned(),
            closure["target"].as_str().expect("target").to_owned(),
            closure["context"].as_str().expect("context").to_owned(),
            closure["features"].to_string(),
        ));
    }
    assert!(contexts.iter().any(|(_, _, name, features)| {
        *name == "broker-layer1-bootstrap-tests" && features.contains("layer1-bootstrap")
    }));
    assert!(
        contexts
            .iter()
            .any(|(_, target, _, _)| target.ends_with("musl"))
    );
    assert!(
        contexts
            .iter()
            .any(|(_, target, _, _)| target.ends_with("gnu"))
    );
}

#[test]
fn approval_policy_rejects_cross_context_expired_and_unapproved_ignores() {
    let mut policy: Value = serde_json::from_str(&read_repo_file(
        "packages/policy-inputs/advisory-policy.json",
    ))
    .expect("policy JSON");
    validate_policy(&policy).expect("checked-in policy");

    let context_keys = policy["contexts"]
        .as_object()
        .expect("contexts")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    let first_key = context_keys.first().expect("first context").clone();
    let second_key = context_keys.get(1).expect("second context").clone();
    let advisory = json!([{
        "id": "RUSTSEC-test",
        "owner": "@vicondoa",
        "expiresAt": "2099-12-31",
        "rationale": "test",
        "approvalMarker": APPROVAL_MARKER
    }]);
    policy["contexts"][&first_key]["advisories"] = advisory.clone();
    policy["contexts"][&second_key]["advisories"] = advisory;
    assert!(validate_policy(&policy).is_err());

    policy["contexts"][&first_key]["advisories"][0]["expiresAt"] =
        Value::String("1970-01-01".to_owned());
    policy["contexts"][&second_key]["advisories"] = json!([]);
    assert!(validate_policy(&policy).is_err());

    policy["contexts"][&first_key]["advisories"][0]["expiresAt"] =
        Value::String("2099-12-31".to_owned());
    policy["contexts"][&first_key]["advisories"][0]["approvalMarker"] =
        Value::String("unapproved".to_owned());
    assert!(validate_policy(&policy).is_err());
}

#[test]
fn approval_policy_requires_exact_protected_ownership_metadata() {
    let mut policy: Value = serde_json::from_str(&read_repo_file(
        "packages/policy-inputs/advisory-policy.json",
    ))
    .expect("policy JSON");
    validate_policy(&policy).expect("checked-in policy");

    policy["protectedOwnership"]["path"] = Value::String("README.md".to_owned());
    assert!(validate_policy(&policy).is_err());

    policy["protectedOwnership"]["path"] = Value::String(".github/CODEOWNERS".to_owned());
    policy["protectedOwnership"]["owner"] = Value::String("@other".to_owned());
    assert!(validate_policy(&policy).is_err());

    policy["protectedOwnership"]["owner"] = Value::String("@vicondoa".to_owned());
    policy["recomputation"]["trustedApprovalRequired"] = Value::Bool(false);
    assert!(validate_policy(&policy).is_err());
}

#[test]
fn root_lock_digest_is_the_recomputed_authority() {
    let mut digest = Sha256::new();
    digest.update(fs::read(repo_root().join("Cargo.lock")).expect("root lock"));
    let expected = format!("{:x}", digest.finalize());
    let closure =
        read_json(&repo_root().join(CONTEXT_ROOT).join(
            "x86_64-linux/x86_64-unknown-linux-gnu/broker-production/production/closure.json",
        ));
    assert_eq!(closure["lock_sha256"], expected);
}

#[test]
fn guest_static_context_uses_reduced_guest_lock() {
    let closure = read_json(
        &repo_root()
            .join(CONTEXT_ROOT)
            .join("x86_64-linux/x86_64-unknown-linux-musl/guestd-static/production/closure.json"),
    );
    assert_eq!(closure["source_authority"], "packages/Cargo.guest.lock");
    assert_ne!(closure["lock_sha256"], Value::String(String::new()));
}
