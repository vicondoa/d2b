//! Fixture-independent supply-chain policy for the Spec 003 root workspace.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use d2b_contract_tests::{read_repo_file, repo_path_exists};
use serde_json::Value;

const ROOT_LOCK: &str = "packages/Cargo.lock";
const GUEST_LOCK: &str = "packages/Cargo.guest.lock";
const BROKER_NESTED_LOCK: &str = "packages/d2b-priv-broker/Cargo.lock";
const GUEST_NESTED_LOCK: &str = "packages/d2b-guest-shell-runner/Cargo.lock";

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

    for path in [
        "x86_64-linux/x86_64-unknown-linux-gnu/broker-production",
        "x86_64-linux/x86_64-unknown-linux-musl/guest-real-libshpool",
        "aarch64-linux/aarch64-unknown-linux-gnu/broker-production",
        "aarch64-linux/aarch64-unknown-linux-musl/guest-real-libshpool",
    ] {
        assert!(
            test_rust.contains(path),
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

    for mutation in [
        ("other-bindgen", "BSD-3-Clause"),
        ("bindgen", "ISC"),
        ("other-inotify", "CC0-1.0"),
    ] {
        assert!(!expected.contains(&(mutation.0.to_owned(), mutation.1.to_owned())));
    }
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
        for system in ["x86_64-linux", "aarch64-linux"] {
            let gnu = if system == "x86_64-linux" {
                "x86_64-unknown-linux-gnu"
            } else {
                "aarch64-unknown-linux-gnu"
            };
            let musl = if system == "x86_64-linux" {
                "x86_64-unknown-linux-musl"
            } else {
                "aarch64-unknown-linux-musl"
            };
            for path in [
                format!(
                    "packages/policy-inputs/{system}/{gnu}/broker-production/production/closure.json"
                ),
                format!(
                    "packages/policy-inputs/{system}/{gnu}/broker-production/policy/metadata.json"
                ),
                format!(
                    "packages/policy-inputs/{system}/{musl}/guest-real-libshpool/production/closure.json"
                ),
                format!(
                    "packages/policy-inputs/{system}/{musl}/guest-real-libshpool/policy/metadata.json"
                ),
            ] {
                assert!(
                    repo_path_exists(&path),
                    "missing generated policy input {path}"
                );
            }
        }
    }
}

#[derive(Clone, Copy)]
struct SelectedPolicyContext {
    system: &'static str,
    target: &'static str,
    context: &'static str,
    package: &'static str,
    features: &'static [&'static str],
}

const SELECTED_POLICY_CONTEXTS: [SelectedPolicyContext; 4] = [
    SelectedPolicyContext {
        system: "x86_64-linux",
        target: "x86_64-unknown-linux-gnu",
        context: "broker-production",
        package: "d2b-priv-broker",
        features: &[],
    },
    SelectedPolicyContext {
        system: "x86_64-linux",
        target: "x86_64-unknown-linux-musl",
        context: "guest-real-libshpool",
        package: "d2b-guest-shell-runner",
        features: &["real-libshpool"],
    },
    SelectedPolicyContext {
        system: "aarch64-linux",
        target: "aarch64-unknown-linux-gnu",
        context: "broker-production",
        package: "d2b-priv-broker",
        features: &[],
    },
    SelectedPolicyContext {
        system: "aarch64-linux",
        target: "aarch64-unknown-linux-musl",
        context: "guest-real-libshpool",
        package: "d2b-guest-shell-runner",
        features: &["real-libshpool"],
    },
];

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct PolicyIdentity {
    name: String,
    version: String,
    source: Option<String>,
}

fn field<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    value.as_object()?.get(name)
}

fn string_field<'a>(value: &'a Value, name: &str) -> Option<&'a str> {
    field(value, name)?.as_str()
}

fn policy_identity(value: &Value) -> Result<PolicyIdentity, String> {
    let source = match field(value, "source") {
        None | Some(Value::Null) => None,
        Some(Value::String(source)) => Some(source.clone()),
        _ => return Err("identity source is not a string or null".to_owned()),
    };
    Ok(PolicyIdentity {
        name: string_field(value, "name")
            .filter(|name| !name.is_empty())
            .ok_or_else(|| "identity name is missing".to_owned())?
            .to_owned(),
        version: string_field(value, "version")
            .filter(|version| !version.is_empty())
            .ok_or_else(|| "identity version is missing".to_owned())?
            .to_owned(),
        source,
    })
}

fn identity_set(value: &Value, name: &str) -> Result<BTreeSet<PolicyIdentity>, String> {
    let values = field(value, name)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{name} must be an array"))?;
    if values.is_empty() {
        return Err(format!("{name} must be nonempty"));
    }
    let mut identities = BTreeSet::new();
    for value in values {
        if !identities.insert(policy_identity(value)?) {
            return Err(format!("{name} contains a duplicate identity"));
        }
    }
    Ok(identities)
}

fn string_array(value: &Value, name: &str) -> Result<Vec<String>, String> {
    field(value, name)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{name} must be an array"))?
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{name} contains a non-string"))
        })
        .collect()
}

fn is_hex(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn parse_toml_string(value: &str) -> Option<String> {
    if !value.starts_with('"') {
        return None;
    }
    serde_json::from_str(value).ok()
}

fn parse_policy_lock(text: &str) -> Result<BTreeSet<PolicyIdentity>, String> {
    let mut records = Vec::<BTreeMap<String, String>>::new();
    let mut current: Option<BTreeMap<String, String>> = None;
    for line in text.lines() {
        let line = line.trim();
        if line == "[[package]]" {
            if let Some(record) = current.take() {
                records.push(record);
            }
            current = Some(BTreeMap::new());
            continue;
        }
        let Some(record) = current.as_mut() else {
            continue;
        };
        let Some((key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if !matches!(key, "name" | "version" | "source") {
            continue;
        }
        let value = raw_value.trim();
        let Some(value) = parse_toml_string(value) else {
            return Err(format!("lock {key} is not a basic string"));
        };
        if record.insert(key.to_owned(), value).is_some() {
            return Err(format!("lock {key} appears twice"));
        }
    }
    if let Some(record) = current {
        records.push(record);
    }
    if records.is_empty() {
        return Err("policy lock has no package records".to_owned());
    }
    let mut identities = BTreeSet::new();
    for record in records {
        let identity = PolicyIdentity {
            name: record
                .get("name")
                .filter(|name| !name.is_empty())
                .cloned()
                .ok_or_else(|| "lock package name is missing".to_owned())?,
            version: record
                .get("version")
                .filter(|version| !version.is_empty())
                .cloned()
                .ok_or_else(|| "lock package version is missing".to_owned())?,
            source: record.get("source").cloned(),
        };
        if !identities.insert(identity) {
            return Err("policy lock contains a duplicate identity".to_owned());
        }
    }
    Ok(identities)
}

fn policy_input_root(context: SelectedPolicyContext, variant: &str) -> String {
    format!(
        "packages/policy-inputs/{}/{}/{}/{}",
        context.system, context.target, context.context, variant
    )
}

fn read_json(path: &str) -> Value {
    serde_json::from_str(&read_repo_file(path))
        .unwrap_or_else(|error| panic!("{path} must be valid JSON: {error}"))
}

fn validate_policy_metadata(
    document: &Value,
    lock_text: &str,
    context: SelectedPolicyContext,
) -> Result<BTreeSet<PolicyIdentity>, String> {
    for (name, expected) in [
        ("system", context.system),
        ("target", context.target),
        ("package", context.package),
        ("root", context.package),
        ("variant", "policy"),
        ("edgeKinds", "normal,build,dev"),
    ] {
        if string_field(document, name) != Some(expected) {
            return Err(format!("{name} does not match selected context"));
        }
    }
    if field(document, "defaultFeatures") != Some(&Value::Bool(false)) {
        return Err("defaultFeatures is not false".to_owned());
    }
    let expected_features = context
        .features
        .iter()
        .map(|feature| (*feature).to_owned())
        .collect::<Vec<_>>();
    if string_array(document, "features")? != expected_features {
        return Err("selected features do not match context".to_owned());
    }
    let identities = identity_set(document, "identities")?;
    let packages = field(document, "packages")
        .and_then(Value::as_array)
        .ok_or_else(|| "packages must be an array".to_owned())?;
    if packages.is_empty() {
        return Err("policy package graph must be nonempty".to_owned());
    }
    let mut package_by_id = BTreeMap::<String, PolicyIdentity>::new();
    for package in packages {
        let id = string_field(package, "id")
            .filter(|id| !id.is_empty())
            .ok_or_else(|| "package id is missing".to_owned())?
            .to_owned();
        let identity = policy_identity(package)?;
        if package_by_id.insert(id, identity).is_some() {
            return Err("policy package ids are not unique".to_owned());
        }
    }
    let package_id_set = package_by_id.keys().cloned().collect::<BTreeSet<_>>();
    let package_identities = package_by_id.values().cloned().collect::<BTreeSet<_>>();
    if identities != package_identities {
        return Err("policy identities do not equal selected packages".to_owned());
    }
    let roots = package_by_id
        .iter()
        .filter(|(_, identity)| identity.name == context.package)
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    if roots.len() != 1 {
        return Err("policy root identity is not unique".to_owned());
    }

    let resolve = field(document, "resolve")
        .and_then(Value::as_object)
        .ok_or_else(|| "resolve must be an object".to_owned())?;
    let resolve_root = resolve
        .get("root")
        .and_then(Value::as_str)
        .ok_or_else(|| "resolve root is missing".to_owned())?
        .to_owned();
    if resolve_root != roots[0] {
        return Err("resolve root does not equal package root".to_owned());
    }
    let nodes = resolve
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| "resolve nodes must be an array".to_owned())?;
    if nodes.is_empty() {
        return Err("resolve graph must be nonempty".to_owned());
    }
    let mut node_by_id = BTreeMap::<String, &Value>::new();
    for node in nodes {
        let id = string_field(node, "id")
            .filter(|id| !id.is_empty())
            .ok_or_else(|| "resolve node id is missing".to_owned())?
            .to_owned();
        if node_by_id.insert(id, node).is_some() {
            return Err("resolve node ids are not unique".to_owned());
        }
        let features = field(node, "features")
            .and_then(Value::as_array)
            .ok_or_else(|| "resolve node features are missing".to_owned())?;
        if !features.iter().all(Value::is_string) {
            return Err("resolve node feature is not a string".to_owned());
        }
    }
    let node_id_set = node_by_id.keys().cloned().collect::<BTreeSet<_>>();
    if node_id_set != package_id_set {
        return Err("resolve nodes do not equal selected packages".to_owned());
    }
    let allowed_kinds = ["normal", "build", "dev"]
        .into_iter()
        .collect::<BTreeSet<_>>();
    for (node_id, node) in &node_by_id {
        let dependencies = field(node, "dependencies")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{node_id} dependencies are missing"))?;
        let dependency_ids = dependencies
            .iter()
            .map(|dependency| {
                dependency
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| format!("{node_id} has a non-string dependency"))
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        if dependency_ids.len() != dependencies.len() || !dependency_ids.is_subset(&node_id_set) {
            return Err(format!("{node_id} has an unclosed dependency edge"));
        }
        let details = field(node, "deps")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{node_id} dependency details are missing"))?;
        let mut detail_ids = BTreeSet::new();
        for detail in details {
            let package_id = string_field(detail, "pkg")
                .ok_or_else(|| format!("{node_id} dependency package id is missing"))?;
            package_by_id
                .get(package_id)
                .ok_or_else(|| format!("{node_id} dependency target is outside graph"))?;
            if string_field(detail, "name")
                .filter(|name| !name.is_empty())
                .is_none()
            {
                return Err(format!("{node_id} dependency name is missing"));
            }
            if !detail_ids.insert(package_id.to_owned()) {
                return Err(format!("{node_id} dependency target is duplicated"));
            }
            let kinds = field(detail, "dep_kinds")
                .and_then(Value::as_array)
                .ok_or_else(|| format!("{node_id} dependency kinds are missing"))?;
            if kinds.is_empty() {
                return Err(format!("{node_id} dependency kinds are empty"));
            }
            for kind in kinds {
                let kind_name = match field(kind, "kind") {
                    None | Some(Value::Null) => "normal",
                    Some(Value::String(kind)) => kind.as_str(),
                    _ => return Err(format!("{node_id} dependency kind is malformed")),
                };
                if !allowed_kinds.contains(kind_name) {
                    return Err(format!("{node_id} dependency kind is not authorized"));
                }
                if let Some(target) = field(kind, "target") {
                    if !target.is_null() && !target.is_string() {
                        return Err(format!("{node_id} dependency target cfg is malformed"));
                    }
                }
            }
        }
        if dependency_ids != detail_ids {
            return Err(format!("{node_id} dependency edge views disagree"));
        }
    }
    let mut reachable = BTreeSet::new();
    let mut pending = vec![resolve_root];
    while let Some(id) = pending.pop() {
        if !reachable.insert(id.clone()) {
            continue;
        }
        let node = node_by_id
            .get(&id)
            .ok_or_else(|| "reachable node is outside graph".to_owned())?;
        pending.extend(
            field(node, "dependencies")
                .and_then(Value::as_array)
                .expect("dependencies checked above")
                .iter()
                .map(|dependency| {
                    dependency
                        .as_str()
                        .expect("string checked above")
                        .to_owned()
                }),
        );
    }
    if reachable != node_id_set {
        return Err("selected policy graph is not root-reachable".to_owned());
    }
    let census = string_field(document, "sourceCensusSha256")
        .ok_or_else(|| "source census digest is missing".to_owned())?;
    if !is_hex(census, 64) {
        return Err("source census digest is malformed".to_owned());
    }
    let lock_identities = parse_policy_lock(lock_text)?;
    if lock_identities != package_identities {
        return Err("policy lock identities do not equal metadata identities".to_owned());
    }
    Ok(identities)
}

fn validate_production_closure(
    document: &Value,
    lock_text: &str,
    policy_identities: &BTreeSet<PolicyIdentity>,
    context: SelectedPolicyContext,
) -> Result<(), String> {
    for (name, expected) in [
        ("system", context.system),
        ("target", context.target),
        ("package", context.package),
        ("root", context.package),
        ("variant", "production"),
        ("edgeKinds", "normal,build"),
    ] {
        if string_field(document, name) != Some(expected) {
            return Err(format!("{name} does not match production context"));
        }
    }
    if field(document, "defaultFeatures") != Some(&Value::Bool(false)) {
        return Err("production defaultFeatures is not false".to_owned());
    }
    let expected_features = context
        .features
        .iter()
        .map(|feature| (*feature).to_owned())
        .collect::<Vec<_>>();
    if string_array(document, "features")? != expected_features {
        return Err("production features do not match context".to_owned());
    }
    let identities = identity_set(document, "identities")?;
    if !identities.is_subset(policy_identities) {
        return Err("production closure is outside policy graph".to_owned());
    }
    let lock_identities = parse_policy_lock(lock_text)?;
    if lock_identities != identities {
        return Err("production lock identities do not equal closure".to_owned());
    }
    Ok(())
}

fn validate_selected_policy_context(context: SelectedPolicyContext) -> BTreeSet<PolicyIdentity> {
    let policy_root = policy_input_root(context, "policy");
    let production_root = policy_input_root(context, "production");
    assert!(Path::new(&policy_root).is_relative());
    assert!(repo_path_exists(&format!("{policy_root}/metadata.json")));
    let policy = read_json(&format!("{policy_root}/metadata.json"));
    let policy_lock = read_repo_file(&format!("{policy_root}/Cargo.lock"));
    let identities =
        validate_policy_metadata(&policy, &policy_lock, context).unwrap_or_else(|error| {
            panic!("{policy_root} structural policy validation failed: {error}")
        });
    let production = read_json(&format!("{production_root}/closure.json"));
    let production_lock = read_repo_file(&format!("{production_root}/Cargo.lock"));
    validate_production_closure(&production, &production_lock, &identities, context)
        .unwrap_or_else(|error| {
            panic!("{production_root} structural production validation failed: {error}")
        });
    identities
}

#[test]
fn selected_policy_artifacts_are_structural_and_root_authoritative() {
    let mut context_keys = BTreeSet::new();
    for context in SELECTED_POLICY_CONTEXTS {
        let key = format!(
            "{}/{}/{}/{}/{}",
            context.system, context.target, context.context, context.package, context.package
        );
        assert!(
            context_keys.insert(key),
            "selected policy context is duplicated"
        );
        validate_selected_policy_context(context);
    }
    assert_eq!(context_keys.len(), SELECTED_POLICY_CONTEXTS.len());
    let flake = read_repo_file("flake.nix");
    assert!(flake.contains("builtins.fromJSON"));
    assert!(flake.contains("builtins.fromTOML"));
    assert!(flake.contains("policyInputCorpusGate"));
    assert!(!flake.contains("grep -Fq '\"system\":"));
    assert!(!flake.contains("grep -Fq '\"target\":"));
}

#[test]
fn selected_policy_artifact_mutations_fail_closed() {
    let context = SELECTED_POLICY_CONTEXTS[1];
    let root = policy_input_root(context, "policy");
    let mut document = read_json(&format!("{root}/metadata.json"));
    let lock = read_repo_file(&format!("{root}/Cargo.lock"));
    let valid = |value: &Value, lock_text: &str| {
        validate_policy_metadata(value, lock_text, context).is_ok()
    };
    assert!(valid(&document, &lock));

    document["system"] = Value::String("aarch64-linux".to_owned());
    assert!(!valid(&document, &lock));
    document = read_json(&format!("{root}/metadata.json"));
    document["target"] = Value::String("x86_64-unknown-linux-gnu".to_owned());
    assert!(!valid(&document, &lock));
    document = read_json(&format!("{root}/metadata.json"));
    document["features"] = Value::Array(Vec::new());
    assert!(!valid(&document, &lock));
    document = read_json(&format!("{root}/metadata.json"));
    document["defaultFeatures"] = Value::Bool(true);
    assert!(!valid(&document, &lock));
    document = read_json(&format!("{root}/metadata.json"));
    document["variant"] = Value::String("production".to_owned());
    assert!(!valid(&document, &lock));
    document = read_json(&format!("{root}/metadata.json"));
    document["edgeKinds"] = Value::String("normal,build".to_owned());
    assert!(!valid(&document, &lock));

    document = read_json(&format!("{root}/metadata.json"));
    document["identities"] = Value::Array(Vec::new());
    assert!(!valid(&document, &lock));
    document = read_json(&format!("{root}/metadata.json"));
    document["packages"] = Value::Array(Vec::new());
    assert!(!valid(&document, &lock));
    document = read_json(&format!("{root}/metadata.json"));
    document["root"] = Value::String("d2b-priv-broker".to_owned());
    assert!(!valid(&document, &lock));

    let root_id = document
        .get("resolve")
        .and_then(|resolve| resolve.get("root"))
        .and_then(Value::as_str)
        .expect("root id");
    let root_node = document["resolve"]["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .position(|node| node["id"] == root_id)
        .expect("root node");
    document = read_json(&format!("{root}/metadata.json"));
    document["resolve"]["nodes"][root_node]["dependencies"] =
        Value::Array(vec![Value::String("missing-policy-edge".to_owned())]);
    assert!(!valid(&document, &lock));

    document = read_json(&format!("{root}/metadata.json"));
    document["resolve"]["nodes"][root_node]["dependencies"] = Value::Array(Vec::new());
    assert!(!valid(&document, &lock));

    document = read_json(&format!("{root}/metadata.json"));
    let first_detail = document["resolve"]["nodes"][root_node]["deps"]
        .as_array()
        .expect("root details")
        .first()
        .expect("root detail")
        .clone();
    let first_pkg = first_detail["pkg"].as_str().expect("detail package");
    let alternate = document["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .filter_map(|package| package["id"].as_str())
        .find(|id| *id != first_pkg)
        .expect("alternate package");
    document["resolve"]["nodes"][root_node]["deps"][0]["pkg"] = Value::String(alternate.to_owned());
    assert!(!valid(&document, &lock));

    document = read_json(&format!("{root}/metadata.json"));
    document["resolve"]["nodes"][root_node]["deps"][0]["dep_kinds"][0]["kind"] =
        Value::String("unauthorized".to_owned());
    assert!(!valid(&document, &lock));

    document = read_json(&format!("{root}/metadata.json"));
    document["resolve"]["nodes"][root_node]["deps"][0]["dep_kinds"] = Value::Array(Vec::new());
    assert!(!valid(&document, &lock));

    let mut mutated_lock = lock.clone();
    let marker = "version = \"";
    let offset = mutated_lock.find(marker).expect("lock version");
    let end = mutated_lock[offset + marker.len()..]
        .find('"')
        .expect("lock version end")
        + offset
        + marker.len();
    mutated_lock.replace_range(offset + marker.len()..end, "0.0.0-mutated");
    document = read_json(&format!("{root}/metadata.json"));
    assert!(!valid(&document, &mutated_lock));
}
